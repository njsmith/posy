use crate::prelude::*;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum SdistFormat {
    Zip,
    TarGz,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct SdistName {
    pub distribution: PackageName,
    pub version: Version,
    pub format: SdistFormat,
}

impl TryFrom<&str> for SdistName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        static SDIST_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^(.*)-([^-]*)\.(zip|tar\.gz)$").unwrap());

        match SDIST_NAME_RE.captures(&value) {
            None => bail!("invalid sdist name"),
            Some(captures) => {
                let distribution: PackageName =
                    captures.get(1).unwrap().as_str().parse()?;
                let version: Version = captures.get(2).unwrap().as_str().parse()?;
                let format = match captures.get(3).unwrap().as_str() {
                    "zip" => SdistFormat::Zip,
                    "tar.gz" => SdistFormat::TarGz,
                    _ => unreachable!(),
                };
                Ok(SdistName {
                    distribution,
                    version,
                    format,
                })
            }
        }
    }
}

try_from_str_boilerplate!(SdistName);

impl Display for SdistName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}-{}.{}",
            self.distribution.as_given(),
            self.version,
            match self.format {
                SdistFormat::Zip => "zip",
                SdistFormat::TarGz => "tar.gz",
            }
        )
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DistInfoDirName {
    pub distribution: PackageName,
    pub version: Version,
}

impl TryFrom<&str> for DistInfoDirName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {

        static DIST_INFO_NAME_RE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^(.*)-([^-]*)\.(dist-info)$").unwrap());

        match DIST_INFO_NAME_RE.captures(&value) {
            None => bail!("invalid sdist name"),
            Some(captures) => {
                let distribution: PackageName =
                    captures.get(1).unwrap().as_str().parse()?;
                let version: Version = captures.get(2).unwrap().as_str().parse()?;
                Ok(DistInfoDirName {
                    distribution,
                    version,
                })
            }
        }
    }
}

try_from_str_boilerplate!(DistInfoDirName);

// https://packaging.python.org/specifications/binary-distribution-format/#file-name-convention
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct WheelName {
    pub distribution: PackageName,
    pub version: Version,
    // According to the spec, the build tag "sorts as an empty tuple if
    // unspecified, else sort as a two-item tuple with the first item being
    // the initial digits as an 'int', and the second item being the remainder
    // of the tag as a 'str'". This is ill-defined, b/c what the heck do
    // you do if there aren't any initial digits? So instead we do:
    //
    //   <no build tag> => (None, "")
    //   1              => (Some(1), "")
    //   1stuff         => (Some(1), "stuff")
    //   stuff          => (None, "stuff")
    //
    // And it sorts like a tuple, where None sorts ahead of Some.
    pub build_number: Option<u32>,
    pub build_name: String,
    // Should probably be OrderedSet, but there isn't one in the stdlib. Vec
    // takes less memory than a set, plus it preserves ordering, which is
    // somewhat nice, and removing duplicates doesn't really matter anyway
    // (.all_tags() will remove them too).
    pub py_tags: Vec<String>,
    pub abi_tags: Vec<String>,
    pub arch_tags: Vec<String>,
}

