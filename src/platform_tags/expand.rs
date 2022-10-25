use std::borrow::Cow;

use crate::prelude::*;

static LINUX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(many|musl)linux_([0-9]+)_([0-9]+)_([a-zA-Z0-9_]*)$").unwrap()
});

static LEGACY_MANYLINUX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^manylinux(2014|2010|1)_([a-zA-Z0-9_]*)").unwrap());

static MACOSX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^macosx_([0-9]+)_([0-9]+)_([a-zA-Z0-9_]*)$").unwrap());

#[derive(Debug, Clone)]
pub struct Platform {
    // smaller number = more preferred
    tag_map: HashMap<String, i32>,
    // earlier = more preferred
    tags: Vec<String>,
    counter: i32,
}

impl Platform {
    pub fn from_core_tag(tag: &str) -> Platform {
        Platform::from_core_tags(&[tag])
    }

    fn empty() -> Platform {
        Platform {
            tag_map: Default::default(),
            tags: Default::default(),
            counter: 0,
        }
    }

    fn push(&mut self, tag: String) {
        if !self.tag_map.contains_key(&tag) {
            self.tag_map.insert(tag.clone(), self.counter);
            self.tags.push(tag);
            self.counter -= 1;
        }
    }

    // assumes core tags are sorted from most-preferred to least-preferred
    pub fn from_core_tags<'a, T, S>(tags: T) -> Platform
    where
        T: IntoIterator<Item = S>,
        S: AsRef<str> + 'a,
    {
        let mut p = Platform::empty();
        for tag in tags.into_iter() {
            for expansion in expand_platform_tag(tag.as_ref()) {
                p.push(expansion)
            }
        }
        p
    }

    pub fn current_platform() -> Result<Platform> {
        Ok(Platform::from_core_tags(super::core_platform_tags()?))
    }

    pub fn compatibility(&self, tag: &str) -> Option<i32> {
        self.tag_map.get(tag).map(|score| *score)
    }

    // create a new Platform that includes a subset of tags from the current platform,
    // and which are all in the same compat group, and also all in the given pybi's
    // compat group.
    //
    // strategy: make a set of preliminary groups based on the pybi. then look at our
    // tags from best -> worst, and use those to restrict the set of groups until
    // there's just one left. (b/c of iteration order, it will be the most-preferred
    // group for our Platform.) that's our group; restrict ourselves to the subset of
    // tags that match that group and return it as a new Platform.
    pub fn restrict(&self, pybi_arch_tags: &Vec<String>) -> Result<Platform> {
        let mut groups = HashSet::<String>::new();
        for tag in pybi_arch_tags.iter() {
            groups.extend(compat_groups(tag)?);
        }
        assert!(!groups.is_empty());
        for tag in &self.tags {
            if groups.len() == 1 {
                break;
            }
            let tag_groups: HashSet<String> = compat_groups(tag)?.into_iter().collect();
            groups.retain(|group| tag_groups.contains(group));
        }
        assert!(groups.len() == 1);
        let the_group = groups.into_iter().next().unwrap();
        let mut p = Platform::empty();
        for tag in &self.tags {
            if compat_groups(tag)?.contains(&the_group) {
                p.push(tag.to_owned());
            }
        }
        Ok(p)
    }

    pub fn infer_platform_machine(&self) -> Result<&'static str> {
        let mut possible = HashSet::<&'static str>::new();
        for tag in &self.tags {
            for compat_group in compat_groups(&tag).unwrap_or(vec![]) {
                match compat_group.as_str() {
                    "macos-x86_64" => {
                        possible.insert("x86_64");
                    },
                    "macos-arm64" => {
                        possible.insert("arm64");
                    },
                    _ => (),
                }
            }
        }
        if possible.len() > 1 {
            bail!("macOS platform selected, but can't tell if you want arm64 or x86-64");
        }
        if possible.len() < 1 {
            bail!("can't infer platform_machine for this platform/pybi");
        }
        Ok(possible.iter().next().unwrap())
    }
}

