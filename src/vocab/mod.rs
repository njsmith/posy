mod core_metadata;
mod extra;
mod package_name;
mod requirement;
mod rfc822ish;
mod wheel_metadata;
mod wheel_name;

// All this stuff is also re-exported from crate::prelude::*

pub use self::core_metadata::CoreMetadata;
pub use self::extra::Extra;
pub use self::package_name::PackageName;
pub use self::wheel_metadata::WheelMetadata;
pub use self::wheel_name::WheelName;
pub use self::requirement::{
    Constraint, Marker, MarkerOp, MarkerValue, ParseExtra, Requirement, RequiresPython,
};

pub use pep440::Version;

// Version upstream just has parse->Option(Version); this convenience function
// saves a lot of hassle.
use crate::prelude::*;
pub fn parse_version(v: &str) -> Result<Version> {
    Version::parse(v).ok_or(anyhow!("Failed to parse PEP 440 version {}", v))
}
