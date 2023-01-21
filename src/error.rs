use crate::prelude::*;
use thiserror::Error;

#[derive(Error, Debug)]
pub enum PosyError {
    #[error("no compatible binaries found for {name} {version}")]
    NoCompatibleBinaries { name: String, version: Version },
    #[error("no compatible pybis found for requirement and platform")]
    NoPybiFound,
    #[error("remote file does not support range requests")]
    LazyRemoteFileNotSupported,
}
