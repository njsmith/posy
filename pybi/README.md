# Pybi (*Py*thon *Bi*nary) format

"Like wheels, but instead of a pre-built python package, it's a
pre-built python interpreter"

End goal: Pypi.org has pre-built packages for all Python versions on
all popular platforms, so automated tools can easily grab any of them
and set it up. It becomes quick and easy to try Python prereleases,
pin Python versions in CI, make a temporary environment to reproduce
a bug report that only happens on an older Python point release, etc.


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

Similar to wheels, a top-level directory `pybi-info/` must exist.
(Rationale: `pybi-info` vs `dist-info` makes sure that tools don't get
confused about which kind of metadata they're looking at; leaving off
the `{name}-{version}` part is fine only one pybi can be installed
into a given directory.) It must contain:

* `.../METADATA`: In the same format as described in the current core
  metadata spec, except that the following keys are forbidden because
  they don't make sense:
  
  * `Requires-Dist`
  * `Provides-Extra`
  * `Requires-Python`

* `.../PYBI`: metadata about the archive itself, in the same RFC822-ish
  format as `METADATA` and `WHEEL` files:
  
  ```
  Pybi-Version: 1.0
  Generator: {name} {version}
  Tag: {platform tag}   # may be repeated
  Build: 1   # optional
  ```

* `.../RECORD`: same as in wheels, except see the note about symlinks, below.

