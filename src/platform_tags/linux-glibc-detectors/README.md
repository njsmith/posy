# The problem

When running on Linux, `posy` needs to know whether glibc is
installed, and which version, for which architectures. This is
difficult because:

- `posy` itself might be linked as a static musl binary, so it can't
  use `dlopen` or call any glibc-specific functions.
- The system might support multiple architectures (e.g. x86-64 and
  i386).
  
# The solution

We have a tiny little program `glibc-detector.c`, which links against
glibc, and does nothing except print out the output of `gnu_get_libc_version()`

We built this program in a Docker container running an old distro, to
make sure it doesn't use any new glibc symbol versions, against a
variety of architectures. Then we save those executables here, so we
don't need access to an old distro at build time – and since by
definition, these executables are unlikely to change!
