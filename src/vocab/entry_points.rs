use crate::prelude::*;

// Partial reimplementation of Python configparser module; just enough to handle
// entry_points.txt.
//
// Limitations compared to configparser:
//
// - Only supports '=' for assignment
// - Names are always case-sensitive
// - No support for continuation lines in values
//
// For entry_points.txt, these are all fine.

static COMMENT: Lazy<Regex> = Lazy::new(|| Regex::new(r"[#;][^\n]*").unwrap());
static EMPTY_LINE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^\s*$").unwrap());
static HEADER_LINE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^\s*\[(?P<name>.+)\]\s*$").unwrap());
static ENTRY_LINE: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?x)
          ^
          (?P<name> .*?)
          \s* = \s*
          (?P<module> [a-zA-Z_][a-zA-Z0-9_.]+)
          \s*
          (: \s* (?P<object> [a-zA-Z_][a-zA-Z0-9_.]+))?
          \s*
          # 'Consumers should support parsing [extras] ... but may then ignore them'
          (\[ .* \])?
          \s*
          $
     ",
    )
    .unwrap()
});

#[derive(Debug)]
#[cfg_attr(test, derive(Serialize))]
pub struct Entrypoint {
    pub name: String,
    pub module: String,
    pub object: Option<String>,
}

pub fn parse_entry_points(contents: &str) -> Result<HashMap<String, Vec<Entrypoint>>> {
    let mut current_section_name = Some(String::new());
    let mut current_entries = Vec::<Entrypoint>::new();
    let mut result = HashMap::<String, Vec<Entrypoint>>::new();
    for line in contents.split('\n') {
        let line = COMMENT.replace(line, "");
        if EMPTY_LINE.is_match(line.as_ref()) {
            continue;
        } else if let Some(captures) = HEADER_LINE.captures(line.as_ref()) {
            let section_name = captures.name("name").unwrap().as_str();
            if !current_entries.is_empty() {
                result.insert(current_section_name.unwrap(), current_entries);
            }
            current_section_name = Some(section_name.into());
            current_entries = Vec::new();
        } else if let Some(captures) = ENTRY_LINE.captures(line.as_ref()) {
            if current_section_name.is_none() {
                bail!("missing section name in entry_points.txt");
            }
            let name = captures.name("name").unwrap().as_str().to_string();
            let module = captures.name("module").unwrap().as_str().to_string();
            let object = captures.name("object").map(|m| m.as_str().to_string());
            current_entries.push(Entrypoint {
                name,
                module,
                object,
            });
        } else {
            bail!("malformed entry_points.txt line: '{line}'");
        }
    }
    if !current_entries.is_empty() {
        result.insert(current_section_name.unwrap(), current_entries);
    }
    Ok(result)
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_basic() {
        // Sample entry_points.txt from https://packaging.python.org/en/latest/specifications/entry-points/
        let ini = indoc! {"
            [console_scripts]
            foo = foomod:main
            # One which depends on extras:
            foobar = foomod:main_bar [bar,baz]

            # pytest plugins refer to a module, so there is no ':obj'
            [pytest11]
            nbval = nbval.plugin
        "};
        let parsed = parse_entry_points(ini).unwrap();
        insta::with_settings!({sort_maps => true}, {
            insta::assert_ron_snapshot!(parsed, @r###"
            {
              "console_scripts": [
                Entrypoint(
                  name: "foo",
                  module: "foomod",
                  object: Some("main"),
                ),
                Entrypoint(
                  name: "foobar",
                  module: "foomod",
                  object: Some("main_bar"),
                ),
              ],
              "pytest11": [
                Entrypoint(
                  name: "nbval",
                  module: "nbval.plugin",
                  object: None,
                ),
              ],
            }
            "###);
        });
    }

    #[test]
    fn test_tricky() {
        let ini = indoc! {"
            [console_scripts]
            foo = foomod :\t main ; different comment style
            foobar   =   foo.bar.baz:quux.main [bar,baz]  # comment
            another = value  \t\t
        "};
        let parsed = parse_entry_points(ini).unwrap();
        insta::with_settings!({sort_maps => true}, {
            insta::assert_ron_snapshot!(parsed, @r###"
            {
              "console_scripts": [
                Entrypoint(
                  name: "foo",
                  module: "foomod",
                  object: Some("main"),
                ),
                Entrypoint(
                  name: "foobar",
                  module: "foo.bar.baz",
                  object: Some("quux.main"),
                ),
                Entrypoint(
                  name: "another",
                  module: "value",
                  object: None,
                ),
              ],
            }
            "###);
        });

        let bad = indoc! {"
            a = b
        "};
        assert!(parse_entry_points(bad).is_err());
    }
}
