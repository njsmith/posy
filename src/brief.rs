use crate::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PinPlatform {
    Local,
}

/// An abstraction of what you need to know about a platform to pin packages for it
impl PinPlatform {
    pub fn markers(
        &self,
        py: PackageName,
        pyver: Version,
        index: super::package_index::PackageIndex,
    ) -> Result<HashMap<String, String>> {
        // some options:
        // - find a pybi match the given settings and get its markers
        // - calculate markers directly for current system (possibly based on some
        //   explicit template), or even return a canned response
        // - build a python locally and then query...
        todo!()
    }
}

/// A high-level description of an environment that a user would like to be able to
/// build. Doesn't necessarily have to be what the user types in exactly, but has to
/// represent their intentions, and *not* anything that requires looking at a package
/// index.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Brief {
    // should this be a PythonRequirement? it can't have a marker expression...
    pub python_requirement: UserRequirement,
    // don't need python_constraints because we always install exactly one python
    pub package_requires: Vec<UserRequirement>,
    pub package_constraints: Vec<UserRequirement>,
    // for now let's make this totally explicit: we allow prereleases iff the package is
    // mentioned here (could be python package or a regular package)
    // and we'll see how far we get with that + diagnostic hints for when the user
    // should add a package here to make things work.
    pub allow_pre: HashSet<PackageName>,
    pub platforms: Vec<PinPlatform>,
}

impl Default for Brief {
    fn default() -> Self {
        Brief {
            python_requirement: "cpython_unofficial >= 3".try_into().unwrap(),
            platforms: vec![PinPlatform::Local],
            ..Default::default()
        }
    }
}

impl Brief {
    pub fn resolve(&self, index: super::package_index::PackageIndex) -> Blueprint {
        todo!()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Blueprint {
    // the brief we were built from
// (except maybe not the platform requests?)

// set of package pins
// links to artifacts + hashes
// dependencies we used to compute the pins + provenance
// split up by platform / marker tags
}

pub struct PyEnv {
    // for regular on-disk envs, need to be able to query to get:
//   markers
//   tags
//   installed packages
// and then renovate to remove existing packages, add new ones
// ideally in parallel, while handling file conflicts gracefully
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

impl PyEnv {
    pub fn run() {
        todo!()
    }
}
