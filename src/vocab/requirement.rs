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
// otherwise by represented by this PEP".
//
// If we do want to parse @ syntax, the problem is more: how do we represent
// them? Because it *replaces* version constraints, so I guess inside the
// Requirement object we'd need something like:
//
//   enum Constraints {
//      Direct(Url),
//      Index(Vec<Constraint>),
//   }
//
// ? But then that complexity propagates through to everything that uses
// Requirements.
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Constraint {
    pub op: CompareOp,
    pub value: String,
}

impl Constraint {
    pub fn satisfied_by(&self, version: &Version) -> Result<bool> {
        Ok(self
            .op
            .to_ranges(&self.value)?
            .into_iter()
            .any(|r| r.contains(version)))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequiresPython {
    pub constraints: Vec<Constraint>,
}

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

    // XX switch this trait, and maybe to avoid allocating in here?
    // pub trait Env {
    //     fn get_marker_var(&self, var: &str) -> Option<&str>;
    // }

    impl Value {
        pub fn eval(&self, env: &HashMap<String, String>) -> Result<String> {
            match self {
                Value::Variable(varname) => {
                    env.get(varname).map(|s| s.clone()).ok_or_else(|| {
                        anyhow!("no environment marker named '{}'", varname)
                    })
                }
                Value::Literal(s) => Ok(s.clone()),
            }
        }
    }

    impl Expr {
        pub fn eval(&self, env: &HashMap<String, String>) -> Result<bool> {
            Ok(match self {
                Expr::And(lhs, rhs) => lhs.eval(env)? && rhs.eval(env)?,
                Expr::Or(lhs, rhs) => lhs.eval(env)? || rhs.eval(env)?,
                Expr::Operator { op, lhs, rhs } => {
                    let lhs_val = lhs.eval(env)?;
                    let rhs_val = rhs.eval(env)?;
                    match op {
                        Op::In => rhs_val.contains(&lhs_val),
                        Op::NotIn => !rhs_val.contains(&lhs_val),
                        Op::Compare(op) => {
                            // If both sides can be parsed as versions (or the RHS can
                            // be parsed as a wildcard with a wildcard-accepting op),
                            // then we do a version comparison
                            if let Ok(lhs_ver) = lhs_val.parse() {
                                if let Ok(rhs_ranges) = op.to_ranges(&rhs_val) {
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
                                Compatible => bail!(
                                    "~= requires valid version strings"
                                ),
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
    pub constraints: Vec<Constraint>,
    pub env_marker: Option<marker::Expr>,
}

peg::parser! {
    grammar parser() for str {
        rule wsp()
            = quiet!{ [' ' | '\t' ] }

        rule letter_or_digit()
            = quiet!{['A'..='Z' | 'a'..='z' | '0'..='9']} / expected!("letter or digit")

        rule _()
            = quiet!{ wsp()* }

        rule version_cmp() -> &'input str
            = $("<=" / "<" / "!=" / "==" / ">=" / ">" / "~=" / "===")

        rule version()
            = (letter_or_digit() / "-" / "_" / "." / "*" / "+" / "!")+

        rule version_one() -> Constraint
            = _ op:version_cmp() _ v:$(version())
            {?
                use CompareOp::*;
                Ok(Constraint {
                    op: match &op[..] {
                        "==" => Equal,
                        "!=" => NotEqual,
                        "<=" => LessThanEqual,
                        "<" => StrictlyLessThan,
                        ">=" => GreaterThanEqual,
                        ">" => StrictlyGreaterThan,
                        "~=" => Compatible,
                        "===" => return Err("'===' is not implemented"),
                        _ => panic!("op can't be {:?}!", op)
                    },
                    value: v.into(),
                })
            }

        rule version_many() -> Vec<Constraint>
            = version_one() ++ (_ ",")

        pub rule versionspec() -> Vec<Constraint>
            = ("(" vm:version_many() ")" { vm }) / version_many()

        rule urlspec() -> Requirement
            = "@" {? Err("direct url references not currently supported") }

        rule not_in() -> &'static str
            = "not" wsp()+ "in" { "not in" }

        rule marker_op() -> &'input str
            = _ op:(version_cmp() / $("in") / not_in()) { op }

        rule python_str_c() -> &'input str
            = $(quiet! { [' ' | '\t' | 'A'..='Z' | 'a'..='z' | '0'..='9' | '(' | ')'
                 | '.' | '{' | '}' | '-' | '_' | '*' | '#' | ':' | ';' | ','
                 | '/' | '?' | '[' | ']' | '!' | '~' | '`' | '@' | '$' | '%'
                 | '^' | '&' | '=' | '+' | '|' | '<' | '>'] })
              / expected!("printable character")

        // PEP 508 says that we don't have to support backslash escapes. It
        // also says that "existing implementations do support them", so the
        // first statement might be a lie -- maybe they're actually in use in
        // the wild. But they're complicated, so we might as well see how far
        // we can get while sticking to the spec.
        rule python_squote_str() -> &'input str
            = "'" s:$((python_str_c() / "\"")*) "'" { s }

        rule python_dquote_str() -> &'input str
            = "\"" s:$((python_str_c() / "'")*) "\"" { s }

        rule python_str() -> marker::Value
            = s:(python_squote_str() / python_dquote_str())
              { marker::Value::Literal(s.to_owned()) }

        rule env_var(parse_extra: ParseExtra) -> marker::Value
            = var:$(
                "python_version" / "python_full_version" / "os_name"
                / "sys_platform" / "platform_release" / "platform_system"
                / "platform_version" / "platform_machine"
                / "platform_python_implementation" / "implementation_name"
                / "implementation_version" / "extra"
              )
              {?
               if ParseExtra::NotAllowed == parse_extra && var == "extra" {
                   return Err("'extra' marker is not valid in this context")
               }
               Ok(marker::Value::Variable(var.to_owned()))
              }

        rule marker_var(parse_extra: ParseExtra) -> marker::Value
            = _ v:(env_var(parse_extra) / python_str()) { v }

        rule marker_expr(parse_extra: ParseExtra) -> marker::Expr
            = _ "(" m:marker(parse_extra) _ ")" { m }
              / lhs:marker_var(parse_extra) op:marker_op() rhs:marker_var(parse_extra)
              {
                  use marker::Expr::Operator;
                  use CompareOp::*;
                  use marker::Op::*;
                  match &op[..] {
                      "<=" => Operator { op: Compare(LessThanEqual), lhs, rhs },
                      "<" => Operator { op: Compare(StrictlyLessThan), lhs, rhs },
                      "!=" => Operator { op: Compare(NotEqual), lhs, rhs },
                      "==" => Operator { op: Compare(Equal), lhs, rhs },
                      ">=" => Operator { op: Compare(GreaterThanEqual), lhs, rhs },
                      ">" => Operator { op: Compare(StrictlyGreaterThan), lhs, rhs },
                      "~=" => Operator { op: Compare(Compatible), lhs, rhs },
                      "in" => Operator { op: In, lhs, rhs },
                      "not in" => Operator { op: NotIn, lhs, rhs },
                      _ => panic!("op can't be {:?}!", op),
                  }
              }

        rule marker_and(parse_extra: ParseExtra) -> marker::Expr
            = lhs:marker_expr(parse_extra) _ "and" _ rhs:marker_expr(parse_extra)
                 { marker::Expr::And(Box::new(lhs), Box::new(rhs)) }
              / marker_expr(parse_extra)

        rule marker_or(parse_extra: ParseExtra) -> marker::Expr
            = lhs:marker_and(parse_extra) _ "or" _ rhs:marker_and(parse_extra)
                 { marker::Expr::Or(Box::new(lhs), Box::new(rhs)) }
              / marker_and(parse_extra)

        rule marker(parse_extra: ParseExtra) -> marker::Expr
            = marker_or(parse_extra)

        rule quoted_marker(parse_extra: ParseExtra) -> marker::Expr
            = ";" _ m:marker(parse_extra) { m }

        rule identifier() -> &'input str
            = $(letter_or_digit() (letter_or_digit() / "-" / "_" / ".")*)

        rule name() -> PackageName
            = n:identifier() {? n.try_into().or(Err("Error parsing package name")) }

        rule extra() -> Extra
            = e:identifier() {? e.try_into().or(Err("Error parsing extra name")) }

        rule extras() -> Vec<Extra>
            = "[" _ es:(extra() ** (_ "," _)) _ "]" { es }

        rule name_req(parse_extra: ParseExtra) -> Requirement
            = name:name()
              _ extras:(extras() / "" { Vec::new() })
              _ constraints:(versionspec() / "" { Vec::new() })
              _ env_marker:(quoted_marker(parse_extra)?)
              {
                  Requirement {
                      name,
                      extras,
                      constraints,
                      env_marker,
                  }
              }

        rule url_req(parse_extra: ParseExtra) -> Requirement
            = name:name()
              _ extras:(extras() / "" { Vec::new() })
              _ url:urlspec()
              _ env_marker:((wsp() q:quoted_marker(parse_extra) { q })?)
            {
                // because urlspec() errors out unconditionally, up above
                unreachable!()
            }

        pub rule specification(parse_extra: ParseExtra) -> Requirement
            = _ r:( url_req(parse_extra) / name_req(parse_extra) ) _ { r }
    }
}

impl Requirement {
    pub fn parse(input: &str, parse_extra: ParseExtra) -> Result<Requirement> {
        let req = parser::specification(input, parse_extra).with_context(|| {
            format!("Failed parsing requirement string {:?})", input)
        })?;
        Ok(req)
    }
}

impl TryFrom<&str> for RequiresPython {
    type Error = anyhow::Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let constraints_or_err = parser::versionspec(input);
        constraints_or_err
            .map(|constraints| RequiresPython { constraints })
            .with_context(|| {
                format!("failed to parse Requires-Python string {:?}", input)
            })
    }
}

try_from_str_boilerplate!(RequiresPython);

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_smoke() {
        let r: Requirement = parser::specification(
            "twisted[tls] >= 20, != 20.1.*; python_version >= '3'",
            ParseExtra::Allowed,
        )
        .unwrap();
        println!("{:?}", r);
    }
}
