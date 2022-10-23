use crate::prelude::*;
use crate::seek_slice::SeekSlice;

use http_cache_semantics::{AfterResponse, BeforeRequest, CachePolicy};
use std::io::SeekFrom;
use std::time::SystemTime;

use super::LazyRemoteFile;
use super::cache::{CacheDir, CacheHandle};
use super::ureq_glue::{do_request_ureq, new_ureq_agent};

const MAX_REDIRECTS: u16 = 5;
const REDIRECT_STATUSES: &[u16] = &[301, 302, 303, 307, 308];

// attached to our HTTP responses, to make testing easier
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CacheStatus {
    Fresh,
    StaleButValidated,
    StaleAndChanged,
    Miss,
    Uncacheable,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CacheMode {
    // Apply regular HTTP caching semantics
    Default,
    // If we have a valid cache entry, return it; otherwise return Err(NotCached)
    OnlyIfCached,
    // Don't look in cache, and don't write to cache
    NoStore,
}

#[derive(Debug)]
pub struct NotCached;

impl Display for NotCached {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "request not in cache, and cache_mode=OnlyIfCached")
    }
}

impl std::error::Error for NotCached {}

pub enum ReadPlusMaybeSeek {
    CanSeek(Box<dyn ReadPlusSeek>),
    CannotSeek(Box<dyn Read>),
}

impl Read for ReadPlusMaybeSeek {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            ReadPlusMaybeSeek::CanSeek(inner) => inner.read(buf),
            ReadPlusMaybeSeek::CannotSeek(inner) => inner.read(buf),
        }
    }
}

impl ReadPlusMaybeSeek {
    fn force_seek(self) -> Result<Box<dyn ReadPlusSeek>> {
        Ok(match self {
            ReadPlusMaybeSeek::CanSeek(inner) => inner,
            ReadPlusMaybeSeek::CannotSeek(mut inner) => {
                let mut tmp = tempfile::tempfile()?;
                std::io::copy(&mut inner, &mut tmp)?;
                Box::new(tmp)
            }
        })
    }
}

fn make_response(
    parts: http::response::Parts,
    body: ReadPlusMaybeSeek,
    cache_status: CacheStatus,
) -> http::Response<ReadPlusMaybeSeek> {
    let mut response = http::Response::from_parts(parts, body);
    response.extensions_mut().insert(cache_status);
    response
}

pub struct Http(Rc<HttpInner>);

impl Http {
    pub fn new(http_cache: CacheDir, hash_cache: CacheDir) -> Http {
        Http(Rc::new(HttpInner {
            agent: new_ureq_agent(),
            http_cache,
            hash_cache,
        }))
    }

    pub fn request(
        &self,
        request: http::Request<()>,
        cache_mode: CacheMode,
    ) -> Result<http::Response<ReadPlusMaybeSeek>> {
        self.0.request(request, cache_mode)
    }

    pub fn get_hashed(
        &self,
        url: &Url,
        maybe_hash: Option<&ArtifactHash>,
        cache_mode: CacheMode,
    ) -> Result<Box<dyn ReadPlusSeek>> {
        self.0.get_hashed(url, maybe_hash, cache_mode)
    }

    pub fn get_lazy(&self, url: &Url) -> Result<Box<dyn ReadPlusSeek>> {
        Ok(Box::new(LazyRemoteFile::new(self.0.clone(), &url)?))
    }
}

pub struct HttpInner {
    agent: ureq::Agent,
    http_cache: CacheDir,
    hash_cache: CacheDir,
}

// pass in Option<ArtifactHash> to request/request_if_cached, thread through to fill_cache
// and the temp_file writing
// and validate hash while writing it out to disk
// also include it in the cache key
// or.... if have hash, use a different path that skips the http cache entirely, and
// just puts it directly into a by-hash cache? if you know the hash you expect and you
// have something with that cache, you don't care about the url anymore.
//

