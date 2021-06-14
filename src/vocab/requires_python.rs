use crate::prelude::*;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiresPython {
    pub specifiers: Vec<Specifier>,
}

impl TryFrom<&str> for RequiresPython {
    type Error = anyhow::Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let specifiers_or_err = super::reqparse::versionspec(input);
        specifiers_or_err
            .map(|specifiers| RequiresPython { specifiers })
            .with_context(|| {
                format!("failed to parse Requires-Python string {:?}", input)
            })
    }
}

try_from_str_boilerplate!(RequiresPython);
