use std::time::Duration;
use ureq::Error::*;

// calls:
// - get_etagged, looking for 304 or 200
// - get_artifact, okay with any success really
// - fetch_range: wants to handle 416 specially

const SLEEP_TIMES: &[u64] = &[250, 500, 1000, 2000, 4000]; // milliseconds
// Copied from pip/_internal/network/session.py
const RETRY_STATUS: &[u16] = &[500, 503, 520, 527];
// https://docs.rs/ureq/2.1.1/ureq/enum.ErrorKind.html
// This is my attempt to pick out the ones that seem (potentially) transient
use ureq::ErrorKind::*;
const RETRY_ERRORKIND: &[ureq::ErrorKind] = &[
    Dns, ConnectionFailed, TooManyRedirects, Io, ProxyConnect,
];


pub fn call_with_retry(
    req: ureq::Request,
) -> std::result::Result<ureq::Response, ureq::Error> {
    // We preserve ureq's native Result/Error types, so users can e.g. customize how
    // they handle 4xx responses.
    //
    // Pip's retry logic is in
    //    pip/_internal/network/session.py
    //    urllib3/util/retry.py
    // - retry on codes 500, 503, 520, 527
    // - sleep time is 0.25 * 2 ** (retries - 1)
    //   so 0.25, 0.50, etc., with 120 as max
    // - it also respects the Retry-After header
    // - also retries on connect-related errors, read errors, "other errors"
    // - default 5 attempts, can be overridden by cmdline option

    let mut iterator = SLEEP_TIMES.iter();
    loop {
        let this_req = req.clone();
        let result = this_req.call();
        match &result {
            Ok(_) => return result,
            Err(Status(status, _)) => {
                if !RETRY_STATUS.contains(status) {
                    return result;
                }
            },
            Err(err @ Transport(_)) => {
                if !RETRY_ERRORKIND.contains(&err.kind()) {
                    return result;
                }
            }
        }
        match iterator.next() {
            Some(sleep_time) => std::thread::sleep(Duration::from_millis(*sleep_time)),
            None => return result,
        }
    }
}
