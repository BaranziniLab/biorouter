#!/usr/bin/env bash
# The local dictation engine's build prerequisites (DD-10 rules 7, 8, 10 and 11, and the
# crate boundaries of DD-5 and P-10) hold in the tree.
#
# Usage:
#   scripts/check-dictation-build-prereqs.sh [--root DIR]   check (CI `guards`)
#   scripts/check-dictation-build-prereqs.sh --self-test    prove each rule fails when broken
#
# When crates/biorouter-dictation exists, the check fails on any of:
#   rule 1  the Dockerfile's `FROM rust:... AS builder` stage installs no `cmake`. whisper.cpp
#           is built with CMake, and the image build would stop at the engine's build script.
#   rule 2  bin/cmake or bin/.cmake-<version>.pkg is missing: the hermit pin that gives every
#           developer and the release Mac the same CMake.
#   rule 3  crates/biorouter-dictation/build.rs has no `.profile("Release")` outside a comment.
#           cmake-rs maps opt-level 0 to Debug and opt-level "z" (the Dockerfile's) to
#           MinSizeRel, and either one makes the engine several times slower.
#   rule 4  biorouter-dictation reaches `bindgen` in Cargo.lock by any route other than
#           aws-lc-sys's own dependency on bindgen. The engine's FFI is written by hand, and
#           bindgen would need libclang on every build host. That one edge is allowed: the
#           model installer's TLS stack reaches aws-lc-sys (reqwest, rustls, aws-lc-rs), the
#           whole workspace already builds it for the same stack in biorouter core, and on every
#           target Biorouter builds its bindgen is optional and it uses pregenerated bindings.
#           Any other route is refused and the error names it: a direct dependency, a route
#           through another crate, or a package below aws-lc-sys that needs bindgen itself.
#           Cargo.lock records no features, so this rule cannot see whether a build turns on
#           aws-lc-sys's `bindgen` or `ssl` feature (or aws-lc-rs's `bindgen`), which would
#           build bindgen without changing the lock. None does today: measured on 2026-11-06,
#           `cargo tree --locked -p biorouter-cli -p biorouter-server -e features -i aws-lc-sys
#           --target all` lists only `default` and `prebuilt-nasm`.
#   rule 5  the closure of biorouter-crew holds biorouter-dictation. The Crew broker is copied
#           to servers over SSH and carries no engine.
#   rule 6  the closure of biorouter-server holds biorouter-dictation-capture. The daemon never
#           links audio capture (P-10); inert until that crate exists (PR-25).
#
# `guards` has no Rust toolchain, so the closures come from Cargo.lock itself: every
# [[package]] block's `name` and the first word of each `dependencies` entry. The lock lists
# the optional and dev dependencies of every package as well, so a closure read this way is a
# superset of what any one build links, which is the right direction for "must not contain".
#
# Plain bash 3.2 (macOS's /bin/bash), awk, grep and sed; nothing else.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# lock_reaches <Cargo.lock> <from> <to> [<through>]: exit 0 when <to> is in the dependency
# closure of <from> (itself included), and print one shortest route as `a -> b -> c`; exit 1
# when it is not. With <through>, that package's own direct dependency on <to> is not
# followed and every other dependency of it is, so the answer is whether <to> can be reached
# by any route other than that one edge, including a route below <through>. A name that appears
# in several versions is one node whose edges are the union of all of them. The answer is
# awk's own exit status, never a `| grep -q`: under pipefail, grep leaving early can kill the
# writer with SIGPIPE and turn a match into a miss.
lock_reaches() {
  awk -v start="$2" -v want="$3" -v through="${4:-}" '
    function flush() {
      if (name != "") { edges[name] = edges[name] " " found }
      name = ""; found = ""; in_deps = 0
    }
    function add(entry,   w) {
      gsub(/^[ \t]*"|"[ \t]*,?[ \t]*$/, "", entry)
      split(entry, w, " ")
      if (w[1] != "") { found = found " " w[1] }
    }
    /^\[\[package\]\]/ { flush(); next }
    /^\[/ { flush(); next }
    /^name = "/ {
      n = $0; sub(/^name = "/, "", n); sub(/".*$/, "", n); name = n; next
    }
    /^dependencies = \[/ {
      rest = $0; sub(/^dependencies = \[/, "", rest)
      if (rest ~ /\]/) {
        sub(/\].*$/, "", rest)
        count = split(rest, items, ",")
        for (i = 1; i <= count; i++) { add(items[i]) }
      } else {
        in_deps = 1
      }
      next
    }
    in_deps && /^\]/ { in_deps = 0; next }
    in_deps { add($0); next }
    END {
      flush()
      head = 1; tail = 1; queue[1] = start; seen[start] = 1
      while (head <= tail) {
        p = queue[head]; head++
        if (p == want) {
          route = p; q = p
          while (q != start) { q = parent[q]; route = q " -> " route }
          print route
          exit 0
        }
        count = split(edges[p], next_deps, " ")
        for (i = 1; i <= count; i++) {
          d = next_deps[i]
          if (through != "" && p == through && p != start && d == want) { continue }
          if (d != "" && !(d in seen)) { seen[d] = 1; parent[d] = p; tail++; queue[tail] = d }
        }
      }
      exit 1
    }
  ' "$1"
}

