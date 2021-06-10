use crate::prelude::*;

pub type Fields = HashMap<String, Vec<String>>;

#[cfg_attr(test, derive(Debug, Deserialize, PartialEq))]
pub struct RFC822ish {
    pub fields: Fields,
    pub body: Option<String>,
}

// Allegedly, a METADATA file is formatted as an RFC822 email message.
// This is absolutely not true. The actual format is "whatever
// the Python stdlib module email.parser does". To probe its behavior, a
// convenient entry point is 'email.message_from_string'.
//
// Overall structure: A series of header lines, then an empty line, then
// the "message body" (= description field, in modern PKG-INFO/METADATA
// files).
//
// email.parser module is also extremely lenient of errors. We'll try to be a
// bit more strict -- we try to be lenient of mangled utf-8, because obviously
// someone must have messed that up in the history of PyPI, and aren't picky
// about stuff like trailing newlines. But we fail on oddities like an empty
// field name or a continuation line at the start of input, where email.parser
// would keep on trucking. Fingers crossed that it works out.
peg::parser! {
    grammar rfc822ish_parser() for str {
        // In real RFC822, only CRLF is legal. email.parser is more lenient.
        rule line_ending()
            = quiet!{"\r\n" / "\r" / "\n"}
              / expected!("end of line")

        rule field_name() -> &'input str
            = quiet!{$(['\x21'..='\x39' | '\x3b'..='\x7e']+)}
              / expected!("field name")

        // email.parser drops any " \t" after the colon, but preserves other
        // whitespace in the field value.
        rule field_separator()
            = ":" [' ' | '\t']*

        rule field_value_piece()
            = [^ '\r' | '\n']*

        rule continuation_line_ending()
            = quiet!{line_ending() [' ' | '\t']} / expected!("continuation line")

        // In real RFC822, continuation lines are folded together into a
        // single line, removing the newline characters. email.parser doesn't
        // do that though -- continuation lines just get embedded newlines.
        // (But you don't include any *trailing* newlines. Those are
        // discarded.)
        rule field_value() -> &'input str
            = $(field_value_piece() ** continuation_line_ending())

        rule field() -> (String, String)
            = n:field_name() field_separator() v:field_value()
                { (n.to_owned(), v.to_owned()) }

        rule fields() -> Vec<(String, String)>
            = field() ** line_ending()

        // I think in real RFC822, the body is mandatory? But in early
        // versions of the metadata spec, PKG-INFO/METADATA files didn't have
        // a body, and email.parser don't care, it does what it wants.
        rule trailing_body() -> String
            = line_ending() line_ending() b:$([_]*) { b.to_owned() }

        // The extra line_ending() is to handle the case where there's
        // no trailing body, and exactly one line ending at EOF. If
        // trailing_body matches then the input will be fully consumed by
        // then; if not, then we might have a stray trailing newline to
        // absorb.
        pub rule rfc822ish() -> RFC822ish
            = f:fields() body:(trailing_body()?) line_ending()?
                 {
                     let mut fields = Fields::new();
                     for (name, value) in f {
                         fields.entry(name).or_insert(Vec::new()).push(value)
                     };
                     RFC822ish { fields, body, }
                 }
    }
}

impl RFC822ish {
    pub fn parse(input: &str) -> Result<RFC822ish> {
        Ok(rfc822ish_parser::rfc822ish(input)?)
    }

    pub fn take_all(&mut self, key: &str) -> Vec<String> {
        match self.fields.remove(key) {
            Some(vec) => vec,
            None => Vec::new(),
        }
    }

    pub fn maybe_take_the(&mut self, key: &str) -> Result<Option<String>> {
        let mut values = self.take_all(key);
        match values.len() {
            0 => Ok(None),
            1 => Ok(values.pop()),
            _ => anyhow::bail!("multiple values for singleton key {}", key),
        }
    }

    pub fn take_the(&mut self, key: &str) -> Result<String> {
        match self.maybe_take_the(key)? {
            Some(result) => Ok(result),
            None => anyhow::bail!("can't find required key {}", key),
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use indoc::indoc;

    #[test]
    fn test_successful_parsing() {
        struct T {
            // Input to parser
            given: &'static str,
            // Expected parsed data structure, written as json
            expected: &'static str,
        }

        let test_cases = vec![
            T {
                given: indoc! {r#"
                   A: b
                   C: d
                      continued

                   this is the
                   body!
                "#},
                expected: indoc! {r#"
                   {
                     "fields": { "A": ["b"], "C": ["d\n   continued"]},
                     "body": "this is the\nbody!\n"
                   }
                "#},
            },
            T {
                given: indoc! {r#"
                   no: body
                "#},
                expected: indoc! {r#"
                   {"fields": {"no": ["body"]}}
                "#},
            },
            T {
                given: indoc! {r#"
                   duplicate: one
                   duplicate: two
                   another: field
                   duplicate: three
                "#},
                expected: indoc! {r#"
                   {"fields": {"duplicate": ["one", "two", "three"], "another": ["field"]}}
                "#},
            },
            T {
                given: indoc! {r#"
                no: trailing newline"#},
                expected: indoc! {r#"
                   {"fields": {"no": ["trailing newline"]}}
                "#},
            },
            T {
                given: "",
                expected: r#"{"fields": {}}"#,
            },
        ];

        for test_case in test_cases {
            let got = RFC822ish::parse(test_case.given);
            println!("{:?} -> {:?}", test_case.given, got);
            let got = got.unwrap();
            let expected: RFC822ish = serde_json::from_str(test_case.expected).unwrap();
            assert_eq!(got, expected);
        }
    }

    #[test]
    fn test_failed_parsing() {
        let test_cases = vec![
            indoc! {r#"
                  continuation line
               at: beginning

               not good
            "#},
            indoc! {r#"
               bad key name: whee
            "#},
            ": no key name\n",
        ];
        for test_case in test_cases {
            let got = RFC822ish::parse(test_case);
            println!("{:?} -> {:?}", test_case, got);
            assert!(got.is_err());
        }
    }
}
