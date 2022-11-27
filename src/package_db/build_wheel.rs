use std::{ffi::OsString, fs, io, path::PathBuf};

use crate::{
    env::{Env, EnvForest},
    kvstore::{KVDirLock, KVDirStore},
    package_db::PackageDB,
    prelude::*,
    resolve::{AllowPre, Blueprint, Brief, NoPybiFound},
    tree::WriteTreeFS,
};

use super::ArtifactInfo;

#[derive(Clone)]
pub struct BuildWheelContext<'a> {
    env_forest: &'a EnvForest,
    build_store: &'a KVDirStore,
    target_python: &'a PackageName,
    target_python_version: &'a Version,
    target_platform: &'a PybiPlatform,
    // XX TODO
    //build_constraints: Vec<UserRequirement>,
    //allow_pre?
    in_flight: Vec<&'a PackageName>,
}

#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "kebab-case", default)]
struct PyprojectBuildSystem {
    requires: Vec<String>,
    build_backend: String,
    backend_path: Vec<String>,
}

impl Default for PyprojectBuildSystem {
    fn default() -> Self {
        Self {
            requires: vec!["setuptools".into(), "wheel".into()],
            build_backend: "setuptools.build_meta:__legacy__".into(),
            backend_path: Vec::new(),
        }
    }
}