# The `apt-get install` package words of the Dockerfile stage `FROM rust:... AS builder`,
# one per line. Continuation lines are joined first; flags and shell words are dropped.
builder_apt_packages() {
  awk '
    function emit(line,   rest, n, i, w) {
      if (line !~ /apt-get[ \t]+install/) { return }
      rest = line
      sub(/^.*apt-get[ \t]+install/, "", rest)
      sub(/&&.*$/, "", rest)
      sub(/;.*$/, "", rest)
      n = split(rest, w, /[ \t]+/)
      for (i = 1; i <= n; i++) {
        if (w[i] != "" && w[i] !~ /^-/) { print w[i] }
      }
    }
    /^[ \t]*#/ { next }
    {
      line = $0
      if (joined != "") { line = joined " " line }
      if (line ~ /\\[ \t]*$/) { sub(/\\[ \t]*$/, "", line); joined = line; next }
      joined = ""
      if (line ~ /^[ \t]*[Ff][Rr][Oo][Mm][ \t]/) {
        in_builder = (line ~ /^[ \t]*[Ff][Rr][Oo][Mm][ \t]+(--[^ \t]+[ \t]+)*rust:[^ \t]+[ \t]+[Aa][Ss][ \t]+builder[ \t]*$/)
        if (in_builder) { found_stage = 1 }
        next
      }
      if (in_builder) { emit(line) }
    }
    END { if (!found_stage) { print "<no-builder-stage>" } }
  ' "$1"
}

check() {
  local root="$1" fail=0 crate lock route
  crate="$root/crates/biorouter-dictation"
  lock="$root/Cargo.lock"
  err() { printf '::error::check-dictation-build-prereqs %s\n' "$1" >&2; fail=1; }

  if [ ! -d "$crate" ]; then
    echo "dictation build prerequisites: crates/biorouter-dictation does not exist yet; nothing to check"
    return 0
  fi

  # Rule 1: cmake in the Docker builder stage.
  if [ ! -f "$root/Dockerfile" ]; then
    err "rule 1: $root/Dockerfile is missing, so the builder stage's cmake cannot be checked"
  else
    local packages
    packages=$'\n'"$(builder_apt_packages "$root/Dockerfile")"$'\n'
    if [[ "$packages" == *$'\n<no-builder-stage>\n'* ]]; then
      err "rule 1: the Dockerfile has no 'FROM rust:... AS builder' stage to check for cmake"
    elif [[ "$packages" != *$'\ncmake\n'* ]]; then
      err "rule 1: the Dockerfile's builder stage installs no cmake; whisper.cpp builds with CMake, so the image build would stop at the engine (add cmake to its apt-get install list)"
    fi
  fi

  # Rule 2: the hermit cmake pin.
  if [ ! -e "$root/bin/cmake" ] && [ ! -L "$root/bin/cmake" ]; then
    err "rule 2: bin/cmake is missing; run bin/hermit install cmake@<version> so every build host uses the same CMake"
  fi
  local pkg found_pkg=0
  for pkg in "$root"/bin/.cmake-*.pkg; do
    if [ -e "$pkg" ] || [ -L "$pkg" ]; then found_pkg=1; fi
  done
  if [ "$found_pkg" -eq 0 ]; then
    err "rule 2: bin/.cmake-<version>.pkg is missing; the hermit cmake pin records its version there"
  fi

  # Rule 3: CMake always configures Release, whatever the Cargo profile.
  if [ ! -f "$crate/build.rs" ]; then
    err "rule 3: crates/biorouter-dictation/build.rs is missing"
  elif ! awk '!/^[ \t]*\/\// && index($0, ".profile(\"Release\")") { found = 1 } END { exit !found }' "$crate/build.rs"; then
    err "rule 3: crates/biorouter-dictation/build.rs does not call .profile(\"Release\"); cmake-rs would build ggml as Debug at opt-level 0 and MinSizeRel at opt-level \"z\""
  fi

  # Rules 4 to 6: dependency closures read from Cargo.lock.
  if [ ! -f "$lock" ]; then
    err "rules 4 to 6: $lock is missing"
  else
    if route=$(lock_reaches "$lock" biorouter-dictation bindgen aws-lc-sys); then
      err "rule 4: biorouter-dictation reaches bindgen in Cargo.lock other than through aws-lc-sys's own dependency on it ($route); the engine's FFI is written by hand (DD-10 rule 8), and only aws-lc-sys, whose bindgen is optional on every target Biorouter builds, may depend on it"
    fi
    if route=$(lock_reaches "$lock" biorouter-crew biorouter-dictation); then
      err "rule 5: biorouter-crew's dependency closure in Cargo.lock holds biorouter-dictation ($route); the Crew broker must not carry the engine"
    fi
    if route=$(lock_reaches "$lock" biorouter-server biorouter-dictation-capture); then
      err "rule 6: biorouter-server's dependency closure in Cargo.lock holds biorouter-dictation-capture ($route); the daemon never links audio capture (P-10)"
    fi
  fi

  [ "$fail" -eq 0 ] || return 1
  echo "dictation build prerequisites: cmake in the Docker builder, the hermit cmake pin, a Release CMake profile, no bindgen outside aws-lc-sys, and no engine in the Crew broker or capture in the daemon"
}

