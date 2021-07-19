use crate::prelude::*;

use crate::cache::{Basket, Cache};
use crate::net::Net;

use super::simple_api_page::{SimpleAPILink, SimpleAPIPage};

// XX probably will want to make this configurable
pub static ROOT_URL: Lazy<Url> = Lazy::new(|| "https://pypi.org/".try_into().unwrap());

#[derive(Debug, Clone)]
pub struct Artifact {
    pub name: ArtifactName,
    pub hash: ArtifactHash,
    pub url: Url,
    pub yanked: Option<String>,
    pub requires_python: Specifiers,
}

pub struct PackageIndex {
    pub cache: Cache,
    pub net: Net,
    pub base_url: Url,
}

fn try_decode_link(p: &PackageName, link: SimpleAPILink) -> Result<Artifact> {
    let mut segments = link
        .url
        .path_segments()
        .ok_or_else(|| anyhow!("url has no filename"))?;
    // unwrap safe because: "When Some is returned [from path_segments], the
    // iterator always contains at least one string"
    let filename = segments.next_back().unwrap();
    let name: ArtifactName = filename.try_into()?;
    if p != name.distribution() {
        bail!(
            "was expecting artifact for package {}, but found one for {} instead",
            p.normalized(),
            name.distribution().normalized()
        );
    }
    let fragment = link
        .url
        .fragment()
        .ok_or_else(|| anyhow!("link has no hash"))?;
    let hash: ArtifactHash = fragment.try_into()?;
    let yanked = link.yanked;
    let requires_python = match link.requires_python {
        Some(value) => Specifiers::try_from(value)?,
        None => Specifiers::any(),
    };
    let mut url = link.url;
    url.set_fragment(None);
    Ok(Artifact {
        name,
        hash,
        url,
        yanked,
        requires_python,
    })
}

impl PackageIndex {
    pub fn releases(&self, p: &PackageName) -> Result<HashMap<Version, Vec<Artifact>>> {
        let url = self.base_url.join(&format!("simple/{}/", p.normalized()))?;
        let text_page = self.net.get_fresh_text(&url)?;
        let api_page: SimpleAPIPage = text_page.try_into()?;

        if let Some(ver_string) = api_page.repository_version {
            if !ver_string.starts_with("1.") {
                bail!("Unrecognized repository API version {:?}", ver_string);
            }
        }

        let mut versions: HashMap<Version, Vec<Artifact>> = HashMap::new();
        for link in api_page.links.into_iter() {
            let link_url = link.url.clone();
            let result = try_decode_link(p, link).with_context(|| {
                format!("In {}, error parsing link: {}", url, link_url)
            });
            match result {
                Err(err) => warn!("Invalid simple API link: {}", err),
                Ok(artifact) => versions
                    .entry(artifact.name.version().clone())
                    .or_default()
                    .push(artifact),
            }
        }

        Ok(versions)
    }

    pub fn wheel_metadata(&self, url: &Url) -> Result<CoreMetadata> {
        use std::io::{Read, Seek};

        println!("Fetching metadata from {}", url);

        if !url.path().ends_with(".whl") {
            bail!("This URL doesn't seem to be a wheel: {}", url);
        }

        if let Some(metadata) = self.cache.get(Basket::WheelMetadata, url.as_str()) {
            return Ok(CoreMetadata::parse(&metadata)?);
        }

        let body = self.net.get_lazy_artifact(&url)?;
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

        for name in &names {
            if name.ends_with(".dist-info/WHEEL") {
                // will error out if the metadata is bad
                WheelMetadata::parse(&get(&mut zip, &name)?)?;
            }
        }

        for name in &names {
            if name.ends_with(".dist-info/METADATA") {
                let metadata = get(&mut zip, &name)?;
                let parsed = CoreMetadata::parse(&metadata)?;
                self.cache
                    .put(Basket::WheelMetadata, url.as_str(), &metadata)?;
                return Ok(parsed);
            }
        }

        anyhow::bail!("didn't find METADATA");
    }
}
