use std::path::Path;

use crate::prelude::*;

use super::rfc822ish::RFC822ish;

/// There are more fields we could add here, but this should be good enough to
/// get started.
#[derive(Debug, Clone)]
#[cfg_attr(test, derive(Serialize))]
pub struct WheelCoreMetadata {
    pub name: PackageName,
    pub version: Version,
    pub requires_dist: Vec<PackageRequirement>,
    pub requires_python: Specifiers,
    pub extras: HashSet<Extra>,
}

#[derive(Debug, Clone)]
#[cfg_attr(test, derive(Serialize))]
pub struct PybiCoreMetadata {
    pub name: PackageName,
    pub version: Version,
    pub environment_marker_variables: HashMap<String, String>,
    pub tags: Vec<String>,
    pub paths: HashMap<String, String>,
}

impl PybiCoreMetadata {
    pub fn path(&self, key: &str) -> Result<&Path> {
        Ok(self.paths.get(key).ok_or(anyhow!("bad pybi: no '{key}' path"))?.as_ref())
    }
}

fn parse_common(input: &[u8]) -> Result<(PackageName, Version, RFC822ish)> {
    let input = String::from_utf8_lossy(input);
    let mut parsed = RFC822ish::parse(&input)?;

    static NEXT_MAJOR_METADATA_VERSION: Lazy<Version> =
        Lazy::new(|| "3".try_into().unwrap());

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
    // much-discussed beforehand that if a tool's authors don't know
    // about it it's because the tool is abandoned anyway.
    let metadata_version: Version = parsed.take_the("Metadata-Version")?.try_into()?;
    if metadata_version >= *NEXT_MAJOR_METADATA_VERSION {
        bail!("unsupported Metadata-Version {}", metadata_version);
    }

    Ok((
        parsed.take_the("Name")?.parse()?,
        parsed.take_the("Version")?.try_into()?,
        parsed,
    ))
}

impl TryFrom<&[u8]> for WheelCoreMetadata {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let (name, version, mut parsed) = parse_common(value)?;

        let mut requires_dist = Vec::new();
        for req_str in parsed.take_all("Requires-Dist").drain(..) {
            requires_dist.push(req_str.try_into()?);
        }

        let requires_python = match parsed.maybe_take_the("Requires-Python")? {
            Some(rp_str) => rp_str.try_into()?,
            None => Specifiers(Vec::new()),
        };

        let mut extras: HashSet<Extra> = HashSet::new();
        for extra in parsed.take_all("Provides-Extra").drain(..) {
            extras.insert(extra.parse()?);
        }

        Ok(WheelCoreMetadata {
            name,
            version,
            requires_dist,
            requires_python,
            extras,
        })
    }
}

impl TryFrom<&[u8]> for PybiCoreMetadata {
    type Error = anyhow::Error;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let (name, version, mut parsed) = parse_common(value)?;

        Ok(PybiCoreMetadata {
            name,
            version,
            environment_marker_variables: serde_json::from_str(
                &parsed.take_the("Pybi-Environment-Marker-Variables")?,
            )?,
            tags: parsed.take_all("Pybi-Wheel-Tag"),
            paths: serde_json::from_str(&parsed.take_the("Pybi-Paths")?)?,
        })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_basic_core_parse() {
        let metadata_text = indoc! {r#"
            Metadata-Version: 2.1
            Name: trio
            Version: 0.16.0
            Summary: A friendly Python library for async concurrency and I/O
            Classifier: Framework :: Trio
            Requires-Python: >=3.6
            Requires-Dist: attrs (>=19.2.0)
            Requires-Dist: sortedcontainers
            Requires-Dist: contextvars[foo] (>=2.1) ; python_version < "3.7"

            The Trio project's goal is...
        "#}
        .as_bytes();

        let metadata: WheelCoreMetadata = metadata_text.try_into().unwrap();

        insta::assert_ron_snapshot!(metadata, @r###"
        WheelCoreMetadata(
          name: "trio",
          version: "0.16.0",
          requires_dist: [
            "attrs >= 19.2.0",
            "sortedcontainers",
            "contextvars[foo] >= 2.1; python_version < \"3.7\"",
          ],
          requires_python: ">= 3.6",
          extras: [],
        )
        "###);
    }

    #[test]
    fn test_basic_pybi_parse() {
        let metadata_text = indoc! {r#"
            Metadata-Version: 2.1
            Name: CPython
            Version: 3.11.2
            Pybi-Environment-Marker-Variables: {"implementation_name": "cpython", "os_name": "posix"}
            pybi-wheel-tag: cp311-cp311-linux_x86_64
            Pybi-Wheel-Tag: py3-none-any
            Pybi-Paths: {"data": ".", "include": "include/python3.11"}

            This is CPython, the standard interpreter for the Python language...
        "#}
        .as_bytes();

        let metadata: PybiCoreMetadata = metadata_text.try_into().unwrap();

        insta::assert_ron_snapshot!(metadata,
            {
                ".paths" => insta::sorted_redaction(),
                ".environment_marker_variables" => insta::sorted_redaction(),
            },
                                    @r###"
        PybiCoreMetadata(
          name: "CPython",
          version: "3.11.2",
          environment_marker_variables: {
            "implementation_name": "cpython",
            "os_name": "posix",
          },
          tags: [
            "cp311-cp311-linux_x86_64",
            "py3-none-any",
          ],
          paths: {
            "data": ".",
            "include": "include/python3.11",
          },
        )
        "###
        );
    }
}
