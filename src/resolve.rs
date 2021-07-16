use crate::prelude::*;
use pubgrub::range::Range;
use pubgrub::report::DerivationTree;
use pubgrub::report::Reporter;
use pubgrub::solver::{Dependencies, DependencyConstraints};

use crate::package_index::{Artifact, PackageIndex};
use std::{borrow::Borrow, cell::RefCell, rc::Rc};

pub fn resolve(
    requirements: &Vec<UserRequirement>,
    env: &HashMap<String, String>,
    index: &PackageIndex,
    preferred_versions: &HashMap<PackageName, Version>,
    consider_prereleases: &dyn Fn(&PackageName) -> bool,
) -> Result<Vec<PinnedPackage>> {
    let state = PubgrubState {
        root_reqs: requirements,
        env,
        index,
        preferred_versions,
        consider_prereleases,
        releases: HashMap::new().into(),
        versions: HashMap::new().into(),
        metadata: HashMap::new().into(),
    };

    // XX this error reporting is terrible. It's a hack to work around PubGrubError not
    // being convertible to anyhow::Error, because anyhow::Error requires Send.
    let result = pubgrub::solver::resolve(&state, ResPkg::Root, ROOT_VERSION.clone());

    use pubgrub::error::PubGrubError::*;

    match result {
        Ok(solution) => Ok(solution
            .into_iter()
            .filter_map(|(pkg, v)| match pkg {
                ResPkg::Root => None,
                ResPkg::Package(_, Some(_)) => None,
                ResPkg::Package(name, None) => Some({
                    let (cm, provenance) = state
                        .metadata
                        .borrow_mut()
                        .remove(&(name.clone(), v.clone()))
                        .unwrap();

                    PinnedPackage {
                        name: name.clone(),
                        version: v.clone(),
                        known_artifacts: state
                            .releases
                            .borrow()
                            .get(&name)
                            .unwrap()
                            .get(&v)
                            .unwrap()
                            .iter()
                            .map(|artifact| {
                                (artifact.url.clone(), artifact.hash.clone())
                            })
                            .collect(),
                        expected_requirements: Rc::try_unwrap(cm)
                            .unwrap()
                            .requires_dist,
                        expected_requirements_source: Rc::try_unwrap(provenance)
                            .unwrap(),
                    }
                }),
            })
            .collect()),
        Err(err) => Err(match err {
            ErrorRetrievingDependencies {
                package,
                version,
                source,
            } => anyhow!("{}", source)
                .context(format!("fetching dependencies of {} v{}", package, version)),
            ErrorChoosingPackageVersion(boxed_err) => {
                anyhow!("{}", boxed_err.to_string())
            }
            ErrorInShouldCancel(boxed_err) => anyhow!("{}", boxed_err.to_string()),
            Failure(s) => anyhow!("{}", s),
            // XX Maybe the empty-range and self-dependency cases should be filtered out
            // inside our code, for robustness?
            DependencyOnTheEmptySet {
                package,
                version,
                dependent,
            } => anyhow!(
                "{} v{}'s dependency on {} has self-contradictory version ranges",
                package,
                version,
                dependent
            ),
            SelfDependency { package, version } => {
                anyhow!("{} v{} depends on itself", package, version)
            }

            NoSolution(mut derivation_tree) => {
                fn dump_tree(tree: &DerivationTree<ResPkg, Version>, depth: usize) {
                    let indent = "   ".repeat(depth);
                    match tree {
                        DerivationTree::External(inner) => {
                            println!("{}external: {}", indent, inner);
                        }
                        DerivationTree::Derived(inner) => {
                            println!("{}derived (id={:?})", indent, inner.shared_id);
                            for (pkg, term) in inner.terms.iter() {
                                println!("{}  {} -> {}", indent, pkg, term);
                            }
                            println!("{}cause 1:", indent);
                            dump_tree(&inner.cause1, depth + 1);
                            println!("{}cause 2:", indent);
                            dump_tree(&inner.cause2, depth + 1);
                        }
                    }
                }

                println!("\n-------- derivation tree --------");
                //println!("{:?}", derivation_tree);
                dump_tree(&derivation_tree, 0);
                derivation_tree.collapse_no_versions();
                println!("\n-------- derivation tree (collapsed) --------");
                //println!("{:?}", derivation_tree);
                dump_tree(&derivation_tree, 0);
                anyhow!(
                    "{}",
                    pubgrub::report::DefaultStringReporter::report(&derivation_tree)
                )
            }
        }),
    }
}

