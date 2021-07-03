# What is this?

Take your pick:

- Me messing around in Rust for fun (just a hobby, won't be big and
  serious like `pip`)
  
- An incomplete but mostly functional implementation of Python's major
  packaging standards in Rust, including a full resolver based on [the
  PubGrub algorithm](https://nex3.medium.com/pubgrub-2fb6470504f) (as
  provided by [`pubgrub`](https://docs.rs/pubgrub/).

- A [draft
  spec](https://github.com/njsmith/posy/blob/main/pybi/README.md) for
  "PyBi" files, which are like wheels but for Python interpreters.
  
- (Someday) A project-oriented Python workflow manager, designed to
  make it easy for beginners to write their first Python script or
  notebook, and then grow with you to developing complex standalone
  apps and libraries with many contributors.
  
- A combined replacement for pyenv, deadsnakes, tox, venv, pip,
  pip-compile/pipenv, and PEP 582, all in a single-file executable
  with zero system requirements (not even Python).

- An 🐘
  [elephant](https://mail.python.org/archives/list/distutils-sig@python.org/thread/YFJITQB37MZOPOFJJF3OAQOY4TOAFXYM/#YFJITQB37MZOPOFJJF3OAQOY4TOAFXYM)
  🐘


## Interpretations/deviations from standards

### Extra name syntax

It's really unclear what the validation and normalization rules for
"extra" names is supposed to be. I decided to use the same rules as
package names. (= alphanumeric or `-_.`, but `-_.` are all treated as
equivalent in comparisons internally... though I'm not quite sure what
the string comparisons in environment markers do, since those cast
back to string before comparing.)


### `===` and `@`

Neither is currently implemented.


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
  
And if you think about it... that's actually because while this rule
is well-defined for a specifier in isolation, it doesn't really make
sense when you're talking about multiple packages with their own
dependencies. E.g., if package A depends on `foo == 2.0a1`, and
package B depends on `foo >= 1.0`, then is it valid to install foo
v2.0a1? It feels like it ought to match all the requirements, but
technically it doesn't... and according to a strict reading of PEP
440, once any package says `foo >= 1.0`, it becomes impossible to ever
use a `foo` pre-release anywhere in the dependency tree, no matter
what other packages say. Pre-release validity is just inherently a
global property, not a property of individual specifiers.
  
So I'm thinking we should use the rule:

- If all available versions are pre-releases, then pre-releases are valid
- If we're updating a set of pins that already contain a pre-release,
  then pre-releases are valid (or at least that specific pre-release
  is)
- Otherwise, to get pre-releases, you have to set some
  environment-level config like `allow-prerelease = ["foo"]`.
