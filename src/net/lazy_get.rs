use crate::prelude::*;

use std::cmp;
use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};
use ureq::Agent;

// semi-arbitrary, but ideally should be large enough to catch all the zip index +
// dist-info data at the end of common wheel files
const LAZY_FETCH_SIZE: u64 = 10_000;

pub struct LazyGet {
    agent: Agent,
    url: Url,
    loaded: BTreeMap<u64, Vec<u8>>,
    length: u64,
    seek_pos: u64,
}

fn slurp(mut data: Box<dyn Read>) -> Result<Vec<u8>> {
    let mut buf = Vec::new();
    data.read_to_end(&mut buf)?;
    Ok(buf)
}

impl Seek for LazyGet {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let LazyGet {
            length, seek_pos, ..
        } = self;
        // Basic structure cribbed from io::Cursor
        // NB: this allows seeking past the end of the file (and then read just
        // returns EOF, I guess)
        let (base_pos, offset) = match pos {
            SeekFrom::Start(offset) => {
                *seek_pos = offset;
                return Ok(offset);
            }
            SeekFrom::End(offset) => (*length, offset),
            SeekFrom::Current(offset) => (*seek_pos, offset),
        };
        let new_pos = if offset >= 0 {
            base_pos.checked_add(offset as u64)
        } else {
            base_pos.checked_sub((offset.wrapping_neg()) as u64)
        };
        match new_pos {
            Some(n) => {
                *seek_pos = n;
                Ok(n)
            }
            None => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid seek to a negative or overflowing positions",
            )),
        }
    }
}

enum RangeResponse {
    Partial {
        offset: u64,
        total_len: u64,
        data: Box<dyn Read>,
    },
    Complete(Box<dyn Read>),
}

fn fetch_range(agent: &Agent, url: &Url, range_header: &str) -> Result<RangeResponse> {
    // The full syntax has a bunch of possibilities that this doesn't account for:
    //   https://datatracker.ietf.org/doc/html/rfc7233#section-4.2
    // but this is the only format that's actually *useful* to us.
    static CONTENT_RANGE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^bytes ([0-9]+)-[0-9]+/([0-9]+)$").unwrap());

    let response = agent
        .request_url("GET", &url)
        .set("Range", range_header)
        .call()?;

    Ok(match response.status() {
        // 206 Partial Content
        206 => {
            match response.header("Content-Range") {
                None => bail!("range response is missing Content-Range"),
                Some(content_range) => {
                    match CONTENT_RANGE_RE.captures(&content_range) {
                        None => bail!("failed to parse Content-Range"),
                        Some(captures) => {
                            // unwraps safe because groups always match a valid int
                            let offset: u64 =
                                captures.get(1).unwrap().as_str().parse()?;
                            let total_len: u64 =
                                captures.get(2).unwrap().as_str().parse()?;
                            RangeResponse::Partial {
                                offset,
                                total_len,
                                data: Box::new(response.into_reader()),
                            }
                        }
                    }
                }
            }
        }
        // 200 Ok -> server doesn't like Range: requests and is just sending the full
        // data
        200 => RangeResponse::Complete(Box::new(response.into_reader())),
        status => bail!("expected 200 or 206 HTTP response, not {}", status),
    })
}

impl LazyGet {
    fn load_range(&mut self, offset: u64) -> Result<()> {
        match fetch_range(
            &self.agent,
            &self.url,
            &format!(
                "bytes={}-{}",
                offset,
                offset.saturating_add(LAZY_FETCH_SIZE)
            ),
        )? {
            RangeResponse::Partial { offset, data, .. } => {
                self.loaded.insert(offset, slurp(data)?);
                Ok(())
            }
            RangeResponse::Complete(_) => {
                bail!("server abruptly stopped understanding range requests?!?")
            }
        }
    }
}

impl Read for LazyGet {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        fn copy_loaded(
            offset: u64,
            loaded: &BTreeMap<u64, Vec<u8>>,
            buf: &mut [u8],
        ) -> Option<usize> {
            // find the btree entry that's closest to the requested offset, if any
            match loaded.range(..=offset).next_back() {
                None => None,
                Some((loaded_offset, loaded_data)) => {
                    if let Ok(slide) =
                        usize::try_from(offset.saturating_sub(*loaded_offset))
                    {
                        let usable_loaded_data = &loaded_data[slide..];
                        let len = cmp::min(buf.len(), usable_loaded_data.len());
                        buf[..len].copy_from_slice(&usable_loaded_data[..len]);
                        Some(len)
                    } else {
                        None
                    }
                }
            }
        }

        fn fix_err<T, E>(input: std::result::Result<T, E>) -> std::io::Result<T>
        where
            E: Into<Box<dyn std::error::Error + Send + Sync>>,
        {
            use std::io::{Error, ErrorKind};
            match input {
                Ok(t) => Ok(t),
                Err(e) => Err(Error::new(ErrorKind::Other, e)),
            }
        }

        let wanted =
            cmp::min(buf.len() as u64, self.length.saturating_sub(self.seek_pos));
        if wanted <= 0 {
            return Ok(0);
        }
        // maybe we already have it in cache?
        if let Some(len) = copy_loaded(self.seek_pos, &mut self.loaded, buf) {
            self.seek_pos = self.seek_pos.saturating_add(fix_err(len.try_into())?);
            return Ok(len);
        }
        // otherwise, we need to fetch + fill in the cache
        fix_err(self.load_range(self.seek_pos))?;
        // now it's definitely in cache
        if let Some(len) = copy_loaded(self.seek_pos, &mut self.loaded, buf) {
            self.seek_pos = self.seek_pos.saturating_add(fix_err(len.try_into())?);
            return Ok(len);
        }
        unreachable!("and you may ask yourself, well, how did I get here?")
    }
}

impl LazyGet {
    pub fn new(agent: &Agent, url: &Url) -> Result<LazyGet> {
        let mut remote = LazyGet {
            agent: agent.clone(),
            url: url.clone(),
            loaded: BTreeMap::new(),
            length: 0,
            seek_pos: 0,
        };
        match fetch_range(&agent, &url, &format!("bytes=-{}", LAZY_FETCH_SIZE))? {
            RangeResponse::Partial {
                offset,
                total_len,
                data,
            } => {
                remote.length = total_len;
                remote.loaded.insert(offset, slurp(data)?);
            }
            RangeResponse::Complete(data) => {
                // Maybe should handle this better, like by falling back on using the
                // artifact cache?
                warn!("Server doesn't support range requests; fetching whole file into memory: {}", url.as_str());
                let buf = slurp(data)?;
                // unwrap safe because: converting usize to u64
                remote.length = buf.len().try_into().unwrap();
                remote.loaded.insert(0, buf);
            }
        }
        Ok(remote)
    }
}
