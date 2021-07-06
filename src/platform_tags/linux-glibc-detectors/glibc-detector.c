/*
 * A tiny C program that tries to fetch the version of glibc that it's run
 * against.
 */

#include <gnu/libc-version.h>
#include <stdio.h>

int main(int argc, char** argv)
{
    puts(gnu_get_libc_version());
    return 0;
}
