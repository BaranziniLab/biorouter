#!/usr/bin/env bash
# The Dockerfile's Rust build fits the 16 GB GitHub runners that build the image.
#
# Usage:
#   scripts/check-docker-build.sh [--root DIR]   check (CI `guards` and docker-image.yml)
#   scripts/check-docker-build.sh --self-test    prove each rule fails when broken
#
# The check fails on either of:
#   rule 1  the `cargo build` in the `FROM rust:... AS builder` stage does not name, with
#           `--bin`, exactly the binaries a later stage copies out of /build/target/release.
#           Without `--bin` a package links every binary it has (biorouter-server has
#           biorouterd, generate_schema and knowledge_ingest_probe), each one a whole program
#           link, and up to three of them ran at once.
#   rule 2  the builder stage sets CARGO_PROFILE_RELEASE_LTO to `true` or `fat`. Fat LTO held
#           17 GB to 18 GB in one rustc process for each of biorouter and biorouterd on
#           linux/arm64 (24.6 GB for the whole builder, measured in a 4 CPU builder on
#           2026-11-16), and the arm64 runner, with 16 GB, was killed there in every nightly
#           run of docker-image.yml before this check existed.
#
# Plain bash 3.2 (macOS's /bin/bash), awk and grep; nothing else.
set -euo pipefail

SCRIPT_PATH="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/$(basename "${BASH_SOURCE[0]}")"
DEFAULT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# One logical Dockerfile line per output line: continuations joined, comments dropped.
logical_lines() {
  awk '
    /^[ \t]*#/ { next }
    {
      line = $0
      if (joined != "") { line = joined " " line }
      if (line ~ /\\[ \t]*$/) { sub(/\\[ \t]*$/, "", line); joined = line; next }
      joined = ""
      print line
    }
    END { if (joined != "") { print joined } }
  ' "$1"
}

# The builder stage's lines, from its FROM to the next FROM.
builder_lines() {
  logical_lines "$1" | awk '
    /^[ \t]*[Ff][Rr][Oo][Mm][ \t]/ {
      in_builder = ($0 ~ /^[ \t]*[Ff][Rr][Oo][Mm][ \t]+(--[^ \t]+[ \t]+)*rust:[^ \t]+[ \t]+[Aa][Ss][ \t]+builder[ \t]*$/)
      if (in_builder) { found = 1 }
      next
    }
    in_builder { print }
    END { if (!found) { print "<no-builder-stage>" } }
  '
}