// really the cases are:
// - fetching an API page:
//   - want to use http cache
//   - get back a url+read, no need for a tempfile or rest of response
// - fetching an artifact with hash:
//   - want to use hash cache
//   - if cache miss, want to get Read and then write it into cache while verifying hash
//   - return Read+Seek from cache
// - fetching an artifact without hash:
//   - want to use http cache
//   - get back either Read+Seek from cache or Read, and if Read need to put it in a
//     tempfile (but the tempfile can be anonymous)

fn fill_cache<R>(
    policy: &CachePolicy,
    mut body: R,
    handle: CacheHandle,
) -> Result<impl Read + Seek>
where
    R: Read,
{
    let mut cache_writer = handle.begin()?;
    serde_cbor::to_writer(&mut cache_writer, policy)?;
    let body_start = cache_writer.stream_position()?;
    std::io::copy(&mut body, &mut cache_writer)?;
    let body_end = cache_writer.stream_position()?;
    drop(body);
    let cache_entry = cache_writer.commit()?.detach_unlocked();
    Ok(SeekSlice::new(cache_entry, body_start, body_end)?)
}

fn read_cache<R>(mut f: R) -> Result<(CachePolicy, impl Read + Seek)>
where
    R: Read + Seek,
{
    let policy: CachePolicy = serde_cbor::from_reader(&mut f)?;
    let start = f.stream_position()?;
    let end = f.seek(SeekFrom::End(0))?;
    let mut body = SeekSlice::new(f, start, end)?;
    body.rewind()?;
    Ok((policy, body))
}

fn key_for_request<T>(req: &http::Request<T>) -> Vec<u8> {
    let mut key: Vec<u8> = Default::default();
    let method = req.method().to_string().into_bytes();
    key.extend(method.len().to_le_bytes());
    key.extend(method);
    let uri = req.uri().to_string().into_bytes();
    key.extend(uri.len().to_le_bytes());
    key.extend(uri);
    key
}

impl HttpInner {
    pub fn new(http_cache: CacheDir, hash_cache: CacheDir) -> HttpInner {
        HttpInner {
            agent: new_ureq_agent(),
            http_cache,
            hash_cache,
        }
    }

    fn one_request(
        &self,
        request: &http::Request<()>,
        cache_mode: CacheMode,
    ) -> Result<http::Response<ReadPlusMaybeSeek>> {
        // http::uri::Uri strips the fragment automatically, so we don't need to worry
        // about it leaking into our cache key.
        let key = key_for_request(&request);
        let maybe_handle = if cache_mode == CacheMode::NoStore {
            None
        } else {
            Some(self.http_cache.get(key.as_slice())?)
        };

        if let Some(handle) = &maybe_handle {
            if let Some(f) = handle.reader() {
                // detach_unlocked releases the reader's hold on the cache entry lock,
                // but the cache handle itself still holds the lock until we release it
                let (old_policy, old_body) = read_cache(f.detach_unlocked())?;
                return match old_policy.before_request(request, SystemTime::now()) {
                    BeforeRequest::Fresh(parts) => Ok(make_response(
                        parts,
                        ReadPlusMaybeSeek::CanSeek(Box::new(old_body)),
                        CacheStatus::Fresh,
                    )),
                    BeforeRequest::Stale {
                        request: new_parts,
                        matches: _,
                    } => {
                        if cache_mode == CacheMode::OnlyIfCached {
                            return Err(NotCached {}.into());
                        }
                        let request = http::Request::from_parts(new_parts, ());
                        let response = do_request_ureq(&self.agent, &request)?;
                        match old_policy.after_response(
                            &request,
                            &response,
                            SystemTime::now(),
                        ) {
                            AfterResponse::NotModified(new_policy, new_parts) => {
                                let new_body = fill_cache(
                                    &new_policy,
                                    old_body,
                                    maybe_handle.unwrap(),
                                )?;
                                Ok(make_response(
                                    new_parts,
                                    ReadPlusMaybeSeek::CanSeek(Box::new(new_body)),
                                    CacheStatus::StaleButValidated,
                                ))
                            }
                            AfterResponse::Modified(new_policy, new_parts) => {
                                let new_body = fill_cache(
                                    &new_policy,
                                    response.into_body(),
                                    maybe_handle.unwrap(),
                                )?;
                                Ok(make_response(
                                    new_parts,
                                    ReadPlusMaybeSeek::CanSeek(Box::new(new_body)),
                                    CacheStatus::StaleAndChanged,
                                ))
                            }
                        }
                    }
                }
            }
        }
        // no cache entry; do the request and make one.
        if cache_mode == CacheMode::OnlyIfCached {
            return Err(NotCached {}.into());
        }
        let response = do_request_ureq(&self.agent, &request)?;
        let new_policy = CachePolicy::new(request, &response);
        let (parts, body) = response.into_parts();
        if !new_policy.is_storable() || maybe_handle.is_none() {
            Ok(make_response(
                parts,
                ReadPlusMaybeSeek::CannotSeek(Box::new(body)),
                CacheStatus::Uncacheable,
            ))
        } else {
            let new_body = fill_cache(&new_policy, body, maybe_handle.unwrap())?;
            Ok(make_response(
                parts,
                ReadPlusMaybeSeek::CanSeek(Box::new(new_body)),
                CacheStatus::Miss,
            ))
        }
    }