self_test() {
  local sandbox failures=0 cases=0
  sandbox=$(mktemp -d "${TMPDIR:-/tmp}/dictation-prereqs-selftest.XXXXXX")
  # shellcheck disable=SC2064 # expand now: the path is fixed for this run
  trap "rm -rf '$sandbox'" EXIT

  # A scratch root holding exactly what the check reads, every rule satisfied.
  clean_root() {
    local r="$1"
    rm -rf "$r"
    mkdir -p "$r/bin" "$r/crates/biorouter-dictation"
    cat > "$r/Dockerfile" <<'EOF'
# syntax=docker/dockerfile:1.4
FROM --platform=$BUILDPLATFORM golang:1.26.8-bookworm AS helper
RUN apt-get update && apt-get install -y --no-install-recommends python3 git && rm -rf /var/lib/apt/lists/*

FROM rust:1.92-bookworm AS builder
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    build-essential \
    cmake \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*
RUN cargo build --release --package biorouter-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates
EOF
    ln -s hermit "$r/bin/.cmake-3.31.6.pkg"
    ln -s .cmake-3.31.6.pkg "$r/bin/cmake"
    printf '#!/bin/sh\n' > "$r/bin/hermit"
    cat > "$r/crates/biorouter-dictation/build.rs" <<'EOF'
fn main() {
    let dst = cmake::Config::new("cmake")
        .profile("Release")
        .define("GGML_NATIVE", "OFF")
        .build();
    println!("cargo:rustc-link-search=native={}/lib", dst.display());
}
EOF
    cat > "$r/Cargo.lock" <<'EOF'
# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "biorouter"
version = "1.0.0"
dependencies = [
 "biorouter-dictation",
 "serde",
]

[[package]]
name = "biorouter-crew"
version = "1.0.0"
dependencies = [
 "serde",
 "sha2 0.10.9",
]

[[package]]
name = "biorouter-dictation"
version = "1.0.0"
dependencies = [
 "cc",
 "cmake",
 "serde",
]

[[package]]
name = "biorouter-server"
version = "1.0.0"
dependencies = [
 "biorouter",
 "biorouter-dictation",
]

[[package]]
name = "cc"
version = "1.2.64"
dependencies = [
 "shlex",
]

[[package]]
name = "cmake"
version = "0.1.58"
dependencies = [
 "cc",
]

[[package]]
name = "serde"
version = "1.0.228"

[[package]]
name = "sha2"
version = "0.10.9"

[[package]]
name = "sha2"
version = "0.11.0"
dependencies = ["digest"]

[[package]]
name = "shlex"
version = "1.3.0"
EOF
  }

  expect() { # expect <pass|fail> <case name> <root> <words the output must contain>
    local want=$1 name=$2 root=$3 needle=$4 output code good=0
    cases=$((cases + 1))
    set +e
    output=$(bash "$SCRIPT_PATH" --root "$root" 2>&1)
    code=$?
    set -e
    if [ "$want" = pass ] && [ "$code" -eq 0 ]; then good=1; fi
    if [ "$want" = fail ] && [ "$code" -ne 0 ]; then good=1; fi
    case "$output" in *"$needle"*) ;; *) good=0 ;; esac
    if [ "$good" -eq 1 ]; then
      echo "ok   $name"
    else
      echo "FAIL $name (exit $code)"
      printf '%s\n' "$output" | sed 's/^/     /'
      failures=$((failures + 1))
    fi
  }

  # Replace one line of a scratch file (BSD and GNU sed differ on -i, so rewrite it).
  drop_lines() { # drop_lines <file> <grep -v pattern>
    grep -v -- "$2" "$1" > "$1.new" || true
    mv "$1.new" "$1"
  }

  # add_package <lock> <name> [<dependency>...]: append a [[package]] block to a scratch lock.
  add_package() {
    local lock=$1 name=$2 dep
    shift 2
    printf '\n[[package]]\nname = "%s"\nversion = "1.0.0"\n' "$name" >> "$lock"
    if [ "$#" -gt 0 ]; then
      printf 'dependencies = [\n' >> "$lock"
      for dep in "$@"; do printf ' "%s",\n' "$dep" >> "$lock"; done
      printf ']\n' >> "$lock"
    fi
  }

  # add_dependency <lock> <package> <dependency>: one more entry in the multi line
  # `dependencies` list of a package the scratch lock already has.
  add_dependency() {
    awk -v pkg="$2" -v dep="$3" '
      { print }
      $0 == ("name = \"" pkg "\"") { want = 1; next }
      want && /^dependencies = \[$/ { print " \"" dep "\","; want = 0 }
      /^$/ { want = 0 }
    ' "$1" > "$1.new"
    mv "$1.new" "$1"
  }

  # The model installer's TLS stack as Cargo.lock records it: bindgen under aws-lc-sys only.
  add_tls_stack() { # add_tls_stack <lock>
    add_dependency "$1" biorouter-dictation reqwest
    add_package "$1" reqwest rustls
    add_package "$1" rustls aws-lc-rs
    add_package "$1" aws-lc-rs aws-lc-sys
    add_package "$1" aws-lc-sys "bindgen 0.72.1" cc cmake
    add_package "$1" bindgen
  }

  local r="$sandbox/root"

  clean_root "$r"
  expect pass "a tree with every prerequisite passes" "$r" "dictation build prerequisites: cmake in the Docker builder"

  clean_root "$r"
  rm -rf "$r/crates/biorouter-dictation"
  drop_lines "$r/Dockerfile" 'cmake'
  rm -f "$r/bin/cmake" "$r/bin/.cmake-3.31.6.pkg"
  expect pass "a tree without the dictation crate has nothing to check" "$r" "does not exist yet"

  clean_root "$r"
  drop_lines "$r/Dockerfile" '^    cmake'
  printf 'RUN apt-get install -y cmake\n' >> "$r/Dockerfile"
  expect fail "cmake only in another stage fails rule 1" "$r" "rule 1: the Dockerfile's builder stage installs no cmake"

  clean_root "$r"
  drop_lines "$r/Dockerfile" 'AS builder'
  expect fail "no builder stage fails rule 1" "$r" "rule 1: the Dockerfile has no"

  clean_root "$r"
  rm -f "$r/bin/cmake"
  expect fail "a missing bin/cmake fails rule 2" "$r" "rule 2: bin/cmake is missing"

  clean_root "$r"
  rm -f "$r/bin/.cmake-3.31.6.pkg"
  expect fail "a missing bin/.cmake-<version>.pkg fails rule 2" "$r" "rule 2: bin/.cmake-<version>.pkg is missing"

  clean_root "$r"
  drop_lines "$r/crates/biorouter-dictation/build.rs" 'profile'
  printf '// .profile("Release") is gone\n' >> "$r/crates/biorouter-dictation/build.rs"
  expect fail "a Release profile only in a comment fails rule 3" "$r" "rule 3: crates/biorouter-dictation/build.rs does not call"

  clean_root "$r"
  rm -f "$r/crates/biorouter-dictation/build.rs"
  expect fail "a missing build.rs fails rule 3" "$r" "rule 3: crates/biorouter-dictation/build.rs is missing"

  clean_root "$r"
  awk '{ print } /^name = "shlex"/ { getline; print; print "dependencies = [\n \"bindgen 0.72.1\",\n]" }' \
    "$r/Cargo.lock" > "$r/Cargo.lock.new"
  printf '\n[[package]]\nname = "bindgen"\nversion = "0.72.1"\n' >> "$r/Cargo.lock.new"
  mv "$r/Cargo.lock.new" "$r/Cargo.lock"
  expect fail "bindgen three steps below the crate fails rule 4" "$r" "(biorouter-dictation -> cc -> shlex -> bindgen)"

  clean_root "$r"
  add_dependency "$r/Cargo.lock" biorouter-dictation "bindgen 0.72.1"
  add_package "$r/Cargo.lock" bindgen
  expect fail "bindgen as a direct dependency of the crate fails rule 4" "$r" "rule 4: biorouter-dictation reaches bindgen in Cargo.lock other than through aws-lc-sys's own dependency on it (biorouter-dictation -> bindgen)"

  clean_root "$r"
  add_tls_stack "$r/Cargo.lock"
  expect pass "bindgen only through aws-lc-sys passes rule 4" "$r" "no bindgen outside aws-lc-sys"

  clean_root "$r"
  add_tls_stack "$r/Cargo.lock"
  add_dependency "$r/Cargo.lock" biorouter-dictation zstd-sys
  add_package "$r/Cargo.lock" zstd-sys "bindgen 0.72.1" cc
  expect fail "bindgen through aws-lc-sys and also through another crate fails rule 4" "$r" "other than through aws-lc-sys's own dependency on it (biorouter-dictation -> zstd-sys -> bindgen)"

  clean_root "$r"
  add_tls_stack "$r/Cargo.lock"
  add_dependency "$r/Cargo.lock" biorouter-dictation "bindgen 0.72.1"
  expect fail "bindgen through aws-lc-sys and also directly fails rule 4" "$r" "other than through aws-lc-sys's own dependency on it (biorouter-dictation -> bindgen)"

  clean_root "$r"
  add_tls_stack "$r/Cargo.lock"
  add_dependency "$r/Cargo.lock" aws-lc-rs zstd-sys
  add_package "$r/Cargo.lock" zstd-sys "bindgen 0.72.1"
  expect fail "bindgen through a sibling of aws-lc-sys fails rule 4" "$r" "(biorouter-dictation -> reqwest -> rustls -> aws-lc-rs -> zstd-sys -> bindgen)"

  clean_root "$r"
  add_tls_stack "$r/Cargo.lock"
  add_dependency "$r/Cargo.lock" aws-lc-sys foo-sys
  add_package "$r/Cargo.lock" foo-sys "bindgen 0.72.1"
  expect fail "bindgen through a package below aws-lc-sys fails rule 4" "$r" "(biorouter-dictation -> reqwest -> rustls -> aws-lc-rs -> aws-lc-sys -> foo-sys -> bindgen)"

  clean_root "$r"
  awk '{ print } /^name = "sha2"/ { getline; print; if ($0 ~ /0.11.0/) { getline; print "dependencies = [\"digest\", \"biorouter-dictation\"]"; next } }' \
    "$r/Cargo.lock" > "$r/Cargo.lock.new"
  mv "$r/Cargo.lock.new" "$r/Cargo.lock"
  expect fail "the engine reached through a second version of a dependency fails rule 5" "$r" "rule 5: biorouter-crew's dependency closure in Cargo.lock holds biorouter-dictation (biorouter-crew -> sha2 -> biorouter-dictation)"

  clean_root "$r"
  awk '{ print } /^name = "biorouter"$/ { getline; print; getline; print; print " \"biorouter-dictation-capture\"," }' \
    "$r/Cargo.lock" > "$r/Cargo.lock.new"
  printf '\n[[package]]\nname = "biorouter-dictation-capture"\nversion = "1.0.0"\n' >> "$r/Cargo.lock.new"
  mv "$r/Cargo.lock.new" "$r/Cargo.lock"
  expect fail "capture reached from the daemon through core fails rule 6" "$r" "(biorouter-server -> biorouter -> biorouter-dictation-capture)"

  clean_root "$r"
  printf '\n[[package]]\nname = "biorouter-dictation-capture"\nversion = "1.0.0"\n' >> "$r/Cargo.lock"
  expect pass "capture present but unreachable from the daemon passes" "$r" "dictation build prerequisites: cmake in the Docker builder"

  if [ "$failures" -ne 0 ]; then
    echo "::error::$failures of $cases self test case(s) did not behave as expected" >&2
    return 1
  fi
  echo "check-dictation-build-prereqs self test: $cases cases behaved as expected"
}

ROOT="$DEFAULT_ROOT"
case "${1:-}" in
  --self-test)
    [ "$#" -eq 1 ] || { echo "usage: $0 [--root DIR] | --self-test" >&2; exit 2; }
    self_test
    ;;
  --root)
    [ "$#" -eq 2 ] || { echo "usage: $0 [--root DIR] | --self-test" >&2; exit 2; }
    ROOT="$(cd "$2" && pwd)"
    check "$ROOT"
    ;;
  "")
    check "$ROOT"
    ;;
  *)
    echo "usage: $0 [--root DIR] | --self-test" >&2
    exit 2
    ;;
esac
