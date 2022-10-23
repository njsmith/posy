use crate::prelude::*;
use elsa::FrozenMap;
use indexmap::IndexMap;
use std::path::Path;

use super::cache::CacheDir;
use super::http::{Http, CacheMode, NotCached};
use super::simple_api::{fetch_simple_api, pack_by_version, ArtifactInfo};

static NO_ARTIFACTS: [ArtifactInfo; 0] = [];

pub struct PackageDB {
    http: Http,
    metadata_cache: CacheDir,
    wheel_cache: CacheDir,
    index_urls: Vec<Url>,

    // memo table to make sure we're internally consistent within a single invocation,
    // and to let us return references instead of copying everything everywhere
    artifacts: FrozenMap<PackageName, Box<IndexMap<Version, Vec<ArtifactInfo>>>>,
}

impl PackageDB {
    pub fn new(index_urls: &[Url], cache_path: &Path) -> PackageDB {
        let http_cache = CacheDir::new(&cache_path.join("http"));
        let hash_cache = CacheDir::new(&cache_path.join("by-hash"));
        PackageDB {
            http: Http::new(http_cache, hash_cache),
            metadata_cache: CacheDir::new(&cache_path.join("metadata")),
            wheel_cache: CacheDir::new(&cache_path.join("local-wheels")),
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
                    &self.http,
                    &index_url.join(&format!("/{}/", p.normalized()))?,
                )?;
                pack_by_version(pi, &mut packed)?;
            }

            // sort into descending order by version
            packed.sort_unstable_by(|v1, _, v2, _| v2.cmp(v1));

            Ok(self.artifacts.insert(p.clone(), Box::new(packed)))
        }
    }

    fn metadata_from_cache(&self, ai: &ArtifactInfo) -> Option<Vec<u8>> {
        match &ai.hash {
            Some(hash) => {
                let entry = self.metadata_cache.get_if_exists(&hash)?;
                let mut reader = entry.reader()?;
                slurp(&mut reader).ok()
            }
            None => None,
        }
    }

    fn put_metadata_in_cache(&self, ai: &ArtifactInfo, blob: &[u8]) -> Result<()> {
        if let Some(hash) = &ai.hash {
            let handle = self.metadata_cache.get(hash)?;
            handle.begin()?.write_all(&blob)?;
        }
        Ok(())
    }

    fn open_artifact<T>(&self, ai: &ArtifactInfo, body: Box<dyn ReadPlusSeek>) -> Result<T>
    where
        T: Artifact,
        ArtifactName: ArtifactNameUnwrap<T::Name>,
        T::Name: Clone,
    {
        let artifact_name = ArtifactNameUnwrap::<T::Name>::try_borrow(&ai.name)
            .unwrap()
            .clone();
        T::new(artifact_name, body)
    }

    pub fn get_metadata<'a, T>(
        &self,
        artifacts: &[&'a ArtifactInfo],
    ) -> Result<(&'a ArtifactInfo, T::Metadata)>
    where
        T: BinaryArtifact,
        ArtifactName: ArtifactNameUnwrap<T::Name>,
        T::Name: Clone,
    {
        let matching = || {
            artifacts.iter().filter(|ai| {
                ArtifactNameUnwrap::<T::Name>::try_borrow(&ai.name).is_some()
            })
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
                }
            }
        }

        // XX TODO: sdist support: check for already-built wheels

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
            let body = self.http.get_lazy(&ai.url)?;
            let artifact = self.open_artifact::<T>(ai, body)?;
            let (blob, metadata) = artifact.metadata()?;
            self.put_metadata_in_cache(ai, &blob)?;
            return Ok((ai, metadata));
        }

        // XX TODO: sdist support: fetch an sdist and build a wheel
        bail!("couldn't find any metadata for {artifacts:?}");
    }

    fn _get_artifact<T>(&self, ai: &ArtifactInfo, cache_mode: CacheMode) -> Result<T>
    where
        T: Artifact,
        ArtifactName: ArtifactNameUnwrap<T::Name>,
        T::Name: Clone,
    {
        let body = self.http.get_hashed(&ai.url, ai.hash.as_ref(), cache_mode)?;
        self.open_artifact::<T>(ai, body)
    }

    pub fn get_artifact<T>(&self, ai: &ArtifactInfo) -> Result<T>
    where
        T: Artifact,
        ArtifactName: ArtifactNameUnwrap<T::Name>,
        T::Name: Clone,
    {
        self._get_artifact(ai, CacheMode::Default)
    }
}
