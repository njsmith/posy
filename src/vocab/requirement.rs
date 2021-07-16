use crate::prelude::*;

// There are two kinds of special exact version constraints that aren't often
// used, and whose semantics are a bit unclear:
//
//  === "some string"
//  @ some_url
//
// Not sure if we should bother supporting them. For === they're easy to parse
// and represent (same as all the other binary comparisons), but I don't know
// what the semantics is, b/c we fully parse all versions. PEP 440 says "The
// primary use case ... is to allow for specifying a version which cannot
// otherwise by represented by this PEP". Maybe if we find ourselves supporting
// LegacyVersion-type versions, we should add this then? Though even then, I'm not sure
// we can convince pubgrub to handle it.
//
// If we do want to parse @ syntax, the problem is more: how do we represent
// them? Because it *replaces* version constraints, so I guess inside the
// Requirement object we'd need something like:
//
//   enum Specifiers {
//      Direct(Url),
//      Index(Vec<Specifier>),
//   }
//
// ? But then that complexity propagates through to everything that uses
// Requirements.
//
// Also, I don't think @ is allowed in public indexes like PyPI?
//
// NB: if we do decide to handle '@', then PEP 508 includes an entire copy of
// (some version of) the standard URL syntax. We don't want to do that, both
// because it's wildly more complicated than required, and because there are
// >3 different standards purpoting to define URL syntax and we don't want to
// take sides. But! The 'packaging' module just does
//
//    URI = Regex(r"[^ ]+")("url")
//
// ...so we can just steal some version of that.

pub mod marker {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Value {
        Variable(String),
        Literal(String),
    }

    #[derive(Debug, Copy, Clone, PartialEq, Eq)]
    pub enum Op {
        Compare(CompareOp),
        In,
        NotIn,
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub enum Expr {
        And(Box<Expr>, Box<Expr>),
        Or(Box<Expr>, Box<Expr>),
        Operator { op: Op, lhs: Value, rhs: Value },
    }

    pub trait Env {
        fn get_marker_var(&self, var: &str) -> Option<&str>;
    }

    impl Value {
        pub fn eval<'a>(&'a self, env: &'a dyn Env) -> Result<&'a str> {
            match self {
                Value::Variable(varname) => env
                    .get_marker_var(&varname)
                    .map(|s| s.as_ref())
                    .ok_or_else(|| {
                        anyhow!("no environment marker named '{}'", varname)
                    }),
                Value::Literal(s) => Ok(s),
            }
        }
    }

