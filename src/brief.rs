use crate::prelude::*;
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
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Brief {
    pub python: PythonRequirement,
    // don't need python_constraints because we always install exactly one python
    pub requires: Vec<UserRequirement>,
    pub constraints: Vec<UserRequirement>,
    // for now let's make this totally explicit: we allow prereleases iff the package is
    // mentioned here (could be python package or a regular package)
    // and we'll see how far we get with that + diagnostic hints for when the user
    // should add a package here to make things work.
    pub allow_pre: HashSet<PackageName>,
    pub allow_pre_all: bool,
    pub platforms: Vec<String>,
}
// should these all be Cow's? if we want to generalize allow_pre to functions (e.g. so
// can do "resolve but with allow_pre=|pkg| {true}"!), then we'll want to be able to
// borrow closures...
// meh for now we'll just leave it hardcoded, and adjust if needed. This also makes
// Debug/Serialize way easier.

#[derive(Debug, Clone)]
pub struct PinnedArtifact {
    pub url: Url,
    pub hashes: Vec<ArtifactHash>,
}

#[derive(Debug, Clone)]
pub struct PinnedPackage {
    pub name: PackageName,
    pub sources: Vec<PinnedArtifact>,
    // This is a hint that's only used for resolving a new Blueprint that's "similar to"
    // an old one. (E.g., when upgrading a single package, or trying to use some saved
    // pins on a new platform.)
    //
    // None means there is no version hint, which should only happen if the package was
    // resolved to an @ requirement.
    pub version: Option<Version>,
    // TODO: expected metadata + its provenance, to catch cases where wheels
    // have mismatched requirements. (just Python-Requires and Requires-Dist?)
}

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub python: PinnedPackage,
    pub packages: Vec<PinnedPackage>,
    pub used_markers: HashMap<marker::Expr, bool>,
}

impl Brief {
    pub fn resolve(
        &self,
        db: &str,
        platform_tags: Vec<String>,
    ) -> Result<Blueprint> {
        self.resolve_with_preferences(db, platform_tags, &|package| None)
    }

    // Version that lets you specify the "preferred" version of some packages.
    // This affects:
    // - try to use those versions if can
    // - will be willing to use those versions even if they're yanked (maybe with a
    // warning)
    pub fn resolve_with_preferences(
        &self,
        db: &str,
        platform_tags: Vec<String>,
        preferred_version: &dyn FnMut(&PackageName) -> Option<Version>,
    ) -> Result<Blueprint>
    {
        todo!()
    }
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

pub trait PyEnvMaker {
    fn make(&self, blueprint: &Blueprint) -> Result<PyEnv>;
}

pub struct PyEnv {
    pub envvars: HashMap<String, String>,
    // destructor? not necessary in short term, but in long term might want to track
    // references so can do GC
}

pub struct ProjectWorkspace {}

pub struct TempWorkspace {}

// represents a _posy directory with persistent named environments
// I guess needs some locking?
impl ProjectWorkspace {
    pub fn get_env(name: &str, blueprint: &Blueprint) -> PyEnv {
        todo!()
    }
}

// represents a temp collection of environments, maybe can do everything with env
// manipulation + share copies of python/packages, including concurrently?
impl TempWorkspace {
    pub fn get_env(blueprint: &Blueprint) -> PyEnv {
        todo!()
    }
}
