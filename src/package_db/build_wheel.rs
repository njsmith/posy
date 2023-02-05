use std::{ffi::OsString, fs, io, path::PathBuf};

use crate::{
    env::Env,
    kvstore::KVDirLock,
    package_db::PackageDB,
    prelude::*,
    resolve::{AllowPre, Blueprint, Brief},
    tree::WriteTreeFS,
};

use super::ArtifactInfo;

// Wheel build context lifecycle:
//
// Top-level call to Brief::resolve or Blueprint::make_env:
// - need to pass in stack of in_flight builds
//
// These call db.metadata/db.artifact, passing in the above + target python + target
// pybi platform
// - that's what you need to do the actual build.
// - for caching: don't need to worry about finding metadata in cache, b/c we have the
// metadata cache
// - but do want to be able to find an already-built wheel. for this want... to know the
// target *wheel* platform, I guess?

// for metadata: don't care about wheel tag; if we have anything cached that's good
// enough (or could skip checking the cache entirely, dunno if that's useful or not)
// if we get a wheel, need to add it to the cache properly, which requires knowing the
// build platform to set the proper name, and maybe some other metadata like the
// blueprint we used
// python version + target PybiPlatform useful as a hint to something that will most
// likely build the wheel, but not too important
// resolve() does have a specific pybi and platform to resolve against though; could
// pass in the ai, and that has the full name
//
// for wheel:
// - we have a specific desired WheelPlatform, and either we match it or we fail
// and we know which pybi generated the wheel platform; if we can build with that it's
// ideal; otherwise try to maximize chances of getting something abi-compatible

// maybe make WheelPlatform an arg to build_wheel? and have the db::get_wheel impl also
// take it, and be responsible for finding the appropriate wheel?

#[derive(Clone)]
pub struct WheelBuilder<'a> {
    db: &'a PackageDB<'a>,
    target_python: &'a PackageName,
    target_python_version: &'a Version,
    build_platforms: Vec<&'a PybiPlatform>,
    build_stack: Vec<&'a PackageName>,
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
        wheel: Wheel,
    },
}

