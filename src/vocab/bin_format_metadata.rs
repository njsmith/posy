use crate::prelude::*;

use super::rfc822ish::RFC822ish;

#[derive(Debug, Clone)]
pub struct WheelMetadata {
    pub root_is_purelib: bool,
}

#[derive(Debug, Clone)]
pub struct PybiMetadata {}

fn parse_bin_metadata_and_check_version(
    input: &[u8],
    version_field: &str,
    next_major: &Version,
) -> Result<RFC822ish> {
    let input: &str = std::str::from_utf8(input)?;
    let mut parsed = RFC822ish::parse(&input)?;

    let version: Version = parsed.take_the(version_field)?.try_into()?;
    if version >= *next_major {
        bail!("unsupported {}: {}", version_field, version);
    }
    Ok(parsed)
}

impl WheelMetadata {
    pub fn parse(input: &[u8]) -> Result<WheelMetadata> {
        static NEXT_MAJOR_WHEEL_VERSION: Lazy<Version> =
            Lazy::new(|| "2".try_into().unwrap());

        let mut parsed = parse_bin_metadata_and_check_version(input, "Wheel-Version", &NEXT_MAJOR_WHEEL_VERSION)?;

        let root_is_purelib = match &parsed.take_the("Root-Is-Purelib")?[..] {
            "true" => true,
            "false" => false,
            other => bail!(
                "Expected 'true' or 'false' for Root-Is-Purelib, not {}",
                other,
            ),
        };

        Ok(WheelMetadata {
            root_is_purelib,
        })
    }
}

impl PybiMetadata {
    pub fn parse(input: &[u8]) -> Result<PybiMetadata> {
        static NEXT_MAJOR_PYBI_VERSION: Lazy<Version> =
            Lazy::new(|| "2".try_into().unwrap());

        parse_bin_metadata_and_check_version(input, "Pybi-Version", &NEXT_MAJOR_PYBI_VERSION)?;
        Ok(PybiMetadata {})
    }
}
