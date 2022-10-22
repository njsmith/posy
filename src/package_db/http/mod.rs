mod http;
pub mod user_agent;
pub mod ureq_glue;
pub mod lazy_remote_file;

pub use self::http::{Http, HttpInner, CacheMode, NotCached};
pub use self::lazy_remote_file::LazyRemoteFile;
use super::cache;
