# Pybi (*Py*thon *Bi*nary) format

"Like wheels, but instead of a python package, it's an entire python
interpreter environment"

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
`cpython-3.9.5-macosx-11_0_universal2.pybi`.)


## File contents

A `.pybi` file is a zip file, that can be unpacked directly into an
arbitrary location and then used as a self-contained Python
environment. There's no `.data` directory or install scheme keys,
because the Python environment knows which install scheme it's using,
so it can just put things in the right places to start with.

Similar to wheels, the directory `{distribution}-{version}.dist-info/`
must exist, and must contain:

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

* `.../RECORD`: same as in wheels.

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

    This is also just a bunch of generally useful information. For
    example, if you have a `pypy3-7.3.2` pybi, and you want to know
    what version of the Python language that supports, then that's
    recorded in the `python_version` marker.

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
  paths = {key: os.path.relpath(path, base_path) for (key, path) in sysconfig.get_paths().items()}

  json.dump({"markers_env": markers_env, "tags": str_tags, "paths": paths}, sys.stdout)
  ```
  

## Todo

Currently on Unix it's common for environments to have e.g.
`bin/python` as a symlink to `bin/python3`, which is a symlink to
`bin/python3.X`. Is that important to preserve? It wouldn't be a huge
deal to allow symlinks in the zip files, but most zip libraries
(including Python's) don't support them by default, so if we want them
to work then we'll need to say that explicitly.

Should we say anything about other tools installed by default, like
e.g. pip? In general it's kind of up to the pybi builder what they
include, but I feel like "by default" the install should be pretty
minimal, because it's easier to add than to take away. Plus tools like
`posy` want to have full control over what's installed into the new
environment, which is harder if there's unexpected stuff in there.

Should we say anything about superfluous bits of the stdlib that take
up a lot of space, like 'tests'?

Should we say anything about script entry points like idle3 and making
them relocatable? I guess all we need to say is like, FYI, if you have
any scripts included in your pybi, they'd better be relocatable b/c
otherwise they just won't work?

Probably should explicitly say that .pyc files shouldn't be included,
to save space? I guess can make it a SHOULD rather than MUST, on the
grounds that if some .pybi builder decides that the space/time
tradeoff is worth it for them then it's not a big deal? IIRC
generating .pyc files locally will also tend to produce better
tracebacks, since tracebacks use the original path to the .py files,
not the current path.