impl<'a> WheelBuilder<'a> {
    pub fn new(
        db: &'a PackageDB,
        target_python: &'a PackageName,
        target_python_version: &'a Version,
        target_platforms: &'a [&'a PybiPlatform],
        build_stack: &'a [&'a PackageName],
    ) -> Result<WheelBuilder<'a>> {
        let mut build_platforms = Vec::new();
        for p in target_platforms {
            if p.is_native()? {
                build_platforms.push(*p);
            }
        }
        if build_platforms.is_empty() {
            build_platforms.extend(PybiPlatform::native_platforms()?)
        }
        Ok(WheelBuilder {
            db,
            target_python,
            target_python_version,
            build_platforms,
            build_stack: build_stack.into(),
        })
    }

    fn new_build_stack(
        &'a self,
        package: &'a PackageName,
    ) -> Result<Vec<&'a PackageName>> {
        if let Some(idx) = self.build_stack.iter().position(|p| p == &package) {
            let bad = self.build_stack[idx..]
                .iter()
                .map(|p| format!("{} -> ", p.as_given()))
                .collect::<String>();
            bail!("build dependency loop: {bad}{}", package.as_given());
        }
        let mut new_build_stack = self.build_stack.clone();
        new_build_stack.push(package);
        Ok(new_build_stack)
    }

    pub fn locally_built_wheel(
        &self,
        sdist_ai: &ArtifactInfo,
        wheel_platform: &WheelPlatform,
    ) -> Result<Wheel> {
        trace!("Building wheel from source for {} {}", sdist_ai.name.distribution().as_given(), sdist_ai.name.version());
        let new_build_stack = self.new_build_stack(sdist_ai.name.distribution())?;

        // check if we already have a usable wheel cached; and if so, find the best one
        let handle = self.db.wheel_cache.lock(sdist_ai.require_hash()?)?;
        fs::create_dir_all(&handle)?;

        let mut best: Option<(i32, OsString, WheelName)> = None;

        for entry in fs::read_dir(&handle)? {
            let entry = entry?;
            let os_name = entry.file_name();
            let str_name = os_name.to_str().ok_or_else(|| {
                eyre!(
                    "invalid unicode in wheel cache entry name {}",
                    os_name.to_string_lossy()
                )
            })?;
            if !str_name.ends_with(".whl") {
                continue;
            }
            let name: WheelName = str_name.parse()?;
            let maybe_score = wheel_platform.max_compatibility(name.all_tags());
            if let Some(score) = maybe_score {
                if best.is_none() || best.as_ref().unwrap().0 < score {
                    best = Some((score, os_name, name))
                }
            }
        }

        if let Some((_, os_name, name)) = best {
            let path = handle.join(os_name);
            return Ok(Wheel::new(name, Box::new(fs::File::open(path)?))?);
        }

        // nothing in cache -- we'll have to build it ourselves (which will implicitly
        // add to the cache)
        match self.pep517(
            sdist_ai,
            Pep517Goal::Wheel,
            Some(handle),
            &new_build_stack,
        )? {
            Pep517Succeeded::Wheel { wheel } => {
                if wheel_platform
                    .max_compatibility(wheel.name().all_tags())
                    .is_some()
                {
                    Ok(wheel)
                } else {
                    bail!("built wheel is not compatible with target environment");
                }
            }
            _ => unreachable!(),
        }
    }

    pub fn locally_built_metadata(
        &self,
        sdist_ai: &ArtifactInfo,
    ) -> Result<(Vec<u8>, WheelCoreMetadata)> {
        trace!("Getting metadata from source for {} {}", sdist_ai.name.distribution().as_given(), sdist_ai.name.version());
        let new_build_stack = self.new_build_stack(sdist_ai.name.distribution())?;

        match self.pep517(
            sdist_ai,
            Pep517Goal::WheelMetadata,
            None,
            &new_build_stack,
        )? {
            Pep517Succeeded::WheelMetadata {
                handle: _handle,
                dist_info,
            } => {
                let metadata_buf = fs::read(dist_info.join("METADATA"))?;
                let metadata = metadata_buf.as_slice().try_into()?;
                Ok((metadata_buf, metadata))
            }
            Pep517Succeeded::Wheel { wheel } => Ok(wheel.metadata()?),
        }
    }

    fn get_env_for_build(
        &self,
        reqs: &[UserRequirement],
        like: Option<&Blueprint>,
        new_build_stack: &[&PackageName],
    ) -> Result<(Blueprint, Env)> {
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
            .resolve(
                self.db,
                &self.build_platforms,
                like,
                new_build_stack,
            )?;
            let env = self.db.build_forest.get_env(
                self.db,
                &blueprint,
                &self.build_platforms,
                new_build_stack,
            )?;
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
            let result =
                brief.resolve(self.db, &self.build_platforms, None, new_build_stack);
            match result {
                Ok(blueprint) => {
                    found_python = Some((brief.python, blueprint));
                    break;
                }
                Err(err) => match err.downcast_ref::<PosyError>() {
                    Some(PosyError::NoPybiFound) => continue,
                    _ => return Err(err),
                },
            }
        }

        let (pyreq, pybi_like) = found_python.ok_or(eyre!(
            "couldn't find any pybis similar to {} {} to build wheels with",
            self.target_python.as_given(),
            self.target_python_version,
        ))?;

        let brief = Brief {
            python: pyreq,
            requirements: reqs.into(),
            allow_pre: Default::default(),
        };
        let blueprint = brief.resolve(
            self.db,
            &self.build_platforms,
            Some(&pybi_like),
            new_build_stack,
        )?;
        let env = self.db.build_forest.get_env(
            self.db,
            &blueprint,
            &self.build_platforms,
            new_build_stack,
        )?;
        Ok((blueprint, env))
    }

    fn pep517(
        &self,
        sdist_ai: &ArtifactInfo,
        goal: Pep517Goal,
        wheel_cache_handle: Option<KVDirLock>,
        new_build_stack: &[&PackageName],
    ) -> Result<Pep517Succeeded> {
        let sdist_hash = sdist_ai.require_hash()?;
        let handle = self.db.build_store.lock(&sdist_hash)?;

        if !handle.exists() {
            let tempdir = handle.tempdir()?;
            let sdist = self.db.get_artifact::<Sdist>(sdist_ai)?;
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
                // Get the name the build backend returned
                let name =
                    String::from_utf8(fs::read(handle.join("build_wheel.out"))?)?;
                let mut wheel_name: WheelName = name.parse()?;
                let wheel_path = build_wheel.join(&name);
                // Get the most-restrictive wheel tag compatible with the build
                // environment.
                let build_env_tag = String::from_utf8(fs::read(
                    handle.join("build_wheel.binary_wheel_tag"),
                )?)?;
                // If this is a binary wheel, then tag it with the platform we built on
                // (so e.g. "linux_x86_64" might become "manylinux_2_32_x86_64")
                let (_, build_arch) = build_env_tag.rsplit_once('-').unwrap();
                if !wheel_name.arch_tags.iter().all(|t| t == "any") {
                    wheel_name.arch_tags = vec![build_arch.into()]
                }
                // Store the wheel in the wheel cache
                let wheel_cache_handle = match wheel_cache_handle {
                    Some(h) => h,
                    None => self.db.wheel_cache.lock(&sdist_hash)?,
                };
                fs::create_dir_all(&wheel_cache_handle)?;
                let target_path = wheel_cache_handle.join(wheel_name.to_string());
                if fs::rename(&wheel_path, &target_path).is_err() {
                    fs::copy(&wheel_path, &target_path)?;
                }
                let opened = fs::File::open(target_path)?;
                let wheel = Wheel::new(wheel_name, Box::new(opened))?;
                return Ok(Pep517Succeeded::Wheel { wheel });
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
            // Otherwise, we're not done. Turn the crank again.
            self.pep517_step(&handle, goal, new_build_stack)?;
        }
    }

    fn pep517_step(
        &self,
        handle: &KVDirLock,
        goal: Pep517Goal,
        new_build_stack: &[&PackageName],
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
                PyprojectBuildSystemStanza::parse_from(&pyproject_str)?
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

        let (blueprint, env) = self.get_env_for_build(
            &build_requires,
            saved_blueprint.as_ref(),
            new_build_stack,
        )?;

        let binary_wheel_tag = env
            .wheel_platform
            .tags()
            .next()
            .ok_or_else(|| eyre!("no wheel tags?"))?;

        serde_json::to_writer(fs::File::create(&saved_blueprint_path)?, &blueprint)?;

        let mut child = std::process::Command::new("python")
            .args([
                handle.join("build-frontend.py").as_os_str(),
                handle.as_os_str(),
                OsString::from(format!("{:?}", goal)).as_ref(),
                OsString::from(binary_wheel_tag).as_ref(),
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
}

/// Used to parse the `[build-system]` table in pyproject.toml.
#[derive(Deserialize, Debug, Serialize)]
#[serde(rename_all = "kebab-case", default)]
struct PyprojectBuildSystemStanza {
    requires: Vec<String>,
    build_backend: String,
    backend_path: Vec<String>,
}

impl Default for PyprojectBuildSystemStanza {
    fn default() -> Self {
        Self {
            requires: vec!["setuptools".into(), "wheel".into()],
            build_backend: "setuptools.build_meta:__legacy__".into(),
            backend_path: Vec::new(),
        }
    }
}

impl PyprojectBuildSystemStanza {
    fn parse_from(s: &str) -> Result<PyprojectBuildSystemStanza> {
        let mut d = s.parse::<toml_edit::Document>()?;
        if let Some(table) = d.remove("build-system") {
            Ok(toml_edit::de::from_item(table)?)
        } else {
            Ok(Default::default())
        }
    }
}
