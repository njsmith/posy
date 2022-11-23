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
    env_forest: &'a EnvForest,
    build_store: &'a KVDirStore,
    target: &'a PybiPlatform,
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

    // conceptually, fetching metadata from an sdist shouldn't require a specific
    // pybi... but realistically it does, esp. b/c of unpredictable python version
    // support
    // so we need to at least pass in a pybi (package, version) to metadata resolution
    //
    // for building the wheel itself, need at least (package, version), but beyond that
    // it's helpful to have exact pybi, and even target platform (for universal2 case)
    //
    // universal2 environments:
    // - if exporting, might want to export universal2 i.e. everything supports both.
    //   might be fine? platform_arch is a significant complication though
    // - when resolving against PybiPlatform::current_platform(), everything's fine
    // - if want to resolve or install an x86-64 environment on arm64 (or an arm64
    //   environment on arm64 from an x86-64 binary), then need to use `arch` or
    //   `posix_spawnattr_setarchpref_np` to override the child process arch.
    //
    //   this is the source of the `arch` command and demonstrates how to do an exec
    //   while switching arches:
    //   https://github.com/apple-oss-distributions/system_cmds/blob/system_cmds-950/arch.tproj/arch.c#L222
    //
    //   ...but also this is messy b/c a single unpacked pybi might have both, and might
    //   need separate bin/ dir trampolines to reliably select one or the other?
    //
    // maybe upgrade VersionHints to a first-class/public concept, and add "platform
    // hint" as a third kind of hint?
    // is this useful anywhere besides implicit sdist building?
    // ...or maybe an optional target platform in the brief?
    // eh. Brief::resolve takes a PybiPlatform so we have that info. maybe put
    // PybiPlatform into BuildWheelContext?
    // and PackageDB methods take a Option<&BuildWheelContext>?
    // ...oh but we want Brief::resolve to also take a BuildWheelContext, so we can
    // share the env_forset and build_store across separate invocations.
    // OH! but the PybiPlatform *does* change as you recurse -- for a second-order wheel
    // build, you want to get a wheel that can run on the first-order interpreter.
    //
    //
    // start at Brief::resolve -> pass in env_forest and build_store and real target
    //   pybiplatform
    // it picks pybi (based on Brief + platform)
    // then fetches metadata, passing in access to env_forest/build_store
    //     + chosen pybi version
    //     + platform
    //   db decides to get metadata from sdist:
    //     pushes into context stack for loop detection
    //     creates Brief with pybi set to contextual pybi package-name, no version
    //       constraint
    //       and resolves with pybi version hint
    //       + if platform is compatible with current platform, use that (or
    //         intersection?)
    //       otherwise use current_platform
    //

    //
    // "closest" version to 3.9.12rc3:
    // - 3.9.12rc3
    // - 3.9.12*
    // - 3.9.* (from highest to lowest)
    // - 3.*.* (from highest to lowest)
    //
    // so: first priority ==, then by number of matching [epoch]+version segments
    // probably this is the best bet anyway when hinting, so we can make it a general
    // thing
    //
    // (also if we currently have a pre-release, this should probably only enable other
    // pre-releases *of the same version*, in general?)

    // realistically, carefully choosing the platform/hints/pybi requirement will work
    // fine
    // so would passing in an explicit ArtifactInfo to resolve
    // just a question which is cleaner to imnplement probably

    fn pep517(
        &self,
        db: &PackageDB,
        ai: &ArtifactInfo,
        goal: Pep517Goal,
    ) -> Result<Pep517Succeeded> {
        let handle = self
            .build_store
            .lock(ai.hash.as_ref().ok_or(eyre!("missing hash"))?)?;

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
