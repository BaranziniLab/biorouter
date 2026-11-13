#!/usr/bin/env bash
# The hostname-call gate (#461): clippy's `disallowed_methods`, configured by
# the repository's clippy.toml, denied over the workspace's lib and bin code.
#
#   scripts/ci/clippy-disallowed-methods.sh [cargo args...]
#
# A script and not a bare command because rust.yml runs it through
# `cross_windows` (scripts/cross-env.sh), which appends `--target <triple>` to
# whatever it is given. Appended to `cargo clippy ... -- -D ...` that flag would
# land after the `--` and reach clippy-driver instead of cargo; here the extra
# arguments go to cargo, before the separator.
set -euo pipefail
exec cargo clippy --workspace --lib --bins --locked "$@" -- -D clippy::disallowed_methods