impl WheelName {
    pub fn all_tags(&self) -> HashSet<String> {
        let mut retval = HashSet::new();
        for py in &self.py_tags {
            for abi in &self.abi_tags {
                for arch in &self.arch_tags {
                    retval.insert(format!("{}-{}-{}", py, abi, arch));
                }
            }
        }
        retval
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct PybiName {
    pub distribution: PackageName,
    pub version: Version,
    pub build_number: Option<u32>,
    pub build_name: String,
    pub arch_tags: Vec<String>,
}

fn generic_parse<'a>(
    value: &'a str,
    suffix: &str,
    tag_parts: u8,
) -> Result<(PackageName, Version, Option<u32>, String, Vec<Vec<String>>)> {
    static VALID_CHARS: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"^[A-Za-z0-9_.+!-]*$").unwrap());

    // names/versions/etc. will be further validated by their respective
    // constructors. This is just to rule out real ridiculous stuff, like
    // spaces or control characters. I *think* this includes all the
    // characters that might reasonably appear in a package
    // name/version/build tag? There isn't actually a spec here, so we'll
    // see.
    if !VALID_CHARS.is_match(value) {
        bail!("Invalid characters in {} name: {:?}", suffix, value);
    }
    let stem = value
        .strip_suffix(suffix)
        .ok_or_else(|| anyhow!("expected {:?} to end in .{}", value, suffix))?;

    let mut pieces: Vec<&str> = stem.split('-').collect();

    static BUILD_TAG_SPLIT: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(^[0-9]*)(.*)$").unwrap());

    let build_number: Option<u32>;
    let build_name: String;
    if pieces.len() == 3 + tag_parts as usize {
        let build_tag = pieces.remove(2);
        if build_tag == "" {
            bail!("found empty build tag: {:?}", value);
        }
        // unwrap safe because: the regex cannot fail
        let captures = BUILD_TAG_SPLIT.captures(build_tag).unwrap();
        build_number = captures.get(1).map(|m| m.as_str().parse().ok()).flatten();
        // unwrap safe because: this group will always match something, even
        // if only the empty string
        build_name = captures.get(2).unwrap().as_str().into();
    } else {
        build_number = None;
        build_name = "".to_owned();
    }

    if pieces.len() != 2 + tag_parts as usize {
        anyhow::bail!("can't parse wheel name '{}'")
    }

    let distribution: PackageName = pieces[0].try_into()?;
    let version: Version = pieces[1].try_into()?;
    let tag_sets: Vec<Vec<String>> = pieces[2..]
        .into_iter()
        .map(|compressed_tag| compressed_tag.split(".").map(|tag| tag.into()).collect())
        .collect();

    Ok((distribution, version, build_number, build_name, tag_sets))
}

fn format_build_tag(build_number: Option<u32>, build_name: &str) -> String {
    match (build_number, &build_name[..]) {
        (None, "") => String::from(""),
        (None, name) => format!("-{}", name),
        (Some(num), name) => format!("-{}{}", num, name),
    }
}

impl TryFrom<&str> for WheelName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let (distribution, version, build_number, build_name, mut tag_sets) =
            generic_parse(value, ".whl", 3)?;

        tag_sets.reverse();

        Ok(WheelName {
            distribution,
            version,
            build_number,
            build_name,
            py_tags: tag_sets.pop().unwrap(),
            abi_tags: tag_sets.pop().unwrap(),
            arch_tags: tag_sets.pop().unwrap(),
        })
    }
}

try_from_str_boilerplate!(WheelName);


impl TryFrom<&str> for PybiName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let (distribution, version, build_number, build_name, mut tag_sets) =
            generic_parse(value, ".pybi", 1)?;

        Ok(PybiName {
            distribution,
            version,
            build_number,
            build_name,
            arch_tags: tag_sets.pop().unwrap(),
        })
    }
}

try_from_str_boilerplate!(PybiName);

impl Display for WheelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{dist}-{ver}{build}-{py_tags}-{abi_tags}-{arch_tags}.whl",
            dist = self.distribution.as_given(),
            ver = self.version,
            build = format_build_tag(self.build_number, &self.build_name),
            py_tags = self.py_tags.join("."),
            abi_tags = self.abi_tags.join("."),
            arch_tags = self.arch_tags.join("."),
        )
    }
}

impl Display for PybiName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{dist}-{ver}{build}-{arch_tags}.pybi",
            dist = self.distribution.as_given(),
            ver = self.version,
            build = format_build_tag(self.build_number, &self.build_name),
            arch_tags = self.arch_tags.join("."),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, SerializeDisplay)]
pub enum ArtifactName {
    Sdist(SdistName),
    Wheel(WheelName),
    Pybi(PybiName),
}

impl ArtifactName {
    pub fn distribution(&self) -> &PackageName {
        match self {
            ArtifactName::Sdist(inner) => &inner.distribution,
            ArtifactName::Wheel(inner) => &inner.distribution,
            ArtifactName::Pybi(inner) => &inner.distribution,
        }
    }

    pub fn version(&self) -> &Version {
        match self {
            ArtifactName::Sdist(inner) => &inner.version,
            ArtifactName::Wheel(inner) => &inner.version,
            ArtifactName::Pybi(inner) => &inner.version,
        }
    }
}

pub trait ArtifactNameUnwrap<T: Clone> {
    fn try_borrow(&self) -> Option<&T>;
}

