use std::{fs, path::PathBuf};

use crate::{
    env::EnvForest,
    kvstore::{KVDirLock, KVDirStore},
    package_db::PackageDB,
    prelude::*,
    tree::WriteTreeFS,
};

use super::ArtifactInfo;

#[derive(Clone)]
pub struct BuildWheelContext<'a> {
    // not sure if we need db here, b/c I think it's already being passed through
    // everywhere that this will be? when we create a child context, that always happens
    // inside a db method...
    db: &'a PackageDB,
    env_forest: &'a EnvForest,
    build_store: &'a KVDirStore,
    // XX TODO
    //build_constraints: Vec<UserRequirement>,
    in_flight: Vec<&'a PackageName>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pep517Goal {
    WheelMetadata,
    Wheel,
}

enum Pep517Succeeded {
    WheelMetadata {
        handle: KVDirLock,
        dist_info: PathBuf,
    },
    Wheel {
        handle: KVDirLock,
        wheel: PathBuf,
    },
}

// take lock on sdist dir
// unpack into sdist/
// have sibling dirs that persistently contain the state for each step that's been
// completed:
// - get_requires_for_build_wheel/result.json
// - prepare_metadata_for_build_wheel/ <- what if it's not defined?
// - build_wheel/
//
// goal is that we can always tell the current state or resume from where we left off,
// using just the directory

impl<'a> BuildWheelContext<'a> {
    pub fn new(
        db: &'a PackageDB,
        env_forest: &'a EnvForest,
        build_dir: &'a KVDirStore,
    ) -> BuildWheelContext<'a> {
        BuildWheelContext {
            db,
            env_forest,
            build_store: build_dir,
            in_flight: Vec::new(),
        }
    }

    pub fn child_context<'name, 'result>(
        &'a self,
        package: &'name PackageName,
    ) -> Result<BuildWheelContext<'result>>
    where
        'a: 'result,
        'name: 'result,
    {
        if let Some(idx) = self.in_flight.iter().position(|p| p == &package) {
            let bad = self.in_flight[idx..]
                .iter()
                .map(|p| format!("{} -> ", p.as_given()))
                .collect::<String>();
            bail!("build dependency loop: {bad}{}", package.as_given());
        }
        let mut child = self.clone();
        child.in_flight.push(&package);
        Ok(child)
    }

    fn pep517(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
        goal: Pep517Goal,
    ) -> Result<Pep517Succeeded> {
        let handle = self
            .build_store
            .lock(ai.hash.as_ref().ok_or(anyhow!("missing hash"))?)?;

        if !handle.exists() {
            let tempdir = handle.tempdir()?;
            let sdist = db.get_artifact::<Sdist>(&ai)?;
            sdist.unpack(&mut WriteTreeFS::new(tempdir.path().join("sdist")))?;
            const BUILD_FRONTEND_PY: &[u8] = include_bytes!("build-frontend.py");
            fs::write(tempdir.path().join("build-frontend.py"), BUILD_FRONTEND_PY)?;
            fs::rename(&tempdir.into_path(), &*handle)?;
        }

        let build_wheel = handle.join("build_wheel");
        let prepare_metadata_for_build_wheel =
            handle.join("prepare_metadata_for_build_wheel");
        loop {
            // If we have a wheel, we're definitely done, no matter what our goal was
            if build_wheel.exists() {
                let name =
                    String::from_utf8(fs::read(handle.join("build_wheel.out"))?)?;
                return Ok(Pep517Succeeded::Wheel {
                    handle,
                    wheel: build_wheel.join(name),
                });
            }
            // Or if our goal is metadata and we have it, we're done
            if goal == Pep517Goal::WheelMetadata
                && prepare_metadata_for_build_wheel.exists()
            {
                let name = String::from_utf8(fs::read(
                    handle.join("prepare_metadata_for_build_wheel.out"),
                )?)?;
                return Ok(Pep517Succeeded::WheelMetadata {
                    handle,
                    dist_info: prepare_metadata_for_build_wheel.join(name),
                });
            }
            // OK, we're not done. Turn the crank again.
            self.pep517_step(&db, &handle, goal)?;
        }
    }

    fn pep517_step(
        &self,
        db: &PackageDB,
        handle: &KVDirLock,
        goal: Pep517Goal,
    ) -> Result<()> {
        // get build-system.requires, build-system.build-backend,
        // build-system.backend-path
        // if we have requires_for_build_wheel, get those
        // construct a brief, resolve it, get an env
        // invoke the frontend, passing in the path and the goal
        // XX TODO need yet another higher-level layer that puts it in the cache
        // ALSO! should serialize our initial blueprint, and also our followup blueprint
        // if it exists (which should be based on our initial blueprint)
        // so that we can record how the wheel was built, and make sure we get the same
        // resolution for prepare_metadata_for_build_wheel and build_wheel (and also
        // skip the resolution step the second time)
        // also need to figure out how to thread pybi through (maybe treat it as
        // "pre-pinned"? and also mess with PybiPlatform to force it to be chosen if
        // possible? or just have an entry point to the wheel resolver specifically)
        todo!()
    }

    pub fn build_metadata(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
    ) -> Result<Option<WheelCoreMetadata>> {
        let name = match ai.name.inner_as::<SdistName>() {
            Some(value) => value,
            None => return Ok(None),
        };

        let context = self.child_context(&name.distribution)?;
        let sdist = db.get_artifact::<Sdist>(&ai)?;

        todo!()
    }

    pub fn build_wheel(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
    ) -> Result<Option<Wheel>> {
        todo!();
    }
}
