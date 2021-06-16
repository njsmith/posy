use crate::prelude::*;
use pubgrub::range::Range;
use pubgrub::solver::{Dependencies, DependencyConstraints};

use crate::pypi::{Artifact, PyPI};
use std::io::Read;
use std::{borrow::Borrow, cell::RefCell};

const ENV: Lazy<HashMap<String, String>> = Lazy::new(|| {
    // Copied from
    //   print(json.dumps(packaging.markers.default_environment(), sort_keys=True, indent=4))
    serde_json::from_str(
        r##"
        {
            "implementation_name": "cpython",
            "implementation_version": "3.8.6",
            "os_name": "posix",
            "platform_machine": "x86_64",
            "platform_python_implementation": "CPython",
            "platform_release": "5.8.0-53-generic",
            "platform_system": "Linux",
            "platform_version": "#60-Ubuntu SMP Thu May 6 07:46:32 UTC 2021",
            "python_full_version": "3.8.6",
            "python_version": "3.8",
            "sys_platform": "linux"
        }
        "##,
    )
    .unwrap()
});

struct HashEnv<'a> {
    basic_env: &'a HashMap<String, String>,
    extra: &'a str,
}

impl<'a> marker::Env for HashEnv<'a> {
    fn get_marker_var(&self, var: &str) -> Option<&str> {
        match var {
            "extra" => Some(self.extra),
            _ => self.basic_env.get(var).map(|s| s.as_ref()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ResPkg {
    Root,
    Package(PackageName, Option<Extra>),
}

static ROOT_VERSION: Lazy<Version> = Lazy::new(|| "1.0".try_into().unwrap());

impl Display for ResPkg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResPkg::Root => write!(f, "*root*"),
            ResPkg::Package(name, None) => write!(f, "{}", name),
            ResPkg::Package(name, Some(extra)) => write!(f, "{}[{}]", name, extra),
        }
    }
}

pub struct PythonDependencies {
    pub pypi: PyPI,
    pub known_artifacts: RefCell<HashMap<PackageName, HashMap<Version, Vec<Artifact>>>>,
    pub known_metadata: RefCell<HashMap<(PackageName, Version), CoreMetadata>>,
    pub root_reqs: Vec<Requirement>,
}

// XX these do *tons* of unnecessary copying, because I decided to try to get it working
// at all before discussing how to make it fast with the borrow checker.
//
// Maybe just make everything Cow?
impl PythonDependencies {
    fn available(&self, package: &PackageName) -> HashMap<Version, Vec<Artifact>> {
        let mut known = self.known_artifacts.borrow_mut();
        match known.get(package) {
            None => {
                // XX THIS UNWRAP HAS TO BE FIXED
                let releases = self.pypi.package_info(&package).unwrap();
                let version_map: HashMap<Version, Vec<Artifact>> = releases
                    .into_iter()
                    .map(|r| (r.version, r.artifacts))
                    .collect();
                known.insert(package.clone(), version_map.clone());
                version_map
            }
            Some(version_map) => version_map.clone(),
        }
    }

    fn available_versions(&self, pkg: &ResPkg) -> Vec<Version> {
        match pkg {
            ResPkg::Root => vec![ROOT_VERSION.clone()],
            ResPkg::Package(name, _) => {
                let mut versions: Vec<Version> =
                    self.available(&name).keys().cloned().collect();
                versions.sort_unstable();
                versions
            }
        }
    }

    fn available_artifacts(
        &self,
        package: &PackageName,
        version: &Version,
    ) -> Vec<Artifact> {
        match self.available(&package).get(&version) {
            Some(artifacts) => artifacts.clone(),
            None => Vec::new(),
        }
    }

    fn requirements_to_pubgrub(
        &self,
        reqs: &Vec<Requirement>,
        dc: &mut DependencyConstraints<ResPkg, Version>,
        extra: &Option<Extra>,
    ) {
        let extra_str: &str = match extra {
            Some(e) => e.normalized(),
            None => "",
        };
        let env = HashEnv {
            basic_env: &*ENV,
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
                // XX bad unwrap
                dc.insert(
                    ResPkg::Package(req.name.clone(), maybe_extra),
                    specifiers_to_pubgrub(&req.specifiers).unwrap(),
                );
            }
        }
    }

    pub fn resolve(
        &self,
    ) -> std::result::Result<
        pubgrub::type_aliases::SelectedDependencies<ResPkg, Version>,
        pubgrub::error::PubGrubError<ResPkg, Version>,
    > {
        pubgrub::solver::resolve(self, ResPkg::Root, ROOT_VERSION.clone())
    }
}

