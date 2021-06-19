use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};

use crate::prelude::*;

pub fn from_commented_json<T>(input: &str) -> T
where
    T: serde::de::DeserializeOwned,
{
    static COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"#.*").unwrap());

    let replaced = COMMENT.replace_all(input, "");
    serde_json::from_str(&replaced).unwrap()
}

pub struct StaticHTTPServer {
    address: SocketAddr,
    tx: Option<tokio::sync::oneshot::Sender<()>>,
    runtime: tokio::runtime::Runtime,
    join_handle: Option<tokio::task::JoinHandle<()>>,
}

impl StaticHTTPServer {
    /// Spins up a static file server for testing against.
    ///
    /// Path can be absolute, or relative to the project root.
    pub fn new<P: AsRef<Path>>(path: P) -> StaticHTTPServer {
        let mut actual_path: PathBuf = env!("CARGO_MANIFEST_DIR").parse().unwrap();
        actual_path.push(path);
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let _guard = runtime.enter();
        let (tx, rx) = tokio::sync::oneshot::channel();
        let (address, server) = warp::serve(warp::fs::dir(actual_path))
            .bind_with_graceful_shutdown(([127, 0, 0, 1], 0), async {
                rx.await.ok();
            });
        let join_handle = runtime.spawn(server);

        StaticHTTPServer {
            address,
            tx: Some(tx),
            runtime,
            join_handle: Some(join_handle),
        }
    }

    pub fn url(&self, path: &str) -> Url {
        let mut url =
            Url::parse(&format!("http://127.0.0.1:{}", self.address.port())).unwrap();
        url.set_path(&path);
        url
    }
}

impl Drop for StaticHTTPServer {
    fn drop(&mut self) {
        self.tx.take().unwrap().send(()).unwrap();
        self.runtime
            .block_on(self.join_handle.take().unwrap())
            .unwrap();
        println!("server closed");
    }
}

mod test {
    use super::*;

    #[test]
    fn test_static_http_server() {
        let server = StaticHTTPServer::new("test-data");
        let agent = ureq::Agent::new();
        let response = agent
            .request_url("GET", &server.url("basic.json"))
            .call()
            .unwrap();
        assert_eq!(response.status(), 200);
        let data: HashMap<String, u32> = response.into_json().unwrap();
        assert_eq!(data.get("hi"), Some(&1));
    }
}
