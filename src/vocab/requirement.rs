use crate::prelude::*;

// There are two kinds of special exact version constraints that aren't often
// used, and whose semantics are a bit unclear:
//
//  === "some string"
//  @ some_url
//
// not sure if we should bother supporting them... currently we do parse ===,
// but error out on the @ syntax.
//
// If we do want to parse @ syntax, then PEP 508 includes an entire copy of
// (some version of) the standard URL syntax. We don't want to do that, both
// because it's wildly more complicated than required, and because there are
// >3 different standards purpoting to define URL syntax and we don't want to
// take sides. But! The 'packaging' module just does
//
//    URI = Regex(r"[^ ]+")("url")
//
// ...so we can just steal some version of that. The bigger question is how to
// represent a URI-based requirement, given that they don't *have* version
// constraints.

#[derive(Debug, Clone)]
pub enum Constraint {
    LessThanEqual(Version),
    StrictlyLessThan(Version),
    NotEqual { version: Version, wildcard: bool },
    Equal { version: Version, wildcard: bool },
    GreaterThanEqual(Version),
    StrictlyGreaterThan(Version),
    Compatible(Version),
    Exactly(String),
}

#[derive(Debug, Clone)]
pub struct RequiresPython {
    pub constraints: Vec<Constraint>,
}

#[derive(Debug, Clone)]
pub enum MarkerValue {
    Variable(String),
    Literal(String),
}

#[derive(Debug, Copy, Clone)]
pub enum MarkerOp {
    LessThanEqual,
    StrictlyLessThan,
    NotEqual,
    Equal,
    GreaterThanEqual,
    StrictlyGreaterThan,
    Compatible,
    In,
    NotIn,
    Exactly,
}

#[derive(Debug, Clone)]
pub enum Marker {
    And(Box<Marker>, Box<Marker>),
    Or(Box<Marker>, Box<Marker>),
    Comparison {
        op: MarkerOp,
        lhs: MarkerValue,
        rhs: MarkerValue,
    },
}

#[derive(Debug, Copy, Clone)]
pub enum ParseExtra {
    Allowed,
    NotAllowed,
}

#[derive(Debug, Clone)]
pub struct Requirement {
    name: PackageName,
    extras: Vec<Extra>,
    constraints: Vec<Constraint>,
    env_marker: Option<Marker>,
}

// A version of 'parse_version' that uses &'static str as its error type, to
// work around limitations in the 'peg' create
fn parse_version_peg(input: &str) -> std::result::Result<Version, &'static str> {
    Version::parse(input).ok_or("Failed to parse PEP 440 version")
}