macro_rules! impl_unwrap {
    ($enum:ident, $type:ty) => {
        impl ArtifactNameUnwrap<$type> for ArtifactName {
            fn try_borrow(&self) -> Option<&$type> {
                if let ArtifactName::$enum(inner) = self {
                    Some(inner)
                } else {
                    None
                }
            }
        }
    }
}

impl_unwrap!(Sdist, SdistName);
impl_unwrap!(Wheel, WheelName);
impl_unwrap!(Pybi, PybiName);

impl TryFrom<&str> for ArtifactName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.ends_with(".whl") {
            Ok(ArtifactName::Wheel(value.try_into()?))
        } else if value.ends_with(".pybi") {
            Ok(ArtifactName::Pybi(value.try_into()?))
        } else {
            Ok(ArtifactName::Sdist(value.try_into()?))
        }
    }
}

impl Display for ArtifactName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ArtifactName::Sdist(inner) => write!(f, "{}", &inner),
            ArtifactName::Wheel(inner) => write!(f, "{}", &inner),
            ArtifactName::Pybi(inner) => write!(f, "{}", &inner),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sdist_name_from_str() {
        let sn: SdistName = "trio-0.19a0.tar.gz".try_into().unwrap();
        assert_eq!(sn.distribution, "trio".try_into().unwrap());
        assert_eq!(sn.version, "0.19a0".try_into().unwrap());
    }

    #[test]
    fn test_wheel_name_from_str() {
        let n: WheelName = "trio-0.18.0-py3-none-any.whl".try_into().unwrap();
        assert_eq!(n.distribution, "trio".try_into().unwrap());
        assert_eq!(n.version, "0.18.0".try_into().unwrap());
        assert_eq!(n.build_number, None);
        assert_eq!(n.build_name, "");
        assert_eq!(n.py_tags, vec!["py3"]);
        assert_eq!(n.abi_tags, vec!["none"]);
        assert_eq!(n.arch_tags, vec!["any"]);

        assert_eq!(n.to_string(), "trio-0.18.0-py3-none-any.whl");
    }

    #[test]
    fn test_wheel_name_from_str_harder() {
        let n: WheelName = "foo.bar-0.1b3-1local-py2.py3-none-any.whl"
            .try_into()
            .unwrap();
        assert_eq!(n.distribution, "foo.bar".try_into().unwrap());
        assert_eq!(n.version, "0.1b3".try_into().unwrap());
        assert_eq!(n.build_number, Some(1));
        assert_eq!(n.build_name, "local");
        assert_eq!(n.py_tags, vec!["py2", "py3"],);
        assert_eq!(n.abi_tags, vec!["none"]);
        assert_eq!(n.arch_tags, vec!["any"]);

        assert_eq!(
            n.all_tags(),
            vec!["py2-none-any", "py3-none-any"]
                .into_iter()
                .map(|s| s.to_owned())
                .collect()
        );

        assert_eq!(n.to_string(), "foo.bar-0.1b3-1local-py2.py3-none-any.whl");
    }

    #[test]
    fn test_pybi_name_from_str() {
        let n: PybiName = "cpython-3.10b1-manylinux_2_17_x86_64.pybi"
            .try_into()
            .unwrap();
        assert_eq!(n.distribution, "cpython".try_into().unwrap());
        assert_eq!(n.version, "3.10b1".try_into().unwrap());
        assert_eq!(n.build_number, None);
        assert_eq!(n.build_name, "");
        assert_eq!(n.arch_tags, vec!["manylinux_2_17_x86_64"]);

        assert_eq!(n.to_string(), "cpython-3.10b1-manylinux_2_17_x86_64.pybi");
    }

    #[test]
    fn test_pybi_name_from_str_harder() {
        let n: PybiName = "foo.bar-0.1b3-1local-win32.win_amd64.pybi"
            .try_into()
            .unwrap();
        assert_eq!(n.distribution, "foo.bar".try_into().unwrap());
        assert_eq!(n.version, "0.1b3".try_into().unwrap());
        assert_eq!(n.build_number, Some(1));
        assert_eq!(n.build_name, "local");
        assert_eq!(n.arch_tags, vec!["win32", "win_amd64"]);

        assert_eq!(n.to_string(), "foo.bar-0.1b3-1local-win32.win_amd64.pybi");
    }
}
