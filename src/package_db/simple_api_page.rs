// Derived from
//   https://github.com/servo/html5ever/blob/master/html5ever/examples/noop-tree-builder.rs
// Which has the following copyright header:
//
// Copyright 2014-2017 The html5ever Project Developers. See the
// COPYRIGHT file at the top-level directory of this distribution.
//
// Licensed under the Apache License, Version 2.0 <LICENSE-APACHE or
// http://www.apache.org/licenses/LICENSE-2.0> or the MIT license
// <LICENSE-MIT or http://opensource.org/licenses/MIT>, at your
// option. This file may not be copied, modified, or distributed
// except according to those terms.

use crate::prelude::*;

use std::borrow::Borrow;
use std::borrow::Cow;
use std::collections::HashMap;
use std::default::Default;

use html5ever::tendril::*;
use html5ever::tree_builder::{ElementFlags, NodeOrText, QuirksMode, TreeSink};
use html5ever::{expanded_name, local_name, namespace_url, ns, parse_document};
use html5ever::{Attribute, ExpandedName, LocalNameStaticSet, QualName};
use string_cache::Atom;

const META_TAG: ExpandedName = expanded_name!(html "meta");
const BASE_TAG: ExpandedName = expanded_name!(html "base");
const A_TAG: ExpandedName = expanded_name!(html "a");
const HREF_ATTR: Atom<LocalNameStaticSet> = html5ever::local_name!("href");
const NAME_ATTR: Atom<LocalNameStaticSet> = html5ever::local_name!("name");
const CONTENT_ATTR: Atom<LocalNameStaticSet> = html5ever::local_name!("content");
static REQUIRES_PYTHON_ATTR: Lazy<Atom<LocalNameStaticSet>> =
    Lazy::new(|| Atom::from("data-requires-python"));
static YANKED_ATTR: Lazy<Atom<LocalNameStaticSet>> =
    Lazy::new(|| Atom::from("data-yanked"));

struct Sink<'a> {
    next_id: usize,
    names: HashMap<usize, QualName>,
    base: Cow<'a, Url>,
    changed_base: bool,
    api_version: Option<String>,
    collected: &'a mut Vec<super::Artifact>,
}

impl<'a> Sink<'a>{
    fn get_id(&mut self) -> usize {
        let id = self.next_id;
        self.next_id += 2;
        id
    }
}

fn get_attr<'a>(
    name: &Atom<LocalNameStaticSet>,
    attrs: &'a Vec<Attribute>,
) -> Option<&'a str> {
    for attr in attrs {
        if attr.name.local == *name {
            return Some(attr.value.as_ref());
        }
    }
    None
}

impl<'a> TreeSink for Sink<'a> {
    type Handle = usize;
    type Output = Self;

    // This is where the actual work happens

    fn create_element(
        &mut self,
        name: QualName,
        attrs: Vec<Attribute>,
        _: ElementFlags,
    ) -> usize {
        if name.expanded() == META_TAG {
            if let Some("pypi:repository-version") = get_attr(&NAME_ATTR, &attrs) {
                self.api_version = get_attr(&CONTENT_ATTR, &attrs).map(String::from);
            }
        }

        if name.expanded() == BASE_TAG {
            // HTML spec says that only the first <base> is respected
            if !self.changed_base {
                self.changed_base = true;
                if let Some(new_base_str) = get_attr(&HREF_ATTR, &attrs) {
                    if let Ok(new_base) = self.base.join(new_base_str) {
                        self.base = Cow::Owned(new_base);
                    }
                }
            }
        }

        if name.expanded() == A_TAG {
            if let Some(url_str) = get_attr(&HREF_ATTR, &attrs) {
                if let Ok(url) = self.base.join(url_str) {
                    // We found a valid link
                    let requires_python =
                        get_attr(REQUIRES_PYTHON_ATTR.borrow(), &attrs)
                            .map(String::from);
                    let yanked =
                        get_attr(YANKED_ATTR.borrow(), &attrs).map(String::from);
                    self.api_page.links.push(SimpleAPILink {
                        url,
                        requires_python,
                        yanked,
                    })
                }
            }
        }

        let id = self.get_id();
        self.names.insert(id, name);
        id
    }

