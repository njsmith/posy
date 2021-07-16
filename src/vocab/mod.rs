mod artifact_name;
mod bin_format_metadata;
mod core_metadata;
mod extra;
mod package_name;
mod reqparse;
mod requirement;
mod rfc822ish;
mod specifier;
mod version;

// All this stuff is also re-exported from crate::prelude::*

pub use self::artifact_name::{ArtifactName, PybiName, SdistName, WheelName};
pub use self::bin_format_metadata::{PybiMetadata, WheelMetadata};
pub use self::core_metadata::CoreMetadata;
pub use self::extra::Extra;
pub use self::package_name::PackageName;
pub use self::requirement::{marker, ParseExtra, Requirement};
pub use self::specifier::{CompareOp, Specifier, Specifiers};
pub use self::version::{Version, VERSION_INFINITY, VERSION_ZERO};
