use std::collections::HashSet;
use once_cell::sync::Lazy;

use anyhow::Result;

use super::package_name::PackageName;
use super::rfc822ish::RFC822ish;
use pep440::Version;

/// There are more fields we could add here, but this should be good enough to
/// get started.
#[derive(Debug, Clone)]
pub struct CoreMetadata {
    pub metadata_version: Version,
    pub name: PackageName,
    pub version: Version,
    pub requires_dist: Vec<String>, // XXX newtype needed
    // or maybe there should be a "matches all" comparator object?
    pub requires_python: Option<String>, // XXX newtype needed
    pub extras: HashSet<String>,         // XXX newtype needed
}

impl CoreMetadata {
    pub fn parse(input: &[u8]) -> Result<CoreMetadata> {
        let input = String::from_utf8_lossy(input);
        let mut parsed = RFC822ish::parse(&input)?;

        static NEXT_MAJOR_METADATA_VERSION: Lazy<Version> = Lazy::new(|| {
            Version::parse("3").unwrap()
        });

        fn version(version_str: &str) -> Result<Version> {
            Version::parse(version_str)
                .ok_or(anyhow::anyhow!("Invalid version {}", version_str))
        }

        let retval = CoreMetadata {
            metadata_version: version(&parsed.take_the("Metadata-Version")?)?,
            name: parsed.take_the("Name")?.parse()?,
            version: version(&parsed.take_the("Version")?)?,
            requires_dist: parsed.take_all("Requires-Dist"),
            requires_python: parsed.maybe_take_the("Requires-Python")?,
            extras: parsed.take_all("Provides-Extra").drain(..).collect(),
        };

        // Quoth https://packaging.python.org/specifications/core-metadata:
        // "Automated tools consuming metadata SHOULD warn if metadata_version
        // is greater than the highest version they support, and MUST fail if
        // metadata_version has a greater major version than the highest
        // version they support (as described in PEP 440, the major version is
        // the value before the first dot)."
        //
        // We do the MUST, but I think I disagree about warning on
        // unrecognized minor revisions. If it's a minor revision, then by
        // definition old software is supposed to be able to handle it "well
        // enough". The only purpose of the warning would be to alert users
        // that they might want to upgrade, or to alert the tool authors that
        // there's a new metadata release. But for users, there are better
        // ways to nudge them to upgrade (e.g. checking on startup, like
        // pip does), and new metadata releases are so rare and so
        // much-discussed beforehand that if a package tool authors don't know
        // about it it's because the tool is abandoned anyway.
        if retval.metadata_version >= *NEXT_MAJOR_METADATA_VERSION {
            anyhow::bail!("unsupported Metadata-Version {}", retval.metadata_version);
        }

        Ok(retval)
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_basic_parse() {
        let metadata_text = indoc! {r#"
            Metadata-Version: 2.1
            Name: trio
            Version: 0.16.0
            Summary: A friendly Python library for async concurrency and I/O
            Classifier: Framework :: Trio
            Requires-Python: >=3.6
            Requires-Dist: attrs (>=19.2.0)
            Requires-Dist: sortedcontainers
            Requires-Dist: contextvars (>=2.1) ; python_version < "3.7"

            The Trio project's goal is...
        "#}
        .as_bytes();

        let metadata = CoreMetadata::parse(metadata_text).unwrap();

        assert_eq!(metadata.metadata_version, Version::parse("2.1").unwrap());
        assert_eq!(metadata.name.normalized(), "trio");
        assert_eq!(metadata.version, Version::parse("0.16.0").unwrap());
        assert_eq!(
            metadata.requires_dist,
            vec![
                "attrs (>=19.2.0)",
                "sortedcontainers",
                r#"contextvars (>=2.1) ; python_version < "3.7""#
            ]
        );
        assert_eq!(metadata.requires_python, Some(">=3.6".into()));
        assert_eq!(metadata.extras, HashSet::new());
    }
}
