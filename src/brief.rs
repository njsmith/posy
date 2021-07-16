use crate::prelude::*;

/// A high-level description of an environment that a user would like to be able to
/// build.
#[derive(Debug, Clone)]
pub struct Brief {
    // python:
    //  [empty]
    //  ">= 3"
    //  "== 3.8.*"
    //  "pypy3 >= 7.3"
    python_requirement: Requirement,

    // maybe just predefine some named python types, and let them define their own as
    // well? py3XX -> cpython_unofficial == "3.XX.*"
    //   with prereleases automatically enabled for pythons that aren't out yet

    // platforms they care about
    // how to specify these? can abstract it but...
    // need to be able to query for env markers? or just pick a pybi, and get the env
    // markers from it?
    // maybe it's just a list of env markers or "native"? or a function mapping python
    // version -> env markers?

    // I guess there's both the user-level case where they specify stuff,
    // and the transient internal environments that we need to create for stuff like
    // building wheels?
    // or temporary overrides
    //
    // or using different ways to make an environment? build environments might want to
    // do tricky stuff on the fly to be fast, might want to use a system python +
    // venv...

    // requirements
    // constraints

    // index servers

    // explicit allow-pre/disallow-pre settings
    //  (default to: allow iff mentioned in a requirement/constraint)
    //  can name the python implementation here too
    // or should these be attached to the requirement lines?
    //  downside of that: want to be able to specify it for transitive dependencies too
    // ... or should it be entirely explicit here, and make the frontend apply those
    //  defaults?

    // constraints and pre-release settings should go together, because they both are
    // ways to control the universe of versions that are considered for a given package

    // some equivalent of cargo's [patch]?

    // I guess this doesn't need to *directly* be a user friendly thing (though being
    // relatively friendly to construct from rust would be nice)
    // e.g. doesn't need implicit handholding
    // it just needs to express the user's intentions that are stable over time, and
    // don't require hitting the network/package index
}

impl Brief {
    // XX needs index and stuff too
    pub fn resolve(&self) -> Blueprint {
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

pub struct ProjectWorkspace {
}

pub struct TempWorkspace {
}

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
