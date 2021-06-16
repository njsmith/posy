use crate::prelude::*;

// XX: move all URL access behind a caching layer
use ureq::Agent;

// XX probably will want to make this configurable
// XX probably switch to using the simple API also
//   nb this will make it possible to fetch requires-python metadata as part of
//   Artifact, I think?
pub static ROOT_URL: Lazy<Url> = Lazy::new(|| "https://pypi.org/".try_into().unwrap());

#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum HashMode {
    SHA256,
}
use HashMode::*;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ArtifactHash {
    pub mode: HashMode,
    pub raw_data: Vec<u8>,
}

impl ArtifactHash {
    pub fn from_hex(mode: HashMode, hex: &str) -> Result<ArtifactHash> {
        Ok(ArtifactHash {
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
    pub hash: ArtifactHash,
    pub url: Url,
    pub yanked: Option<String>,
}

pub struct PackageIndex {
    agent: Agent,
    base_url: Url,
}

impl PackageIndex {
    pub fn new(agent: Agent, base_url: Url) -> PackageIndex {
        PackageIndex { agent, base_url }
    }
}

impl PackageIndex {
    pub fn releases(&self, p: &PackageName) -> Result<HashMap<Version, Vec<Artifact>>> {
        #[derive(Debug, Deserialize)]
        struct PyPIDigests {
            sha256: String,
        }

        #[derive(Debug, Deserialize)]
        struct PyPIArtifact {
            digests: PyPIDigests,
            url: Url,
            yanked_reason: Option<String>,
        }

        #[derive(Debug, Deserialize)]
        struct ReleasesPage {
            releases: HashMap<String, Vec<PyPIArtifact>>,
        }

        let url = self
            .base_url
            .join(&format!("pypi/{}/json/", p.normalized()))?;
        let page: ReleasesPage =
            self.agent.request_url("GET", &url).call()?.into_json()?;

        let mut releases = HashMap::new();
        for (ver, pypi_artifacts) in page.releases {
            let artifacts = pypi_artifacts
                .into_iter()
                .map(|pa| {
                    Ok(Artifact {
                        hash: ArtifactHash::from_hex(SHA256, &pa.digests.sha256)?,
                        url: pa.url,
                        yanked: pa.yanked_reason,
                    })
                })
                .collect::<Result<Vec<Artifact>>>()?;

            // XX for robustification will probably need to tolerate versions that fail
            // to parse here, so one weird old version doesn't make it impossible to
            // work with a package entirely.
            releases.insert(ver.try_into()?, artifacts);
        }

        Ok(releases)
    }
}

impl PackageIndex {
    pub fn wheel_metadata(&self, url: &Url) -> Result<CoreMetadata> {
        use std::io::{Cursor, Read, Seek};

        println!("Fetching and parsing {}", url);

        if !url.path().ends_with(".whl") {
            bail!("This URL doesn't seem to be a wheel: {}", url);
        }

        let resp = self.agent.request_url("GET", &url).call()?;
        let mut body = Vec::new();
        resp.into_reader().read_to_end(&mut body)?;
        let body = Cursor::new(body);
        let mut zip = zip::ZipArchive::new(body)?;
        let names: Vec<String> = zip.file_names().map(|s| s.to_owned()).collect();

        fn get<T: Read + Seek>(
            zip: &mut zip::ZipArchive<T>,
            name: &str,
        ) -> Result<Vec<u8>> {
            let mut buf = Vec::new();
            let mut zipfile = zip.by_name(name)?;
            zipfile.read_to_end(&mut buf)?;
            Ok(buf)
        }

        for name in names {
            if name.ends_with(".dist-info/WHEEL") {
                // will error out if the metadata is bad
                WheelMetadata::parse(&get(&mut zip, &name)?)?;
            }
            if name.ends_with(".dist-info/METADATA") {
                return Ok(CoreMetadata::parse(&get(&mut zip, &name)?)?);
            }
        }

        anyhow::bail!("didn't find METADATA");
    }
}
