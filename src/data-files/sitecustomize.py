import os, sys, site

if "POSY_ENV_PYTHONPATH" in os.environ:
    paths = os.environ["POSY_ENV_PYTHONPATH"].split(":")
    for path in paths:
        site.addsitedir(path)
else:
    sys.stderr.write("This Python is managed by, and should be launched by, Posy.\n")
    sys.stderr.write("Unexpected things may happen if you continue.\n")