fn whl_url_to_metadata(agent: &ureq::Agent, url: &Url) -> Result<CoreMetadata> {
    use std::io::{Cursor, Seek};

    println!("Fetching and parsing {}", url);

    let resp = agent.request_url("GET", &url).call()?;
    let mut body = Vec::new();
    resp.into_reader().read_to_end(&mut body)?;
    let body = Cursor::new(body);
    let mut zip = zip::ZipArchive::new(body)?;
    let names: Vec<String> = zip.file_names().map(|s| s.to_owned()).collect();

    fn get<T: Read + Seek>(
        zip: &mut zip::ZipArchive<T>,
        name: &str,
    ) -> Result<Vec<u8>> {
        let mut buf = Vec::new();
        let mut zipfile = zip.by_name(name)?;
        zipfile.read_to_end(&mut buf)?;
        Ok(buf)
    }

    for name in names {
        if name.ends_with(".dist-info/WHEEL") {
            // will error out if the metadata is bad
            WheelMetadata::parse(&get(&mut zip, &name)?)?;
        }
        if name.ends_with(".dist-info/METADATA") {
            return Ok(CoreMetadata::parse(&get(&mut zip, &name)?)?);
        }
    }

    anyhow::bail!("didn't find METADATA");
}

fn specifiers_to_pubgrub(specs: &Specifiers) -> Result<Range<Version>> {
    let mut final_range = Range::any();
    for spec in &specs.0 {
        let spec_range = spec.to_ranges()?.into_iter().fold(Range::none(), |accum, r| {
            accum.union(&Range::between(r.start, r.end))
        });
        final_range = final_range.intersection(&spec_range);
    }
    Ok(final_range)
}

impl pubgrub::solver::DependencyProvider<ResPkg, Version> for PythonDependencies {
    fn choose_package_version<T, U>(
        &self,
        potential_packages: impl Iterator<Item = (T, U)>,
    ) -> Result<(T, Option<Version>), Box<dyn std::error::Error>>
    where
        T: Borrow<ResPkg>,
        U: Borrow<Range<Version>>,
    {
        // XXXX need to fetch metadata *before* picking a version, because it
        // might turn out that there's a bad Requires-Python or the metadata
        // is invalid or something (bad environment markers maybe, or
        // environment markers that depend on a feature we don't know for the
        // given python). Which might be recoverable errors -- we don't want
        // them to blow up the whole resolution process if there's a
        // Requires-Python, we just want to skip that package version and keep
        // going.

        let count_valid = |(p, range): &(T, U)| {
            self.available_versions(p.borrow())
                .into_iter()
                .filter(|v| range.borrow().contains(v.borrow()))
                .count()
        };

        let (pkg, range) = potential_packages
            .min_by_key(count_valid)
            .ok_or_else(|| anyhow!("No packages found within range"))?;

        println!(
            "Looking for versions of {} ({:?})",
            pkg.borrow(),
            range.borrow()
        );

        match pkg.borrow() {
            ResPkg::Root => Ok((pkg, Some(ROOT_VERSION.clone()))),
            ResPkg::Package(name, _) => {
                // why does this have to be 'parse' instead of 'try_into'?! it is a mystery
                let python_version: Version =
                    ENV.get("python_version").unwrap().parse()?;

                for version in self.available_versions(pkg.borrow()).into_iter().rev() {
                    if !range.borrow().contains(&version) {
                        println!("Version {} is out of range", version);
                        continue;
                    }

                    println!("Considering {} v{}", pkg.borrow(), version);

                    let mut known_m = self.known_metadata.borrow_mut();
                    let e = known_m.entry((name.clone(), version.clone()));
                    let metadata = e.or_insert_with(|| {
                        // XX bad unwrap
                        // XX need to track provenance of the metadata we end up using
                        // (or I guess could extract it at the end?)
                        let artifact = self
                            .available_artifacts(name, &version)
                            .into_iter()
                            .filter(|a| a.url.path().ends_with(".whl"))
                            .next()
                            .unwrap();
                        // XX bad unwrap
                        whl_url_to_metadata(&self.pypi.agent, &artifact.url).unwrap()
                    });

                    // check if this version is even compatible with our python
                    match metadata.requires_python.satisfied_by(&python_version) {
                        Err(e) => {
                            println!("Error checking Requires-Python: {}; skipping", e);
                            continue;
                        }
                        Ok(false) => {
                            println!(
                                "Python {} doesn't satisfy Requires-Python: {:?}",
                                python_version, metadata.requires_python
                            );
                            continue;
                        }
                        Ok(true) => return Ok((pkg, Some(version.clone()))),
                    }
                }

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
        println!("Fetching dependencies for {} v{}", pkg, version);

        match pkg {
            ResPkg::Root => {
                let mut dc: DependencyConstraints<ResPkg, Version> =
                    vec![].into_iter().collect();
                self.requirements_to_pubgrub(&self.root_reqs, &mut dc, &None);
                println!("root dc: {:?}", dc);
                Ok(Dependencies::Known(dc))
            }
            ResPkg::Package(name, extra) => {
                // unwrap() is safe here, b/c we never give pubgrub a package/version unless we
                // have already fetched the metadata.
                let metadata = self
                    .known_metadata
                    .borrow()
                    .get(&(name.clone(), version.clone()))
                    .unwrap()
                    .clone();

                // why can't I call ::new on this?
                let mut dc: DependencyConstraints<ResPkg, Version> =
                    vec![].into_iter().collect();

                self.requirements_to_pubgrub(&metadata.requires_dist, &mut dc, &extra);

                if let Some(_) = extra {
                    dc.insert(
                        ResPkg::Package(name.clone(), None),
                        Range::exact(version.clone()),
                    );
                }

                Ok(Dependencies::Known(dc))
            }
        }
    }
}
