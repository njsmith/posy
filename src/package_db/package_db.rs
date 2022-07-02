use crate::prelude::*;
use elsa::FrozenMap;
use indexmap::IndexMap;
use std::io::{self, Read, Seek};
use std::path::Path;
use ureq::Agent;

use super::simple_api::{fetch_simple_api, pack_by_version, ArtifactInfo};

static NO_ARTIFACTS: [ArtifactInfo; 0] = [];

pub struct PackageDB {
    agent: Agent,
    disk_cache: super::cache::PackageCache,
    index_urls: Vec<Url>,

    // memo table to make sure we're internally consistent within a single invocation,
    // and to let us return references instead of copying everything everywhere
    artifacts: FrozenMap<PackageName, Box<IndexMap<Version, Vec<ArtifactInfo>>>>,
}

impl PackageDB {
    pub fn new(agent: &Agent, index_urls: &[Url], cache_path: &Path) -> PackageDB {
        PackageDB {
            disk_cache: super::cache::PackageCache::new(cache_path),
            agent: agent.clone(),
            index_urls: index_urls.into(),
            artifacts: Default::default(),
        }
    }

    pub fn artifacts_for_release(
        &self,
        p: &PackageName,
        v: &Version,
    ) -> Result<&[ArtifactInfo]> {
        if let Some(artifacts) = self.available_artifacts(p)?.get(v) {
            Ok(&artifacts)
        } else {
            Ok(&NO_ARTIFACTS)
        }
    }

    // always sorted from most recent to least recent
    pub fn available_artifacts(
        &self,
        p: &PackageName,
    ) -> Result<&IndexMap<Version, Vec<ArtifactInfo>>> {
        if let Some(cached) = self.artifacts.get(&p) {
            Ok(cached)
        } else {
            let mut packed: IndexMap<Version, Vec<ArtifactInfo>> = Default::default();

            for index_url in self.index_urls.iter() {
                let pi = fetch_simple_api(
                    &self.agent,
                    &index_url.join(&format!("/{}/", p.normalized()))?,
                    &self.disk_cache.index_pages,
                )?;
                pack_by_version(pi, &mut packed)?;
            }

            // sort into descending order by version
            packed.sort_unstable_by(|v1, _, v2, _| v2.cmp(v1));

            Ok(self.artifacts.insert(p.clone(), Box::new(packed)))
        }
    }

    pub fn metadata(&self, artifacts: &[&ArtifactInfo]) -> Result<CoreMetadata> {
        todo!()
    }

    pub fn get_artifact(
        &self,
        url: &Url,
        hash: &ArtifactHash,
    ) -> Result<impl Read + Seek> {
        todo!()
    }
}
