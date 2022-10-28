use crate::prelude::*;
use elsa::FrozenMap;
use pubgrub::range::Range;
use pubgrub::report::DerivationTree;
use pubgrub::report::Reporter;
use pubgrub::solver::{Dependencies, DependencyConstraints};

use crate::package_db::PackageDB;
use std::borrow::Borrow;

#[derive(Debug, Clone)]
pub struct ExpectedMetadata {
    pub provenance: Url,
    pub requires_dist: Vec<PackageRequirement>,
    pub requires_python: Specifiers,
    pub extras: HashSet<Extra>,
}

struct PubgrubState<'a> {
    // These are inputs to the resolve process
    db: &'a PackageDB,
    root_reqs: &'a Vec<UserRequirement>,
    env: &'a HashMap<String, String>,

    python_full_version: Version,
    // record of the metadata we used, so we can record it and validate it later when
    // using the pins
    expected_metadata: FrozenMap<(PackageName, Version), Box<ExpectedMetadata>>,
    // These are sorted with most-preferred first.
    versions: FrozenMap<PackageName, Vec<&'a Version>>,
}

fn get_or_fill<'a, 'b, K, V, F>(
    map: &'a FrozenMap<K, V>,
    key: &'b K,
    f: F,
) -> Result<&'a V::Target>
where
    K: Eq + std::hash::Hash + Clone,
    F: FnOnce() -> Result<V>,
    V: stable_deref_trait::StableDeref,
{
    if let Some(v) = map.get(key) {
        Ok(v)
    } else {
        Ok(map.insert(key.to_owned(), f()?))
    }
}

impl<'a> PubgrubState<'a> {
    fn metadata(&self, release: &(PackageName, Version)) -> Result<&ExpectedMetadata> {
        get_or_fill(&self.expected_metadata, release, || {
            let ais = self.db.artifacts_for_release(&release.0, &release.1)?;
            let (ai, wheel_metadata) = self.db.get_metadata::<Wheel, _>(ais)?;
            Ok(Box::new(ExpectedMetadata {
                provenance: ai.url.clone(),
                requires_dist: wheel_metadata.requires_dist,
                requires_python: wheel_metadata.requires_python,
                extras: wheel_metadata.extras,
            }))
        })
    }

    fn versions(&self, package: &PackageName) -> Result<&[&Version]> {
        get_or_fill(&self.versions, &package, || {
            let artifacts = self.db.available_artifacts(&package)?;
            let mut versions = Vec::<&Version>::new();
            for (version, ais) in artifacts.iter() {
                if version.is_prerelease() {
                    continue;
                }
                for ai in ais {
                    if ai.yanked.yanked {
                        continue;
                    }
                    if let Some(requires_python) = &ai.requires_python {
                        let requires_python: Specifiers = requires_python.parse()?;
                        if !requires_python.satisfied_by(&self.python_full_version)? {
                            continue;
                        }
                    }
                    versions.push(version);
                }
            }
            versions.sort_unstable();
            Ok(versions)
        })
    }
}