    impl Display for Value {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Value::Variable(var) => write!(f, "{}", var),
                Value::Literal(literal) => {
                    if literal.contains('"') {
                        write!(f, "'{}'", literal)
                    } else {
                        write!(f, "\"{}\"", literal)
                    }
                }
            }
        }
    }

    impl Expr {
        pub fn eval(&self, env: &dyn Env) -> Result<bool> {
            Ok(match self {
                Expr::And(lhs, rhs) => lhs.eval(env)? && rhs.eval(env)?,
                Expr::Or(lhs, rhs) => lhs.eval(env)? || rhs.eval(env)?,
                Expr::Operator { op, lhs, rhs } => {
                    let lhs_val = lhs.eval(env)?;
                    let rhs_val = rhs.eval(env)?;
                    match op {
                        Op::In => rhs_val.contains(lhs_val),
                        Op::NotIn => !rhs_val.contains(lhs_val),
                        Op::Compare(op) => {
                            // If both sides can be parsed as versions (or the RHS can
                            // be parsed as a wildcard with a wildcard-accepting op),
                            // then we do a version comparison
                            if let Ok(lhs_ver) = lhs_val.parse() {
                                if let Ok(rhs_ranges) = op.to_ranges(rhs_val) {
                                    return Ok(rhs_ranges
                                        .into_iter()
                                        .any(|r| r.contains(&lhs_ver)));
                                }
                            }
                            // Otherwise, we do a simple string comparison
                            use CompareOp::*;
                            match op {
                                LessThanEqual => lhs_val <= rhs_val,
                                StrictlyLessThan => lhs_val < rhs_val,
                                NotEqual => lhs_val != rhs_val,
                                Equal => lhs_val == rhs_val,
                                GreaterThanEqual => lhs_val >= rhs_val,
                                StrictlyGreaterThan => lhs_val > rhs_val,
                                Compatible => {
                                    bail!("~= requires valid version strings")
                                }
                            }
                        }
                    }
                }
            })
        }
    }

    impl Display for Expr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                // XX maybe it would be nice to reduce redundant parentheses here?
                Expr::And(lhs, rhs) => write!(f, "({} and {})", lhs, rhs)?,
                Expr::Or(lhs, rhs) => write!(f, "({} or {})", lhs, rhs)?,
                Expr::Operator { op, lhs, rhs } => write!(
                    f,
                    "{} {} {}",
                    lhs,
                    match op {
                        Op::Compare(compare_op) => compare_op.to_string(),
                        Op::In => "in".to_string(),
                        Op::NotIn => "not in".to_string(),
                    },
                    rhs,
                )?,
            }
            Ok(())
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum ParseExtra {
    Allowed,
    NotAllowed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Requirement {
    pub name: PackageName,
    pub extras: Vec<Extra>,
    pub specifiers: Specifiers,
    pub env_marker: Option<marker::Expr>,
}

impl Requirement {
    pub fn parse(input: &str, parse_extra: ParseExtra) -> Result<Requirement> {
        let req =
            super::reqparse::requirement(input, parse_extra).with_context(|| {
                format!("Failed parsing requirement string {:?})", input)
            })?;
        Ok(req)
    }
}

impl Display for Requirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name.as_given())?;
        if !self.extras.is_empty() {
            write!(f, "[")?;
            let mut first = true;
            for extra in &self.extras {
                if !first {
                    write!(f, ",")?;
                }
                first = false;
                write!(f, "{}", extra.as_given())?;
            }
            write!(f, "]")?;
        }
        if !self.specifiers.0.is_empty() {
            write!(f, " {}", self.specifiers)?;
        }
        if let Some(env_marker) = &self.env_marker {
            write!(f, "; {}", env_marker)?;
        }
        Ok(())
    }
}

#[derive(
    Shrinkwrap, Debug, Clone, PartialEq, Eq, DeserializeFromStr, SerializeDisplay,
)]
pub struct PackageRequirement(Requirement);

impl Display for PackageRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<&str> for PackageRequirement {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(PackageRequirement(Requirement::parse(
            value,
            ParseExtra::Allowed,
        )?))
    }
}

try_from_str_boilerplate!(PackageRequirement);

#[derive(
    Shrinkwrap, Debug, Clone, PartialEq, Eq, DeserializeFromStr, SerializeDisplay,
)]
pub struct UserRequirement(Requirement);

impl Display for UserRequirement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl TryFrom<&str> for UserRequirement {
    type Error = anyhow::Error;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        Ok(UserRequirement(Requirement::parse(
            value,
            ParseExtra::NotAllowed,
        )?))
    }
}

try_from_str_boilerplate!(UserRequirement);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_package_requirement_basics() {
        let r: PackageRequirement =
            "twisted[tls] >= 20, != 20.1.*; python_version >= '3' and extra == 'hi'"
                .try_into()
                .unwrap();
        insta::assert_debug_snapshot!(r);
    }

    #[test]
    fn test_user_requirement_basics() {
        assert!(UserRequirement::try_from("twisted; extra == 'hi'").is_err());
        let r: UserRequirement = "twisted[tls] >= 20, != 20.1.*; python_version >= '3'"
            .try_into()
            .unwrap();
        insta::assert_debug_snapshot!(r);
    }

    #[test]
    fn test_requirement_roundtrip() {
        let reqs = vec![
            "foo",
            "foo (>=2, <3)",
            "foo >=1,<2, ~=3.1, ==0.0.*, !=7, >10, <= 8",
            "foo[bar,baz, quux]",
            "foo; python_version >= '3' and sys_platform == \"win32\" or sys_platform != \"linux\"",
            "foo.bar-baz (~=7); 'win' in sys_platform or 'linux' not in sys_platform",
        ];
        for req in reqs {
            let ur: UserRequirement = req.try_into().unwrap();
            assert_eq!(ur, ur.to_string().try_into().unwrap());

            let pr: PackageRequirement = req.try_into().unwrap();
            assert_eq!(pr, pr.to_string().try_into().unwrap());
        }
    }
}
