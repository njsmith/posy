use crate::prelude::*;
use super::super::cache::MutableFileCache;
use super::project_info::ProjectInfo;

use ureq::Agent;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct CacheEntry {
    etag: Option<String>,
    last_modified: Option<String>,
    content_type: String,
    body: String,
}

pub fn fetch_simple_api(agent: &Agent, url: &Url, cache: &MutableFileCache) -> Result<ProjectInfo> {
    let handle = cache.get_handle(&url.to_string().as_bytes())?;
    let mut req = agent.request_url("GET", &url);
    let maybe_cache_entry: Option<CacheEntry> = handle.reader().and_then(
        // ignore errors from serde_cbor here, because it's just a cache
        |read| serde_cbor::from_reader(read).ok()
    );
    let mut req = agent.request_url("GET", &url);
    req = req.set("Cache-Control", "max-age=0");
    if let Some(cached) = &maybe_cache_entry {
        if let Some(etag) = &cached.etag {
            req = req.set("If-None-Match", &etag);
        }
        if let Some(date) = &cached.last_modified {
            req = req.set("If-Modified-Since", &date);
        }
    }
    let resp = crate::net::call_with_retry(req)?;
    let cache_entry = match resp.status() {
        // 304 Not Modified
        304 =>
            if let Some(cache_entry) = maybe_cache_entry {
                cache_entry
            } else {
                bail!("how did we get 304 when we don't have a cache?!?")
            },
        // 200 Ok
        200 => {
            // Have to take ownership here because resp.into_string() destroys
            // the headers
            let etag = resp.header("ETag").map(String::from);
            let last_modified = resp.header("Last-Modified").map(String::from);
            let content_type = resp.content_type().to_string();
            let body = resp.into_string()?;
            let new_entry = CacheEntry {
                etag,
                last_modified,
                content_type,
                body,
            };
            handle.replace(&|write| {
                serde_cbor::to_writer(write, &new_entry)?;
                Ok(())
            });
            new_entry
        },
        other => bail!("expected HTTP status 200 or 304, not {}", other),
    };
    Ok(super::parse_html(&url, &cache_entry.content_type, &cache_entry.body)?)
}
