# What is this?

You have a few choices:

- Me messing around in Rust for fun (just a hobby, won't be big and
  serious like `pip`)
  
- An incomplete but functional implementation of Python's packaging
  standards in Rust, including a full resolver based on [the PubGrub
  algorithm](https://nex3.medium.com/pubgrub-2fb6470504f) (as provided
  by [`pubgrub`](https://docs.rs/pubgrub/)).

- A [draft
  spec](https://github.com/njsmith/posy/blob/main/pybi/README.md) for
  "PyBi" files, which are like wheels but for Python interpreters.
  
Someday:

- A project-oriented Python workflow manager, designed to
  make it easy for beginners to write their first Python script or
  notebook, and then grow with you to developing complex standalone
  apps and libraries with many contributors.
  
- A combined replacement for pyenv, deadsnakes, tox, venv, pip,
  pip-compile/pipenv, and PEP 582, all in a single-file executable
  with zero system requirements (not even Python).

- An 🐘
  [elephant](https://mail.python.org/archives/list/distutils-sig@python.org/thread/YFJITQB37MZOPOFJJF3OAQOY4TOAFXYM/#YFJITQB37MZOPOFJJF3OAQOY4TOAFXYM)
  🐘


# The Vision

The goal is for posy to act as a kind of high-level frontend to python: you
install posy, then run `posy [args] some_python.py` and it takes care of
everything up until entering the python interpreter. That includes:

- installing Python (posy is a pure-rust single-file binary; it doesn't assume
  you have anything else installed)
- installing dependencies from wheels/sdists (it's a PEP 517 build "frontend")
- environment management 
- (cross-platform) locking (for both the interpreter + packages)
- run commands in environment, or export a self-contained redistributable
  environment (e.g. to drop in a docker image)
- nice UX for setting this stuff up and managing it, hopefully

(NOTE: not all of these are implemented yet!)

But the following is *not* in scope:

- a PEP 517 build *backend*: use setuptools, flit, meson-python, py-build-cmake,
  ...or whatever build framework you want. Or none at all, if you're not
  creating a redistributable package.

  [XX TODO: insert link to pypi search once we [have a
  classifier](https://discuss.python.org/t/improving-discoverability-of-build-backends/20140/)
  so we don't have to play favorites on which projects we list here.]

- a testing framework, a code formatter, a linter, ... Python already has good
  tools for all that stuff, and we don't plan to duplicate them. But posy can
  set up the environment they need and run them for you!


# Packaging features I don't (currently) plan to implement

### `===`

PEP 440 defines a `===` operator, for comparing non-PEP 440-compliant versions.
Posy only supports PEP 440-compliant versions.


### The `platform_release` and `platform_version` environment marker variables

These are values like:

```
 'platform_release': '5.19.0-23-generic',
 'platform_version': '#24-Ubuntu SMP PREEMPT_DYNAMIC Fri Oct 14 15:39:57 UTC 2022',
```

Technically, you're supposed to be able to make dependencies vary depending on
these strings. But these are so quirky and machine-specific that I don't see how
to implement that in posy's model, or why anyone would want them.


### Prereleases in specifiers

According to PEP 440, specifiers like `>= 2.0a1` are supposed to
change meaning depending on whether or not the literal version
contains a prerelease marker. So like, `>= 2.0` *doesn't* match
`2.1a1`, because that's a prerelease, and regular specifiers never
match prereleases. But `>= 2.0a1` *does* match `2.1a1`, because the
presence of a prerelease in the specifier makes it legal for
prerelease versions to match.
  
I don't think I can actually implement this using the `pubgrub`
system, since it collapses multiple specifiers for the same package
into a single set of valid ranges, and there's no way to preserve the
information about which ranges were derived from specifiers that
included prerelease suffixes, and which ranges weren't.
  
And if you think about it... that's actually because while this rule is
well-defined for a specifier in isolation, it doesn't really make sense when
you're talking about multiple packages with their own dependencies. E.g., if
package A depends on `foo == 2.0a1`, and package B depends on `foo >= 1.0`, then
is it valid to install foo v2.0a1? It feels like it ought to match all the
requirements, but technically it doesn't... according to a strict reading of PEP
440, once any package says `foo >= 1.0`, it becomes impossible to ever use a
`foo` pre-release anywhere in the dependency tree, no matter what other packages
say. Pre-release validity is just inherently a global property, not a property
of individual specifiers.
  
So I'm thinking we should use the rule:

- If all available versions are pre-releases, then pre-releases are valid
- If we're updating a set of pins that already contain a pre-release,
  then pre-releases are valid (or at least that specific pre-release
  is)
- Otherwise, to get pre-releases, you have to set some
  environment-level config like `allow-prerelease = ["foo"]`.
