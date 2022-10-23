use crate::prelude::*;

use indexmap::IndexMap;

// Generic structs representing the information carried a simple Simple API response
// body for a single project, whether using HTML (PEP 503) or JSON (PEP 691). But it's
// modelled after PEP 691 API, and the serde stuff is all to prepare for that.

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct Meta {
    pub version: String,
}


#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawDistInfoMetadata {
    NoHashes(bool),
    WithHashes(HashMap<String, String>),
}


#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq, Serialize)]
#[serde(from = "Option<RawDistInfoMetadata>")]
pub struct DistInfoMetadata {
    pub available: bool,
    // TODO: support multiple hashes here too
    pub hash: Option<ArtifactHash>,
}

impl From<Option<RawDistInfoMetadata>> for DistInfoMetadata {
    fn from(maybe_raw: Option<RawDistInfoMetadata>) -> Self {
        match maybe_raw {
            None => Default::default(),
            Some(raw) => match raw {
                RawDistInfoMetadata::NoHashes(available) => Self { available, hash: None },
                RawDistInfoMetadata::WithHashes(_) => {
                    // XX FIXME metadata hash support w/ PEP 691
                    Self { available: true, hash: None }
                }
            }
        }
    }
}

// derive(Default) makes NoReason(false) as the default, which is correct
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
enum RawYanked {
    NoReason(bool),
    WithReason(String),
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq, Serialize)]
#[serde(from = "RawYanked")]
pub struct Yanked {
    pub yanked: bool,
    pub reason: Option<String>,
}

impl From<RawYanked> for Yanked {
    fn from(raw: RawYanked) -> Self {
        match raw {
            RawYanked::NoReason(yanked) => Self { yanked, reason: None },
            RawYanked::WithReason(reason) => Self { yanked: true, reason: Some(reason) },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
//#[serde(rename_all = "kebab-case")]
pub struct ArtifactInfo {
    pub name: ArtifactName,
    pub url: Url,
    // TODO: the json api allows this to be a map of algorithm->hex string, with
    // any number of entries
    // How do we handle multiple entries? simple API only has one hash, and for initial
    // implementation warehouse's json API only has one hash, and supporting multiple
    // hashes raises design questions for our caching strategy and lockfiles so... meh
    // just gonna make that future-me's problem...
    pub hash: Option<ArtifactHash>,
    pub requires_python: Option<String>,
//    #[serde(default)]
    pub dist_info_metadata: DistInfoMetadata,
//    #[serde(default)]
    pub yanked: Yanked,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ProjectInfo {
    pub meta: Meta,
    pub artifacts: Vec<ArtifactInfo>,
}

pub fn pack_by_version(pi: ProjectInfo, map: &mut IndexMap<Version, Vec<ArtifactInfo>>) -> Result<()> {
    if !pi.meta.version.starts_with("1.") {
        bail!("unknown package index api version {}", pi.meta.version);
    }

    for ai in pi.artifacts.into_iter() {
        let entry = map.entry(ai.name.version().clone());
        entry.or_insert_with(Default::default).push(ai);
    }

    Ok(())
}
