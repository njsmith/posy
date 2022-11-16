pub use std::collections::{HashMap, HashSet};
pub use std::fmt::Display;
pub use std::io::{Read, Seek, Write};
pub use std::rc::Rc;
pub use std::str::FromStr;

pub use shrinkwraprs::Shrinkwrap;

pub use eyre::{eyre, bail, Result, WrapErr};
pub use derivative::Derivative;
pub use log::{info, trace, warn};
pub use once_cell::sync::Lazy;
pub use regex::Regex;
pub use serde::{Deserialize, Serialize};
pub use serde_with::{DeserializeFromStr, SerializeDisplay};
pub use url::Url;

pub use crate::platform_tags::{PybiPlatform, WheelPlatform, Platform};

pub use crate::tree::NicePathBuf;
pub use crate::try_from_str_boilerplate;
pub use crate::vocab::*;

pub use crate::context;

use directories::ProjectDirs;
pub static PROJECT_DIRS: Lazy<ProjectDirs> = Lazy::new(|| {
    // ...Can this actually return None?
    ProjectDirs::from("", "Trio Collective", env!("CARGO_PKG_NAME")).unwrap()
});

pub trait ReadPlusSeek: Read + Seek {}
impl<T> ReadPlusSeek for T where T: Read + Seek {}

pub fn slurp<T: Read>(f: &mut T) -> Result<Vec<u8>> {
    let mut data = Vec::<u8>::new();
    f.read_to_end(&mut data)?;
    Ok(data)
}
