/*
 * Empty stand-in for the POSIX <unistd.h>, which the MSVC toolchain does not
 * ship.
 *
 * The `rs-x11-hash` crate compiles its C sources with clang (hardcoded in its
 * build script) and its `sph_types.h` includes <unistd.h> unconditionally, even
 * though it uses nothing from it. Official Windows builds cross-compile from
 * Linux with mingw, which does provide the header, so this only matters for a
 * native `x86_64-pc-windows-msvc` build.
 *
 * Put this directory on the compiler's include path for that target:
 *
 *     CFLAGS_x86_64_pc_windows_msvc=-I<repo>/build-support/windows-msvc
 *
 * See the "Building natively on Windows (MSVC)" section of CONTRIBUTING.md.
 */
