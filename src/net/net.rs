use crate::prelude::*;
use std::borrow::Cow;
use std::io::{self, Read, Seek};
use ureq::Agent;

use crate::cache::{Basket, Cache};

#[derive(Clone)]
pub struct Net {
    pub agent: Agent,
    pub cache: Cache,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SmallTextPage {
    pub content_type: String,
    pub body: String,
}

// workaround for rustc not liking 'dyn Read + Seek'
pub trait ReadPlusSeek: Read + Seek {}
impl<T: Read + Seek> ReadPlusSeek for T {}

impl Net {
    /// Intended for fetching small API pages, like JSON documents etc. Always
    /// revalidates, so users can see the packages they uploaded show up immediately.
    /// See: https://github.com/pypa/pip/pull/5791
    /// Also loads the whole document into a text string, respecting charset.
    pub fn get_fresh_text(&self, url: &Url) -> Result<SmallTextPage> {
        #[derive(Serialize, Deserialize, Debug)]
        struct CacheEntry<'a> {
            etag: Option<String>,
            last_modified: Option<String>,
            page: Cow<'a, SmallTextPage>,
        }

        let maybe_cached_data = self.cache.get(Basket::FreshText, url.as_str());
        let maybe_cached: Option<CacheEntry> = match &maybe_cached_data {
            Some(cached_data) => serde_cbor::from_slice(cached_data.as_slice()).ok(),
            None => None,
        };
        let mut req = self.agent.request_url("GET", &url);
        // ask any intermediate caches/CDNs to revalidate their own cached copy
        req = req.set("Cache-Control", "max-age=0");
        if let Some(cached) = &maybe_cached {
            if let Some(etag) = &cached.etag {
                req = req.set("If-None-Match", etag);
            }
            if let Some(date) = &cached.last_modified {
                req = req.set("If-Modified-Since", &date);
            }
        }
        let resp = super::retry::call_with_retry(req)?;
        match resp.status() {
            // 304 Not Modified
            304 => Ok(maybe_cached
                .ok_or_else(|| {
                    anyhow!("how did we get 304 when we don't have a cache?!?")
                })?
                .page
                .into_owned()),
            // 200 Ok
            200 => {
                // Have to take ownership here because resp.into_string() destroys the
                // headers
                let etag = resp.header("ETag").map(String::from);
                let date = resp.header("Last-Modified").map(String::from);
                let content_type = resp.content_type().to_string();
                let body = resp.into_string()?;
                let page = SmallTextPage { content_type, body };
                if etag.is_some() || date.is_some() {
                    let new_entry = CacheEntry {
                        etag,
                        last_modified: date,
                        page: Cow::Borrowed(&page),
                    };
                    self.cache.put_file(Basket::FreshText, url.as_str(), |f| {
                        serde_cbor::to_writer(f, &new_entry)?;
                        Ok(())
                    })?;
                } else {
                    warn!("Resource has no ETag/Date; can't cache: {}", url);
                }
                Ok(page)
            }
            status => bail!("expected HTTP status 200 or 304, not {}", status),
        }
    }

    /// Intended for for fetching large, immutable artifacts, like wheels and sdists.
    /// Streams data rather than loading it into memory, caches it, and assumes that the
    /// cache will be valid forever.
    ///
    /// XX maybe should push hash validation down into this layer, so we don't have to
    /// redo it every time we pull the same artifact from cache? But awkward if we ever
    /// need to fetch artifacts where we don't have a hash ahead of time, or we have
    /// different hashes (e.g. sha256 vs sha512).
    pub fn get_artifact(&self, url: &Url) -> Result<impl Read + Seek> {
        if let Some(f) = self.cache.get_file(Basket::Artifact, url.as_str()) {
            Ok(f)
        } else {
            self.cache.put_file(Basket::Artifact, url.as_str(), |f| {
                let resp =
                    super::retry::call_with_retry(self.agent.request_url("GET", &url))?;
                io::copy(&mut resp.into_reader(), f)?;
                Ok(())
            })
        }
    }

    /// Return a file-like object that lazily pulls in data from the given URL as
    /// needed. This is intended to be used with ZipArchive to pull out metadata files
    /// from remote wheels/pybis without downloading the whole file.
    ///
    /// Doesn't do any caching, because we assume the layer above will cache METADATA or
    /// whatever it's looking for once it finds it. (But if we already have the full
    /// artifact cached, then it does use that.)
    pub fn get_lazy_artifact(&self, url: &Url) -> Result<Box<dyn ReadPlusSeek>> {
        Ok(
            if let Some(f) = self.cache.get_file(Basket::Artifact, url.as_str()) {
                Box::new(f)
            } else {
                Box::new(super::lazy_remote_file::LazyRemoteFile::new(
                    &self.agent,
                    &url,
                )?)
            },
        )
    }
}
