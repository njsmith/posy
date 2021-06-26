use crate::prelude::*;

#[derive(PartialEq, Eq, Debug)]
pub struct SdistName {
    pub distribution: PackageName,
    pub version: Version,
}

impl TryFrom<&str> for SdistName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        static SDIST_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^([^-]*)-([^-]*)\.(zip|tar\.gz)").unwrap());

        match SDIST_NAME_RE.captures(&value) {
            None => bail!("invalid sdist name"),
            Some(captures) => {
                let distribution: PackageName =
                    captures.get(1).unwrap().as_str().parse()?;
                let version: Version = captures.get(2).unwrap().as_str().parse()?;
                Ok(SdistName {
                    distribution,
                    version,
                })
            }
        }
    }
}

try_from_str_boilerplate!(SdistName);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sdist_name_from_str() {
        let sn: SdistName = "trio-0.19a0.tar.gz".try_into().unwrap();
        assert_eq!(sn.distribution, "trio".try_into().unwrap());
        assert_eq!(sn.version, "0.19a0".try_into().unwrap());
    }
}
