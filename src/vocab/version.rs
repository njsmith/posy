use crate::prelude::*;
use std::hash::{Hash, Hasher};

// We lean on the 'pep440' crate for the heavy lifting part of representing
// versions, but wrap it in our own type so that we can e.g. make it Hashable
// and play nice with pubgrub.

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Version(pub pep440::Version);

impl Version {
    pub const ZERO: Lazy<Version> = Lazy::new(|| "0a0.dev0".try_into().unwrap());

    // XX BUG IN pep440 crate: the actuall smallest post-prefix is .post0. And X.Y.post0
    // is strictly larger than X.Y. BUT, PEP 440 treats these as the same. (This may
    // also screw up our hashing, but I'll worry about that later...).
    pub const SMALLEST_POST: Option<u32> = Some(1);

    pub const INFINITY: Lazy<Version> = Lazy::new(|| {
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

    pub fn satisfies(&self, specifiers: &Specifiers) -> Result<bool> {
        for specifier in &specifiers.0 {
            if !specifier.satisfied_by(self)? {
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
