use anyhow::{Context, Result};
use std::collections::HashMap;

// A parsed version of a package METADATA or PKG-INFO file, as per
// https://packaging.python.org/specifications/core-metadata/
pub type Fields = HashMap<String, Vec<String>>;

pub struct CoreMetadata {
    pub fields: Fields,
    // XX this is apparently supposed to be treated as a Description field
    pub readme: String,
}

mod parser_internals {
    use nom::bytes::complete::{is_a, is_not, take_while};
    use nom::character::complete::one_of;
    use nom::combinator::rest;
    use nom::multi::many1;
    use nom::sequence::separated_pair;
    use nom::{IResult, Parser};
    use nom_supreme::{
        error::ErrorTree,
        final_parser::{final_parser, Location},
        parser_ext::ParserExt,
        tag::complete::tag,
    };

    type ParseResult<'a, O> = IResult<&'a str, O, ErrorTree<&'a str>>;

    // Allegedly, a METADATA file is formatted as an RFC822 email message.
    // This is absolutely not true. The actual format is "whatever
    // the Python stdlib module email.message_from_string does".
    //
    // Overall structure: A series of header lines, then an empty line, then
    // the "message body" (= package readme)
    //
    // A line ending is: \n, \r, or \r\n
    //
    // A header line is:
    // - field name + separator + field value + line ending
    // field names are:
    // - a sequence of one or more characters in the set [\041-\071\073-\176]
    //   or put another way: anything from \041-\176 except for ':'
    // The field name/value separator is:
    // - a colon + zero or more spaces or tabs
    // The field value is:
    // - everything after the separator, until the end of the line, not
    //   including the end of line.
    //   BUT we keep reading if the line ending is followed by a space or tab!
    //   So e.g.:
    //
    //     "foo: \tbar  \n  baz\r\n"
    //
    //   ...will produce the field value:
    //
    //     "bar  \n  baz"
    //
    // Some notable differences from RFC 822:
    // - continuation lines preserve newlines; RFC822 says that they should be
    //   replaced by spaces.
    // - RFC822 says that \r\n is mandatory at the end of all lines
    // - RFC822's definitions of whitespace are a bit different
    //
    // The 'email' module is also extremely lenient of errors. We'll try to be
    // a bit more strict -- we try to be lenient of mangled utf-8, because
    // obviously someone must have messed that up in the history of PyPI, but
    // we fail on oddities like a missing

    pub fn parse_metadata(input: &str) -> Result<super::CoreMetadata, ErrorTree<Location>> {
        // This has to be an actual function, not just a combinator object,
        // because nom's type system is awkward and if it were a combinator
        // there would be no way to use it multiple times below, even as
        // borrows.
        fn line_ending(input: &str) -> ParseResult<()> {
            tag("\r\n")
                .or(tag("\r"))
                .or(tag("\n"))
                .map(|_| ())
                .parse(input)
        }

        fn is_field_name_char(c: char) -> bool {
            let i = c as u32;
            0o41 <= i && i <= 0o176 && c != ':'
        }

        let field_name = take_while(is_field_name_char).context("field name");
        let field_separator = tag(":").and(is_a(" \t")).context("field separator");

        let value_line_piece = is_not("\r\n");
        let continuation_marker = line_ending.and(one_of(" \t"));
        let field_value = nom::multi::separated_list1(continuation_marker, value_line_piece)
            .recognize()
            .context("field value");

        let field = separated_pair(field_name, field_separator, field_value)
            .context("field");
        let fields = many1(field.terminated(line_ending)).context("fields");

        let metadata = separated_pair(fields, line_ending, rest);
        let mut parse = final_parser(metadata);
        let (fields_vec, readme_ref) = parse(input)?;
        let mut fields = super::Fields::new();
        for (field_name, field_value) in fields_vec {
            fields
                .entry(field_name.to_owned())
                .or_insert(Vec::new())
                .push(field_value.to_owned());
        }
        let readme = readme_ref.to_owned();
        Ok(super::CoreMetadata { fields, readme })
    }
}

impl CoreMetadata {
    pub fn parse(data: &str) -> Result<CoreMetadata> {
        parser_internals::parse_metadata(data).context("Error parsing package metadata")
    }
}

#[test]
fn test_parsing() {
    let msg = CoreMetadata::parse("A: b\nC: d\n  continued\n\nHello!\nThis is fun!").unwrap();
    let expected_fields: Fields = vec![
        ("A".to_string(), vec!["b".to_string()]),
        ("C".to_string(), vec!["d\n  continued".to_string()]),
    ]
    .into_iter()
    .collect();
    assert_eq!(msg.fields, expected_fields);
    assert_eq!(msg.readme, "Hello!\nThis is fun!");
}
