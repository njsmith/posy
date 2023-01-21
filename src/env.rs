use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use crate::kvstore::KVDirStore;
use crate::package_db::{ArtifactInfo, PackageDB, WheelBuilder};
use crate::resolve::{PinnedPackage, WheelResolveMetadata};
use crate::trampolines::{FindPython, ScriptPlatform, TrampolineMaker};
use crate::tree::WriteTreeFS;
use crate::{platform_tags::PybiPlatform, prelude::*, resolve::Blueprint};

// site.py as $stdlib/site.py
// imports sitecustomize, which can use site.addsitedir to add directories that will be
// processed for .pth files
//   probably put it in $purelib?
// can we disable user site stuff?
// looks like user site stuff is processed before we have a chance to run. it's even
// processed before regular global site-packages is, so even a .pth file won't be soon
// enough.
//   can disable it by patching site.py with s/^ENABLE_USER_SITE = None/ENABLE_USER_SITE = False/

// PEP 668:
// create EXTERNALLY-MANAGED in $stdlib, with useful error message
// [externally-managed]
// Error=...

// pybi: generic code just unpacks; EnvForest wants to do fixups
// wheels: generic code... takes paths dict, I guess?
//   oh plus needs a strategy for how executable wrappers find python
//     for EnvForest, probably $POSY_PYTHON; for export, path relative to executable
//     or I guess can use distlib's launchers, with either #!/usr/bin/env python.exe for
//     find-on-path, or #!./python.exe for relative path

pub struct EnvForest {
    store: KVDirStore,
}

