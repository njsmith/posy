use crate::prelude::*;

use super::_http::{CacheMode, HttpInner};
use std::cmp;
use std::collections::BTreeMap;
use std::io::{self, Read, Seek, SeekFrom};

// semi-arbitrary, but ideally should be large enough to catch all the zip index +
// dist-info data at the end of common wheel files
const LAZY_FETCH_SIZE: u64 = 10_000;

pub struct LazyRemoteFile {
    http: Rc<HttpInner>,
    url: Url,
    loaded: BTreeMap<u64, Vec<u8>>,
    length: u64,
    seek_pos: u64,
}

impl Seek for LazyRemoteFile {
    fn seek(&mut self, pos: SeekFrom) -> io::Result<u64> {
        let LazyRemoteFile {
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
                "invalid seek to a negative or overflowing position",
            )),
        }
    }
}

enum RangeResponse {
    NotSatisfiable {
        total_len: u64,
    },
    Partial {
        offset: u64,
        total_len: u64,
        data: Box<dyn Read>,
    },
    Complete(Box<dyn Read>),
}

fn fetch_range(
    http: &HttpInner,
    method: &str,
    url: &Url,
    range_header: &str,
) -> Result<RangeResponse> {
    context!("Attempting range read on {url}");
    // The full syntax has a bunch of possibilities that this doesn't account for:
    //   https://datatracker.ietf.org/doc/html/rfc7233#section-4.2
    // but this is the only format that's actually *useful* to us.
    static CONTENT_RANGE_RE: Lazy<regex::bytes::Regex> = Lazy::new(|| {
        regex::bytes::Regex::new(r"^bytes ([0-9]+)-[0-9]+/([0-9]+)$").unwrap()
    });
    static CONTENT_RANGE_LEN_ONLY_RE: Lazy<regex::bytes::Regex> =
        Lazy::new(|| regex::bytes::Regex::new(r"^bytes [^/]*/([0-9]+)$").unwrap());

    let request = http::Request::builder()
        .method(method)
        .uri(url.as_str())
        .header("Range", range_header)
        .body(())?;
    let response = http.request(request, CacheMode::NoStore)?;

    fn str_capture<'a>(c: &'a regex::bytes::Captures, g: usize) -> Result<&'a str> {
        Ok(std::str::from_utf8(c.get(g).unwrap().as_bytes())?)
    }

    Ok(match response.status().as_u16() {
        // 206 Partial Content
        206 => {
            match response.headers().get("Content-Range") {
                None => bail!("range response is missing Content-Range"),
                Some(content_range) => {
                    match CONTENT_RANGE_RE.captures(content_range.as_bytes()) {
                        None => bail!("failed to parse Content-Range"),
                        Some(captures) => {
                            // unwraps safe because groups always match a valid int
                            let offset: u64 = str_capture(&captures, 1)?.parse()?;
                            let total_len: u64 = str_capture(&captures, 2)?.parse()?;
                            RangeResponse::Partial {
                                offset,
                                total_len,
                                data: Box::new(response.into_body()),
                            }
                        }
                    }
                }
            }
        }
        // 416 Range Not Satisfiable
        // e.g. because we requested past the end or beginning (particularly likely on
        // our first fetch, where we don't know how large the file is)
        416 => match response.headers().get("Content-Range") {
            None => bail!("416 response is missing Content-Range"),
            Some(content_range) => {
                match CONTENT_RANGE_LEN_ONLY_RE.captures(content_range.as_bytes()) {
                    None => bail!("failed to parse 416 Content-Range"),
                    Some(captures) => {
                        let total_len: u64 = str_capture(&captures, 1)?.parse()?;
                        RangeResponse::NotSatisfiable { total_len }
                    }
                }
            }
        },
        // 200 Ok -> server doesn't like Range: requests and is just sending the full
        // data
        200 => RangeResponse::Complete(Box::new(response.into_body())),
        status => bail!("expected 200 or 206 HTTP response, not {}", status),
    })
}

impl LazyRemoteFile {
    fn load_range(&mut self, offset: u64, length: u64) -> Result<()> {
        match fetch_range(
            &self.http,
            "GET",
            &self.url,
            &format!("bytes={}-{}", offset, offset.saturating_add(length) - 1),
        )? {
            RangeResponse::NotSatisfiable { .. } => {
                bail!("Server didn't like my range request and I don't know why");
            }
            RangeResponse::Partial {
                offset, mut data, ..
            } => {
                self.loaded.insert(offset, slurp(&mut data)?);
                Ok(())
            }
            RangeResponse::Complete(_) => {
                bail!("server abruptly stopped understanding range requests?!?")
            }
        }
    }
}

impl Read for LazyRemoteFile {
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
                        if slide < loaded_data.len() {
                            let usable_loaded_data = &loaded_data[slide..];
                            let len = cmp::min(buf.len(), usable_loaded_data.len());
                            buf[..len].copy_from_slice(&usable_loaded_data[..len]);
                            Some(len)
                        } else {
                            None
                        }
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

