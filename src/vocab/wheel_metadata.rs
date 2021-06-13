use crate::prelude::*;

use super::rfc822ish::RFC822ish;

#[derive(Debug, Clone)]
pub struct WheelMetadata {
    pub wheel_version: Version,
    pub root_is_purelib: bool,
}

impl WheelMetadata {
    pub fn parse(input: &[u8]) -> Result<WheelMetadata> {
        let input: &str = std::str::from_utf8(input)?;
        let mut parsed = RFC822ish::parse(&input)?;

        static NEXT_MAJOR_WHEEL_VERSION: Lazy<Version> =
            Lazy::new(|| "2".try_into().unwrap());

        let wheel_version = parsed.take_the("Wheel-Version")?.try_into()?;

        if wheel_version >= *NEXT_MAJOR_WHEEL_VERSION {
            bail!("unsupported Wheel-Version {}", wheel_version);
        }

        let root_is_purelib = match &parsed.take_the("Root-Is-Purelib")?[..] {
            "true" => true,
            "false" => false,
            other => bail!(
                "Expected 'true' or 'false' for Root-Is-Purelib, not {}",
                other,
            ),
        };

        Ok(WheelMetadata {
            wheel_version,
            root_is_purelib,
        })
    }
}
