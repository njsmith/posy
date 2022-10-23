mod http;
pub mod lazy_remote_file;
pub mod ureq_glue;
pub mod user_agent;

pub use self::http::{CacheMode, Http, HttpInner, NotCached};
pub use self::lazy_remote_file::LazyRemoteFile;
use super::cache;
