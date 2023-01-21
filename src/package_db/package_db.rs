use crate::env::EnvForest;
use crate::prelude::*;
use elsa::FrozenMap;
use indexmap::IndexMap;
use std::path::Path;

use super::http::{CacheMode, Http, NotCached};
use super::simple_api::{fetch_simple_api, pack_by_version, ArtifactInfo};
use crate::kvstore::{KVDirStore, KVFileStore};

static NO_ARTIFACTS: [ArtifactInfo; 0] = [];

pub struct PackageDB<'a> {
    http: Http,
    metadata_cache: KVFileStore,
    index_urls: Vec<Url>,

    pub(super) wheel_cache: KVDirStore,
    pub(super) build_forest: &'a EnvForest,
    pub(super) build_store: &'a KVDirStore,

    // memo table to make sure we're internally consistent within a single invocation,
    // and to let us return references instead of copying everything everywhere
    artifacts: FrozenMap<PackageName, Box<IndexMap<Version, Vec<ArtifactInfo>>>>,
}

impl<'db> PackageDB<'db> {
    pub fn new(
        index_urls: &[Url],
        cache_path: &Path,
        build_forest: &'db EnvForest,
        build_store: &'db KVDirStore,
    ) -> Result<PackageDB<'db>> {
        let http_cache = KVFileStore::new(&cache_path.join("http"))?;
        let hash_cache = KVFileStore::new(&cache_path.join("by-hash"))?;
        Ok(PackageDB {
            http: Http::new(http_cache, hash_cache),
            metadata_cache: KVFileStore::new(&cache_path.join("metadata"))?,
            wheel_cache: KVDirStore::new(&cache_path.join("local-wheels"))?,
            index_urls: index_urls.into(),
            build_forest,
            build_store,
            artifacts: Default::default(),
        })
    }

    pub fn artifacts_for_version(
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
        context!("Looking up available files for {}", p.as_given());
        if let Some(cached) = self.artifacts.get(&p) {
            Ok(cached)
        } else {
            let mut packed: IndexMap<Version, Vec<ArtifactInfo>> = Default::default();

            for index_url in self.index_urls.iter() {
                let maybe_pi = fetch_simple_api(
                    &self.http,
                    &index_url.join(&format!("{}/", p.normalized()))?,
                )?;
                if let Some(pi) = maybe_pi {
                    pack_by_version(pi, &mut packed)?;
                }
            }

            // sort artifact-infos (arbitrarily) by name, just to have a consistent
            // order from run-to-run (and make resolution output more consistent)
            for artifact_infos in packed.values_mut() {
                artifact_infos.sort_by(|a, b| a.name.cmp(&b.name));
            }
            // sort into descending order by version
            packed.sort_unstable_by(|v1, _, v2, _| v2.cmp(v1));

            Ok(self.artifacts.insert(p.clone(), Box::new(packed)))
        }
    }

    fn metadata_from_cache(&self, ai: &ArtifactInfo) -> Option<Vec<u8>> {
        slurp(&mut self.metadata_cache.get(&ai.hash.as_ref()?)?).ok()
    }

    fn put_metadata_in_cache(&self, ai: &ArtifactInfo, blob: &[u8]) -> Result<()> {
        if let Some(hash) = &ai.hash {
            self.metadata_cache
                .get_or_set(&hash, |w| Ok(w.write_all(&blob)?))?;
        }
        Ok(())
    }

    fn open_artifact<T>(
        &self,
        ai: &ArtifactInfo,
        body: Box<dyn ReadPlusSeek>,
    ) -> Result<T>
    where
        T: Artifact,
    {
        let artifact_name = ai
            .name
            .inner_as::<T::Name>()
            .ok_or_else(|| {
                eyre!("{} is not a {}", ai.name, std::any::type_name::<T>())
            })?
            .clone();
        Ok(T::new(artifact_name, body)?)
    }

    pub fn get_metadata<'a, T, B>(
        &self,
        artifacts: &'a [B],
        builder: Option<&T::Builder<'_>>,
    ) -> Result<(&'a ArtifactInfo, T::Metadata)>
    where
        B: std::borrow::Borrow<ArtifactInfo>,
        T: BinaryArtifact,
    {
        let matching = || {
            artifacts
                .iter()
                .map(|ai| ai.borrow())
                .filter(|ai| ai.is::<T>())
        };

        // have we cached any of these artifacts' metadata before?
        for ai in matching() {
            if let Some(cm) = self.metadata_from_cache(ai) {
                return Ok((ai, T::parse_metadata(cm.as_slice())?));
            }
        }

        // have we cached any of the artifacts themselves?
        for ai in matching() {
            let res = self._get_artifact::<T>(ai, CacheMode::OnlyIfCached);
            match res {
                Ok(artifact) => {
                    let (blob, metadata) = artifact.metadata()?;
                    self.put_metadata_in_cache(ai, &blob)?;
                    return Ok((ai, metadata));
                }
                Err(err) => match err.downcast_ref::<NotCached>() {
                    Some(_) => continue,
                    None => return Err(err),
                },
            }
        }

        // okay, we don't have it locally; gotta actually hit the network.

        // XX TODO: PEP 658 support
        // also, extra complication: when dist_info_metadata is available, we might also
        // have a hash for the metadata. Should we check it, and how does that interact
        // with caching? I guess that when TUF arrives we'll need to look carefully to
        // make sure all that data we fetch is TUF-protected, and in the mean time we're
        // relying on the index+https being trustworthy anyway -- both to give us the
        // hashes, and also for the lazy_remote_file path that can't validate any
        // hashes. (But then why are we validating hashes when we download artifacts? I
        // guess it's really only important when *installing* where we want to confirm
        // hashes haven't changed since someone else resolved, not *resolving*, where we
        // collect the hashes in the first place, and this function is on the resolve
        // path?)
        //
        // for ai in matching() {
        //     if ai.dist_info_metadata.available {
        //         todo!()
        //     }
        // }

        // try pulling the metadata out of a remote wheel, and cache it for later
        for ai in matching() {
            let body = self.http.get_lazy(ai)?;
            let artifact = self.open_artifact::<T>(ai, body)?;
            let (blob, metadata) = artifact.metadata()?;
            self.put_metadata_in_cache(ai, &blob)?;
            return Ok((ai, metadata));
        }

        // Finally, if all else fails, see if we can fetch an sdist and built it
        if let Some(builder) = builder {
            // don't use matching() here because that filters for binary artifacts
            for ai in artifacts.iter().map(|b| b.borrow()) {
                if let Some(result) = T::locally_built_metadata(&builder, &ai) {
                    let (blob, metadata) = result?;
                    self.put_metadata_in_cache(ai, &blob)?;
                    return Ok((ai, metadata));
                }
            }
        }

        bail!(
            "couldn't find any {} metadata for {:#?}",
            std::any::type_name::<T>(),
            artifacts.iter().map(|ai| ai.borrow()).collect::<Vec<_>>()
        );
    }

    fn _get_artifact<T>(&self, ai: &ArtifactInfo, cache_mode: CacheMode) -> Result<T>
    where
        T: Artifact,
    {
        let body = self
            .http
            .get_hashed(&ai.url, ai.hash.as_ref(), cache_mode)?;
        self.open_artifact::<T>(ai, body)
    }

    pub fn get_artifact<T>(&self, ai: &ArtifactInfo) -> Result<T>
    where
        T: Artifact,
    {
        self._get_artifact(ai, CacheMode::Default)
    }

    pub fn get_locally_built_binary<T: BinaryArtifact>(
        &self,
        ai: &ArtifactInfo,
        builder: &T::Builder<'_>,
        platform: &T::Platform,
    ) -> Option<Result<T>> {
        T::locally_built_binary(&builder, &ai, &platform)
    }
}
