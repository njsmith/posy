use std::borrow::Cow;
use std::fs;
use std::path::{Path, PathBuf};

use crate::kvstore::KVDirStore;
use crate::package_db::{ArtifactInfo, PackageDB};
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

fn pick_pinned<'a, 'b, T: BinaryArtifact>(
    db: &'a PackageDB,
    platforms: &[&'b T::Platform],
    pin: &PinnedPackage,
) -> Result<(T, &'a ArtifactInfo, &'b T::Platform)>
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
                return Ok((db.get_artifact(ai)?, &ai, platform));
            }
        }
    }
    bail!(
        "no compatible artifacts found for {} {}",
        pin.name.as_given(),
        pin.version
    );
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

    pub fn get_env(
        &self,
        db: &PackageDB,
        blueprint: &Blueprint,
        pybi_platforms: &[&PybiPlatform],
    ) -> Result<Env> {
        let (pybi, pybi_ai, pybi_platform) =
            pick_pinned::<Pybi>(&db, &pybi_platforms, &blueprint.pybi)?;
        let pybi_hash = pybi_ai.hash.as_ref().ok_or(eyre!("pybi missing hash"))?;
        let (_, pybi_metadata) = pybi.metadata()?;
        let pybi_root = self.store.get_or_set(&pybi_hash, |path| {
            context!("Unpacking {}", pybi.name());
            pybi.unpack(&mut WriteTreeFS::new(&path))?;
            EnvForest::munge_unpacked_pybi(&path, &pybi_metadata)?;
            Ok(())
        })?;
        let wheel_platform = pybi_platform.wheel_platform(&pybi_metadata)?;
        let trampoline_maker =
            TrampolineMaker::new(FindPython::FromEnv, ScriptPlatform::Both);

        let paths: HashMap<String, NicePathBuf> = HashMap::from([
            ("scripts".into(), "bin".try_into().unwrap()),
            ("purelib".into(), "lib".try_into().unwrap()),
            ("platlib".into(), "lib".try_into().unwrap()),
            ("data".into(), ".".try_into().unwrap()),
        ]);

        let wheel_roots = blueprint
            .wheels
            .iter()
            .map(|(pin, expected_metadata)| {
                let (wheel, wheel_ai, _) =
                    pick_pinned::<Wheel>(&db, &[&wheel_platform], &pin)?;
                let wheel_hash =
                    wheel_ai.hash.as_ref().ok_or(eyre!("wheel missing hash"))?;
                let (_, got_metadata) = wheel.metadata()?;
                let got_metadata = WheelResolveMetadata::from(&wheel_ai, &got_metadata);
                if got_metadata.inner != expected_metadata.inner {
                    bail!(
                        indoc::indoc! {"
                          When installing {} v{}:
                            When resolving, we used metadata from {}
                            Now we're trying to install {}
                          These wheels should have had the same metadata, but they don't!

                          Metadata from {}:
                          {}

                          Metadata from {}:
                          {}
                    "},
                        pin.name.as_given(),
                        pin.version,
                        expected_metadata.provenance,
                        got_metadata.provenance,
                        expected_metadata.provenance,
                        serde_json::to_string_pretty(&expected_metadata.inner)?,
                        got_metadata.provenance,
                        serde_json::to_string_pretty(&got_metadata.inner)?,
                    );
                }
                let wheel_root = self.store.get_or_set(&wheel_hash, |path| {
                    wheel.unpack(&paths, &trampoline_maker, WriteTreeFS::new(&path))?;
                    Ok(())
                })?;
                Ok(wheel_root)
            })
            .collect::<Result<Vec<_>>>()?;

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