* `.../pybi.json`: A JSON file. Example:

  ```json
  {
      "markers_env": {
          "implementation_name": "cpython",
          "implementation_version": "3.9.5",
          "os_name": "posix",
          "platform_machine": "x86_64",
          "platform_python_implementation": "CPython",
          "platform_system": "Linux",
          "python_full_version": "3.9.5",
          "python_version": "3.9",
          "sys_platform": "linux"
      },
      "tags": [
          "cp39-cp39-PLATFORM",
          "cp39-abi3-PLATFORM",
          "cp39-none-PLATFORM",
          "cp38-abi3-PLATFORM",
          "cp37-abi3-PLATFORM",
          "cp36-abi3-PLATFORM",
          "cp35-abi3-PLATFORM",
          "cp34-abi3-PLATFORM",
          "cp33-abi3-PLATFORM",
          "cp32-abi3-PLATFORM",
          "py39-none-PLATFORM",
          "py3-none-PLATFORM",
          "py38-none-PLATFORM",
          "py37-none-PLATFORM",
          "py36-none-PLATFORM",
          "py35-none-PLATFORM",
          "py34-none-PLATFORM",
          "py33-none-PLATFORM",
          "py32-none-PLATFORM",
          "py31-none-PLATFORM",
          "py30-none-PLATFORM",
          "py39-none-any",
          "py3-none-any",
          "py38-none-any",
          "py37-none-any",
          "py36-none-any",
          "py35-none-any",
          "py34-none-any",
          "py33-none-any",
          "py32-none-any",
          "py31-none-any",
          "py30-none-any"
      ]
      "paths": {
          "data": ".",
          "include": "include/python3.9",
          "platinclude": "include/python3.9",
          "platlib": "lib/python3.9/site-packages",
          "platstdlib": "lib/python3.9",
          "purelib": "lib/python3.9/site-packages",
          "scripts": "bin",
          "stdlib": "lib/python3.9"
      },
  }
  ```

  More formally, it must be an object with the following keys:

  * `markers_env`: The value of all PEP 508 marker values that are
    static across installs of this Pybi. (So e.g., it should have
    `python_version`, but not `platform_version`, which on my system
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

  * `tags`: The PEP 425 tags supported by this interpreter, in
    preference order, except that the special platform tag `PLATFORM`
    should replace any platform tags that depend on the final
    installation system.

    Rationale: Pybi installers already need to be able to compute the
    set of platform tags for a given system in order to determine
    whether a `.pybi` file is compatible. So the idea is that they can
    combine their own list of platform tags with this list of
    platform-independent tags to determine which wheels are compatible
    with a given Pybi environment, without installing or running the
    Pybi environment.

    TODO: is this really workable? There are also cases where the
    platform tags depend on the installed binary. For example, most
    Windows *systems* are compatible with both `win32` and
    `win_amd64`, and you can use either for the Python install, but
    then once you pick one you have to use the same platform for
    wheels. So a pybi installer needs to know that `win32` and
    `win_amd64` are both ok tags for pybis, but it feels awkward to
    also expect it to hardcode the knowledge of how those affect the
    resulting wheels.
    
    I guess in this particular case it could be worked around within
    the pybi itself. On Windows, the interpreter also determines the
    full set of platform tags, so there's no need to use the
    `PLATFORM` wildcard at all -- the `pybi.json` can just list the
    full set of explicit tags. But there are probably similar cases on
    other systems? E.g. Linux 32/64 bit, or macOS on ARM with Rosetta
    2?
    
  * `paths`: The install paths needed to install wheels, as relative
    paths starting at the root of the zip file.

    Rationale: `tags` and `paths` together should be enough to let an
    installer choose wheels and install them into an unpacked pybi
    environment, without invoking Python.

    In addition: it must be possible to invoke the Python interpreter
    by running `{paths["scripts"]}/python`. If there are alternative
    interpreter entry points (e.g. `pythonw` for Windows GUI apps),
    then they should also be in that directory under their
    conventional names, with no version number attached.

  You can probably generate a valid `pybi.json` file by doing:

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
  # For some reason, macOS framework builds report their base_path as a directory deep
  # inside the framework
  while "Python.framework" in base_path:
      base_path = os.path.dirname(base_path)
  paths = {key: os.path.relpath(path, base_path) for (key, path) in sysconfig.get_paths().items()}

  json.dump({"markers_env": markers_env, "tags": str_tags, "paths": paths}, sys.stdout)
  ```
  

## Symlinks

Currently, symlinks are used by default in all Unix Python installs
(e.g., `bin/python3 -> bin/python3.9`). And fruthermore, symlinks are
*required* to store macOS framework builds in `pybi` files. So,
`.pybi` files must be able to represent symlinks.


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
name/of/symlink,symlink=path/to/symlink/target
```

That is: we use a special "hash function" called `symlink`, and then
store the actual symlink target as the "hash value". And the length is
left empty.

Rationale: we're already committed to the `RECORD` file containing a
redundant version of everything in the main archive, so for symlinks
we at least need to store some kind of hash, plus some kind of flag to
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
but it seems useful to let pybi's be unpacked by the regular `unzip`
tool, and it only understands the Info-Zip extensions.


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
  `bar/blah.py`.

Unpackers MUST verify that these rules are followed, because without
them attackers could create evil symlinks like `foo -> /etc/passwd` or
`foo -> ../../../../../etc/passwd` and cause havoc.


## Sdists

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

  Rationale: for regular standalone python installers, like the ones
  distributed by Python.org, you probably want to include at least
  `pip`, to [avoid bootstrapping
  issues](https://www.python.org/dev/peps/pep-0453/). But pybis are
  designed to be installed by "smart" installers, that consume the
  pybi as part of some kind of environment setup automation. It's
  easier for these installers to start from a blank slate and then add
  whatever they need, than for them to start with some preinstalled
  packages that they may or may not want.

- Include the full stdlib, *except* for `test`.

  Rationale: the top-level `test` module contains CPython's own test
  suite. It's huge (CPython without `test` is ~37 MB, then `test` adds
  another ~25 MB on top of that!), and essentially never used by
  regular user code. Also, as precedent, the official nuget packages,
  the official manylinux images, and multiple Linux distributions all
  leave it out, and this hasn't caused any major problems.
  
  So this seems like the best way to balance broad compatibility with
  reasonable download/install sizes.

- I'm not shipping any `.pyc` files. They can be generated on the
  final system at minimal cost, and it removes a source of
  location-dependence. (`.pyc` files store the absolute path of the
  corresponding `.py` file and include it in tracebacks; but, pybis
  are relocatable, so the correct path isn't known until after
  install.)