# The binary names later stages copy out of the builder's release folder, sorted.
shipped_binaries() {
  logical_lines "$1" | awk '
    /^[ \t]*[Cc][Oo][Pp][Yy][ \t]/ && /--from=builder/ {
      n = split($0, w, /[ \t]+/)
      for (i = 1; i <= n; i++) {
        if (w[i] ~ /^\/build\/target\/release\/[^\/]+$/) {
          name = w[i]; sub(/^.*\//, "", name); print name
        }
      }
    }
  ' | sort -u
}

# The `--bin` names of the builder's `cargo build` lines, sorted; `<all-bins>` for `--bins`
# and `<no-cargo-build>` when the stage runs none.
built_binaries() {
  printf '%s\n' "$1" | awk '
    /cargo[ \t]+build/ {
      found = 1
      rest = $0
      sub(/^.*cargo[ \t]+build/, "", rest)
      sub(/&&.*$/, "", rest)
      sub(/;.*$/, "", rest)
      n = split(rest, w, /[ \t]+/)
      for (i = 1; i <= n; i++) {
        if (w[i] == "--bins") { print "<all-bins>" }
        else if (w[i] == "--bin" && i < n) { print w[i + 1] }
        else if (w[i] ~ /^--bin=/) { v = w[i]; sub(/^--bin=/, "", v); print v }
      }
    }
    END { if (!found) { print "<no-cargo-build>" } }
  ' | sort -u
}

check() {
  local root="$1" fail=0 dockerfile builder shipped built lto
  dockerfile="$root/Dockerfile"
  err() { printf '::error::check-docker-build %s\n' "$1" >&2; fail=1; }

  if [ ! -f "$dockerfile" ]; then
    err "$dockerfile is missing"
    return 1
  fi
  builder="$(builder_lines "$dockerfile")"
  if [ "$builder" = "<no-builder-stage>" ]; then
    err "the Dockerfile has no 'FROM rust:... AS builder' stage to check"
    return 1
  fi

  # Rule 1: the cargo build names exactly the binaries that are shipped.
  shipped="$(shipped_binaries "$dockerfile")"
  built="$(built_binaries "$builder")"
  if [ -z "$shipped" ]; then
    err "rule 1: no stage copies a binary out of /build/target/release, so the shipped binaries cannot be checked"
  elif [ "$built" = "<no-cargo-build>" ]; then
    err "rule 1: the builder stage runs no cargo build"
  elif [ -z "$built" ]; then
    err "rule 1: the builder's cargo build names no --bin, so each package links every binary it has, each one a whole program link; name the shipped ones ($(printf '%s' "$shipped" | tr '\n' ' ' | sed 's/ $//')) with --bin"
  elif [ "$built" != "$shipped" ]; then
    err "rule 1: the builder's cargo build links [$(printf '%s' "$built" | tr '\n' ' ' | sed 's/ $//')] but the image ships [$(printf '%s' "$shipped" | tr '\n' ' ' | sed 's/ $//')]; name exactly the shipped binaries with --bin, because a package without --bin links every binary it has and each one is a whole program link"
  fi

  # Rule 2: no fat LTO.
  lto="$(printf '%s\n' "$builder" | awk '
    /^[ \t]*[Ee][Nn][Vv][ \t]/ {
      n = split($0, w, /[ \t]+/)
      for (i = 2; i <= n; i++) {
        if (w[i] ~ /^CARGO_PROFILE_RELEASE_LTO=/) { v = w[i]; sub(/^CARGO_PROFILE_RELEASE_LTO=/, "", v) }
        else if (w[i] == "CARGO_PROFILE_RELEASE_LTO" && i < n) { v = w[i + 1] }
      }
    }
    END { gsub(/["\047]/, "", v); print tolower(v) }
  ')"
  case "$lto" in
    true | fat)
      err "rule 2: the builder stage sets CARGO_PROFILE_RELEASE_LTO=$lto; fat LTO held 17 GB to 18 GB in one rustc process per binary on linux/arm64, more than the 16 GB runner that builds the image (use thin)"
      ;;
  esac

  [ "$fail" -eq 0 ] || return 1
  echo "docker build: the builder links only the shipped binaries ($(printf '%s' "$shipped" | tr '\n' ' ' | sed 's/ $//')) and uses no fat LTO"
}

self_test() {
  local sandbox failures=0 cases=0 r
  sandbox=$(mktemp -d "${TMPDIR:-/tmp}/docker-build-selftest.XXXXXX")
  # shellcheck disable=SC2064 # expand now: the path is fixed for this run
  trap "rm -rf '$sandbox'" EXIT
  r="$sandbox/root"

  # A scratch root whose Dockerfile satisfies both rules.
  clean_root() {
    rm -rf "$r"
    mkdir -p "$r"
    cat > "$r/Dockerfile" <<'EOF'
# syntax=docker/dockerfile:1.4
FROM rust:1.92-bookworm AS builder
WORKDIR /build
ENV CARGO_PROFILE_RELEASE_LTO=thin
ENV CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1
RUN printf 'ready\n' && \
    cargo build --release --locked --package biorouter-cli --package biorouter-server \
      --bin biorouter --bin biorouterd && \
    touch /build/rust.ready

FROM debian:bookworm-slim
COPY --from=builder /build/target/release/biorouter /usr/local/bin/biorouter
COPY --from=builder /build/target/release/biorouterd /usr/local/bin/biorouterd
EOF
  }

  expect() { # expect <pass|fail> <case name> <words the output must contain>
    local want=$1 name=$2 needle=$3 output code good=0
    cases=$((cases + 1))
    set +e
    output=$(bash "$SCRIPT_PATH" --root "$r" 2>&1)
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

  # rewrite <sed-free awk program>: apply an awk filter to the scratch Dockerfile.
  rewrite() {
    awk "$1" "$r/Dockerfile" > "$r/Dockerfile.new"
    mv "$r/Dockerfile.new" "$r/Dockerfile"
  }

  clean_root
  expect pass "the shipped binaries and thin LTO pass" "links only the shipped binaries (biorouter biorouterd)"

  clean_root
  rewrite '{ sub(/ --bin biorouter --bin biorouterd/, ""); print }'
  expect fail "a cargo build with no --bin fails rule 1" "rule 1: the builder's cargo build names no --bin"

  clean_root
  rewrite '{ sub(/--bin biorouterd/, "--bin biorouterd --bin generate_schema"); print }'
  expect fail "an extra binary the image does not ship fails rule 1" "links [biorouter biorouterd generate_schema]"

  clean_root
  rewrite '{ sub(/--bin biorouter --bin biorouterd/, "--bins"); print }'
  expect fail "--bins fails rule 1" "links [<all-bins>]"

  clean_root
  rewrite '/target\/release\/biorouterd/ { next } { print }'
  expect fail "a built binary that no stage copies fails rule 1" "but the image ships [biorouter]"

  clean_root
  rewrite '{ sub(/--bin biorouter --bin biorouterd/, "--bin=biorouter --bin=biorouterd"); print }'
  expect pass "--bin=NAME is read like --bin NAME" "links only the shipped binaries"

  clean_root
  rewrite '{ sub(/LTO=thin/, "LTO=true"); print }'
  expect fail "fat LTO as true fails rule 2" "rule 2: the builder stage sets CARGO_PROFILE_RELEASE_LTO=true"

  clean_root
  rewrite '{ sub(/LTO=thin/, "LTO=\"fat\""); print }'
  expect fail "fat LTO quoted fails rule 2" "rule 2: the builder stage sets CARGO_PROFILE_RELEASE_LTO=fat"

  clean_root
  rewrite '{ sub(/ENV CARGO_PROFILE_RELEASE_LTO=thin/, "ENV CARGO_PROFILE_RELEASE_LTO fat"); print }'
  expect fail "the ENV key value form fails rule 2" "CARGO_PROFILE_RELEASE_LTO=fat"

  clean_root
  rewrite '{ sub(/ENV CARGO_PROFILE_RELEASE_LTO=thin/, "# ENV CARGO_PROFILE_RELEASE_LTO=true"); print }'
  expect pass "fat LTO only in a comment passes" "uses no fat LTO"

  clean_root
  rewrite '{ sub(/AS builder/, "AS compile"); print }'
  expect fail "no builder stage fails" "has no 'FROM rust:... AS builder' stage"

  if [ "$failures" -ne 0 ]; then
    echo "::error::$failures of $cases self test case(s) did not behave as expected" >&2
    return 1
  fi
  echo "check-docker-build self test: $cases cases behaved as expected"
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
