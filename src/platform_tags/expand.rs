use std::borrow::Cow;

use crate::prelude::*;

static LINUX_RE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"^(many|musl)linux_([0-9]+)_([0-9]+)_([a-zA-Z0-9_]*)$").unwrap()
});

static LEGACY_MANYLINUX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^manylinux(2014|2010|1)_([a-zA-Z0-9_]*)").unwrap());

static MACOSX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^macosx_([0-9]+)_([0-9]+)_([a-zA-Z0-9_]*)$").unwrap());

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

#[cfg(test)]
mod test {
    use super::*;

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