        let bytes_wanted =
            cmp::min(buf.len() as u64, self.length.saturating_sub(self.seek_pos));
        if bytes_wanted == 0 {
            return Ok(0);
        }
        // maybe we already have it in cache?
        if let Some(len) = copy_loaded(self.seek_pos, &self.loaded, buf) {
            self.seek_pos = self.seek_pos.saturating_add(fix_err(len.try_into())?);
            return Ok(len);
        }
        // otherwise, we need to fetch + fill in the cache
        // first find the empty gap around our current position
        let gap_start = match self.loaded.range(..=self.seek_pos).next_back() {
            Some((loaded_offset, loaded_data)) => {
                loaded_offset + (loaded_data.len() as u64)
            }
            None => 0,
        };
        let gap_end = match self.loaded.range(self.seek_pos + 1..).next() {
            Some((loaded_offset, _)) => *loaded_offset,
            None => self.length,
        };
        let fetch_start = if gap_end - self.seek_pos < LAZY_FETCH_SIZE {
            gap_end.saturating_sub(LAZY_FETCH_SIZE)
        } else {
            self.seek_pos
        };
        let fetch_end = fetch_start + LAZY_FETCH_SIZE;
        let fetch_start = fetch_start.clamp(gap_start, gap_end);
        let fetch_end = fetch_end.clamp(gap_start, gap_end);
        fix_err(self.load_range(fetch_start, fetch_end - fetch_start))?;
        // now it's definitely in cache
        if let Some(len) = copy_loaded(self.seek_pos, &self.loaded, buf) {
            self.seek_pos = self.seek_pos.saturating_add(fix_err(len.try_into())?);
            return Ok(len);
        }
        unreachable!("and you may ask yourself, well, how did I get here?")
    }
}

impl LazyRemoteFile {
    pub fn new(http: Rc<HttpInner>, url: &Url) -> Result<LazyRemoteFile> {
        context!("Fetching metadata for {url}");
        // Instead of doing a HEAD request to get the length, it would be more efficient
        // to fetch the end of the file and the length in a single Range: request
        // (because we know that the first thing we'll do with a LazyRemoteFile is read
        // the zip index at the end of the file). This is supposed to be possible with
        // 'Range: bytes=-1234' syntax, but unfortunately PyPI's Fastly configuration
        // changed in Dec 2022 to break this functionality:
        //
        //    https://github.com/pypi/warehouse/issues/12823
        //
        // If this gets fixed we could switch to doing a GET request instead.
        let length = match fetch_range(&http, "HEAD", url, "bytes=0-1")? {
            RangeResponse::NotSatisfiable { total_len } => total_len,
            RangeResponse::Partial { total_len, .. } => total_len,
            RangeResponse::Complete(_) => Err(PosyError::LazyRemoteFileNotSupported)?,
        };
        Ok(LazyRemoteFile {
            http,
            url: url.clone(),
            loaded: BTreeMap::new(),
            length,
            seek_pos: 0,
        })
    }
}

#[cfg(test)]
mod test {
    use std::fs::File;
    use std::io::prelude::*;

    use crate::kvstore::KVFileStore;

    use super::*;

    fn tmp_http() -> (tempfile::TempDir, Rc<HttpInner>) {
        let caches = tempfile::tempdir().unwrap();
        let http = HttpInner::new(
            KVFileStore::new(&caches.path().join("http")).unwrap(),
            KVFileStore::new(&caches.path().join("hashed")).unwrap(),
        );
        (caches, Rc::new(http))
    }

    #[test]
    fn test_fetch_range() {
        let tempdir = tempfile::tempdir().unwrap();
        let server = crate::test_util::StaticHTTPServer::new(tempdir.path());
        {
            let mut f = File::create(tempdir.path().join("blobby")).unwrap();
            f.write_all(&[0; 1000]).unwrap();
            f.write_all(&[1; 1000]).unwrap();
            f.write_all(&[2; 1000]).unwrap();
        }
        let url = server.url("blobby");
        let (_caches, http) = tmp_http();

        let rr = fetch_range(&http, "GET", &url, "bytes=900-999").unwrap();
        if let RangeResponse::Partial {
            offset,
            total_len,
            mut data,
        } = rr
        {
            assert_eq!(offset, 900);
            assert_eq!(total_len, 3000);
            let buf = slurp(&mut data).unwrap();
            assert_eq!(buf.as_slice(), [0; 100]);
        } else {
            panic!();
        }

        let rr = fetch_range(&http, "GET", &url, "bytes=1010-1020").unwrap();
        if let RangeResponse::Partial {
            offset,
            total_len,
            mut data,
        } = rr
        {
            assert_eq!(offset, 1010);
            assert_eq!(total_len, 3000);
            let buf = slurp(&mut data).unwrap();
            assert_eq!(buf.as_slice(), [1; 11]);
        } else {
            panic!();
        }

        // If the server doesn't understand our Range: header, falls back on sending the
        // whole file
        let rr = fetch_range(&http, "GET", &url, "octets=1010-1020").unwrap();
        if let RangeResponse::Complete(mut data) = rr {
            let buf = slurp(&mut data).unwrap();
            assert_eq!(buf.len(), 3000);
        } else {
            panic!();
        }

        // Fetching an invalid range at least tells us what the valid range is
        let rr = fetch_range(&http, "GET", &url, "bytes=10000-20000").unwrap();
        if let RangeResponse::NotSatisfiable { total_len } = rr {
            assert_eq!(total_len, 3000);
        } else {
            panic!();
        }

        // Error propagation happens
        let res = fetch_range(&http, "GET", &server.url("missing"), "bytes=100-200");
        assert!(res.is_err());
    }

