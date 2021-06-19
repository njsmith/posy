mod lazy_get;

use crate::prelude::*;

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::time::Duration;
use ureq::Agent;

use super::cache::{Basket, Cache};

// Applied to all requests
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
// Used for fetching API result pages, etc.
const SMALL_TIMEOUT: Duration = Duration::from_secs(60);
// Used for fetching potentially-large artifacts
const LARGE_TIMEOUT: Duration = Duration::from_secs(60 * 60);

pub struct Net {
    agent: Agent,
    cache: Cache,
}

impl Net {
    pub fn get_etagged(url: &Url) -> Result<Vec<u8>> {
        #[derive(Serialize, Deserialize, Debug)]
        struct CacheEntry<'a> {
            etag: &'a [u8],
            body: &'a [u8],
        }

        // check cache + revalidate
        // else fill cache
        todo!();
    }

    pub fn get_artifact(
        url: &Url,
        hash: super::package_index::ArtifactHash,
    ) -> Result<impl Read + Seek> {
        // check cache
        // else fill cache
        // either way check hash
        // then return file handle

        todo!();
        Ok(File::open("")?)
    }

    pub fn get_lazy_artifact(&self, url: &Url) -> Result<impl Read + Seek> {
        Ok(lazy_get::LazyGet::new(&self.agent, &url)?)
    }
}
