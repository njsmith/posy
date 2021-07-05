mod core_metadata;
mod extra;
mod package_name;
mod reqparse;
mod requirement;
mod rfc822ish;
mod specifier;
mod version;
mod bin_format_metadata;
mod artifact_name;

// All this stuff is also re-exported from crate::prelude::*

pub use self::core_metadata::CoreMetadata;
pub use self::extra::Extra;
pub use self::package_name::PackageName;
pub use self::requirement::{marker, ParseExtra, Requirement};
pub use self::specifier::{CompareOp, Specifier, Specifiers};
pub use self::version::Version;
pub use self::bin_format_metadata::{WheelMetadata, PybiMetadata};
pub use self::artifact_name::{WheelName, PybiName, SdistName, ArtifactName};