fn parse_version_wildcard(
    input: &str,
) -> std::result::Result<(Version, bool), &'static str> {
    let (vstr, wildcard) = if let Some(vstr) = input.strip_suffix(".*") {
        (vstr, true)
    } else {
        (input, false)
    };
    let version = parse_version_peg(vstr)?;
    Ok((version, wildcard))
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
            = $("<=" / "<" / "!=" / "===" / "==" / ">=" / ">" / "~=")

        rule version()
            = (letter_or_digit() / "-" / "_" / "." / "*" / "+" / "!")+

        rule version_one() -> Constraint
            = _ op:version_cmp() _ v:$(version())
            {?
                use Constraint::*;
                Ok(match &op[..] {
                    "===" => Exactly(v.into()),
                    "==" | "!=" => {
                        let (version, wildcard) = parse_version_wildcard(v)?;
                        if op == "==" {
                            Equal { version, wildcard }
                        } else {
                            NotEqual { version, wildcard }
                        }
                    },
                    _ => {
                        let version = parse_version_peg(v)?;
                        match &op[..] {
                            "<=" => LessThanEqual(version),
                            "<" => StrictlyLessThan(version),
                            ">=" => GreaterThanEqual(version),
                            ">" => StrictlyGreaterThan(version),
                            "~=" => Compatible(version),
                            _ => panic!("op can't be {:?}!", op)
                        }
                    },
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

        rule python_str() -> MarkerValue
            = s:(python_squote_str() / python_dquote_str())
              { MarkerValue::Literal(s.to_owned()) }

        rule env_var(parse_extra: ParseExtra) -> MarkerValue
            = var:$(
                "python_version" / "python_full_version" / "os_name"
                / "sys_platform" / "platform_release" / "platform_system"
                / "platform_version" / "platform_machine"
                / "platform_python_implementation" / "implementation_name"
                / "implementation_version" / "extra"
              )
              {?
               if let ParseExtra::NotAllowed = parse_extra {
                   if var == "extra" {
                       return Err("'extra' marker is not valid in this context")
                   }
               }
               Ok(MarkerValue::Variable(var.to_owned()))
              }

        rule marker_var(parse_extra: ParseExtra) -> MarkerValue
            = _ v:(env_var(parse_extra) / python_str()) { v }

        rule marker_expr(parse_extra: ParseExtra) -> Marker
            = _ "(" m:marker(parse_extra) _ ")" { m }
              / lhs:marker_var(parse_extra) op:marker_op() rhs:marker_var(parse_extra)
              {
                  use Marker::Comparison;
                  use MarkerOp::*;
                  match &op[..] {
                      "<=" => Comparison { op: LessThanEqual, lhs, rhs },
                      "<" => Comparison { op: StrictlyLessThan, lhs, rhs },
                      "!=" => Comparison { op: NotEqual, lhs, rhs },
                      "==" => Comparison { op: Equal, lhs, rhs },
                      ">=" => Comparison { op: GreaterThanEqual, lhs, rhs },
                      ">" => Comparison { op: StrictlyGreaterThan, lhs, rhs },
                      "~=" => Comparison { op: Compatible, lhs, rhs },
                      "in" => Comparison { op: In, lhs, rhs },
                      "not in" => Comparison { op: NotIn, lhs, rhs },
                      "===" => Comparison { op: Exactly, lhs, rhs },
                      _ => panic!("op can't be {:?}!", op),
                  }
              }

        rule marker_and(parse_extra: ParseExtra) -> Marker
            = lhs:marker_expr(parse_extra) _ "and" _ rhs:marker_expr(parse_extra)
                 { Marker::And(Box::new(lhs), Box::new(rhs)) }
              / marker_expr(parse_extra)

        rule marker_or(parse_extra: ParseExtra) -> Marker
            = lhs:marker_and(parse_extra) _ "or" _ rhs:marker_and(parse_extra)
                 { Marker::Or(Box::new(lhs), Box::new(rhs)) }
              / marker_and(parse_extra)

        rule marker(parse_extra: ParseExtra) -> Marker
            = marker_or(parse_extra)

        rule quoted_marker(parse_extra: ParseExtra) -> Marker
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

// I guess the other way to do it would be to evaluate the requirements with a
// known python, and 'extra' set to <matches nothing>, <extra1>, <extra2> , ...
// and for each one do concrete evaluation? Or do abstract evaluation and only
// evaluate "extra"?

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
    fn test_parse() {
        let r: Requirement = parser::specification(
            "twisted[tls] >= 20, != 20.1.*; python_version >= '3'",
            ParseExtra::Allowed,
        )
        .unwrap();
        println!("{:?}", r);
        panic!()
    }
}

// Some ideas for working with MarkerExprs?

// #[derive(Debug)]
// pub enum MarkerAccum<T> {
//     And(T, T),
//     Or(T, T),
//     Comparison {
//         op: MarkerOp,
//         lhs: MarkerValue,
//         rhs: MarkerValue,
//     },
// }

// impl MarkerExpr {
//     pub fn reduce<T, F>(self, f: F) -> T
//     where
//         F: FnMut(MarkerAccum<T>) -> T,
//     {
//         match self {
//             MarkerExpr::And(lhs, rhs) => {
//                 f(MarkerAccum::<T>::And(lhs.reduce(f), rhs.reduce(f)))
//             }
//             MarkerExpr::Or(lhs, rhs) => {
//                 f(MarkerAccum::<T>::Or(lhs.reduce(f), rhs.reduce(f)))
//             }
//             MarkerExpr::Comparison { op, lhs, rhs } => {
//                 f(MarkerAccum::<T>::Comparison { op, lhs, rhs })
//             }
//         }
//     }
// }

// fn extra_reducer(
//     val: MarkerAccum<Result<(Option<Extra>, Option<MarkerExpr>)>>,
// ) -> Result<(Option<Extra>, Option<MarkerExpr>)> {
//     use MarkerOp::*;
//     use MarkerValue::*;

//     Ok(match val {
//         MarkerAccum::Comparison { op, lhs, rhs } => {
//             // The only two cases where we want to do something special are
//             // 'extra == "lit"' and '"lit" == extra'. Handle those here, and
//             // let everything else fall through.
//             if let Equal = op {
//                 match (lhs, rhs) {
//                     (Variable(var), Literal(lit)) => {
//                         if var == "extra" {
//                             return Ok((Some(lit.parse()?), None));
//                         }
//                     }
//                     (Literal(lit), Variable(var)) => {
//                         if var == "extra" {
//                             return Ok((Some(lit.parse()?), None));
//                         }
//                     }
//                     _ => (),
//                 }
//             }

//             // For everything else, make sure there's no 'extra' marker and
//             // then pass through our input unchanged

//             if let Variable(var) = lhs {
//                 if var == "extra" {
//                     bail!("invalid 'extra' marker");
//                 }
//             }

//             if let Variable(var) = rhs {
//                 if var == "extra" {
//                     bail!("invalid 'extra' marker");
//                 }
//             }

//             (None, Some(MarkerExpr::Comparison { op, lhs, rhs }))
//         }
//         MarkerAccum::And(lhs, rhs) => {
//             let (lhs_maybe_extra, lhs_maybe_expr) = lhs?;
//             let (rhs_maybe_extra, rhs_maybe_expr) = rhs?;
//             let maybe_expr = match (lhs_maybe_expr, rhs_maybe_expr) {
//                 (None, None) => None,
//                 (None, Some(rhs_expr)) => Some(rhs_expr),
//                 (Some(lhs_expr), None) => Some(lhs_expr),
//                 (Some(lhs_expr), Some(rhs_expr)) => {
//                     Some(MarkerExpr::And(lhs_expr.into(), rhs_expr.into()))
//                 }
//             };
//             let maybe_extra = match (lhs_maybe_extra, rhs_maybe_extra) {
//                 (None, None) => None,
//                 (None, Some(_)) => rhs_maybe_extra,
//                 (Some(_), None) => lhs_maybe_extra,
//                 (Some(_), Some(_)) => bail!("multiple 'extra' markers"),
//             };
//             (maybe_extra, maybe_expr)
//         }
//         MarkerAccum::Or(lhs, rhs) => {
//             let (lhs_maybe_extra, lhs_maybe_expr) = lhs?;
//             let (rhs_maybe_extra, rhs_maybe_expr) = rhs?;
//             if lhs_maybe_expr.is_some() || rhs_maybe_expr.is_some() {
//                 bail!("extra marker can't be inside an 'or'");
//             }
//             (
//                 None,
//                 Some(MarkerExpr::Or(
//                     lhs_maybe_expr.unwrap().into(),
//                     rhs_maybe_expr.unwrap().into(),
//                 )),
//             )
//         }
//     })
// }

// // Simplifications:
// // - 'StrictlyLessThan' can be lowered to (<= and !=), likewise for 'GreaterLessThan'
// // - 'Not' lets us drop 'Or', 'StrictlyGreaterThan', 'NotEqual', 'NotIn'
// // - We force all expressions to have a variable on the LHS and literal on the
// //   RHS. Technically PEP 508 allows variable/variable and literal/literal
// //   comparisons, but c'mon.
// enum LoweredMarkerExpr {
//     And(Box<LoweredMarkerExpr>, Box<LoweredMarkerExpr>),
//     Not(Box<LoweredMarkerExpr>),
//     // variable, literal
//     StrictlyLessThan(String, String),
//     // variable, literal
//     Equal(String, String),
//     // variable, literal
//     Compatible(String, String),
//     // variable, literal
//     In(String, String),
//     // variable, literal
//     Exactly(String, String),
// }
