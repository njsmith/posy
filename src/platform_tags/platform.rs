use super::expand::expand_platform_tag;
use crate::prelude::*;
use indexmap::IndexSet;
use once_cell::sync::OnceCell;

fn compatibility(tags: &IndexSet<String>, tag: &str) -> Option<i32> {
    tags.get_index_of(tag).map(|score| -(score as i32))
}

#[derive(Debug, Clone)]
pub struct PybiPlatform {
    tags: IndexSet<String>,
}

#[derive(Debug, Clone)]
pub struct WheelPlatform {
    tags: IndexSet<String>,
}

pub trait Platform {
    fn tags(&self) -> indexmap::set::Iter<'_, String>;

    fn compatibility(&self, tag: &str) -> Option<i32>;

    fn max_compatibility<T, S>(&self, tags: T) -> Option<i32>
    where
        T: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        tags.into_iter()
            .filter_map(|t| self.compatibility(t.as_ref()))
            .max()
    }
}

impl Platform for PybiPlatform {
    fn tags(&self) -> indexmap::set::Iter<'_, String> {
        self.tags.iter()
    }

    fn compatibility(&self, tag: &str) -> Option<i32> {
        compatibility(&self.tags, tag)
    }
}

impl Platform for WheelPlatform {
    fn tags(&self) -> indexmap::set::Iter<'_, String> {
        self.tags.iter()
    }

    fn compatibility(&self, tag: &str) -> Option<i32> {
        compatibility(&self.tags, tag)
    }
}

static NATIVE_PLATFORMS: OnceCell<Vec<PybiPlatform>> = OnceCell::new();

static NATIVE_PLATFORM_REFS: OnceCell<Vec<&'static PybiPlatform>> = OnceCell::new();

/// A PybiPlatform represents a family of mutually-consistent ABIs; e.g. "win32" or
/// "x86-64 manylinux 2.20 or less".
///
/// Each platform object has a "core tag", which is the most restrictive tag it supports
/// (e.g. manylinux_2_20_x86_64), and then zero or more secondary tags that are implied
/// by the core tag (e.g. manylinux_2_19_x86_64, manylinux_2_18_x86_64, ...).
///
/// A PybiPlatform is "native" if all the ABIs it represents can be run on the local
/// system (the one that posy is running on). `native_platforms` returns (our best
/// attempt to figure out) all the ABIs that can be run on the current system.
impl PybiPlatform {
    pub fn new(core_tag: &str) -> PybiPlatform {
        PybiPlatform {
            tags: expand_platform_tag(core_tag.as_ref()).into_iter().collect(),
        }
    }

    pub fn core_tag(&self) -> &str {
        &self.tags[0]
    }

    pub fn native_platforms() -> Result<&'static [&'static PybiPlatform]> {
        let platforms = NATIVE_PLATFORMS.get_or_try_init(|| -> Result<_> {
            let tags = super::core_platform_tags()?
                .iter()
                .map(|s| PybiPlatform::new(&s))
                .collect();

            Ok(tags)
        })?;
        let refs = NATIVE_PLATFORM_REFS.get_or_init(|| platforms.iter().collect());
        Ok(refs.as_slice())
    }

    pub fn is_native(&self) -> Result<bool> {
        let natives = PybiPlatform::native_platforms()?;
        Ok(natives
            .iter()
            .any(|native| native.compatibility(self.core_tag()).is_some()))
    }

    pub fn wheel_platform(&self, metadata: &PybiCoreMetadata) -> Result<WheelPlatform> {
        let mut wheel_tags = IndexSet::new();
        for wheel_tag_template in &metadata.tags {
            if let Some(prefix) = wheel_tag_template.strip_suffix("-PLATFORM") {
                for platform_tag in &self.tags {
                    wheel_tags.insert(format!("{prefix}-{platform_tag}"));
                }
            } else {
                wheel_tags.insert(wheel_tag_template.into());
            }
        }

        Ok(WheelPlatform { tags: wheel_tags })
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_pybi_platform() {
        let platform = PybiPlatform::new("manylinux2014_x86_64");
        println!("{:#?}", platform);

        assert!(platform.compatibility("manylinux_2_17_x86_64").is_some());
        assert!(platform.compatibility("manylinux_2_10_x86_64").is_some());
        assert!(platform.compatibility("manylinux_2_17_aarch64").is_none());
        assert!(platform.compatibility("manylinux_2_30_x86_64").is_none());
        assert!(
            platform.compatibility("manylinux_2_17_x86_64").unwrap()
                > platform.compatibility("manylinux_2_10_x86_64").unwrap()
        );
    }

    #[test]
    fn test_pybi_platform_to_wheel_platform() {
        let pybi_platform = PybiPlatform::new("macosx_11_0_arm64");

        let fake_metadata: PybiCoreMetadata = indoc! {b"
            Metadata-Version: 2.1
            Name: cpython
            Version: 3.11
            Pybi-Environment-Marker-Variables: {}
            Pybi-Paths: {}
            Pybi-Wheel-Tag: foo-bar-PLATFORM
            Pybi-Wheel-Tag: foo-none-any
            Pybi-Wheel-Tag: foo-baz-PLATFORM
        "}
        .as_slice()
        .try_into()
        .unwrap();

        // given a pybi that can handle both, on a platform that can handle both, pick
        // the preferred platform and restrict to it.
        let wheel_platform = pybi_platform.wheel_platform(&fake_metadata).unwrap();
        assert!(wheel_platform
            .compatibility("foo-bar-macosx_11_0_arm64")
            .is_some());
        assert!(wheel_platform
            .compatibility("foo-bar-macosx_11_0_x86_64")
            .is_none());

        // also tags are sorted properly
        assert!(
            wheel_platform
                .compatibility("foo-bar-macosx_11_0_arm64")
                .unwrap()
                > wheel_platform
                    .compatibility("foo-bar-macosx_10_0_arm64")
                    .unwrap()
        );
        assert!(
            wheel_platform
                .compatibility("foo-bar-macosx_10_0_arm64")
                .unwrap()
                > wheel_platform.compatibility("foo-none-any").unwrap()
        );
        assert!(
            wheel_platform.compatibility("foo-none-any").unwrap()
                > wheel_platform
                    .compatibility("foo-baz-macosx_11_0_arm64")
                    .unwrap()
        );
    }
}