// the goal here is to figure out which platform tags can possibly co-exist within a
// single process. E.g. all manylinux_*_x86_64 tags can potentially co-exist, but
// manylinux+musllinux can't co-exist, and neither can x86_64 + arm64. To model this, we
// say a "compat group" is an ad hoc string like "manylinux-x86_64", that only includes
// the parts that necessarily determine compatibility.
//
// It's a Vec because some tags, like macosx_*_universal2, are ambiguous: it could fit
// in a process with macosx-x86_64, or macosx-arm64.
fn compat_groups(tag: &str) -> Result<Vec<String>> {
    // Windows tags are all unique
    if tag.starts_with("win") {
        return Ok(vec![tag.into()]);
    }
    if let Some(captures) = MACOSX_RE.captures(tag) {
        let arch = captures.get(3).unwrap().as_str();
        let compat_arches = match arch {
            "x86_64" | "intel" | "fat64" | "fat3" | "universal" => vec!["x86_64"],
            "arm64" => vec!["arm64"],
            "universal2" => vec!["x86_64", "arm64"],
            _ => bail!("Unrecognized macOS architecture {arch}"),
        };
        return Ok(compat_arches
            .into_iter()
            .map(|a| format!("macos-{a}"))
            .collect());
    }
    if let Some(captures) = LINUX_RE.captures(tag) {
        let variant = captures.get(1).unwrap().as_str();
        let arch = captures.get(4).unwrap().as_str();
        return Ok(vec![format!("{variant}linux-{arch}")]);
    }
    if let Some(captures) = LEGACY_MANYLINUX_RE.captures(tag) {
        let arch = captures.get(2).unwrap().as_str();
        return Ok(vec![format!("manylinux-{arch}")]);
    }
    bail!("unsupported platform tag {tag}");
}

// Given a platform tag like "manylinux_2_17_x86_64" or "win32", returns a vector of
// other platform tags that are guaranteed to be supported on any machine that supports
// the given tag. The vector is sorted so "better" tags come before "worse" tags.
//
// Unrecognized tags are passed through unchanged.
pub fn expand_platform_tag(tag: &str) -> Vec<String> {
    let mut tag = Cow::Borrowed(tag);
    if let Some(captures) = LEGACY_MANYLINUX_RE.captures(tag.as_ref()) {
        let which = captures.get(1).unwrap().as_str();
        let arch = captures.get(2).unwrap().as_str();
        let new_prefix = match which {
            "2014" => "manylinux_2_17",
            "2010" => "manylinux_2_12",
            "1" => "manylinux_2_5",
            _ => unreachable!(), // enforced by the regex pattern
        };
        tag = Cow::Owned(format!("{}_{}", new_prefix, arch));
    }

    if let Some(captures) = LINUX_RE.captures(tag.as_ref()) {
        let variant = captures.get(1).unwrap().as_str();
        let major: u32 = captures.get(2).unwrap().as_str().parse().unwrap();
        let max_minor: u32 = captures.get(3).unwrap().as_str().parse().unwrap();
        let arch = captures.get(4).unwrap().as_str();

        let mut tags = Vec::<String>::new();
        for minor in (0..=max_minor).rev() {
            tags.push(format!("{variant}linux_{major}_{minor}_{arch}"));
            if variant == "many" {
                match (major, minor) {
                    (2, 17) => tags.push(format!("manylinux2014_{}", arch)),
                    (2, 12) => tags.push(format!("manylinux2010_{}", arch)),
                    (2, 5) => tags.push(format!("manylinux1_{}", arch)),
                    _ => (),
                }
            }
        }
        return tags;
    }

    if let Some(captures) = MACOSX_RE.captures(tag.as_ref()) {
        let major: u32 = captures.get(1).unwrap().as_str().parse().unwrap();
        let minor: u32 = captures.get(2).unwrap().as_str().parse().unwrap();
        let arch = captures.get(3).unwrap().as_str();

        if major >= 10 {
            // arch has to be x86_64 or arm64, not universal2/intel/etc.
            // because if all you know is that a machine can run universal2 binaries, this
            // actually tells you nothing whatsoever about whether it can run x86_64 or
            // arm64 binaries! (it might only run the other kind). I guess it does tell you
            // that it can run universal2 binaries though?
            // If someone requests pins for universal2, we should probably hard-code that to
            // instead pin for {x86_64, arm64} (though in many cases they'll be the same,
            // b/c there are in fact universal2 pybis?)
            // (no point in supporting ppc/ppc64/i386 at this point)
            let arches = match arch {
                // https://docs.python.org/3/distutils/apiref.html#distutils.util.get_platform
                "x86_64" => vec![
                    "x86_64",
                    "universal2",
                    "intel",
                    "fat64",
                    "fat3",
                    "universal",
                ],
                "arm64" => vec!["arm64", "universal2"],
                _ => vec![arch],
            };

            let max_10_minor = if major == 10 { minor } else { 15 };
            let macos_10_vers = (0..=max_10_minor).rev().map(|minor| (10, minor));
            let macos_11plus_vers = (11..=major).rev().map(|major| (major, 0));
            let all_vers = macos_11plus_vers.chain(macos_10_vers);

            return all_vers
                .flat_map(|(major, minor)| {
                    arches
                        .iter()
                        .map(move |arch| format!("macosx_{}_{}_{}", major, minor, arch))
                })
                .collect();
        }
    }

    // fallback/passthrough
    vec![tag.to_string()]
}

