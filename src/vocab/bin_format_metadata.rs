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
) -> Result<RFC822ish> {
    let input: &str = std::str::from_utf8(input)?;
    let mut parsed = RFC822ish::parse(&input)?;

    let version = parsed.take_the(version_field)?;
    if !version.starts_with("1.") {
        bail!("unsupported {}: {:?}", version_field, version);
    }

    Ok(parsed)
}

impl WheelMetadata {
    pub fn parse(input: &[u8]) -> Result<WheelMetadata> {
        let mut parsed = parse_bin_metadata_and_check_version(input, "Wheel-Version")?;

        let root_is_purelib = match &parsed.take_the("Root-Is-Purelib")?[..] {
            "true" => true,
            "false" => false,
            other => bail!(
                "Expected 'true' or 'false' for Root-Is-Purelib, not {}",
                other,
            ),
        };

        Ok(WheelMetadata { root_is_purelib })
    }
}

impl PybiMetadata {
    pub fn parse(input: &[u8]) -> Result<PybiMetadata> {
        parse_bin_metadata_and_check_version(input, "Pybi-Version")?;
        Ok(PybiMetadata {})
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_wheel_metadata() {
        let wm = WheelMetadata::parse(b"Wheel-Version: 1.0\nRoot-Is-Purelib: true\n")
            .unwrap();
        assert!(wm.root_is_purelib);

        let wm = WheelMetadata::parse(b"Wheel-Version: 1.0\nRoot-Is-Purelib: false\n")
            .unwrap();
        assert!(!wm.root_is_purelib);

        assert!(WheelMetadata::parse(b"").is_err());
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 2.0\nRoot-Is-Purelib: true\n")
                .is_err()
        );
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 1.0\nRoot-Is-Purelib: maybe\n")
                .is_err()
        );
        assert!(
            WheelMetadata::parse(b"Wheel-Version: 1.9\nRoot-Is-Purelib: true\n")
                .is_ok()
        );
    }

    #[test]
    fn test_pybi_metadata() {
        assert!(PybiMetadata::parse(b"").is_err());
        assert!(PybiMetadata::parse(b"Pybi-Version: 1.0\n").is_ok());
        assert!(PybiMetadata::parse(b"Pybi-Version: 1.9\n").is_ok());
        assert!(PybiMetadata::parse(b"Pybi-Version: 2.0\n").is_err());
    }
}