#[derive(Debug)]
pub struct PinnedPackage {
    pub name: PackageName,
    pub version: Version,
    pub known_artifacts: Vec<(Url, super::package_index::ArtifactHash)>,
    // For install-time consistency checking/debugging
    pub expected_requirements: Vec<PackageRequirement>,
    pub expected_requirements_source: String,
}

struct HashMapEnv<'a> {
    basic_env: &'a HashMap<String, String>,
    extra: &'a str,
}

impl<'a> marker::Env for HashMapEnv<'a> {
    fn get_marker_var(&self, var: &str) -> Option<&str> {
        match var {
            "extra" => Some(self.extra),
            _ => self.basic_env.get(var).map(|s| s.as_ref()),
        }
    }
}

// A "package" for purposes of resolving. This is an extended version of what PyPI
// considers a package, in two ways.
//
// First, the pubgrub crate assumes that resolution always starts with a single required
// package==version. So we make a virtual "root" package, pass that to pubgrub as our
// initial requirement, and then we tell pubgrub that Root package depends on our actual
// requirements. (It'd be nice if pubgrub just took a DependencyConstraints to start
// with, but, whatever.)
//
// Second, extras. To handle them properly, we create virtual packages for each extra.
// So e.g. "foo[bar,baz]" really means "foo, but with the [bar] and [baz] requirements
// added to its normal set". But that's not a concept that pubgrub understands. So
// instead, we pretend that there are two packages "foo[bar]" and "foo[baz]", and their
// requirements are:
//
// - the requirements of 'foo', when evaluated with the appropriate 'extra' set
// - a special requirement on itself 'foo', with the exact same version.
//
// Result: if we wanted "foo[bar,baz]", we end up with "foo", plus all the [bar] and
// [baz] requirements at the same version. So at the end, we can just go through and
// discard all the virtual extra packages, to get the real set of packages.
//
// This trick is stolen from pip's resolver. It also puts us in a good place if reified
// extras[1] ever become a thing, because we're basically reifying them already.
//
// [1] https://mail.python.org/pipermail/distutils-sig/2015-October/027364.html
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResPkg {
    Root,
    Package(PackageName, Option<Extra>),
}

static ROOT_VERSION: Lazy<Version> = Lazy::new(|| "0".try_into().unwrap());

impl Display for ResPkg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResPkg::Root => write!(f, "<root>"),
            ResPkg::Package(name, None) => write!(f, "{}", name.as_given()),
            ResPkg::Package(name, Some(extra)) => {
                write!(f, "{}[{}]", name.as_given(), extra.as_given())
            }
        }
    }
}

struct PubgrubState<'a> {
    // These are inputs to the resolve process
    index: &'a PackageIndex,
    root_reqs: &'a Vec<UserRequirement>,
    env: &'a HashMap<String, String>,
    preferred_versions: &'a HashMap<PackageName, Version>,
    consider_prereleases: &'a dyn Fn(&PackageName) -> bool,

    // Rest of these are memo tables, to make sure that we provide consistent answers to
    // PubGrub's queries within a single run.
    releases: RefCell<HashMap<PackageName, Rc<HashMap<Version, Vec<Artifact>>>>>,
    // These are sorted with most-preferred first.
    versions: RefCell<HashMap<PackageName, Rc<Vec<Version>>>>,
    // The String is some kind of provenance info for the metadata, so that if later on
    // when using the resolved pins we find an inconsistency, we can track down where
    // our assumptions came from.
    metadata: RefCell<HashMap<(PackageName, Version), (Rc<CoreMetadata>, Rc<String>)>>,
}

use std::collections::hash_map::Entry::*;
impl<'a> PubgrubState<'a> {
    fn releases(
        &self,
        package: &PackageName,
    ) -> Result<Rc<HashMap<Version, Vec<Artifact>>>> {
        // https://users.rust-lang.org/t/issue-with-hashmap-and-fallible-update/44960
        let mut memo = self.releases.borrow_mut();
        Ok(if let Some(e) = memo.get(&package) {
            e.clone()
        } else {
            let value = Rc::new(self.index.releases(&package)?);
            memo.insert(package.clone(), value.clone());
            value
        })
    }