    // Everything else is just boilerplate to make html5ever happy

    fn finish(self) -> Self {
        self
    }

    fn get_document(&mut self) -> usize {
        0
    }

    fn get_template_contents(&mut self, target: &usize) -> usize {
        target + 1
    }

    fn same_node(&self, x: &usize, y: &usize) -> bool {
        x == y
    }

    fn elem_name(&self, target: &usize) -> ExpandedName {
        self.names.get(target).expect("not an element").expanded()
    }

    fn create_comment(&mut self, _text: StrTendril) -> usize {
        self.get_id()
    }

    fn create_pi(&mut self, _target: StrTendril, _value: StrTendril) -> usize {
        // HTML doesn't have processing instructions
        unreachable!()
    }

    fn append_before_sibling(
        &mut self,
        _sibling: &usize,
        _new_node: NodeOrText<usize>,
    ) {
    }

    fn append_based_on_parent_node(
        &mut self,
        _element: &usize,
        _prev_element: &usize,
        _new_node: NodeOrText<usize>,
    ) {
    }

    fn parse_error(&mut self, _msg: Cow<'static, str>) {}
    fn set_quirks_mode(&mut self, _mode: QuirksMode) {}
    fn append(&mut self, _parent: &usize, _child: NodeOrText<usize>) {}

    fn append_doctype_to_document(
        &mut self,
        _: StrTendril,
        _: StrTendril,
        _: StrTendril,
    ) {
    }
    // This is only called on <html> and <body> tags, so we don't need to worry about it
    fn add_attrs_if_missing(&mut self, _target: &usize, _attrs: Vec<Attribute>) {}
    fn remove_from_parent(&mut self, _target: &usize) {}
    fn reparent_children(&mut self, _node: &usize, _new_parent: &usize) {}
    fn mark_script_already_started(&mut self, _node: &usize) {}
}

pub fn collect_artifacts(url: &Url, content_type: &str, body: &str,
collected: &mut Vec<super::Artifact>) -> Result<()> {
        if value.content_type != "text/html" {
            bail!(
                "simple API page expected Content-Type: text/html, but got {}",
                value.content_type
            )
        }

        let sink = Sink {
            next_id: 1,
            base: url,
            changed_base: false,
            names: HashMap::new(),
            api_page: Default::default(),
        };

        Ok(parse_document(sink, Default::default())
            .one(value.body)
            .api_page)
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn test_sink_simple() {
        let html_page = SmallTextPage {
            url: Url::parse("https://example.com/old-base/").unwrap(),
            content_type: "text/html".into(),
            body: r#"<html>
                <head>
                  <meta name="pypi:repository-version" content="1.0">
                  <base href="https://example.com/new-base/">
                </head>
                <body>
                  <a href="link1">link1</a>
                  <a href="/link2" data-yanked="some reason">link2</a>
                  <a href="link3" data-requires-python=">= 3.17">link3</a>
                </body>
              </html>
            "#
            .into(),
        };
        let parsed: SimpleAPIPage = html_page.try_into().unwrap();
        assert_eq!(
            parsed,
            SimpleAPIPage {
                repository_version: Some("1.0".to_owned()),
                links: vec![
                    SimpleAPILink {
                        url: Url::parse("https://example.com/new-base/link1").unwrap(),
                        requires_python: None,
                        yanked: None,
                    },
                    SimpleAPILink {
                        url: Url::parse("https://example.com/link2").unwrap(),
                        requires_python: None,
                        yanked: Some("some reason".to_owned()),
                    },
                    SimpleAPILink {
                        url: Url::parse("https://example.com/new-base/link3").unwrap(),
                        requires_python: Some(">= 3.17".to_owned()),
                        yanked: None,
                    },
                ]
            }
        )
    }
}
