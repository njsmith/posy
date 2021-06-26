mod core_metadata;
mod extra;
mod package_name;
mod reqparse;
mod requirement;
mod rfc822ish;
mod specifier;
mod version;
mod wheel_metadata;
mod wheel_name;
mod sdist_name;

// All this stuff is also re-exported from crate::prelude::*

pub use self::core_metadata::CoreMetadata;
pub use self::extra::Extra;
pub use self::package_name::PackageName;
pub use self::requirement::{marker, ParseExtra, Requirement};
pub use self::specifier::{CompareOp, Specifier, Specifiers};
pub use self::version::Version;
pub use self::wheel_metadata::WheelMetadata;
pub use self::wheel_name::WheelName;
pub use self::sdist_name::SdistName;
