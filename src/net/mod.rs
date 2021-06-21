// XX automatic retries!

mod lazy_remote_file;
mod retry;
mod user_agent;

pub use user_agent::user_agent;

use crate::prelude::*;

use std::io::{self, Read, Seek};
use ureq::Agent;

use super::cache::{Basket, Cache};

#[derive(Clone)]
pub struct Net {
    pub agent: Agent,
    pub cache: Cache,
}

impl Net {
    /// Intended for fetching small API pages, like JSON documents etc., that have
    /// etags. Revalidates each time, so users can see the packages they uploaded show
    /// up immediately.
    pub fn get_etagged(&self, url: &Url) -> Result<Vec<u8>> {
        #[derive(Serialize, Deserialize, Debug)]
        struct CacheEntry<'a> {
            etag: &'a str,
            #[serde(with = "serde_bytes")]
            body: &'a [u8],
        }

        let maybe_cached_data = self.cache.get(Basket::Etagged, url.as_str());
        let maybe_cached: Option<CacheEntry> = match &maybe_cached_data {
            Some(cached_data) => serde_cbor::from_slice(cached_data.as_slice()).ok(),
            None => None,
        };
        let mut req = self.agent.request_url("GET", &url);
        if let Some(cached) = &maybe_cached {
            req = req.set("If-None-Match", cached.etag);
        }
        let resp = retry::call_with_retry(req)?;
        match resp.status() {
            // 304 Not Modified
            304 => Ok(maybe_cached.unwrap().body.to_owned()),
            // 200 Ok
            200 => {
                let mut new_body = Vec::new();
                let etag = resp.header("ETag").map(String::from);
                resp.into_reader().read_to_end(&mut new_body)?;
                if let Some(etag) = etag {
                    let new_entry = CacheEntry {
                        etag: &etag,
                        body: new_body.as_slice(),
                    };
                    self.cache.put_file(Basket::Etagged, url.as_str(), |f| {
                        serde_cbor::to_writer(f, &new_entry)?;
                        Ok(())
                    })?;
                } else {
                    warn!("Resource has no ETag; can't cache: {}", url);
                }
                Ok(new_body)
            }
            status => bail!("expected HTTP status 200 or 304, not {}", status),
        }
    }

    /// Intended for for fetching large, immutable artifacts, like wheels and sdists.
    /// Streams data rather than loading it into memory, caches it, and assumes that the
    /// cache will be valid forever.
    ///
    /// XX maybe should push hash validation down into this layer, so we don't have to
    /// redo it every time we pull the same artifact from cache?
    pub fn get_artifact(&self, url: &Url) -> Result<impl Read + Seek> {
        if let Some(f) = self.cache.get_file(Basket::Artifact, url.as_str()) {
            Ok(f)
        } else {
            self.cache.put_file(Basket::Artifact, url.as_str(), |f| {
                let resp = retry::call_with_retry(self.agent.request_url("GET", &url))?;
                io::copy(&mut resp.into_reader(), f)?;
                Ok(())
            })
        }
    }

    /// Return a file-like object that lazily pulls in data from the given URL as
    /// needed. This is intended to be used with ZipArchive to pull out METADATA files
    /// from remote wheels without downloading the whole wheel.
    ///
    /// Doesn't do any caching, because we assume the layer above will cache METADATA or
    /// whatever it's looking for once it finds it.
    pub fn get_lazy_artifact(&self, url: &Url) -> Result<impl Read + Seek> {
        Ok(lazy_remote_file::LazyRemoteFile::new(&self.agent, &url)?)
    }
}