    pub fn request(
        &self,
        mut request: http::Request<()>,
        cache_mode: CacheMode,
    ) -> Result<http::Response<ReadPlusMaybeSeek>> {
        let max_redirects = if request.method() == http::method::Method::GET {
            MAX_REDIRECTS
        } else {
            0
        };
        for attempt in 0..=max_redirects {
            let url = Url::parse(&request.uri().to_string())?;
            let mut response = self.one_request(&request, cache_mode)?;
            if REDIRECT_STATUSES.contains(&response.status().as_u16()) {
                if attempt < max_redirects {
                    if let Some(target) = response.headers().get("Location") {
                        let target_str = std::str::from_utf8(target.as_bytes())?;
                        let full_target = url.join(target_str)?;
                        *request.uri_mut() = full_target.to_string().try_into()?;
                        continue;
                    }
                } else {
                    bail!("hit redirection limit at {}", url);
                }
            }
            // attach the actual URL to the response, so our caller knows where it came
            // from (e.g. to resolve relative links)
            response.extensions_mut().insert(url);
            return Ok(response);
        }
        unreachable!()
    }

    pub fn get_hashed(
        &self,
        url: &Url,
        maybe_hash: Option<&ArtifactHash>,
        cache_mode: CacheMode,
    ) -> Result<Box<dyn ReadPlusSeek>> {
        let request = http::Request::builder().uri(url.as_str()).body(())?;
        if maybe_hash.is_some() && cache_mode != CacheMode::NoStore {
            let hash = maybe_hash.unwrap();
            let handle = self.hash_cache.get(hash)?;
            if let Some(reader) = handle.reader() {
                Ok(Box::new(reader.detach_unlocked()))
            } else {
                if cache_mode == CacheMode::OnlyIfCached {
                    Err(NotCached {}.into())
                } else {
                    // fetch and store into the artifact cache, bypassing the regular
                    // http cache
                    let mut body = self.request(request, CacheMode::NoStore)?.into_body();
                    let mut outer_writer = hash.checker(handle.begin()?)?;
                    std::io::copy(&mut body, &mut outer_writer)?;
                    let inner_writer = outer_writer.finish()?;
                    Ok(Box::new(inner_writer.commit()?.detach_unlocked()))
                }
            }
        } else {
            Ok(self
                .request(request, cache_mode)?
                .into_body()
                .force_seek()?)
        }
    }
}
