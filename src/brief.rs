use crate::{
    package_db::{ArtifactInfo, PackageDB},
    platform_tags::Platform,
    prelude::*,
};
// blueprint:
// - exactly one pinned pybi
// - list of marker predicates + value that were relied on to generate this, so can
//   generalize to other pythons (not used for now, could defer?)
// - exact list of package+version(+url for @ dependencies?)
// - hashes

/// A high-level description of an environment that a user would like to be able to
/// build. Doesn't necessarily have to be what the user types in exactly, but has to
/// represent their intentions, and *not* anything that requires looking at a package
/// index.
#[derive(Debug, Clone)]
pub struct Brief {
    pub python: PythonRequirement,
    // don't need python_constraints because we always install exactly one python
    pub requires: Vec<UserRequirement>,
    // XX TODO
    //pub constraints: Vec<UserRequirement>,
    // for now let's make this totally explicit: we allow prereleases iff the package is
    // mentioned here (could be python package or a regular package)
    // and we'll see how far we get with that + diagnostic hints for when the user
    // should add a package here to make things work.
    // XX TODO
    //pub allow_pre: HashSet<PackageName>,
    //pub allow_pre_all: bool,
}

#[derive(Debug, Clone)]
pub struct PinnedArtifact {
    pub url: Url,
    pub hashes: Vec<ArtifactHash>,
}

#[derive(Debug, Clone)]
pub struct PinnedPackage {
    pub name: PackageName,
    pub version: Version,
    pub hashes: Vec<ArtifactHash>,
    // TODO: expected metadata + its provenance, to catch cases where wheels
    // have mismatched requirements. (just Python-Requires and Requires-Dist?)
}

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub python: PinnedPackage,
    pub packages: Vec<PinnedPackage>,
    // XX TODO
    //pub used_markers: HashMap<marker::Expr, bool>,
    // XX TODO: metadata + provenance relied on
}

fn pick_best_pybi<'a>(
    artifact_infos: &'a Vec<ArtifactInfo>,
    platform: &Platform,
) -> Option<&'a ArtifactInfo> {
    artifact_infos
        .iter()
        .filter_map(|ai| {
            if let ArtifactName::Pybi(name) = &ai.name {
                // if a pybi has multiple platform tags, score it as whichever tag gets
                // the highest score (if any)
                let best_score = name
                    .arch_tags
                    .iter()
                    .filter_map(|ai| platform.compatibility(ai))
                    .max();
                // then re-attach the ai associated with this score
                best_score.map(|score| (ai, score))
            } else {
                None
            }
        })
        .max_by_key(|(_, score)| *score)
        .map(|(ai, _)| ai)
}

fn resolve_pybi<'a>(
    db: &'a PackageDB,
    req: &PythonRequirement,
    platform: &Platform,
) -> Result<&'a ArtifactInfo> {
    let available = db.available_artifacts(&req.name)?;
    for (version, artifact_infos) in available.iter() {
        if req.specifiers.satisfied_by(&version)? {
            if let Some(ai) = pick_best_pybi(&artifact_infos, platform) {
                return Ok(ai);
            }
        }
    }
    bail!("no compatible pybis found for requirement and platform");
}

impl Brief {
    pub fn resolve(&self, db: &PackageDB, platform: &Platform) -> Result<Blueprint> {
        let pybi_ai = resolve_pybi(&db, &self.python, &platform)?;
        // XX TODO: figure out how platform changes after pybi is selected (e.g. on a
        // system that has both manylinux+musllinux compatibility, we can pick a pybi
        // for either but once we do we have fewer choices for wheels).
        // maybe we need some way to trim down a platform list to "can all coexist in
        // the same process"? discard everything that's inconsistent with a
        // higher-ranked tag?
        // (and same machinery should be able to figure out platform_machine for
        // markers, maybe?)
        // ...for the resolution phase tho, we don't need to know about wheel tags at
        // all. We assume all wheels for a given package+version have the same metadata.
        // We just need to know the environment marker values (so only the universal2
        // case is actually problematic). And for the pybi part of the pin, I guess we
        // pick an arbitrary pybi that satisfies the version+platform constraints, and
        // write down that version + the environment markers we needed for the pins?
        // (Fortunately in practice the marker variables are very unlikely to change
        // given a specific CPython release + OS + ISA.)
        let (_, pybi_metadata) = db
            .get_metadata::<Pybi>(&[pybi_ai])
            .with_context(|| format!("fetching metadata for {}", pybi_ai.url))?;
        let pybi_name = pybi_ai.name.inner_as::<PybiName>().unwrap();

        let mut env_marker_vars = pybi_metadata.environment_marker_variables.clone();
        if !env_marker_vars.contains_key("platform_machine") {
            let restricted_platform = platform.restrict(&pybi_name.arch_tags)?;
            env_marker_vars.insert(
                "platform_machine".to_string(),
                restricted_platform.infer_platform_machine()?.to_string(),
            );
        }

        println!("{:#?}", pybi_metadata);
        println!("{:#?}", env_marker_vars);

        // pull out environment marker variables

        todo!()
    }

    // Version that lets you specify the "preferred" version of some packages.
    // This affects:
    // - try to use those versions if can
    // - will be willing to use those versions even if they're yanked (maybe with a
    // warning)
    // TODO
    // pub fn resolve_with_preferences(
    //     &self,
    //     db: &str,
    //     platform_tags: Vec<String>,
    //     preferred_version: &dyn FnMut(&PackageName) -> Option<Version>,
    // ) -> Result<Blueprint> {
    //     todo!()
    // }
}

// PackageIndex needs to memoize metadata within a single run, so things like multiple
// resolutions will be consistent
//  (maybe PackageDB would be a better name?)
// and also eventually provide built wheels on demand
//
// also want to pull out the "pick a version" logic from resolve.rs somehow so can share
// it with pick-a-python-version logic. quite a bit is shared actually:
// - prioritizing newest version, unless have a previous blueprint
// - yank handling (also depends on previous blueprint)
// - pre handling (and retry if resolution failed)
// - package pinning output struct

// need a version of this that tries to stick to a previous blueprint too
// (affects: yanking, package preference)

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
