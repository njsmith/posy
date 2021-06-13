use crate::prelude::*;

// XX: move all URL access behind a caching layer
use ureq::Agent;

// XX probably will want to make this configurable
static ROOT_URL: Lazy<Url> = Lazy::new(|| "https://pypi.org/pypi/".try_into().unwrap());

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum HashMode {
    SHA256,
}
use HashMode::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Hash {
    pub mode: HashMode,
    pub raw_data: Vec<u8>,
}

impl Hash {
    pub fn from_hex(mode: HashMode, hex: &str) -> Result<Hash> {
        Ok(Hash {
            mode,
            raw_data: data_encoding::HEXLOWER_PERMISSIVE.decode(hex.as_bytes())?,
        })
    }

    pub fn to_base64_urlsafe_unpadded(&self) -> String {
        data_encoding::BASE64URL_NOPAD.encode(&self.raw_data)
    }
}

#[derive(Debug, Clone)]
pub struct Artifact {
    pub hash: Hash,
    pub url: Url,
    pub yanked: bool,
}

#[derive(Debug, Clone)]
pub struct Release {
    pub package: PackageName,
    pub version: Version,
    pub artifacts: Vec<Artifact>,
}

#[derive(Debug, Deserialize)]
struct PyPIDigests {
    sha256: String,
}

#[derive(Debug, Deserialize)]
struct PyPIArtifact {
    digests: PyPIDigests,
    url: Url,
    yanked: bool,
}

#[derive(Debug, Deserialize)]
struct ReleasesPage {
    releases: HashMap<String, Vec<PyPIArtifact>>,
}

pub struct PyPI {
    pub agent: Agent,
}

impl PyPI {
    pub fn package_info(&self, p: &PackageName) -> Result<Vec<Release>> {
        let url = ROOT_URL.join(&format!("{}/json", p))?;
        let page: ReleasesPage =
            self.agent.request_url("GET", &url).call()?.into_json()?;

        let mut releases = Vec::new();
        for (ver, pypi_artifacts) in page.releases {
            let mut artifacts = Vec::new();

            for pypi_artifact in pypi_artifacts {
                artifacts.push(Artifact {
                    hash: Hash::from_hex(SHA256, &pypi_artifact.digests.sha256)?,
                    url: pypi_artifact.url,
                    yanked: pypi_artifact.yanked,
                })
            }
            releases.push(Release {
                package: p.clone(),
                version: ver.try_into()?,
                artifacts,
            });
        }

        Ok(releases)
    }
}
