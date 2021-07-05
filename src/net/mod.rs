mod lazy_remote_file;
mod retry;
mod user_agent;
mod net;

pub use user_agent::user_agent;
pub use net::{Net, SmallTextPage, ReadPlusSeek};
