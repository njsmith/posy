pub use std::collections::{HashMap, HashSet};
pub use std::convert::{TryFrom, TryInto};
pub use std::fmt::Display;
pub use std::rc::Rc;
pub use std::str::FromStr;

pub use anyhow::{anyhow, bail, Context, Result};
pub use derivative::Derivative;
pub use once_cell::sync::Lazy;
pub use regex::Regex;
pub use serde::{Deserialize, Serialize};
pub use serde_with::{DeserializeFromStr, SerializeDisplay};
pub use url::Url;
pub use log::{info, trace, warn};

pub use crate::try_from_str_boilerplate;
pub use crate::vocab::*;

use directories::ProjectDirs;
pub static PROJECT_DIRS: Lazy<ProjectDirs> = Lazy::new(|| {
    // ...Can this actually return None?
    ProjectDirs::from("", "Trio Collective", env!("CARGO_PKG_NAME")).unwrap()
});
