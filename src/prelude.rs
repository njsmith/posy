pub use std::collections::{HashMap, HashSet};
pub use std::fmt::Display;
pub use std::io::{Read, Seek, Write};
pub use std::rc::Rc;
pub use std::str::FromStr;

pub use fn_error_context::context;
pub use shrinkwraprs::Shrinkwrap;

pub use anyhow::{anyhow, bail, Context, Result};
pub use derivative::Derivative;
pub use log::{info, trace, warn};
pub use once_cell::sync::Lazy;
pub use regex::Regex;
pub use serde::{Deserialize, Serialize};
pub use serde_with::{DeserializeFromStr, SerializeDisplay};
pub use url::Url;

pub use crate::try_from_str_boilerplate;
pub use crate::vocab::*;

use directories::ProjectDirs;
pub static PROJECT_DIRS: Lazy<ProjectDirs> = Lazy::new(|| {
    // ...Can this actually return None?
    ProjectDirs::from("", "Trio Collective", env!("CARGO_PKG_NAME")).unwrap()
});

pub trait ReadPlusSeek: Read + Seek {}
impl<T: Read + Seek> ReadPlusSeek for T {}

pub fn slurp<T: Read>(f: &mut T) -> Result<Vec<u8>> {
    let mut data = Vec::<u8>::new();
    f.read_to_end(&mut data)?;
    Ok(data)
}