    #[test]
    fn test_lazy_remote_file_explicit() {
        let tempdir = tempfile::tempdir().unwrap();
        let server = crate::test_util::StaticHTTPServer::new(tempdir.path());
        {
            let mut f = File::create(tempdir.path().join("blobby")).unwrap();
            f.write_all(&[0; 13000]).unwrap();
            f.write_all(&[1; 13000]).unwrap();
            f.write_all(&[2; 13000]).unwrap();
        }
        let (_caches, http) = tmp_http();

        let mut lazy = LazyRemoteFile::new(http, &server.url("blobby")).unwrap();

        assert_eq!(lazy.seek(SeekFrom::End(0)).unwrap(), 3 * 13000);
        assert_eq!(lazy.seek(SeekFrom::Start(0)).unwrap(), 0);

        lazy.seek(SeekFrom::End(-10)).unwrap();
        let mut buf = [0xff; 1000];
        assert_eq!(lazy.read(&mut buf).unwrap(), 10);
        assert_eq!(buf[..10], [2; 10]);

        lazy.seek(SeekFrom::Start(5000)).unwrap();
        let mut buf = [0xff; 1000];
        lazy.read_exact(&mut buf).unwrap();
        assert_eq!(buf, [0; 1000]);

        lazy.seek(SeekFrom::Start(12900)).unwrap();
        let mut buf = [0xff; 1000];
        lazy.read_exact(&mut buf).unwrap();
        let mut expected: [u8; 1000] = [0xff; 1000];
        expected[..100].fill(0);
        expected[100..].fill(1);
        assert_eq!(buf, expected);
    }

    #[test]
    fn test_lazy_remote_file_randomized() {
        use std::iter::repeat_with;
        const BLOBBY_SIZE: u64 = 1_000_000;

        let tempdir = tempfile::tempdir().unwrap();
        let server = crate::test_util::StaticHTTPServer::new(tempdir.path());
        {
            let mut f = File::create(tempdir.path().join("blobby")).unwrap();
            let rng = fastrand::Rng::with_seed(0);
            let data: Vec<u8> = repeat_with(|| rng.u8(..))
                .take(BLOBBY_SIZE as usize)
                .collect();
            f.write_all(&data).unwrap();
        }
        let (_caches, http) = tmp_http();

        // Reads the given number of bytes, unless it hits EOF, in which case it reads
        // everything available
        fn read_exactish<T: Read + Seek>(
            r: &mut T,
            pos: SeekFrom,
            count: usize,
        ) -> Vec<u8> {
            r.seek(pos).unwrap();
            let mut buf: Vec<u8> = Vec::new();
            buf.resize(count, 0);
            match r.read_exact(&mut buf) {
                Ok(_) => buf,
                Err(ref e) if e.kind() == io::ErrorKind::UnexpectedEof => {
                    r.seek(pos).unwrap();
                    r.read_to_end(&mut buf).unwrap();
                    buf
                }
                other => {
                    other.unwrap();
                    unreachable!()
                }
            }
        }

        for seed in 0..5 {
            let rng = fastrand::Rng::with_seed(seed);
            let mut f = File::open(tempdir.path().join("blobby")).unwrap();
            let mut lazy =
                LazyRemoteFile::new(http.clone(), &server.url("blobby")).unwrap();

            for _ in 0..100 {
                let seek = if rng.bool() {
                    SeekFrom::Start(rng.u64(..BLOBBY_SIZE))
                } else {
                    SeekFrom::End(rng.i64(-(BLOBBY_SIZE as i64)..=0))
                };

                let read_size = rng.usize(1_000..15_000);
                let f_buf = read_exactish(&mut f, seek, read_size);
                let lazy_buf = read_exactish(&mut lazy, seek, read_size);

                assert_eq!(f_buf, lazy_buf);
            }
        }
    }
}
