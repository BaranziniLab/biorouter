#!/usr/bin/env bash
# Decides whether rust.yml's macOS or Windows job runs the full test suite or
# the pull request subset, and writes `full=true|false` to GITHUB_OUTPUT.
#
#   scripts/ci/choose-os-suite.sh windows|macos
#
# Runs in the job's checkout of a pull request, which is GitHub's merge commit
# (refs/pull/N/merge) fetched with `fetch-depth: 2`, so HEAD^1 is the base the
# change was merged onto and `git diff HEAD^1 HEAD` is the change itself.
#
# Full when:
#   * CI_FULL_SUITE is 'true' (a push to main, the nightly, a manual dispatch,
#     or a pull request labelled ci:full), or
#   * the change touches code specific to that OS:
#       - a code path (under crates/, scripts/ or vendor/) that names the OS,
#         such as shell_sandbox/windows.rs or system_auth_macos.rs;
#       - a Rust file whose diff hunks (the changed lines and three lines of
#         context) hold a cfg for the OS. The hunks and not the whole file,
#         because shared files such as session_manager.rs carry a Windows cfg
#         somewhere, and a whole-file match sent most pull requests to the
#         full suite;
#       - the toolchain (rust-toolchain.toml, .cargo/config.toml), and on
#         Windows the dependency set too (Cargo.lock, any Cargo.toml): a C or
#         C++ dependency can build on mingw and fail on MSVC. A Mac needs no
#         such rule: everyone who works here builds on one before pushing;
#   * the change cannot be read (no base commit in the checkout). Failing
#     toward the full suite costs time; failing toward the subset would cost
#     coverage.
#
# Otherwise the subset: nothing in the change runs only on that OS, so the
# ubuntu jobs (every suite) and the cross-check (every target's compile) cover
# it, and the full suite runs again on main after merge. Integration test files
# (crates/*/tests/) never select the suite: the few that run on macOS or
# Windows run in the subset as well, and the rest run on ubuntu only.
set -euo pipefail

os="${1:?usage: choose-os-suite.sh windows|macos}"
out="${GITHUB_OUTPUT:-/dev/stdout}"

decide() {
  echo "full=$1" >> "$out"
  if [ "$1" = true ]; then echo "suite: full ($2)"; else echo "suite: pull request subset ($2)"; fi
  exit 0
}

case "$os" in
  windows)
    path_re='windows|win32|msvc|\.ps1$'
    # `cfg(windows)`, `cfg!(windows)`, `not(windows)`, `all(test, windows)`,
    # `any(unix, windows)` and the target_os / target_family spellings.
    code_re='target_os = "windows"|target_family = "windows"|[(, !]windows[),]'
    deps_re='(^|/)(Cargo\.lock|Cargo\.toml|rust-toolchain\.toml)$|(^|/)\.cargo/config(\.toml)?$'
    ;;
  macos)
    path_re='macos|darwin|apple|seatbelt'
    code_re='target_os = "macos"|target_vendor = "apple"'
    deps_re='(^|/)rust-toolchain\.toml$|(^|/)\.cargo/config(\.toml)?$'
    ;;
  *) echo "usage: choose-os-suite.sh windows|macos" >&2; exit 2 ;;
esac

[ "${CI_FULL_SUITE:-true}" = true ] && decide true "not an unlabelled pull request"

# CHOOSE_SUITE_BASE / CHOOSE_SUITE_HEAD replay a decision locally, e.g.
#   CI_FULL_SUITE=false CHOOSE_SUITE_BASE=$(git merge-base origin/main X) \
#   CHOOSE_SUITE_HEAD=X GITHUB_OUTPUT=/dev/null scripts/ci/choose-os-suite.sh windows
head="${CHOOSE_SUITE_HEAD:-HEAD}"
base=$(git rev-parse -q --verify "${CHOOSE_SUITE_BASE:-HEAD^1}") \
  || decide true "the checkout holds no base commit to diff against"
files=$(git diff --name-only "$base" "$head")
[ -n "$files" ] || decide true "the change lists no files"

hits=()
while IFS= read -r f; do
  [ -n "$f" ] || continue
  [[ "$f" =~ ^crates/[^/]+/tests/ ]] && continue
  # A path names the OS only where it is code: a document or a packaging
  # workflow about Windows changes no behaviour these suites test.
  if [[ "$f" =~ ^(crates|scripts|vendor)/ ]] && [[ "$f" != *.md ]] \
      && grep -qiE "$path_re" <<<"$f"; then
    hits+=("$f (path)")
  elif grep -qE "$deps_re" <<<"$f"; then
    hits+=("$f (dependencies or toolchain)")
  # `grep -E >/dev/null`, not `grep -q`: under pipefail an early exit can
  # SIGPIPE `git diff` on a large diff and turn a match into a miss.
  elif [[ "$f" == *.rs ]] && git diff -U3 "$base" "$head" -- "$f" | grep -E "$code_re" >/dev/null; then
    hits+=("$f (cfg in the change)")
  fi
done <<<"$files"

if [ "${#hits[@]}" -gt 0 ]; then
  printf '  %s\n' "${hits[@]}"
  decide true "${#hits[@]} changed file(s) are specific to $os"
fi
decide false "no changed file is specific to $os"
