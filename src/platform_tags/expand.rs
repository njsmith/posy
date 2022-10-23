use crate::prelude::*;

static LINUX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^(many|musl)linux_([0-9]+)_([0-9]+)_(.*)").unwrap());

static MACOSX_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^macosx_([0-9]+)_([0-9]+)_(.*)").unwrap());

// Given a platform tag like "manylinux_2_17_x86_64" or "win32", returns a vector of
// other platform tags that are guaranteed to be supported on any machine that supports
// the given tag. The vector is sorted so "better" tags come before "worse" tags.
//
// Unrecognized tags are passed through unchanged.
pub fn expand_platform_tag(tag: &str) -> Vec<String> {
    if let Some(captures) = LINUX_RE.captures(tag) {
        let variant = captures.get(1).unwrap().as_str();
        let major: u32 = captures.get(2).unwrap().as_str().parse().unwrap();
        let minor: u32 = captures.get(3).unwrap().as_str().parse().unwrap();
        let arch = captures.get(4).unwrap().as_str();

        return (0..=minor)
            .rev()
            .flat_map(|minor| {
                let mut tags =
                    vec![format!("{}linux_{}_{}_{}", variant, major, minor, arch)];
                if variant == "many" {
                    match (major, minor) {
                        (2, 17) => tags.push(format!("manylinux2014_{}", arch)),
                        (2, 12) => tags.push(format!("manylinux2010_{}", arch)),
                        (2, 5) => tags.push(format!("manylinux1_{}", arch)),
                        _ => (),
                    }
                }
                tags
            })
            .collect();
    }

    if let Some(captures) = MACOSX_RE.captures(tag) {
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
                        .map(move |arch| format!("macos_{}_{}_{}", major, minor, arch))
                })
                .collect();
        }
    }

    // fallback/passthrough
    vec![tag.to_string()]
}

pub fn current_platform_tags() -> Result<Vec<String>> {
    Ok(super::core_platform_tags()?.drain(..).flat_map(|t| expand_platform_tag(&t)).collect())
}