impl PyprojectBuildSystem {
    fn parse_from(s: &str) -> Result<PyprojectBuildSystem> {
        let mut d = s.parse::<toml_edit::Document>()?;
        if let Some(table) = d.remove("build-system") {
            Ok(toml_edit::de::from_item(table)?)
        } else {
            Ok(Default::default())
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pep517Goal {
    WheelMetadata,
    Wheel,
}

enum Pep517Succeeded {
    WheelMetadata {
        handle: KVDirLock,
        dist_info: PathBuf,
    },
    Wheel {
        handle: KVDirLock,
        wheel: PathBuf,
    },
}

impl<'a> BuildWheelContext<'a> {
    pub fn new(
        env_forest: &'a EnvForest,
        build_store: &'a KVDirStore,
        target_python: &'a PackageName,
        target_python_version: &'a Version,
        target_platform: &'a PybiPlatform,
    ) -> BuildWheelContext<'a> {
        BuildWheelContext {
            env_forest,
            build_store,
            target_python,
            target_python_version,
            target_platform,
            in_flight: Vec::new(),
        }
    }

    pub fn child_context<'name, 'result>(
        &'a self,
        package: &'name PackageName,
    ) -> Result<BuildWheelContext<'result>>
    where
        'a: 'result,
        'name: 'result,
    {
        if let Some(idx) = self.in_flight.iter().position(|p| p == &package) {
            let bad = self.in_flight[idx..]
                .iter()
                .map(|p| format!("{} -> ", p.as_given()))
                .collect::<String>();
            bail!("build dependency loop: {bad}{}", package.as_given());
        }
        let mut child = self.clone();
        child.in_flight.push(&package);
        Ok(child)
    }

    fn get_env_for_build(
        &self,
        db: &PackageDB,
        reqs: &[UserRequirement],
        like: Option<&Blueprint>,
    ) -> Result<(Blueprint, Env)> {
        let build_platform = if self.target_platform.is_native()? {
            self.target_platform
        } else {
            PybiPlatform::current_platform()?
        };

        // if we've already resolved a version of this environment, then we can skip
        // over the tricky stuff and just re-use the pybi + any matching wheels
        if like.is_some() {
            let blueprint = Brief {
                python: PythonRequirement::try_from(Requirement {
                    name: self.target_python.clone(),
                    extras: Default::default(),
                    specifiers: Default::default(),
                    env_marker_expr: Default::default(),
                })
                .unwrap(),
                requirements: reqs.into(),
                allow_pre: Default::default(),
            }
            .resolve(&db, &build_platform, like)?;
            let env = self.env_forest.get_env(&db, &blueprint, &build_platform)?;
            return Ok((blueprint, env));
        }

        let pieces = self.target_python_version.0.release.len();
        let same_minor = pep440::Version {
            epoch: self.target_python_version.0.epoch,
            release: self.target_python_version.0.release[..std::cmp::min(2, pieces)]
                .into(),
            pre: None,
            post: None,
            dev: None,
            local: Vec::new(),
        };

        let candidate_pyreqs = [
            // Ideally, we can find a Python that's an exact match to the target python.
            PythonRequirement::try_from(Requirement {
                name: self.target_python.clone(),
                specifiers: Specifiers(vec![Specifier {
                    op: CompareOp::Equal,
                    value: self.target_python_version.to_string(),
                }]),
                extras: Default::default(),
                env_marker_expr: Default::default(),
            })
            .unwrap(),
            // If not, if target is 3.10.x, try to find some 3.10.* version (b/c for
            // CPython at least, the C ABI is stable within a minor release).
            PythonRequirement::try_from(Requirement {
                name: self.target_python.clone(),
                specifiers: Specifiers(vec![Specifier {
                    op: CompareOp::Equal,
                    value: format!("{}.*", same_minor),
                }]),
                extras: Default::default(),
                env_marker_expr: Default::default(),
            })
            .unwrap(),
            // And if even that doesn't work, just pick the newest available python and
            // hope for the best.
            PythonRequirement::try_from(Requirement {
                name: self.target_python.clone(),
                extras: Default::default(),
                specifiers: Default::default(),
                env_marker_expr: Default::default(),
            })
            .unwrap(),
        ];

        let mut found_python = None;

        for candidate in candidate_pyreqs {
            let allow_pre = if self.target_python_version.is_prerelease() {
                AllowPre::Some([self.target_python.clone()].into())
            } else {
                Default::default()
            };
            let brief = Brief {
                python: candidate,
                requirements: Vec::new(),
                allow_pre,
            };
            let result = brief.resolve(&db, &build_platform, None);
            match result {
                Ok(blueprint) => {
                    found_python = Some((brief.python, blueprint));
                    break;
                }
                Err(err) => {
                    if err.downcast_ref::<NoPybiFound>().is_some() {
                        continue;
                    } else {
                        return Err(err);
                    }
                }
            }
        }

        let (pyreq, pybi_like) = found_python.ok_or(eyre!(
            "couldn't find any pybis similar to {} {} to build wheels with",
            self.target_python.as_given(),
            self.target_python_version
        ))?;

        let brief = Brief {
            python: pyreq,
            requirements: reqs.into(),
            allow_pre: Default::default(),
        };
        let blueprint = brief.resolve(&db, &build_platform, Some(&pybi_like))?;
        let env = self.env_forest.get_env(&db, &blueprint, &build_platform)?;
        Ok((blueprint, env))
    }

    fn pep517(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
        goal: Pep517Goal,
    ) -> Result<Pep517Succeeded> {
        let handle = self
            .build_store
            .lock(ai.hash.as_ref().ok_or(eyre!("missing hash"))?)?;

        if !handle.exists() {
            let tempdir = handle.tempdir()?;
            let sdist = db.get_artifact::<Sdist>(&ai)?;
            let unpack_path = tempdir.path().join("sdist");
            sdist.unpack(&mut WriteTreeFS::new(&unpack_path))?;
            const BUILD_FRONTEND_PY: &[u8] =
                include_bytes!("data-files/build-frontend.py");
            fs::write(tempdir.path().join("build-frontend.py"), BUILD_FRONTEND_PY)?;
            fs::rename(&tempdir.into_path(), &*handle)?;
        }

        let build_wheel = handle.join("build_wheel");
        let prepare_metadata_for_build_wheel =
            handle.join("prepare_metadata_for_build_wheel");
        loop {
            // If we have a wheel, we're definitely done, no matter what our goal was
            if build_wheel.exists() {
                let name =
                    String::from_utf8(fs::read(handle.join("build_wheel.out"))?)?;
                return Ok(Pep517Succeeded::Wheel {
                    handle,
                    wheel: build_wheel.join(name),
                });
            }
            // Or if our goal is metadata and we have it, we're done
            if goal == Pep517Goal::WheelMetadata
                && prepare_metadata_for_build_wheel.exists()
            {
                let name = String::from_utf8(fs::read(
                    handle.join("prepare_metadata_for_build_wheel.out"),
                )?)?;
                return Ok(Pep517Succeeded::WheelMetadata {
                    handle,
                    dist_info: prepare_metadata_for_build_wheel.join(name),
                });
            }
            // OK, we're not done. Turn the crank again.
            self.pep517_step(&db, &handle, goal)?;
        }
    }

    fn pep517_step(
        &self,
        db: &PackageDB,
        handle: &KVDirLock,
        goal: Pep517Goal,
    ) -> Result<()> {
        let mut sdist_entries = fs::read_dir(&handle.join("sdist"))?
            .collect::<Result<Vec<_>, io::Error>>()?;
        if sdist_entries.len() != 1 {
            bail!("expected sdist to contain exactly one top-level directory");
        }
        let sdist_root = sdist_entries.pop().unwrap().path();

        let build_system = match fs::read(sdist_root.join("pyproject.toml")) {
            Ok(pyproject_bytes) => {
                context!("parsing pyproject.toml");
                let pyproject_str = String::from_utf8(pyproject_bytes)?;
                PyprojectBuildSystem::parse_from(&pyproject_str)?
            }
            Err(ref e) if e.kind() == io::ErrorKind::NotFound => Default::default(),
            Err(e) => Err(e)?,
        };
        let build_system_path = handle.join("build-system.json");
        serde_json::to_writer(fs::File::create(&build_system_path)?, &build_system)?;

        let get_requires_for_build_wheel = handle.join("get_requires_for_build_wheel");
        let dynamic_requires: Vec<String> =
            match fs::File::open(get_requires_for_build_wheel) {
                Ok(f) => serde_json::from_reader(f)?,
                Err(ref e) if e.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(e) => Err(e)?,
            };

        let saved_blueprint_path = handle.join("saved-blueprint.json");
        let saved_blueprint: Option<Blueprint> = fs::File::open(&saved_blueprint_path)
            .ok()
            .and_then(|f| serde_json::from_reader(f).ok());

        let mut build_requires = build_system.requires;
        build_requires.extend(dynamic_requires);
        let build_requires = build_requires
            .into_iter()
            .map(|s| s.parse())
            .collect::<Result<Vec<_>>>()?;

        let (blueprint, env) =
            self.get_env_for_build(&db, &build_requires, saved_blueprint.as_ref())?;

        serde_json::to_writer(fs::File::create(&saved_blueprint_path)?, &blueprint)?;

        let mut child = std::process::Command::new("python")
            .args([
                handle.join("build-frontend.py").as_os_str(),
                handle.as_os_str(),
                OsString::from(format!("{:?}", goal)).as_ref(),
            ])
            .stdin(std::process::Stdio::null())
            .current_dir(&sdist_root)
            .envs(env.env_vars()?)
            .spawn()?;

        let status = child.wait()?;
        if !status.success() {
            bail!("Build failed (exit status: {status})");
        }

        Ok(())
    }

    pub fn build_metadata(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
    ) -> Result<Option<WheelCoreMetadata>> {
        let name = match ai.name.inner_as::<SdistName>() {
            Some(value) => value,
            None => return Ok(None),
        };

        let context = self.child_context(&name.distribution)?;
        match context.pep517(&db, ai, Pep517Goal::WheelMetadata)? {
            Pep517Succeeded::WheelMetadata {
                handle: _handle,
                dist_info,
            } => Ok(Some(
                fs::read(dist_info.join("METADATA"))?
                    .as_slice()
                    .try_into()?,
            )),
            Pep517Succeeded::Wheel {
                handle: _handle,
                wheel,
            } => {
                let f = fs::File::open(&wheel)?;
                let name = wheel
                    .file_name()
                    .ok_or(eyre!("no wheel filename?"))?
                    .to_str()
                    .ok_or(eyre!("wheel name is invalid utf-8?"))?;
                let wheel = Wheel::new(name.try_into()?, Box::new(f))?;
                // XX TODO: stash in cache
                // alongside platform + blueprint
                // lookup: for metadata, no constraints
                //
                // for wheels: build-constraints are the only reason need blueprint, and
                // that's fine whatever
                // for platform: I guess really only want to use these if we are
                // building for a native platform? and then only need to distinguish
                // between which native platform it was built for.
                // (and only if the wheel has native code)
                // (maybe compute an "effective" name for wheel, e.g. linux->manylinux?)
                //
                // realistically, for a given cache, the current platform is not going
                // to change much, and when it does it will change by adding new
                // versions to the supported list, not removing them. and in the rare
                // exceptions it's fine to rebuild.
                //
                // ...but really we want a unique name for the wheel so we have a single
                // place to put it in the EnvForest.
                // So maybe: two places to look. platform-independent wheels go under
                // sdist hash + "any" or similar, and non-platform-independent wheels go
                // under
                //  sdist hash
                //  + compat-group from expand.rs
                //  + highest tag from pybi (e.g. cp310-cp310-PLATFORM)
                //  (+ eventually, any build config like constraints/allow-pre)
                Ok(Some(wheel.metadata()?.1))
            }
        }
    }

    pub fn build_wheel(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
    ) -> Result<Option<Wheel>> {
        todo!();
    }
}