    fn versions(&self, pkg: &ResPkg) -> Result<Rc<Vec<Version>>> {
        Ok(match pkg {
            ResPkg::Root => Rc::new(vec![ROOT_VERSION.clone()]),
            ResPkg::Package(name, _) => {
                let mut memo = self.versions.borrow_mut();
                if let Some(e) = memo.get(&name) {
                    e.clone()
                } else {
                    let releases = self.releases(&name)?;
                    // first filter out yanked versions
                    let unyanked = releases
                        .iter()
                        .filter_map(|(version, artifacts)| {
                            if artifacts.iter().all(|a| a.yanked.is_some()) {
                                None
                            } else {
                                Some(version.clone())
                            }
                        })
                        .collect::<Vec<Version>>();
                    // then check if all the ones that are left are prereleases --
                    // if so, then prereleases are ok to consider.
                    let all_pres = unyanked.iter().all(|v| v.is_prerelease());
                    let pre_ok = all_pres || (self.consider_prereleases)(&name);

                    let mut versions = unyanked
                        .into_iter()
                        .filter_map(|v| {
                            if !pre_ok && v.is_prerelease() {
                                None
                            } else {
                                Some(v)
                            }
                        })
                        .collect::<Vec<Version>>();

                    versions.sort_unstable();
                    if let Some(pref) = self.preferred_versions.get(&name) {
                        versions.push(pref.clone());
                    }
                    versions.reverse();
                    let value = Rc::new(versions);
                    memo.insert(name.clone(), value.clone());
                    value
                }
            }
        })
    }

    fn metadata_from_artifacts(
        &self,
        artifacts: &Vec<Artifact>,
    ) -> Result<(Rc<CoreMetadata>, Rc<String>)> {
        // first, try to find an un-yanked wheel
        for artifact in artifacts {
            if artifact.url.path().ends_with(".whl") && artifact.yanked.is_none() {
                let cm = self.index.wheel_metadata(&artifact.url)?;
                return Ok((Rc::new(cm), Rc::new(artifact.url.to_string())));
            }
        }
        todo!("figure out what to do if no un-yanked wheels");
    }

    fn metadata(
        &self,
        package: &PackageName,
        version: &Version,
    ) -> Result<(Rc<CoreMetadata>, Rc<String>)> {
        let mut memo = self.metadata.borrow_mut();
        let key = (package.clone(), version.clone());
        Ok(match memo.entry(key) {
            Occupied(e) => e.get().clone(),
            Vacant(e) => {
                let releases = self.releases(package)?;
                let artifacts = releases.get(version).ok_or_else(|| {
                    anyhow!("where did {} v{} come from?", package.as_given(), version)
                })?;
                e.insert(self.metadata_from_artifacts(&artifacts)?).clone()
            }
        })
    }

    fn requirements_to_pubgrub<'r, R, I>(
        &self,
        reqs: I,
        dc: &mut DependencyConstraints<ResPkg, Version>,
        extra: &Option<Extra>,
    )
        where R: std::ops::Deref<Target=Requirement> + 'r, I: Iterator<Item = &'r R>
    {
        let extra_str: &str = match extra {
            Some(e) => e.normalized(),
            None => "",
        };
        let env = HashMapEnv {
            basic_env: &self.env,
            extra: extra_str,
        };

        for req in reqs {
            if let Some(expr) = &req.env_marker {
                // XX bad unwrap
                if !expr.eval(&env).unwrap() {
                    return;
                }
            }

            let mut maybe_extras: Vec<Option<Extra>> =
                req.extras.iter().map(|e| Some(e.clone())).collect();
            if maybe_extras.is_empty() {
                maybe_extras.push(None);
            }

            for maybe_extra in maybe_extras {
                let pkg = ResPkg::Package(req.name.clone(), maybe_extra);
                // XX bad unwrap
                let range = specifiers_to_pubgrub(&req.specifiers).unwrap();
                println!("adding dependency: {} {}", pkg, range);
                dc.insert(pkg, range);
            }
        }
    }
}

