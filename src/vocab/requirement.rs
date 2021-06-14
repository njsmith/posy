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
        Literal(Rc<str>),
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
        Operator {
            op: Op,
            lhs: Value,
            rhs: Value,
        },
    }

    use std::rc::Rc;
    pub trait Env {
        fn get_marker_var(&self, var: &str) -> Option<Rc<str>>;
    }

    impl Value {
        pub fn eval(&self, env: &dyn Env) -> Result<Rc<str>> {
            match self {
                Value::Variable(varname) => {
                    env.get_marker_var(&varname).ok_or_else(|| {
                        anyhow!("no environment marker named '{}'", varname)
                    })
                }
                Value::Literal(s) => Ok(s.clone()),
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
                        Op::In => rhs_val.contains(lhs_val.as_ref()),
                        Op::NotIn => !rhs_val.contains(lhs_val.as_ref()),
                        Op::Compare(op) => {
                            // If both sides can be parsed as versions (or the RHS can
                            // be parsed as a wildcard with a wildcard-accepting op),
                            // then we do a version comparison
                            if let Ok(lhs_ver) = lhs_val.parse() {
                                if let Ok(rhs_ranges) = op.to_ranges(rhs_val.as_ref()) {
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
    pub specifiers: Vec<Specifier>,
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

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_smoke() {
        let r = Requirement::parse(
            "twisted[tls] >= 20, != 20.1.*; python_version >= '3'",
            ParseExtra::Allowed,
        )
        .unwrap();
        println!("{:?}", r);
    }
}
