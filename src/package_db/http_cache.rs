use crate::net::call_with_retry;
use crate::prelude::*;

use http_cache_semantics::{AfterResponse, BeforeRequest, CachePolicy};
use slice::IoSlice;
use std::io::{Read, Seek, SeekFrom};
use std::time::SystemTime;

use super::cache::{MutableFileCache, MutableFileCacheHandle};

pub trait ReadPlusSeek: Read + Seek {}
impl<T: Read + Seek> ReadPlusSeek for T {}

pub enum CacheStatus {
    Fresh,
    StaleButValidated,
    StaleAndChanged,
    Miss,
    Uncacheable,
}

pub struct HttpResponse {
    pub status: http::StatusCode,
    pub headers: http::HeaderMap,
    pub body: Box<dyn ReadPlusSeek>,
    // just for testing/debugging:
    pub cache_status: CacheStatus,
}

struct Net {
    agent: ureq::Agent,
}

fn fill_cache<R>(
    policy: &CachePolicy,
    mut body: R,
    handle: MutableFileCacheHandle,
) -> Result<impl Read + Seek>
where
    R: Read,
{
    let mut cache_entry = handle.begin()?;
    serde_cbor::to_writer(&mut cache_entry, policy)?;
    let body_start = cache_entry.stream_position()?;
    std::io::copy(&mut body, &mut cache_entry)?;
    let length = cache_entry.stream_position()? - body_start;
    drop(body);
    let cache_entry = cache_entry.commit()?.detach_unlocked();
    Ok(IoSlice::new(cache_entry, body_start, length)?)
}

fn read_cache<R>(mut f: R) -> Result<(CachePolicy, impl Read + Seek)>
where
    R: Read + Seek,
{
    let policy: CachePolicy = serde_cbor::from_reader(&mut f)?;
    let start = f.stream_position()?;
    let end = f.seek(SeekFrom::End(0))?;
    let mut body = IoSlice::new(f, start, end)?;
    body.rewind()?;
    Ok((policy, body))
}

impl Net {
    fn do_request_ureq(
        &self,
        req: &http::Request<()>,
    ) -> Result<http::Response<impl Read>> {
        // use ureq to perform the request (this is the only part you need to swap out to
        // use a different HTTP client
        let mut ureq_req = self
            .agent
            .request_url(req.method().as_str(), &Url::parse(&req.uri().to_string())?);
        for (name, value) in req.headers().into_iter() {
            ureq_req =
                ureq_req.set(name.as_str(), std::str::from_utf8(value.as_bytes())?);
        }
        let ureq_response = call_with_retry(ureq_req)?;
        let mut response = http::Response::builder().status(ureq_response.status());
        for name in ureq_response.headers_names() {
            for value in ureq_response.all(&name) {
                response = response.header(&name, value);
            }
        }
        Ok(response.body(ureq_response.into_reader())?)
    }

    fn request(
        &self,
        req: http::Request<()>,
        cache: &MutableFileCache,
    ) -> Result<HttpResponse> {
        // http::uri::Uri strips the fragment automatically, so we don't need to worry about
        // it leaking into our cache key.
        let key = cache_key(&req.uri().to_string(), &req.method().to_string());
        let mut handle = cache.get_handle(&key)?;

        // cache file format: ZipFile, uncompressed, with entries "policy" and "body"
        if let Some(mut f) = handle.reader() {
            let (old_policy, old_body) = read_cache(f)?;

            match old_policy.before_request(&req, SystemTime::now()) {
                BeforeRequest::Fresh(parts) => {
                    return Ok(HttpResponse {
                        status: parts.status,
                        headers: parts.headers,
                        body: Box::new(old_body),
                        cache_status: CacheStatus::Fresh,
                    });
                }
                BeforeRequest::Stale { request, matches } => {
                    req = http::Request::from_parts(request, ());
                    let mut response = self.do_request_ureq(&req)?;
                    match old_policy.after_response(&req, &response, SystemTime::now())
                    {
                        AfterResponse::NotModified(new_policy, new_parts) => {
                            let new_body = fill_cache(&new_policy, old_body, handle)?;
                            Ok(HttpResponse {
                                status: new_parts.status,
                                headers: new_parts.headers,
                                body: Box::new(new_body),
                                cache_status: CacheStatus::StaleButValidated,
                            })
                        }
                        AfterResponse::Modified(new_policy, new_parts) => {
                            let new_body =
                                fill_cache(&new_policy, response.into_body(), handle)?;
                            Ok(HttpResponse {
                                status: new_parts.status,
                                headers: new_parts.headers,
                                body: Box::new(new_body),
                                cache_status: CacheStatus::StaleButValidated,
                            })
                        }
                    }
                }
            }
        } else {
            // no cache entry at all; do the request and make one.
            let mut response = self.do_request_ureq(&req)?;
            let new_policy = CachePolicy::new(&req, &response);
            let (parts, mut body) = response.into_parts();
            if !new_policy.is_storable() {
                let mut tmp = tempfile::tempfile()?;
                std::io::copy(&mut body, &mut tmp);
                Ok(HttpResponse {
                    status: parts.status,
                    headers: parts.headers,
                    body: Box::new(tmp),
                    cache_status: CacheStatus::Uncacheable,
                })
            } else {
                let body = fill_cache(&new_policy, body, handle)?;
                Ok(HttpResponse {
                    status: parts.status,
                    headers: parts.headers,
                    body: Box::new(body),
                    cache_status: CacheStatus::Miss,
                })
            }
        }
    }
}

fn cache_key(url: &str, method: &str) -> Vec<u8> {
    let mut raw_key: Vec<u8> = Default::default();
    raw_key.extend(method.len().to_le_bytes());
    raw_key.extend(method.as_bytes().iter());
    raw_key.extend(url.as_bytes().iter());
    raw_key
}
