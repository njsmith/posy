use crate::prelude::*;
use crate::cache::{Basket, Cache};
use zip::ZipArchive;

use super::net::ReadPlusSeek;

pub struct LazyZipArtifact {
    cache: Cache,
    url: Url,
    zip: ZipArchive<Box<dyn ReadPlusSeek>>,
}

impl LazyZipArtifact {
    pub fn get(&self, path: &str) -> Result<{

    }
}
