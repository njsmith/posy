use crate::prelude::*;

use std::io::Read;
use std::time::Duration;
use ureq::{Agent, AgentBuilder, Error::*, OrAnyStatus};

use super::user_agent::user_agent;

pub fn new_ureq_agent() -> Agent {
    AgentBuilder::new()
        .user_agent(&user_agent())
        // we handle redirects in the caching layer
        .redirects(0)
        .timeout_read(Duration::from_secs(15))
        .timeout_write(Duration::from_secs(15))
        .build()
}

const SLEEP_TIMES: &[u64] = &[250, 500, 1000, 2000, 4000]; // milliseconds
                                                           // Copied from pip/_internal/network/session.py
const RETRY_STATUS: &[u16] = &[500, 503, 520, 527];
// https://docs.rs/ureq/2.1.1/ureq/enum.ErrorKind.html
// This is my attempt to pick out the ones that seem (potentially) transient
use ureq::ErrorKind::*;
const RETRY_ERRORKIND: &[ureq::ErrorKind] =
    &[Dns, ConnectionFailed, TooManyRedirects, Io, ProxyConnect];

fn call_with_retry(
    req: ureq::Request,
) -> std::result::Result<ureq::Response, ureq::Error> {
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
            }
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

pub fn do_request_ureq(
    agent: &Agent,
    req: &http::Request<()>,
) -> Result<http::Response<impl Read>> {
    let mut ureq_req =
        agent.request_url(req.method().as_str(), &Url::parse(&req.uri().to_string())?);
    for (name, value) in req.headers().into_iter() {
        ureq_req = ureq_req.set(name.as_str(), std::str::from_utf8(value.as_bytes())?);
    }
    let ureq_response = call_with_retry(ureq_req).or_any_status()?;
    let mut response = http::Response::builder().status(ureq_response.status());
    for name in ureq_response.headers_names() {
        for value in ureq_response.all(&name) {
            response = response.header(&name, value);
        }
    }
    Ok(response.body(ureq_response.into_reader())?)
}
