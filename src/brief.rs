use crate::{
    package_db::{ArtifactInfo, PackageDB},
    platform_tags::PybiPlatform,
    prelude::*,
    resolve::{resolve_wheels, ExpectedMetadata},
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
    pub requirements: Vec<UserRequirement>,
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

impl Display for PinnedPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} {} ({} known hashes)",
            self.name.as_given(),
            self.version,
            self.hashes.len()
        )
    }
}

#[derive(Debug, Clone)]
pub struct Blueprint {
    pub pybi: PinnedPackage,
    pub wheels: Vec<(PinnedPackage, ExpectedMetadata)>,
}

impl Display for Blueprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pybi: {}\n", self.pybi)?;
        for (wheel, em) in &self.wheels {
            write!(f, "wheel: {} (metadata from {})\n", wheel, em.provenance)?;
        }
        Ok(())
    }
}

fn pick_best_pybi<'a>(
    artifact_infos: &'a Vec<ArtifactInfo>,
    platform: &PybiPlatform,
) -> Option<&'a ArtifactInfo> {
    artifact_infos
        .iter()
        .filter_map(|ai| {
            if let ArtifactName::Pybi(name) = &ai.name {
                platform
                    .max_compatibility(name.arch_tags.iter())
                    .map(|score| (ai, score))
            } else {
                None
            }
        })
        .max_by_key(|(_, score)| *score)
        .map(|(ai, _)| ai)
}

// XX TODO: merge with version preference logic in resolve.rs, b/c this should have
// similar handling of prereleases, yanks, previous-blueprint-hints, etc.
fn resolve_pybi<'a>(
    db: &'a PackageDB,
    req: &PythonRequirement,
    platform: &PybiPlatform,
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

fn pinned(
    db: &PackageDB,
    name: PackageName,
    version: Version,
) -> Result<PinnedPackage> {
    let hashes = db
        .artifacts_for_release(&name, &version)?
        .iter()
        .filter_map(|ai| ai.hash.clone())
        .collect::<Vec<_>>();
    Ok(PinnedPackage {
        name,
        version,
        hashes,
    })
}

impl Brief {
    pub fn resolve(&self, db: &PackageDB, platform: &PybiPlatform) -> Result<Blueprint> {
        let pybi_ai = resolve_pybi(&db, &self.python, &platform)?;
        let (_, pybi_metadata) = db
            .get_metadata::<Pybi, _>(&[pybi_ai])
            .with_context(|| format!("fetching metadata for {}", pybi_ai.url))?;
        let pybi_name = pybi_ai.name.inner_as::<PybiName>().unwrap();

        let mut env_marker_vars = pybi_metadata.environment_marker_variables.clone();
        if !env_marker_vars.contains_key("platform_machine") {
            let wheel_platform =
                platform.wheel_platform_for_pybi(&pybi_name, &pybi_metadata)?;
            env_marker_vars.insert(
                "platform_machine".to_string(),
                wheel_platform.infer_platform_machine()?.to_string(),
            );
        }

        let resolved_wheels = resolve_wheels(db, &self.requirements, &env_marker_vars)?;
        let mut wheels = Vec::<(PinnedPackage, ExpectedMetadata)>::new();
        for (p, v, em) in resolved_wheels {
            wheels.push((pinned(&db, p, v)?, em));
        }

        Ok(Blueprint {
            pybi: pinned(
                &db,
                pybi_name.distribution.to_owned(),
                pybi_name.version.to_owned(),
            )?,
            wheels,
        })
    }
}
