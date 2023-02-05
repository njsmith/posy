use crate::prelude::*;
use crate::seek_slice::SeekSlice;

use http_cache_semantics::{AfterResponse, BeforeRequest, CachePolicy};
use std::io::SeekFrom;
use std::time::SystemTime;

use super::super::ArtifactInfo;
use super::ureq_glue::{do_request_ureq, new_ureq_agent};
use super::LazyRemoteFile;
use crate::kvstore::{KVFileLock, KVFileStore};

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
    pub fn new(http_cache: KVFileStore, hash_cache: KVFileStore) -> Http {
        Http(Rc::new(HttpInner::new(http_cache, hash_cache)))
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

    pub fn get_lazy(&self, ai: &ArtifactInfo) -> Result<Box<dyn ReadPlusSeek>> {
        match LazyRemoteFile::new(self.0.clone(), &ai.url) {
            Ok(lazy) => Ok(Box::new(lazy)),
            Err(err) => {
                match err.downcast_ref::<PosyError>() {
                    // Doesn't support Range: requests, or similar issue. Fall back on
                    // fetching the whole file via the normal path.
                    Some(PosyError::LazyRemoteFileNotSupported) => Ok(
                        self.get_hashed(&ai.url, ai.hash.as_ref(), CacheMode::Default)?
                    ),
                    _ => Err(err)?,
                }
            }
        }
    }
}

pub struct HttpInner {
    agent: ureq::Agent,
    http_cache: KVFileStore,
    hash_cache: KVFileStore,
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
    handle: KVFileLock,
) -> Result<impl Read + Seek>
where
    R: Read,
{
    let mut cache_writer = handle.begin()?;
    ciborium::ser::into_writer(policy, &mut cache_writer)?;
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
    let policy: CachePolicy = ciborium::de::from_reader(&mut f)?;
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
    // http::uri::Uri strips the fragment automatically, so we don't need to worry about
    // it leaking into our cache key.
    let uri = req.uri().to_string().into_bytes();
    key.extend(uri.len().to_le_bytes());
    key.extend(uri);
    key
}

impl HttpInner {
    pub fn new(http_cache: KVFileStore, hash_cache: KVFileStore) -> HttpInner {
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
        if cache_mode == CacheMode::NoStore {
            let (parts, body) = do_request_ureq(&self.agent, request)?.into_parts();
            Ok(make_response(
                parts,
                ReadPlusMaybeSeek::CannotSeek(Box::new(body)),
                CacheStatus::Uncacheable,
            ))
        } else {
            let key = key_for_request(request);
            let lock = self.http_cache.lock(&key.as_slice())?;

            // common code from the two paths where we need to store a new response
            // (cache miss and cache stale)
            let handle_new = |new_policy: CachePolicy,
                              new_parts,
                              body,
                              cache_status,
                              lock: KVFileLock| {
                if !new_policy.is_storable() {
                    lock.remove()?;
                    Ok(make_response(
                        new_parts,
                        ReadPlusMaybeSeek::CannotSeek(Box::new(body)),
                        CacheStatus::StaleAndChanged,
                    ))
                } else {
                    let new_body = fill_cache(&new_policy, body, lock)?;
                    Ok(make_response(
                        new_parts,
                        ReadPlusMaybeSeek::CanSeek(Box::new(new_body)),
                        cache_status,
                    ))
                }
            };

            if let Some(f) = lock.reader() {
                // we have to detach_unlocked here because 'old_body' takes ownership of
                // the passed-in reader, and the reader's lifetime holds the lock alive.
                // detach_unlocked lets go of that lifetime, but we still have 'lock' so
                // the lock itself remains.
                let (old_policy, old_body) = read_cache(f.detach_unlocked())?;
                match old_policy.before_request(request, SystemTime::now()) {
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
                                let new_body = fill_cache(&new_policy, old_body, lock)?;
                                Ok(make_response(
                                    new_parts,
                                    ReadPlusMaybeSeek::CanSeek(Box::new(new_body)),
                                    CacheStatus::StaleButValidated,
                                ))
                            }
                            AfterResponse::Modified(new_policy, new_parts) => {
                                drop(old_body);
                                handle_new(
                                    new_policy,
                                    new_parts,
                                    response.into_body(),
                                    CacheStatus::StaleAndChanged,
                                    lock,
                                )
                            }
                        }
                    }
                }
            } else {
                // no cache entry; do the request and make one.
                if cache_mode == CacheMode::OnlyIfCached {
                    return Err(NotCached {}.into());
                }
                let response = do_request_ureq(&self.agent, request)?;
                let new_policy = CachePolicy::new(request, &response);
                let (parts, body) = response.into_parts();
                handle_new(new_policy, parts, body, CacheStatus::Miss, lock)
            }
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
            if cache_mode == CacheMode::OnlyIfCached {
                self.hash_cache.get(&hash).ok_or_else(||NotCached {}.into())
            } else {
                assert!(cache_mode == CacheMode::Default);
                Ok(self.hash_cache.get_or_set(&hash, |mut w| {
                    let mut body =
                        self.request(request, CacheMode::NoStore)?.into_body();
                    let mut checker = hash.checker(&mut w)?;
                    std::io::copy(&mut body, &mut checker)?;
                    checker.finish()?;
                    Ok(())
                })?)
            }
        } else {
            Ok(self
                .request(request, cache_mode)?
                .into_body()
                .force_seek()?)
        }
    }
}
