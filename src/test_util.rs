use crate::prelude::*;

pub fn from_commented_json<T>(input: &str) -> T
where
    T: serde::de::DeserializeOwned,
{
    static COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"#.*").unwrap());

    let replaced = COMMENT.replace_all(input, "");
    serde_json::from_str(&replaced).unwrap()
}