fn pick_pinned_binary<'a, 'b, T: BinaryArtifact>(
    db: &'a PackageDB,
    platforms: &[&'b T::Platform],
    pin: &PinnedPackage,
) -> Result<(&'a ArtifactInfo, &'b T::Platform)>
where
    T::Name: BinaryName,
{
    for platform in platforms {
        let mut scored_candidates = db
            .artifacts_for_version(&pin.name, &pin.version)?
            .iter()
            .filter_map(|ai| {
                if let Some(name) = ai.name.inner_as::<T::Name>() {
                    if let Some(score) =
                        platform.max_compatibility(name.all_tags().iter())
                    {
                        return Some((ai, score));
                    }
                }
                None
            })
            .collect::<Vec<_>>();
        scored_candidates.sort_unstable_by_key(|(_, score)| *score);
        for (ai, _) in scored_candidates {
            if ai.hash.is_none() {
                warn!("best scoring artifact {} has no hash", ai.name);
            } else if !pin.hashes.contains(ai.hash.as_ref().unwrap()) {
                warn!("best scoring artifact {} does not appear in lock file (maybe need to update pins?)", ai.name);
            } else {
                return Ok((&ai, platform));
            }
        }
    }
    Err(PosyError::NoCompatibleBinaries {
        name: pin.name.as_given().to_owned(),
        version: pin.version.to_owned(),
    })?
}

impl EnvForest {
    pub fn new(base: &Path) -> Result<EnvForest> {
        Ok(EnvForest {
            store: KVDirStore::new(&base)?,
        })
    }

    fn munge_unpacked_pybi(path: &Path, metadata: &PybiCoreMetadata) -> Result<()> {
        let stdlib = path.join(metadata.path("stdlib")?.to_native());
        fs::write(
            &stdlib.join("EXTERNALLY-MANAGED"),
            include_bytes!("data-files/EXTERNALLY-MANAGED"),
        )?;
        let purelib = path.join(metadata.path("purelib")?.to_native());
        fs::write(
            &purelib.join("sitecustomize.py"),
            include_bytes!("data-files/sitecustomize.py"),
        )?;
        let site_py = fs::read(stdlib.join("site.py"))?;
        static USER_SITE_RE: Lazy<regex::bytes::Regex> = Lazy::new(|| {
            regex::bytes::Regex::new(r"(?m)^ENABLE_USER_SITE = None").unwrap()
        });
        let new_site_py =
            USER_SITE_RE.replace(&site_py, &b"ENABLE_USER_SITE = False"[..]);
        if let Cow::Borrowed(_) = new_site_py {
            bail!("pybi's site.py has unexpected structure; couldn't disable user site-packages");
        }
        fs::write(stdlib.join("site.py"), &new_site_py)?;
        Ok(())
    }

    // do we already have the

    pub fn get_env(
        &self,
        db: &PackageDB,
        blueprint: &Blueprint,
        pybi_platforms: &[&PybiPlatform],
        build_stack: &[&PackageName],
    ) -> Result<Env> {
        let (pybi_ai, pybi_platform) =
            pick_pinned_binary::<Pybi>(&db, &pybi_platforms, &blueprint.pybi)?;
        let pybi_hash = pybi_ai.require_hash()?;
        let pybi_root = self.store.get_or_set(&pybi_hash, |path| {
            let pybi = db.get_artifact::<Pybi>(pybi_ai)?;
            context!("Unpacking {}", pybi_ai.name);
            pybi.unpack(&mut WriteTreeFS::new(&path))?;
            let (_, pybi_metadata) = pybi.metadata()?;
            EnvForest::munge_unpacked_pybi(&path, &pybi_metadata)?;
            Ok(())
        })?;
        let pybi_metadata: PybiCoreMetadata =
            fs::read(pybi_root.join("pybi-info").join("METADATA"))?
                .as_slice()
                .try_into()?;
        let wheel_platform = pybi_platform.wheel_platform(&pybi_metadata)?;
        let pybi_platform_slice = [pybi_platform];
        let wheel_builder = WheelBuilder::new(
            &db,
            &pybi_metadata.name,
            &pybi_metadata.version,
            &pybi_platform_slice,
            &build_stack,
        )?;
        let trampoline_maker =
            TrampolineMaker::new(FindPython::FromEnv, ScriptPlatform::Both);

        let paths: HashMap<String, NicePathBuf> = HashMap::from([
            ("scripts".into(), "bin".try_into().unwrap()),
            ("purelib".into(), "lib".try_into().unwrap()),
            ("platlib".into(), "lib".try_into().unwrap()),
            ("data".into(), ".".try_into().unwrap()),
        ]);

        let mut wheel_roots = Vec::new();

        for (pin, expected_metadata) in &blueprint.wheels {
            context!("installing {} {}", pin.name.as_given(), pin.version);
            let (ai, wheel_root) =
                match pick_pinned_binary::<Wheel>(&db, &[&wheel_platform], &pin) {
                    Ok((wheel_ai, _)) => {
                        // we're using a binary wheel
                        context!("using binary wheel from {}", wheel_ai.url);
                        let wheel_hash = wheel_ai.require_hash()?;
                        let wheel_root =
                            self.store.get_or_set(&wheel_hash, |path| {
                                let wheel = db.get_artifact::<Wheel>(&wheel_ai)?;
                                wheel.unpack(
                                    &paths,
                                    &trampoline_maker,
                                    WriteTreeFS::new(&path),
                                )?;
                                Ok(())
                            })?;
                        (wheel_ai, wheel_root)
                    }
                    Err(err) => {
                        match err.downcast_ref::<PosyError>() {
                            Some(PosyError::NoCompatibleBinaries { .. }) => (),
                            _ => return Err(err),
                        };
                        // couldn't find a compatible wheel; see if we have an sdist
                        if let Some(sdist_ai) = db
                            .artifacts_for_version(&pin.name, &pin.version)?
                            .iter()
                            .find(|ai| ai.is::<Sdist>())
                        {
                            context!("using sdist from {}", sdist_ai.url);
                            let sdist_hash = sdist_ai.require_hash()?;
                            let handle = self.store.lock(&sdist_hash)?;
                            fs::create_dir_all(&handle)?;
                            // first check if we already have any unpacked wheels
                            // that we can use
                            let mut candidates = Vec::new();
                            for entry in fs::read_dir(&handle)? {
                                let entry = entry?;
                                let name = match entry.file_name().into_string() {
                                    Ok(name) => name,
                                    Err(_) => continue,
                                };
                                if !name.ends_with(".whl") {
                                    continue;
                                }
                                let wheel_name: WheelName = name.as_str().try_into()?;
                                if let Some(score) = wheel_platform
                                    .max_compatibility(wheel_name.all_tags())
                                {
                                    candidates.push((score, name));
                                }
                            }
                            if let Some((_, name)) =
                                candidates.iter().max_by_key(|(score, _)| score)
                            {
                                (sdist_ai, handle.join(name))
                            } else {
                                // couldn't find one already installed... try to
                                // build one and install it
                                // unwrap is ok b/c we know we're passing an sdist
                                // ai here
                                let local_wheel = db
                                    .get_locally_built_binary::<Wheel>(
                                        &sdist_ai,
                                        &wheel_builder,
                                        &wheel_platform,
                                    )
                                    .unwrap()?;
                                let tmp = handle.tempdir()?;
                                local_wheel.unpack(
                                    &paths,
                                    &trampoline_maker,
                                    WriteTreeFS::new(&tmp),
                                )?;
                                let wheel_root =
                                    handle.join(local_wheel.name().to_string());
                                fs::rename(tmp.into_path(), &wheel_root)?;
                                (sdist_ai, wheel_root)
                            }
                        } else {
                            bail!("no compatible wheel or sdist found");
                        }
                    }
                };

            // OK, we have an installed wheel. Find its metadata so we can confirm it's
            // consistent with what the blueprint was expecting.
            let mut top_levels = Vec::new();
            for entry in fs::read_dir(&wheel_root.join("lib"))? {
                let entry = entry?;
                if let Ok(name) = entry.file_name().into_string() {
                    top_levels.push(name);
                }
            }
            let dist_info = Wheel::find_special_wheel_dir(
                top_levels,
                &pin.name,
                &pin.version,
                ".dist-info",
            )?
            .ok_or(eyre!(".dist-info/ missing"))?;
            let found_metadata: WheelCoreMetadata =
                fs::read(Path::new(&dist_info).join("METADATA"))?
                    .as_slice()
                    .try_into()?;
            let found_metadata = WheelResolveMetadata::from(&ai, &found_metadata);

            if found_metadata.inner != expected_metadata.inner {
                bail!(
                    indoc::indoc! {"
                          Metadata mismatch!
                            When resolving, we used metadata from {}
                            Now we're trying to install {}
                          These should have had the same wheel metadata, but they don't!

                          Metadata from {}:
                          {}

                          Metadata from {}:
                          {}
                    "},
                    expected_metadata.provenance,
                    found_metadata.provenance,
                    expected_metadata.provenance,
                    serde_json::to_string_pretty(&expected_metadata.inner)?,
                    found_metadata.provenance,
                    serde_json::to_string_pretty(&found_metadata.inner)?,
                );
            }

            wheel_roots.push(wheel_root);
        }

        let pybi_bin = pybi_root.join(pybi_metadata.path("scripts")?.to_native());
        let (python_basename, pythonw_basename) = if cfg!(unix) {
            ("python", "python")
        } else {
            ("python.exe", "pythonw.exe")
        };
        let python = pybi_bin.join(python_basename);
        let pythonw = pybi_bin.join(pythonw_basename);

        let mut bin_dirs = Vec::<PathBuf>::new();
        bin_dirs.push(pybi_bin);
        bin_dirs.extend(wheel_roots.iter().map(|root| root.join("bin")));

        let lib_dirs = wheel_roots.iter().map(|root| root.join("lib")).collect();

        Ok(Env {
            platform_core_tag: pybi_platform.core_tag().into(),
            wheel_platform,
            python,
            pythonw,
            bin_dirs,
            lib_dirs,
        })
    }
}

pub struct Env {
    // XX TODO for GC support: hold a lock to prevent anything from being GC'ed out from
    // under us
    pub platform_core_tag: String,
    pub wheel_platform: WheelPlatform,
    pub python: PathBuf,
    pub pythonw: PathBuf,
    pub bin_dirs: Vec<PathBuf>,
    pub lib_dirs: Vec<PathBuf>,
}

impl Env {
    pub fn env_vars(
        &self,
    ) -> Result<impl IntoIterator<Item = (&'static str, std::ffi::OsString)>> {
        let mut vars = Vec::new();

        let old_path = std::env::var_os("PATH").ok_or(eyre!("no $PATH?"))?;
        let mut new_paths = self.bin_dirs.clone();
        new_paths.extend(std::env::split_paths(&old_path));
        let new_path = std::env::join_paths(&new_paths)?;

        vars.push(("PATH", new_path));
        vars.push(("POSY_PYTHON", self.python.clone().into_os_string()));
        vars.push(("POSY_PYTHONW", self.pythonw.clone().into_os_string()));
        vars.push((
            "POSY_PYTHON_PACKAGES",
            std::env::join_paths(&self.lib_dirs)?,
        ));

        Ok(vars)
    }
}

// pub trait PyEnvMaker {
//     fn make(&self, blueprint: &Blueprint) -> Result<PyEnv>;
// }

// pub struct PyEnv {
//     pub envvars: HashMap<String, String>,
//     // destructor? not necessary in short term, but in long term might want to track
//     // references so can do GC
// }

// pub struct ProjectWorkspace {}

// pub struct TempWorkspace {}

// // represents a _posy directory with persistent named environments
// // I guess needs some locking?
// impl ProjectWorkspace {
//     pub fn get_env(name: &str, blueprint: &Blueprint) -> PyEnv {
//         todo!()
//     }
// }

// // represents a temp collection of environments, maybe can do everything with env
// // manipulation + share copies of python/packages, including concurrently?
// impl TempWorkspace {
//     pub fn get_env(blueprint: &Blueprint) -> PyEnv {
//         todo!()
//     }
// }
