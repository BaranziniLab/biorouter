#!/usr/bin/env bash
# Fail if production code opens a socket that a Windows child process inherits.
#
# THREAT MODEL. This gate stops MISTAKES (a developer or a model writing an
# inheritable socket call into production code), not a determined evader. It
# closes every bypass that is cheap to close: an `#[allow]` in any spelling (see
# below), a `clippy` cfg predicate (`#[cfg(not(clippy))]` ships and is never
# linted, because clippy compiles with `--cfg clippy`), a build script or proc
# macro that reads clippy's environment (CLIPPY_*, RUSTC_WORKSPACE_WRAPPER), a
# toolchain whose clippy no longer knows the lint by name, and a clippy.toml
# entry that names no function. What it does NOT close, by name:
#   * a dependency that binds or connects inside its own code (clippy.toml);
#   * a socket made through an API the configuration does not list (a direct
#     `windows-sys` call, say);
#   * a `clippy` predicate or clippy detection no text spells: a proc macro
#     building the identifier from a string, a build script that finds clippy
#     through an environment variable name it assembles at run time or through
#     its parent process, a `--cfg` from RUSTFLAGS or `.cargo/config.toml`;
#   * code behind a Cargo feature no workspace member enables (scripts/release.sh
#     passes no `--features`, so such code is not in a release either).
#
# On Windows, tokio's `TcpListener::bind` and `TcpStream::connect` (and the rest
# of the list in scripts/clippy-non-inheritable/clippy.toml) create the socket
# through mio, which calls a plain `socket()`, and Windows makes that
# inheritable. std's `Command` spawns every child with `bInheritHandles = TRUE`,
# so each child spawned while such a socket is open gets its own handle to it
# and keeps it open after BioRouter closes it: a listener's port stays bound, a
# connection never sends its FIN. `biorouter::net::bind_non_inheritable`
# explains it in full; that helper and `biorouter::net::connect_non_inheritable`
# are the fix.
#
# This is a COMPILER check: clippy's `disallowed_methods`, which matches the
# resolved function, so an alias, a `Self::bind`, a glob import or a macro all
# name the same function and are all caught.
#
# What it lints: `--lib --bins` in the `release` profile, i.e. the production
# code that ships. `#[cfg(test)]` code and `tests/` are not compiled, so a test
# may bind a tokio listener freely; `cfg(not(debug_assertions))` code, compiled
# only in a release build, is linted, and `cfg(debug_assertions)` code, which
# never ships, is not. `cfg(windows)` code is linted only when this runs with
# `--target x86_64-pc-windows-gnu`, which the `cross-check` CI job does.
#
# ⚠ The lint runs at `--force-warn`, and that is the point of it. At `-D`, as it
# used to, an `#[allow]` in the source silences it, and a reviewer did so seven
# ways a text scan for allow attributes never saw (the renamed
# `clippy::disallowed_method`, `clippy::r#disallowed_methods`, a spaced
# `clippy:: disallowed_methods`, an allow a macro emits, `#![allow]` in a
# `#[path]` module or an `include!`d file outside `crates/*/src`, and a call in a
# clap `default_value_t`, which clap_derive wraps in `#[allow(clippy::style)]`).
# No attribute can lower a force-warned lint, so there is no allow to look for:
# every call site is reported, and it is gate.py reading cargo's JSON, not
# rustc's exit status, that fails on one. Every other clippy lint is allowed, and
# ordinary rustc warnings are counted and do not fail it.
#
# Exit status, so a CI log says which kind of failure it is:
#   0  no site, and every production crate was linted
#   1  SITES: production code calls a function the configuration forbids
#   2  BROKEN: the check did not complete (a compile error, a clippy.toml clippy
#      cannot parse, cargo failing, a crate never linted, a clippy that does not
#      know `clippy::disallowed_methods`); it proves nothing
#   3  CONFIGURATION: the gate would guard less than it says: clippy.toml (an
#      entry in a form the gate does not read, a crate no production code links,
#      a path clippy cannot resolve to a function), or source that hides code
#      from clippy (a `clippy` cfg predicate anywhere under crates/, build-time
#      code that reads clippy's environment)
# When several happen, 2 wins over 3 and 3 over 1; all are printed.
#
# The implementation is scripts/clippy-non-inheritable/gate.py (Python, for its
# TOML and JSON parsers: the Windows cross container has no `jq`).
#
# Usage: scripts/check-non-inheritable-sockets.sh [cargo args, e.g. --target <triple>]
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# tomllib is Python 3.11+. macOS's /usr/bin/python3 is 3.9, so look past it.
for python in python3 python3.14 python3.13 python3.12 python3.11; do
  if command -v "$python" >/dev/null 2>&1 &&
    "$python" -c 'import sys; sys.exit(sys.version_info < (3, 11))' 2>/dev/null; then
    exec "$python" "$ROOT/scripts/clippy-non-inheritable/gate.py" "$@"
  fi
done
echo "::error::the socket gate needs Python 3.11 or newer (for tomllib); none was found on PATH" >&2
echo "The gate did NOT run, so it proves nothing." >&2
exit 2