fn specifiers_to_pubgrub(specs: &Specifiers) -> Result<Range<Version>> {
    let mut final_range = Range::any();
    for spec in &specs.0 {
        let spec_range =
            spec.to_ranges()?
                .into_iter()
                .fold(Range::none(), |accum, r| {
                    accum.union(&if r.end < *VERSION_INFINITY {
                        Range::between(r.start, r.end)
                    } else {
                        Range::higher_than(r.start)
                    })
                });
        final_range = final_range.intersection(&spec_range);
    }
    Ok(final_range)
}

impl<'a> pubgrub::solver::DependencyProvider<ResPkg, Version> for PubgrubState<'a> {
    fn choose_package_version<T, U>(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<Version>), Box<dyn std::error::Error>>
    where
        T: Borrow<ResPkg>,
        U: Borrow<Range<Version>>,
    {
        println!("----> pubgrub called choose_package_version");
        // Heuristic: for our next package candidate, use the package with the fewest
        // remaining versions to consider. This tends to drive to either a workable
        // version or a conflict as fast as possible.
        let count_valid = |(p, range): &(T, U)| match self.versions(p.borrow()) {
            Err(_) => 0,
            Ok(versions) => versions
                .iter()
                .filter(|v| range.borrow().contains(v))
                .count(),
        };

        let (pkg, range) = potential_packages
            .map(|(p, range)| {
                println!(
                    "-> For {}, allowed range is: {}",
                    p.borrow(),
                    range.borrow()
                );
                (p, range)
            })
            .min_by_key(count_valid)
            .ok_or_else(|| anyhow!("No packages found within range"))?;

        println!(
            "Chose package {}; now let's decide which version",
            pkg.borrow(),
        );

        match pkg.borrow() {
            ResPkg::Root => {
                println!("<---- decision: root package magic version 0");
                Ok((pkg, Some(ROOT_VERSION.clone())))
            }
            ResPkg::Package(name, _) => {
                // why does this have to be 'parse' instead of 'try_into'?! it is a
                // mystery
                let python_full_version: Version =
                    self.env.get("python_full_version").unwrap().parse()?;

                for version in self.versions(pkg.borrow())?.iter() {
                    if !range.borrow().contains(&version) {
                        println!("Version {} is out of range", version);
                        continue;
                    }

                    println!("Considering {} v{}", pkg.borrow(), version);

                    let (metadata, _) = self.metadata(&name, &version)?;

                    // check if this version is even compatible with our python
                    match metadata.requires_python.satisfied_by(&python_full_version) {
                        Err(e) => {
                            println!("Error checking Requires-Python: {}; skipping", e);
                            continue;
                        }
                        Ok(false) => {
                            println!(
                                "Python {} doesn't satisfy Requires-Python: {:?}",
                                python_full_version, metadata.requires_python
                            );
                            continue;
                        }
                        Ok(true) => {
                            println!("<---- decision: {} v{}", pkg.borrow(), version);
                            return Ok((pkg, Some(version.clone())));
                        }
                    }
                }

                println!("<---- decision: no versions of {} in range", pkg.borrow());
                Ok((pkg, None))
            }
        }
    }

    fn get_dependencies(
        &self,
        pkg: &ResPkg,
        version: &Version,
    ) -> std::result::Result<
        pubgrub::solver::Dependencies<ResPkg, Version>,
        Box<dyn std::error::Error>,
    > {
        println!("----> pubgrub called get_dependencies {} v{}", pkg, version);

        match pkg {
            ResPkg::Root => {
                let mut dc: DependencyConstraints<ResPkg, Version> =
                    vec![].into_iter().collect();
                self.requirements_to_pubgrub(self.root_reqs.iter(), &mut dc, &None);
                println!("<---- dependencies complete");
                Ok(Dependencies::Known(dc))
            }
            ResPkg::Package(name, extra) => {
                let (metadata, _) = self.metadata(&name, &version)?;

                // why can't I call ::new on this?
                let mut dc: DependencyConstraints<ResPkg, Version> =
                    vec![].into_iter().collect();

                self.requirements_to_pubgrub(metadata.requires_dist.iter(), &mut dc, &extra);

                if let Some(_) = extra {
                    dc.insert(
                        ResPkg::Package(name.clone(), None),
                        Range::exact(version.clone()),
                    );
                }

                println!("<---- dependencies complete");
                Ok(Dependencies::Known(dc))
            }
        }
    }
}
