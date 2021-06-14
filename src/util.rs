use crate::prelude::*;

/// Work around an annoyance in Rust's standard traits -- if you define
/// TryFrom<&str>, then you probably also want TryFrom<String> and FromStr,
/// and the implementation is trivial in terms of TryFrom<&str>. So this macro
/// just generates the boilerplate for you.
#[macro_export]
macro_rules! try_from_str_boilerplate {
    ($name:ident) => {
        impl std::convert::TryFrom<String> for $name {
            type Error = anyhow::Error;

            fn try_from(s: String) -> Result<Self, Self::Error> {
                (&*s).try_into()
            }
        }

        impl std::str::FromStr for $name {
            type Err = anyhow::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                s.try_into()
            }
        }
    };
}

#[cfg(test)]
pub fn from_commented_json<T>(input: &str) -> T
    where T: serde::de::DeserializeOwned
{
    static COMMENT: Lazy<Regex> = Lazy::new(|| {
        Regex::new(r"#.*").unwrap()
    });

    let replaced = COMMENT.replace_all(input, "");
    serde_json::from_str(&replaced).unwrap()
}
