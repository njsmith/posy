use crate::prelude::*;

// https://packaging.python.org/specifications/binary-distribution-format/#file-name-convention
#[derive(PartialEq, Eq, Debug)]
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
    // (.tags() will remove them too).
    pub py_tags: Vec<String>,
    pub abi_tags: Vec<String>,
    pub arch_tags: Vec<String>,
}

impl WheelName {
    pub fn compressed_tag(&self) -> String {
        format!(
            "{}-{}-{}",
            self.py_tags.join("."),
            self.abi_tags.join("."),
            self.arch_tags.join(".")
        )
    }

    pub fn tags(&self) -> HashSet<String> {
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

impl TryFrom<&str> for WheelName {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        // names/versions/etc. will be further validated by their respective
        // constructors. This is just to rule out real ridiculous stuff, like
        // spaces or control characters. I *think* this includes all the
        // characters that might reasonably appear in a package
        // name/version/build tag? There isn't actually a spec here, so we'll
        // see.
        static VALID_CHARS: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"^[A-Za-z0-9_.+!-]*$").unwrap());

        if !VALID_CHARS.is_match(value) {
            anyhow::bail!("Invalid characters in wheel name {:?}", value);
        }

        let basename = value.strip_suffix(".whl").unwrap_or(value);

        let mut pieces: Vec<&str> = basename.split('-').collect();

        let build_tag;
        if pieces.len() == 6 {
            build_tag = pieces.remove(2)
        } else {
            build_tag = "";
        }

        static BUILD_TAG_SPLIT: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"(^[0-9]*)(.*)$").unwrap());

        // unwrap safe because: the regex cannot fail
        let captures = BUILD_TAG_SPLIT.captures(build_tag).unwrap();
        let build_number: Option<u32> =
            captures.get(1).map(|m| m.as_str().parse().ok()).flatten();
        // unwrap safe because: this group will always match something, even
        // if only the empty string
        let build_name = captures.get(2).unwrap().as_str();

        if pieces.len() != 5 {
            anyhow::bail!("can't parse wheel name '{}'")
        }

        fn split_compressed(tag: &str) -> Vec<String> {
            tag.split(".").map(|p| p.into()).collect()
        }

        Ok(WheelName {
            distribution: pieces[0].try_into()?,
            version: pieces[1].try_into()?,
            build_number,
            build_name: build_name.into(),
            py_tags: split_compressed(pieces[2]),
            abi_tags: split_compressed(pieces[3]),
            arch_tags: split_compressed(pieces[4]),
        })
    }
}

try_from_str_boilerplate!(WheelName);

impl Display for WheelName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let build_tag = match (self.build_number, &self.build_name[..]) {
            (None, "") => String::from(""),
            (None, name) => format!("-{}", name),
            (Some(num), name) => format!("-{}{}", num, name),
        };
        write!(
            f,
            "{dist}-{ver}{build}-{ctag}.whl",
            dist = self.distribution,
            ver = self.version,
            build = build_tag,
            ctag = self.compressed_tag()
        )
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_wheel_name_from_str() {
        let wn: WheelName = "trio-0.18.0-py3-none-any.whl".try_into().unwrap();
        assert_eq!(wn.distribution, "trio".try_into().unwrap());
        assert_eq!(wn.version, "0.18.0".try_into().unwrap());
        assert_eq!(wn.build_number, None);
        assert_eq!(wn.build_name, "");
        assert_eq!(wn.py_tags, vec!["py3"]);
        assert_eq!(wn.abi_tags, vec!["none"]);
        assert_eq!(wn.arch_tags, vec!["any"]);

        assert_eq!(wn.to_string(), "trio-0.18.0-py3-none-any.whl");
    }

    #[test]
    fn test_wheel_name_from_str_harder() {
        let wn: WheelName = "foo.bar-0.1b3-1local-py2.py3-none-any.whl"
            .try_into()
            .unwrap();
        assert_eq!(wn.distribution, "foo.bar".try_into().unwrap());
        assert_eq!(wn.version, "0.1b3".try_into().unwrap());
        assert_eq!(wn.build_number, Some(1));
        assert_eq!(wn.build_name, "local");
        assert_eq!(wn.py_tags, vec!["py2", "py3"],);
        assert_eq!(wn.abi_tags, vec!["none"]);
        assert_eq!(wn.arch_tags, vec!["any"]);

        assert_eq!(wn.compressed_tag(), "py2.py3-none-any");
        assert_eq!(
            wn.tags(),
            vec!["py2-none-any", "py3-none-any"]
                .drain(..)
                .map(|s| s.to_owned())
                .collect()
        );

        assert_eq!(wn.to_string(), "foo.bar-0.1b3-1local-py2.py3-none-any.whl");
    }
}

/*

pub struct TagPreference {
    tag_to_priority: HashMap<String, usize>,
}

// XX should be from iterable maybe?
impl<T: IntoIterator<Item = String>> From<T> for TagPreference {
    fn from(tags: T) -> Self {
        let retval = HashMap::new();

        for (i, tag) in tags.into_iter().enumerate() {
            retval.entry(tag).or_insert(i);
        }

        TagPreference { tag_to_priority: retval }
    }
}

pub fn best_wheel<T>(wheels: T, tag_pref: &TagPreference)
    where T: IntoIterator<Item = WheelName>
{
    for wheel in wheels {
    }
}

*/
