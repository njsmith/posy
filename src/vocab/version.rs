use crate::prelude::*;

// We lean on the 'pep440' crate for the heavy lifting part of representing versions,
// but wrap it in our own type so that we can e.g. make it play nice with pubgrub.

#[derive(
    Clone,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Debug,
    Hash,
    SerializeDisplay,
    DeserializeFromStr,
)]
pub struct Version(pub pep440::Version);

pub static VERSION_ZERO: Lazy<Version> = Lazy::new(|| "0a0.dev0".try_into().unwrap());

pub static VERSION_INFINITY: Lazy<Version> = Lazy::new(|| {
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

impl Version {
    pub fn is_prerelease(&self) -> bool {
        self.0.pre.is_some() || self.0.dev.is_some()
    }

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
        // - You *can* attach a .postN after anything else. And a .devN after that. So
        // to get the next possible value, attach a .post0.dev0.
        if let Some(dev) = &mut new.0.dev {
            *dev += 1;
        } else if let Some(post) = &mut new.0.post {
            *post += 1;
        } else {
            new.0.post = Some(0);
            new.0.dev = Some(0);
        }
        new
    }
}

impl TryFrom<&str> for Version {
    type Error = eyre::Report;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        pep440::Version::parse(value)
            .map(Version)
            .ok_or_else(|| eyre!("Failed to parse PEP 440 version {}", value))
    }
}

try_from_str_boilerplate!(Version);

impl Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl pubgrub::version::Version for Version {
    fn lowest() -> Self {
        VERSION_ZERO.to_owned()
    }

    fn bump(&self) -> Self {
        self.next()
    }
}
