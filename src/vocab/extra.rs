// 'Extra' string format is not well specified. It looks like what pip does is
// run things through pkg_resources.safe_extra, which does:
//
//   re.sub('[^A-Za-z0-9.-]+', '_', extra).lower()
//
// So A-Z becomes a-z, a-z 0-9 . - are preserved, and any contiguous run of
// other characters becomes a single _.
//
// OTOH, PEP 508's grammar for requirement specifiers says that extras have to
// be "identifiers", which means: first char [A-Za-z0-9], remaining chars also
// allowed to include -_.
//
// I guess for now I'll just pretend that they act the same as package names,
// and see how long I can get away with it.
//
// There's probably a better way to factor this and reduce code duplication...

use crate::prelude::*;

#[derive(Debug, Clone, Deserialize, Serialize, Hash, PartialEq, Eq)]
pub struct Extra(PackageName);

impl Extra {
    pub fn as_given(&self) -> &str {
        &self.0.as_given()
    }

    pub fn normalized(&self) -> &str {
        &self.0.normalized()
    }
}

impl TryFrom<&str> for Extra {
    type Error = eyre::Report;

    fn try_from(s: &str) -> Result<Self> {
        let p: PackageName = s.try_into()?;
        Ok(Extra(p))
    }
}

try_from_str_boilerplate!(Extra);
