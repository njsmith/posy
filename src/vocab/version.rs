use crate::prelude::*;
use std::hash::{Hash, Hasher};
use std::ops::Range;

// We lean on the 'pep440' crate for the heavy lifting part of representing
// versions, but wrap it in our own type so that we can e.g. make it Hashable
// and play nice with pubgrub.

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pep440::Version);

impl Version {
    const ZERO: Lazy<Version> = Lazy::new(|| "0a0.dev0".try_into().unwrap());

    // XX BUG IN pep440 crate: the actuall smallest post-prefix is .post0. And X.Y.post0
    // is strictly larger than X.Y. BUT, PEP 440 treats these as the same. (This may
    // also screw up our hashing, but I'll worry about that later...).
    const SMALLEST_POST: Option<u32> = Some(1);

    const INFINITY: Lazy<Version> = Lazy::new(|| {
        // Technically there is no largest PEP 440 version. But this should be good
        // enough that no-one will notice the difference...
        Version(pep440::Version {
            epoch: u32::MAX,
            release: vec![u32::MAX, u32::MAX, u32::MAX],
            pre: None,
            post: Some(u32::MAX),
            dev: None,
            local: vec![],
        })
    });

    /// Returns the smallest PEP 440 version that is larger than self.
    pub fn next(&self) -> Version {
        let mut new = self.clone();
        // The rules are here:
        //
        //   https://www.python.org/dev/peps/pep-0440/#summary-of-permitted-suffixes-and-relative-ordering
        //
        // The relevant ones for this:
        //
        // - You can't attach a .postN after a .devN. So if you have a .devN,
        //   then the next possible version is .dev(N+1)
        //
        // - You can't attach a .postN after a .postN. So if you already have
        //   a .postN, then the next possible value is .post(N+1).
        //
        // - You *can* attach a .postN after anything else. So to get the next
        //   possible value, attach a .post0.
        if let Some(dev) = &mut new.0.dev {
            *dev += 1;
        } else if let Some(post) = &mut new.0.post {
            *post += 1;
        } else {
            new.0.post = Version::SMALLEST_POST;
        }
        new
    }

    pub fn satisfies(&self, constraints: &Vec<Constraint>) -> Result<bool>
    {
        for constraint in constraints {
            if !constraint.satisfied_by(self)? {
                return Ok(false);
            }
        }
        Ok(true)
    }
}

impl TryFrom<&str> for Version {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        pep440::Version::parse(value)
            .map(|v| Version(v))
            .ok_or_else(|| anyhow!("Failed to parse PEP 440 version {}", value))
    }
}

try_from_str_boilerplate!(Version);

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Hash for Version {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // This is pretty inefficient compared to hashing the elements
        // individually, but that gets awkward because there are embedded
        // enums that aren't hashable either. XX we should fix this upstream in the
        // pep440 crate.
        self.0.normalize().hash(state)
    }
}

impl pubgrub::version::Version for Version {
    fn lowest() -> Self {
        Version::ZERO.to_owned()
    }

    fn bump(&self) -> Self {
        self.next()
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CompareOp {
    LessThanEqual,
    StrictlyLessThan,
    NotEqual,
    Equal,
    GreaterThanEqual,
    StrictlyGreaterThan,
    Compatible,
}

fn parse_version_wildcard(input: &str) -> Result<(Version, bool)> {
    let (vstr, wildcard) = if let Some(vstr) = input.strip_suffix(".*") {
        (vstr, true)
    } else {
        (input, false)
    };
    let version: Version = vstr.try_into()?;
    if wildcard
        && (version.0.pre.is_some()
            || version.0.post.is_some()
            || version.0.dev.is_some()
            || !version.0.local.is_empty())
    {
        bail!("Invalid PEP 440 wildcard (no suffixes allowed): {}", input);
    }
    Ok((version, wildcard))
}

/// Converts a comparison like ">= 1.2" into a union of [half, open) ranges.
///
/// Has to take a string, not a Version, because == and != can take "wildcards", which
/// are not valid versions.
// XX local version handling -- I think everything except == and != is supposed to error
// out if the rhs version has a local segment?
impl CompareOp {
    pub fn to_ranges(&self, rhs: &str) -> Result<Vec<Range<Version>>> {
        use CompareOp::*;
        let (version, wildcard) = parse_version_wildcard(rhs)?;
        Ok(if wildcard {
            // =~ X.* correspond to the half-open range
            // [X.dev0, (X+1).dev0)
            let mut low = version.clone();
            low.0.dev = Some(0);
            let mut high = version.clone();
            *high.0.release.last_mut().unwrap() += 1;
            high.0.dev = Some(0);
            match self {
                Equal => vec![low..high],
                NotEqual => {
                    vec![Version::ZERO.clone()..low, high..Version::INFINITY.clone()]
                }
                _ => bail!("Can't use wildcard with {:?}", self),
            }
        } else {
            // no wildcards here
            match self {
                // These two are simple
                LessThanEqual => vec![Version::ZERO.clone()..version.next()],
                GreaterThanEqual => vec![version.clone()..Version::INFINITY.clone()],
                // These are also pretty simple, because we took care of the wildcard
                // cases up above.
                Equal => vec![version.clone()..version.next()],
                NotEqual => vec![
                    Version::ZERO.clone()..version.clone(),
                    version.next()..Version::INFINITY.clone(),
                ],
                // "The exclusive ordered comparison >V MUST NOT allow a post-release of
                // the given version unless V itself is a post release."
                // So >V normally becomes >=(V+1).dev0
                StrictlyGreaterThan => match version.0.post {
                    Some(_) => vec![version.next()..Version::INFINITY.clone()],
                    None => {
                        let mut new_min = version.clone();
                        *new_min.0.release.last_mut().unwrap() += 1;
                        new_min.0.pre = None;
                        new_min.0.post = None;
                        new_min.0.dev = Some(0);
                        new_min.0.local = vec![];
                        vec![new_min..Version::INFINITY.clone()]
                    }
                },
                // "The exclusive ordered comparison <V MUST NOT allow a pre-release of
                // the specified version unless the specified version is itself a
                // pre-release."
                StrictlyLessThan => {
                    if (&version.0.pre, &version.0.dev) == (&None, &None) {
                        let mut new_max = version.clone();
                        new_max.0.post = None;
                        new_max.0.local = vec![];
                        vec![Version::ZERO.clone()..new_max]
                    } else {
                        // Otherwise, some kind of pre-release
                        vec![Version::ZERO.clone()..version]
                    }
                }
                // ~= X.Y.suffixes is the same as >= X.Y.suffixes && == X.*
                // So it's a half-open range:
                //   [X.Y.suffixes, X.(Y+1).dev0)
                Compatible => {
                    let mut new_max = Version(pep440::Version {
                        epoch: version.0.epoch,
                        release: version.0.release.clone(),
                        pre: None,
                        post: None,
                        dev: Some(0),
                        local: vec![],
                    });
                    *new_max.0.release.last_mut().unwrap() += 1;
                    vec![version..new_max]
                }
            }
        })
    }
}
