# Pybi (*Py*thon *Bi*nary) format

"Like wheels, but instead of a pre-built python package, it's a
pre-built python interpreter"

End goal: Pypi.org has pre-built packages for all Python versions on
all popular platforms, so automated tools can easily grab any of them
and set it up. It becomes quick and easy to try Python prereleases,
pin Python versions in CI, make a temporary environment to reproduce
a bug report that only happens on an older Python point release, etc.

Example pybi builds:

- [pybi.vorpus.org](https://pybi.vorpus.org)
- [tooling used to create the above](https://github.com/njsmith/pybi-tools)


## Filename

Filename: `{distribution}-{version}[-{build tag}]-{platform tag}.pybi`

Same definition as [PEP 427's wheel file name
format](https://www.python.org/dev/peps/pep-0427/#file-name-convention),
except dropping the `{python tag}` and `{abi tag}` and changing the
extension from `.whl` → `.pybi`.

For example:

* `cpython-3.9.3-manylinux_2014.pybi`
* `cpython-3.10b2-win_amd64.pybi`

Just like for wheels, if a pybi supports multiple platforms, you can
separate them by dots to make a "compressed tag set":

* `cpython-3.9.5-macosx_11_0_x86_64.macosx_11_0_arm64.pybi`

(Though in practice this probably won't be used much, e.g. the above
filename is more idiomatically written as
`cpython-3.9.5-macosx_11_0_universal2.pybi`.)


## File contents

A `.pybi` file is a zip file, that can be unpacked directly into an
arbitrary location and then used as a self-contained Python
environment. There's no `.data` directory or install scheme keys,
because the Python environment knows which install scheme it's using,
so it can just put things in the right places to start with.

The "arbitrary location" part is important: the pybi can't contain any
hardcoded absolute paths. In particular, any preinstalled scripts
MUST NOT embed absolute paths in their shebang lines.

Similar to wheels' `.dist-info` directory, the pybi archive must
contain a top-level directory named `pybi-info/`. (Rationale: calling
it `pybi-info` instead `dist-info` makes sure that tools don't get
confused about which kind of metadata they're looking at; leaving off
the `{name}-{version}` part is fine because only one pybi can be
installed into a given directory.) The `pybi-info/` directory contains
at least the following files:

* `.../PYBI`: metadata about the archive itself, in the same RFC822-ish
  format as `METADATA` and `WHEEL` files:
  
  ```
  Pybi-Version: 1.0
  Generator: {name} {version}
  Tag: {platform tag}   # may be repeated
  Build: 1   # optional
  ```

* `.../RECORD`: same as in wheels, except see the note about symlinks, below.

* `.../METADATA`: In the same format as described in the current core
  metadata spec, except that the following keys are forbidden because
  they don't make sense:
  
  * `Requires-Dist`
  * `Provides-Extra`
  * `Requires-Python`

  And also there are some new, required keys described below.
  
### Pybi-specific core metadata

Example of new `METADATA` fields:

```
Pybi-Environment-Markers: {"implementation_name": "cpython", "implementation_version": "3.10.8", "os_name": "posix", "platform_machine": "x86_64", "platform_system": "Linux", "python_full_version": "3.10.8", "platform_python_implementation": "CPython", "python_version": "3.10", "sys_platform": "linux"}
Pybi-Paths: {"stdlib": "lib/python3.10", "platstdlib": "lib/python3.10", "purelib": "lib/python3.10/site-packages", "platlib": "lib/python3.10/site-packages", "include": "include/python3.10", "platinclude": "include/python3.10", "scripts": "bin", "data": "."}
Pybi-Wheel-Tag: cp310-cp310-PLATFORM
Pybi-Wheel-Tag: cp310-abi3-PLATFORM
Pybi-Wheel-Tag: cp310-none-PLATFORM
Pybi-Wheel-Tag: cp39-abi3-PLATFORM
Pybi-Wheel-Tag: cp38-abi3-PLATFORM
Pybi-Wheel-Tag: cp37-abi3-PLATFORM
Pybi-Wheel-Tag: cp36-abi3-PLATFORM
Pybi-Wheel-Tag: cp35-abi3-PLATFORM
Pybi-Wheel-Tag: cp34-abi3-PLATFORM
Pybi-Wheel-Tag: cp33-abi3-PLATFORM
Pybi-Wheel-Tag: cp32-abi3-PLATFORM
Pybi-Wheel-Tag: py310-none-PLATFORM
Pybi-Wheel-Tag: py3-none-PLATFORM
Pybi-Wheel-Tag: py39-none-PLATFORM
Pybi-Wheel-Tag: py38-none-PLATFORM
Pybi-Wheel-Tag: py37-none-PLATFORM
Pybi-Wheel-Tag: py36-none-PLATFORM
Pybi-Wheel-Tag: py35-none-PLATFORM
Pybi-Wheel-Tag: py34-none-PLATFORM
Pybi-Wheel-Tag: py33-none-PLATFORM
Pybi-Wheel-Tag: py32-none-PLATFORM
Pybi-Wheel-Tag: py31-none-PLATFORM
Pybi-Wheel-Tag: py30-none-PLATFORM
Pybi-Wheel-Tag: py310-none-any
Pybi-Wheel-Tag: py3-none-any
Pybi-Wheel-Tag: py39-none-any
Pybi-Wheel-Tag: py38-none-any
Pybi-Wheel-Tag: py37-none-any
Pybi-Wheel-Tag: py36-none-any
Pybi-Wheel-Tag: py35-none-any
Pybi-Wheel-Tag: py34-none-any
Pybi-Wheel-Tag: py33-none-any
Pybi-Wheel-Tag: py32-none-any
Pybi-Wheel-Tag: py31-none-any
Pybi-Wheel-Tag: py30-none-any
```

In more detail:

* `Pybi-Environment-Markers`: The value of all PEP 508 environment marker values
    that are static across installs of this Pybi, as a JSON dict. (So e.g., it
    should have `python_version`, but not `platform_version`, which on my system
    looks like `#60-Ubuntu SMP Thu May 6 07:46:32 UTC 2021`).
    
    Rationale: In many cases, this should allow a resolver running on
    Linux to compute package pins for a Python environment on Windows,
    or vice-versa, so long as the resolver has access to the target
    platform's .pybi file. (Note that Requires-Python constraints can
    be checked by using the `python_full_version` value.)

    The markers are also just generally useful information to have
    accessible. For example, if you have a `pypy3-7.3.2` pybi, and you
    want to know what version of the Python language that supports,
    then that's recorded in the `python_version` marker.

    TODO: what to do about macOS universal2 pybis, where `platform_machine`
    depends on which half of the fat binary you're using? Some options:
    
    - Don't have fat pybis; just have a separate pybi for x86-64 and arm64.
      Works okay for automated tooling I guess? What can you do what a fat
      binary that you can't with individual binaries - maybe a sufficiently
      clever packager could create an entire universal2 environment, that's
      portable to both x86-64 and arm64 systems? Not sure how viable it would be
      to get all the individual wheels to cooperate...
    
    - Tell projects that are building tools to do pinning to special-case
      universal2 pybis and fill in the `platform_machine` themselves.

    TODO: I think in posy I'm just going to refuse to support `platform_release`
    and `platform_version`. And maybe we should have the conversation about
    whether they should be deprecated officially, in general? They're the only
    two markers that can't be reasonably treated as static, and I can't figure
    out any way that anyone could actually use them for anything useful anyway.

    (I guess deprecating `platform_machine` would also be a possibility to
    consider? It's slightly problematic for `universal2` wheels, and it's not
    clear how useful it is – does anyone actually make their requirements
    conditional on the target architecture?) (edit: since writing the previous
    paragraph I spent a bunch of time fixing requirements at work to use
    `platform_machine`, because it turns out `psycopg2-binary` is broken on
    linux/arm64, but not linux/x86-64. So... never mind, `platform_machine` is
    useful :-). But an installer that can pick which pybi and wheels to install
    can also figure out `platform_machine`.)

  * `Pybi-Paths`: The install paths needed to install wheels (same keys as
    `sysconfig.get_paths()`), as relative paths starting at the root of the zip
    file, as a JSON dict.

    It must be possible to invoke the Python interpreter by running
    `{paths["scripts"]}/python`. If there are alternative interpreter
    entry points (e.g. `pythonw` for Windows GUI apps), then they
    should also be in that directory under their conventional names,
    with no version number attached.

    Rationale: `Pybi-Paths` and `Pybi-Wheel-Tag`s (see below) are together enough to
    let an installer choose wheels and install them into an unpacked pybi
    environment, without invoking Python. Besides, we need to write down the
    interpreter location somewhere, so it's two birds with one stone.

  * `Pybi-Wheel-Tag`: The wheel tags supported by this interpreter, in preference
    order (most-preferred first, least-preferred last), except that the special
    platform tag `PLATFORM` should replace any platform tags that depend on the
    final installation system.

    Discussion: It would be nice™ if installers could compute a pybi's
    corresponding wheel tags ahead of time, so that they could install
    wheels into the unpacked pybi without needing to actually invoke
    the python interpreter to query its tags – both for efficiency and
    to allow for more exotic use cases like setting up a Windows
    environment from a Linux host.

    But unfortunately, it's impossible to compute the full set of
    platform tags supported by a Python installation ahead of time,
    because they can depend on the final system:
    
    - A pybi tagged `manylinux_2_12_x86_64` can always use wheels
      tagged as `manylinux_2_12_x86_64`. It also *might* be able to
      use wheels tagged `manylinux_2_17_x86_64`, but only if the final
      installation system has glibc 2.17+.
      
    - A pybi tagged `macosx_11_0_universal2` (= x86-64 + arm64 support
      in the same binary) might be able to use wheels tagged as
      `macosx_11_0_arm64`, but only if it's installed on an "Apple
      Silicon" machine.

    In these two cases, an installation tool can still work out the
    appropriate set of wheel tags by computing the local platform
    tags, taking the wheel tag templates from `pybi.json`, and
    swapping in the actual supported platforms in place of the magic
    `PLATFORM` string. Since pybi installers already need to compute
    platform tags to pick a pybi in the first place, this is pretty
    simple.

    However, there are other cases that are even more complicated:

    - You can (usually) run both 32- and 64-bit apps on 64-bit
      Windows. So a pybi installer might compute the set of allowable
      pybi tags as [`win32`, `win_amd64`]. But you can't then just
      take that set and swap it into the pybi's wheel tag template or
      you get nonsense:
      
      ```
        [
          "cp39-cp39-win32",
          "cp39-cp39-win_amd64",
          "cp39-abi3-win32",
          "cp39-abi3-win_amd64",
          ...
        ]
      ```

      To handle this, the installer needs to somehow understand that a
      `manylinux_2_12_x86_64` pybi can use a `manylinux_2_17_x86_64`
      wheel, even though those tags are different, but a `win32` pybi
      *can't* use a `win_amd64` wheel, because those tags are
      different.

      And similar issues arise for other 64-bit OSes.

    - A pybi tagged `macosx_11_0_universal2` might be able to use
      wheels tagged as `macosx_11_0_x86_64`, but only if it's
      installed on an x86-64 machine *or* it's installed on an ARM
      machine *and* the interpreter is invoked with the magic
      incantation that tells macOS to run a binary in x86-64 mode. So
      how the installer plans to invoke the pybi matters too!

    So actually using `Pybi-Wheel-Tag` values is less trivial than it might
    seem, and they're probably only useful with fairly sophisticated tooling.
    But, smart pybi installers will already have to understand a lot of these
    platform compatibility issues in order to select a working pybi, and for the
    cross-platform pinning/environment building case, users can potentially
    provide whatever information is needed to disambiguate exactly what platform
    they're targeting. So, it's pretty useful.

  You can probably generate these values by running this script on the built
  interpreter:

  ```python
  import packaging.markers
  import packaging.tags
  import sysconfig
  import os.path
  import json
  import sys

  markers_env = packaging.markers.default_environment()
  # Delete any keys that depend on the final installation
  del markers_env["platform_release"]
  del markers_env["platform_version"]
  # Darwin binaries are often multi-arch, so play it safe and
  # delete the architecture marker. (Better would be to only
  # do this if the pybi actually is multi-arch.)
  if markers_env["sys_platform"] == "darwin":
      del markers_env["platform_machine"]

  # Copied and tweaked version of packaging.tags.sys_tags
  tags = []
  interp_name = packaging.tags.interpreter_name()
  if interp_name == "cp":
      tags += list(packaging.tags.cpython_tags(platforms=["xyzzy"]))
  else:
      tags += list(packaging.tags.generic_tags(platforms=["xyzzy"]))

  tags += list(packaging.tags.compatible_tags(platforms=["xyzzy"]))

  # Gross hack: packaging.tags normalizes platforms by lowercasing them,
  # so we generate the tags with a unique string and then replace it
  # with our special uppercase placeholder.
  str_tags = [str(t).replace("xyzzy", "PLATFORM") for t in tags]

  (base_path,) = sysconfig.get_config_vars("installed_base")
  # For some reason, macOS framework builds report their
  # installed_base as a directory deep inside the framework.
  while "Python.framework" in base_path:
      base_path = os.path.dirname(base_path)
  paths = {key: os.path.relpath(path, base_path) for (key, path) in sysconfig.get_paths().items()}

  json.dump({"markers_env": markers_env, "tags": str_tags, "paths": paths}, sys.stdout)
  ```
  
This emits a JSON dict on stdout with separate entries for each set of
pybi-specific tags. 


## Symlinks

Currently, symlinks are used by default in all Unix Python installs
(e.g., `bin/python3 -> bin/python3.9`). And furthermore, symlinks are
*required* to store macOS framework builds in `pybi` files. So, unlike
wheel files, `.pybi` files must be able to represent symlinks.


### Representing symlinks in zip files

The de-facto standard for representing symlinks in zip files is the
Info-Zip symlink extension, which works as follows:

- The symlink's target path is stored as if it were the file contents
- The top 4 bits of the Unix permissions field are set to `0xa`, i.e.:
  `permissions & 0xf000 == 0xa000`
- The Unix permissions field, in turn, is stored as the top 16 bits of
  the "external attributes" field.
  
So if using Python's `zipfile` module, you can check whether a
`ZipInfo` represents a symlink by doing:

```python
(zip_info.external_attr >> 16) & 0xf000 == 0xa000
```

Or if using Rust's `zip` crate, the equivalent check is:

```rust
fn is_symlink(zip_file: &zip::ZipFile) -> bool {
    match zip_file.unix_mode() {
        Some(mode) => mode & 0xf000 == 0xa000,
        None => false,
    }
}
```

If you're on Unix, your `zip` command probably understands this format
already.


### Representing symlinks in RECORD files

Normally, a `RECORD` file lists each file + its hash + its length:

```csv
my/favorite/file,sha256=...,12345
```

For symlinks, we instead write:

```csv
name/of/symlink,symlink=path/to/symlink/target,
```

That is: we use a special "hash function" called `symlink`, and then
store the actual symlink target as the "hash value". And the length is
left empty.

Rationale: we're already committed to the `RECORD` file containing a
redundant check on everything in the main archive, so for symlinks we
at least need to store some kind of hash, plus some kind of flag to
indicate that this is a symlink. Given that symlink target strings are
roughly the same size as a hash, we might as well store them directly.
This also makes the symlink information easier to access for tools
that don't understand the Info-Zip symlink extension, and makes it
possible to losslessly unpack and repack a Unix pybi on a Windows
system, which someone might find handy at some point.


### Storing symlinks in `pybi` files

When a pybi creator stores a symlink, they MUST use both of the
mechanisms defined above: storing it in the zip archive directly using
the Info-Zip representation, and also recording it in the `RECORD`
file.

Pybi consumers SHOULD validate that the symlinks in the archive and
`RECORD` file are consistent with each other.

We also considered using *only* the `RECORD` file to store symlinks,
but then the vanilla `unzip` tool wouldn't be able to unpack them, and
that would make it hard to install a pybi from a shell script.


### Limitations

Symlinks enable a lot of potential messiness. To keep things under
control, we impose the following restrictions:

- Symlinks MUST NOT be used in `.pybi`s targeting Windows, or other
  platforms that are missing first-class symlink support.

- Symlinks MUST NOT be used inside the `.pybi-info` directory.
  (Rationale: there's no need, and it makes things simpler for
  resolvers that need to extract info from `.pybi-info` without
  unpacking the whole archive.)

- Symlink targets MUST be relative paths, and MUST be inside the pybi
  directory.
  
- If `A/B/...` is recorded as a symlink in the archive, then there
  MUST NOT be any other entries in the archive named like `A/B/.../C`.
  
  For example, if an archive has a symlink `foo -> bar`, and then
  later in the archive there's a regular file named `foo/blah.py`,
  then a naive unpacker could potentially end up writing a file called
  `bar/blah.py`. Don't be naive.

Unpackers MUST verify that these rules are followed, because without
them attackers could create evil symlinks like `foo -> /etc/passwd` or
`foo -> ../../../../../etc` + `foo/passwd -> ...` and cause havoc.


## Sdists (or not)

It might be cool to have an "sdist" equivalent for pybis, i.e., some
kind of format for a Python source release that's structured-enough to
let tools automatically fetch and build it into a pybi, for platforms
where prebuilt pybis aren't available. But, this isn't necessary for
the MVP and opens a can of worms, so let's ignore it for now.


## What packages should be included in a pybi?

When building a pybi, you MAY pick and choose what exactly goes
inside. For example, you could include some preinstalled packages in
the pybi's `site-packages` directory, or prune out bits of the stdlib
that you don't want. We can't stop you! Just make sure that if you do
preinstall packages, then you also include the correct metadata
(`.dist-info` etc.), so that it's possible for tools to figure out
what's going on.

But, here's what I'm doing for my prototype "general purpose" pybi's:

- Make sure `site-packages` is *empty*.

  Rationale: for traditional standalone python installers that are
  targeted at end-users, you probably want to include at least `pip`,
  to [avoid bootstrapping
  issues](https://www.python.org/dev/peps/pep-0453/). But pybis are
  different: they're designed to be installed by "smart" tooling, that
  consume the pybi as part of some kind of larger automated deployment
  process. It's easier for these installers to start from a blank
  slate and then add whatever they need, than for them to start with
  some preinstalled packages that they may or may not want.

- Include the full stdlib, *except* for `test`.

  Rationale: the top-level `test` module contains CPython's own test
  suite. It's huge (CPython without `test` is ~37 MB, then `test` adds
  another ~25 MB on top of that!), and essentially never used by
  regular user code. Also, as precedent, the official nuget packages,
  the official manylinux images, and multiple Linux distributions all
  leave it out, and this hasn't caused any major problems.
  
  So this seems like the best way to balance broad compatibility with
  reasonable download/install sizes.

- I'm not shipping any `.pyc` files. They take up space in the
  download, can be generated on the final system at minimal cost, and
  dropping them removes a source of location-dependence. (`.pyc` files
  store the absolute path of the corresponding `.py` file and include
  it in tracebacks; but, pybis are relocatable, so the correct path
  isn't known until after install.)
