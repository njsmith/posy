use crate::prelude::*;

#[derive(Debug, Clone, DeserializeFromStr, Derivative)]
#[derivative(Hash, PartialEq, Eq)]
pub struct PackageName {
    #[derivative(Hash = "ignore", PartialEq = "ignore")]
    as_given: String,
    normalized: String,
}

impl PackageName {
    pub fn as_given(&self) -> &str {
        &self.as_given
    }

    pub fn normalized(&self) -> &str {
        &self.normalized
    }
}

impl TryFrom<&str> for PackageName {
    type Error = anyhow::Error;

    fn try_from(as_given: &str) -> Result<Self, Self::Error> {
        // https://packaging.python.org/specifications/core-metadata/#name
        static NAME_VALIDATE: Lazy<Regex> = Lazy::new(|| {
            Regex::new(r"(?i-u)^([A-Z0-9]|[A-Z0-9][A-Z0-9._-]*[A-Z0-9])$").unwrap()
        });
        // https://www.python.org/dev/peps/pep-0503/#normalized-names
        static NAME_NORMALIZE: Lazy<Regex> =
            Lazy::new(|| Regex::new(r"[-_.]").unwrap());

        if !NAME_VALIDATE.is_match(as_given) {
            return Err(anyhow!("Invalid package name {:?}", as_given));
        }
        let as_given = as_given.to_owned();

        let mut normalized = NAME_NORMALIZE.replace_all(&as_given, "-").to_string();
        normalized.make_ascii_lowercase();

        Ok(PackageName {
            as_given,
            normalized,
        })
    }
}

try_from_str_boilerplate!(PackageName);

#[cfg(test)]
mod test {
    use std::convert::TryInto;

    use super::*;

    #[test]
    fn test_packagename_basics() {
        let name1: PackageName = "Foo-Bar-Baz".try_into().unwrap();
        assert_eq!(name1.as_given(), "Foo-Bar-Baz");
        assert_eq!(name1.normalized(), "foo-bar-baz");

        let name2: PackageName = "foo_bar.baz".try_into().unwrap();
        assert_eq!(name2.as_given(), "foo_bar.baz");
        assert_eq!(name2.normalized(), "foo-bar-baz");

        assert_eq!(name1, name2);

        let name3: PackageName = "foo-barbaz".try_into().unwrap();
        assert_ne!(name1, name3);
    }

    #[test]
    fn test_packagename_validation() {
        let name: Result<PackageName> = "foobar baz".try_into();
        assert!(name.is_err());

        let name: Result<PackageName> = "foobarbaz!".parse();
        assert!(name.is_err());
    }

    #[test]
    fn test_packagename_serde() {
        let direct: PackageName = "foo-bar_baz".try_into().unwrap();
        let via_serde: Vec<PackageName> =
            serde_json::from_str(r#"["foo_bar.baz"]"#).unwrap();
        assert_eq!(via_serde[0], direct);
        assert_eq!(via_serde[0].as_given(), "foo_bar.baz");
        assert_eq!(via_serde[0].normalized(), "foo-bar-baz");

        let bad: serde_json::Result<PackageName> =
            serde_json::from_str(r#" "foo bar" "#);
        assert!(bad.is_err());
    }

    #[test]
    fn test_packagename_hash() {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        fn calculate_hash<T: Hash>(t: &T) -> u64 {
            let mut s = DefaultHasher::new();
            t.hash(&mut s);
            s.finish()
        }

        let name1: PackageName = "foo_bar".try_into().unwrap();
        let name2: PackageName = "foo.bar".try_into().unwrap();

        let name_other: PackageName = "foobar".try_into().unwrap();

        assert_eq!(calculate_hash(&name1), calculate_hash(&name2));
        // Technically there's a very small chance of this failing due to an unlucky
        // collision, but it's 1/2**64 assuming we have a good hash function, and only
        // re-sampled when Rust changes their hash function. So I think we're safe.
        assert_ne!(calculate_hash(&name1), calculate_hash(&name_other));
    }
}
