use std::path::{PathBuf, Path};

use crate::kvstore::KVDirStore;
use crate::package_db::{ArtifactInfo, PackageDB};
use crate::{brief::Blueprint, platform_tags::Platform, prelude::*};

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

pub struct EnvForest {
    store: KVDirStore,
}

impl EnvForest {
    fn ensure_unpacked<T>(&self, ai: &ArtifactInfo, artifact: &T) -> Result<impl AsRef<Path>>
    where
        T: BinaryArtifact,
    {
        let hash = ai.hash.as_ref().ok_or(anyhow!("no hash"))?;
        Ok(self.store.get_or_set(&hash, |path| Ok(artifact.unpack(&path)?))?)
    }

    pub fn get_env(
        &self,
        db: &PackageDB,
        blueprint: &Blueprint,
        platform: &Platform,
    ) -> Result<Env> {
        let pybi_ai = db
            .artifacts_for_release(&blueprint.pybi.name, &blueprint.pybi.version)?
            .iter()
            .filter_map(|ai| {
                if let Some(name) = ai.name.inner_as::<PybiName>() {
                    if let Some(score) =
                        platform.max_compatibility(name.arch_tags.iter())
                    {
                        return Some((ai, score));
                    }
                }
                None
            })
            .max_by_key(|(_, score)| *score)
            .map(|(ai, _)| ai)
            .ok_or(anyhow!("no compatible pybis found"))?;
        let pybi_hash = pybi_ai.hash.as_ref().ok_or(anyhow!("pybi has no hash"))?;
        if !blueprint.pybi.hashes.contains(&pybi_hash) {
            // XX TODO maybe should filter it out during the selection stage instead?
            // or even better, give error messages saying what's happening (warning if
            // hashes rule out the best artifact, error if they rule out all artifacts,
            // different error if there aren't any artifacts to start with, etc.)
            bail!("pybi hash is not in list of pinned hashes");
        }
        let pybi = db.get_artifact::<Pybi>(pybi_ai)?;
        let (_, pybi_metadata) = pybi.metadata()?;
        let pybi_root = self.ensure_unpacked(pybi_ai, &pybi)?;

        // XX TODO: pass a fixup closure to ensure_unpacked?
        // need to fixup site.py and sitecustomize.py and EXTERNALLY-MANAGED

        // then pick wheels and unpack them
        // their fixups:
        // - put .data/{pure,plat}lib somewhere that the env stuff later can find it
        //   (maybe lay it out as $dir/bin, $dir/purelib, $dir/platlib?)
        // - fix up #!python/#!pythonw scripts
        // - create wrappers for script entrypoints
        //
        // maybe Wheel::unpack should take a dict of paths? in forest mode we can fill
        // them in with our ad hoc stuff; in venv mode can pass the pybi paths.
        // and maybe ditto for the forwarding, b/c there again venv mode will want to
        // use a relative-to-script lookup and forest mode will want to use a PATH
        // lookup. (Or maybe that's just an enum.)

        todo!()
    }
}

pub struct Env {
    bin_dirs: Vec<PathBuf>,
    wheel_roots: Vec<PathBuf>,
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
