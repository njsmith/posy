# Note: on Ubuntu, `apt install qemu-user-static` makes it possible to run this script
# with all arches.
#
# To rebuild specific versions:
#   python rebuild.py x86_64 i686
#
# To rebuild all versions:
#   python rebuild.py

# This image uses glibc 2.28. But, it has lots of arches available, it runs well under
# qemu-user-static for me (as opposed to centos:7, where yum segfaults on arm), and
# based on 'readelf' output + experiments I believe that the particular symbols we're
# using are stable enough that 2.28 should still work fine. And if I'm wrong we can
# always fix it :-)
BASE_IMAGE = "debian:buster-slim"

# docker platforms from the "OS/ARCH" column at
#   https://hub.docker.com/_/debian?tab=tags&page=1&ordering=last_updated&name=buster
PY_ARCH_TO_DOCKER_PLATFORM = {
    "x86_64": "linux/amd64",
    "i686": "linux/386",
    "aarch64": "linux/arm64/v8",
    "armv7l": "linux/arm/v7",
    "ppc64le": "linux/ppc64le",
    "s390x": "linux/s390x",
}

import sys
import os
import subprocess

py_arches = sys.argv[1:]
if not py_arches:
    py_arches = PY_ARCH_TO_DOCKER_PLATFORM.keys()

for py_arch in py_arches:
    print(f"-- Building for {py_arch} --")
    docker_arch = PY_ARCH_TO_DOCKER_PLATFORM[py_arch]
    exec_name = f"glibc-detector-{py_arch}"

    subprocess.run(
        [
            "docker", "run", "--rm", "-it",
            f"--platform={docker_arch}",
            "-v", f"{os.getcwd()}:/host",
            BASE_IMAGE,
            "bash", "-c",
            f"""
                set -euxo pipefail
                cd /host
                #yum install -y gcc
                apt update && apt install -y gcc
                gcc -Os glibc-detector.c -o {exec_name}
                strip {exec_name}
                chown {os.getuid()}:{os.getgid()} {exec_name}
                ./{exec_name}
            """
        ],
        check=True
    )