pub fn resolve_wheels(
    db: &PackageDB,
    requirements: &Vec<UserRequirement>,
    env: &HashMap<String, String>,
) -> Result<Vec<(PackageName, Version, ExpectedMetadata)>> {
    let state = PubgrubState {
        db,
        root_reqs: requirements,
        env,
        python_full_version: env
            .get("python_full_version")
            .ok_or(anyhow!(
                "Missing 'python_full_version' environment marker variable"
            ))?
            .parse()?,
        expected_metadata: Default::default(),
        versions: Default::default(),
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
                    (
                        name.clone(),
                        v.clone(),
                        state.expected_metadata.get(&(name, v)).unwrap().clone(),
                    )
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

struct HashMapEnv<'a> {
    basic_env: &'a HashMap<String, String>,
    extra: Option<&'a str>,
}

impl<'a> marker::Env for HashMapEnv<'a> {
    fn get_marker_var(&self, var: &str) -> Option<&str> {
        match var {
            // we want 'extra' to have some value, because looking it up shouldn't be an
            // error. But we want that value to be something that will never match a
            // real extra. We use an empty string.
            "extra" => Some(self.extra.unwrap_or("")),
            _ => self.basic_env.get(var).map(|s| s.as_str()),
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

impl<'a> PubgrubState<'a> {
    fn requirements_to_pubgrub<'r, R, I>(
        &self,
        reqs: I,
        dc: &mut DependencyConstraints<ResPkg, Version>,
        extra: Option<&Extra>,
    ) -> Result<()>
    where
        R: std::ops::Deref<Target = Requirement> + 'r,
        I: Iterator<Item = &'r R>,
    {
        let env = HashMapEnv {
            basic_env: &self.env,
            extra: extra.map(|e| e.normalized()),
        };

        for req in reqs {
            if let Some(expr) = &req.env_marker_expr {
                if !expr.eval(&env)? {
                    continue;
                }
            }

            let mut maybe_extras: Vec<Option<Extra>> =
                req.extras.iter().map(|e| Some(e.clone())).collect();
            if maybe_extras.is_empty() {
                maybe_extras.push(None);
            }

            for maybe_extra in maybe_extras {
                let pkg = ResPkg::Package(req.name.clone(), maybe_extra);
                let range = specifiers_to_pubgrub(&req.specifiers)?;
                println!("adding dependency: {} {}", pkg, range);
                dc.insert(pkg, range);
            }
        }
        Ok(())
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
        mut potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<Version>), Box<dyn std::error::Error>>
    where
        T: Borrow<ResPkg>,
        U: Borrow<Range<Version>>,
    {
        println!("----> pubgrub called choose_package_version");
        // XX TODO: laziest possible heuristic, just pick the first package offered
        let (respkg, range) = potential_packages.next().unwrap();

        match respkg.borrow() {
            ResPkg::Root => {
                println!("<---- decision: root package magic version 0");
                Ok((respkg, Some(ROOT_VERSION.clone())))
            }
            ResPkg::Package(name, _) => {
                for &version in self.versions(&name)?.iter().rev() {
                    if !range.borrow().contains(version) {
                        println!("Version {} is out of range", version);
                        continue;
                    }

                    let metadata = self.metadata(&(name.clone(), version.clone()))?;
                    if !metadata
                        .requires_python
                        .satisfied_by(&self.python_full_version)?
                    {
                        Err(anyhow!(
                            "{} {}: bad requires-python, but pypi didn't tell us!",
                            name.as_given(),
                            version
                        ))?;
                    }
                    println!("<---- decision: {} {}", respkg.borrow(), version);
                    return Ok((respkg, Some(version.clone())));
                }

                println!(
                    "<---- decision: no versions of {} in range",
                    respkg.borrow()
                );
                Ok((respkg, None))
            }
        }
    }

    fn get_dependencies(
        &self,
        pkg: &ResPkg,
        version: &Version,
    ) -> Result<
        pubgrub::solver::Dependencies<ResPkg, Version>,
        Box<dyn std::error::Error>,
    > {
        println!("----> pubgrub called get_dependencies {} v{}", pkg, version);

        match pkg {
            ResPkg::Root => {
                let mut dc: DependencyConstraints<ResPkg, Version> =
                    vec![].into_iter().collect();
                self.requirements_to_pubgrub(self.root_reqs.iter(), &mut dc, None)?;
                println!("<---- dependencies complete");
                Ok(Dependencies::Known(dc))
            }
            ResPkg::Package(name, extra) => {
                let metadata = self.metadata(&(name.clone(), version.clone()))?;

                let mut dc: DependencyConstraints<ResPkg, Version> = Default::default();

                self.requirements_to_pubgrub(
                    metadata.requires_dist.iter(),
                    &mut dc,
                    extra.as_ref(),
                )?;

                if let Some(inner) = extra {
                    if !metadata.extras.contains(inner) {
                        Err(anyhow!(
                            "package {} has no extra [{}]",
                            name.as_given(),
                            inner.as_given()
                        ))?;
                    }
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