pub fn current_platform_tags() -> Result<Vec<String>> {
    Ok(super::core_platform_tags()?
        .drain(..)
        .flat_map(|t| expand_platform_tag(&t))
        .collect())
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_platform() {
        let platform = Platform::from_core_tag("manylinux2014_x86_64");
        println!("{:#?}", platform);

        assert!(platform.compatibility("manylinux_2_17_x86_64").is_some());
        assert!(platform.compatibility("manylinux_2_10_x86_64").is_some());
        assert!(platform.compatibility("manylinux_2_17_aarch64").is_none());
        assert!(platform.compatibility("manylinux_2_30_x86_64").is_none());
        assert!(
            platform.compatibility("manylinux_2_17_x86_64").unwrap()
                > platform.compatibility("manylinux_2_10_x86_64").unwrap()
        );

        let multi_platform =
            Platform::from_core_tags(["manylinux2014_x86_64", "musllinux_1_3_x86_64"]);
        println!("{:#?}", multi_platform);
        assert!(
            multi_platform
                .compatibility("manylinux_2_17_x86_64")
                .unwrap()
                > multi_platform
                    .compatibility("musllinux_1_2_x86_64")
                    .unwrap()
        );
    }

    #[test]
    fn test_platform_restrict() {
        let platform =
            Platform::from_core_tags(vec!["macosx_11_0_arm64", "macosx_11_0_x86_64"]);

        // given a pybi that can handle both, on a platform that can handle both, pick
        // the preferred platform and restrict to it.
        let arm_only = platform.restrict(&vec!["macosx_10_15_universal2".to_owned()]).unwrap();
        assert!(arm_only.compatibility("macosx_11_0_arm64").is_some());
        assert!(arm_only.compatibility("macosx_11_0_x86_64").is_none());

        // but if the pybi only supports one, go with that
        let x86_64_only = platform.restrict(&vec!["macosx_10_15_x86_64".to_owned()]).unwrap();
        assert!(x86_64_only.compatibility("macosx_11_0_arm64").is_none());
        assert!(x86_64_only.compatibility("macosx_11_0_x86_64").is_some());
    }

    #[test]
    fn test_expand_platform_tag() {
        insta::assert_ron_snapshot!(expand_platform_tag("win32"), @r###"
        [
          "win32",
        ]
        "###);
        insta::assert_ron_snapshot!(expand_platform_tag("win_amd64"), @r###"
        [
          "win_amd64",
        ]
        "###);

        insta::assert_ron_snapshot!(expand_platform_tag("macosx_10_10_x86_64"), @r###"
        [
          "macosx_10_10_x86_64",
          "macosx_10_10_universal2",
          "macosx_10_10_intel",
          "macosx_10_10_fat64",
          "macosx_10_10_fat3",
          "macosx_10_10_universal",
          "macosx_10_9_x86_64",
          "macosx_10_9_universal2",
          "macosx_10_9_intel",
          "macosx_10_9_fat64",
          "macosx_10_9_fat3",
          "macosx_10_9_universal",
          "macosx_10_8_x86_64",
          "macosx_10_8_universal2",
          "macosx_10_8_intel",
          "macosx_10_8_fat64",
          "macosx_10_8_fat3",
          "macosx_10_8_universal",
          "macosx_10_7_x86_64",
          "macosx_10_7_universal2",
          "macosx_10_7_intel",
          "macosx_10_7_fat64",
          "macosx_10_7_fat3",
          "macosx_10_7_universal",
          "macosx_10_6_x86_64",
          "macosx_10_6_universal2",
          "macosx_10_6_intel",
          "macosx_10_6_fat64",
          "macosx_10_6_fat3",
          "macosx_10_6_universal",
          "macosx_10_5_x86_64",
          "macosx_10_5_universal2",
          "macosx_10_5_intel",
          "macosx_10_5_fat64",
          "macosx_10_5_fat3",
          "macosx_10_5_universal",
          "macosx_10_4_x86_64",
          "macosx_10_4_universal2",
          "macosx_10_4_intel",
          "macosx_10_4_fat64",
          "macosx_10_4_fat3",
          "macosx_10_4_universal",
          "macosx_10_3_x86_64",
          "macosx_10_3_universal2",
          "macosx_10_3_intel",
          "macosx_10_3_fat64",
          "macosx_10_3_fat3",
          "macosx_10_3_universal",
          "macosx_10_2_x86_64",
          "macosx_10_2_universal2",
          "macosx_10_2_intel",
          "macosx_10_2_fat64",
          "macosx_10_2_fat3",
          "macosx_10_2_universal",
          "macosx_10_1_x86_64",
          "macosx_10_1_universal2",
          "macosx_10_1_intel",
          "macosx_10_1_fat64",
          "macosx_10_1_fat3",
          "macosx_10_1_universal",
          "macosx_10_0_x86_64",
          "macosx_10_0_universal2",
          "macosx_10_0_intel",
          "macosx_10_0_fat64",
          "macosx_10_0_fat3",
          "macosx_10_0_universal",
        ]
        "###);
        insta::assert_ron_snapshot!(expand_platform_tag("macosx_12_0_universal2"), @r###"
        [
          "macosx_12_0_universal2",
          "macosx_11_0_universal2",
          "macosx_10_15_universal2",
          "macosx_10_14_universal2",
          "macosx_10_13_universal2",
          "macosx_10_12_universal2",
          "macosx_10_11_universal2",
          "macosx_10_10_universal2",
          "macosx_10_9_universal2",
          "macosx_10_8_universal2",
          "macosx_10_7_universal2",
          "macosx_10_6_universal2",
          "macosx_10_5_universal2",
          "macosx_10_4_universal2",
          "macosx_10_3_universal2",
          "macosx_10_2_universal2",
          "macosx_10_1_universal2",
          "macosx_10_0_universal2",
        ]
        "###);

        insta::assert_ron_snapshot!(expand_platform_tag("manylinux_2_3_aarch64"), @r###"
        [
          "manylinux_2_3_aarch64",
          "manylinux_2_2_aarch64",
          "manylinux_2_1_aarch64",
          "manylinux_2_0_aarch64",
        ]
        "###);

        insta::assert_ron_snapshot!(expand_platform_tag("manylinux1_x86_64"), @r###"
        [
          "manylinux_2_5_x86_64",
          "manylinux1_x86_64",
          "manylinux_2_4_x86_64",
          "manylinux_2_3_x86_64",
          "manylinux_2_2_x86_64",
          "manylinux_2_1_x86_64",
          "manylinux_2_0_x86_64",
        ]
        "###);

        insta::assert_ron_snapshot!(expand_platform_tag("manylinux_2_24_x86_64"), @r###"
        [
          "manylinux_2_24_x86_64",
          "manylinux_2_23_x86_64",
          "manylinux_2_22_x86_64",
          "manylinux_2_21_x86_64",
          "manylinux_2_20_x86_64",
          "manylinux_2_19_x86_64",
          "manylinux_2_18_x86_64",
          "manylinux_2_17_x86_64",
          "manylinux2014_x86_64",
          "manylinux_2_16_x86_64",
          "manylinux_2_15_x86_64",
          "manylinux_2_14_x86_64",
          "manylinux_2_13_x86_64",
          "manylinux_2_12_x86_64",
          "manylinux2010_x86_64",
          "manylinux_2_11_x86_64",
          "manylinux_2_10_x86_64",
          "manylinux_2_9_x86_64",
          "manylinux_2_8_x86_64",
          "manylinux_2_7_x86_64",
          "manylinux_2_6_x86_64",
          "manylinux_2_5_x86_64",
          "manylinux1_x86_64",
          "manylinux_2_4_x86_64",
          "manylinux_2_3_x86_64",
          "manylinux_2_2_x86_64",
          "manylinux_2_1_x86_64",
          "manylinux_2_0_x86_64",
        ]
        "###);

        insta::assert_ron_snapshot!(expand_platform_tag("musllinux_1_2_x86_64"), @r###"
        [
          "musllinux_1_2_x86_64",
          "musllinux_1_1_x86_64",
          "musllinux_1_0_x86_64",
        ]
        "###);
    }
}
