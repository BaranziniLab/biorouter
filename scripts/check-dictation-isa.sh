#!/usr/bin/env bash
# Old processors get a sentence, not a crash (DD-12, DD-10 rule 2).
#
# Usage:
#   scripts/check-dictation-isa.sh [--no-status] [BIN_DIR]   check (rust.yml cross-build-nightly)
#   scripts/check-dictation-isa.sh --self-test               prove each step can fail
#
# BIN_DIR (default target/x86_64-unknown-linux-gnu/release) holds the cross built Linux
# `biorouter` and `biorouterd`. The engine's ggml code is compiled for x86-64-v3 (AVX2, FMA,
# F16C, BMI2), and a processor without those instructions must still start Biorouter and be
# told, in a sentence, that local dictation cannot run on it. A compiler that vectorizes one
# static initializer would instead kill `--version` on such a processor before any check of
# ours runs. In a `debian:bookworm-slim` amd64 container, this script:
#   1. runs `--version` of both binaries natively, and the dictation self test natively, as the
#      positive control that proves the binaries and fixtures are good;
#   2. runs `--version` of both under `qemu-x86_64 -cpu Nehalem` (SSE4.2, no AVX): exit 0;
#   3. runs the self test under Nehalem: exit 20 with MSG-CPU on stderr;
#   4. unless --no-status, runs `biorouter dictation status --format json` under Nehalem and
#      requires engine.state "unsupported_cpu" with "avx2" among the missing features (PR-10).
#
# The container gets the packages the shipped deb declares (from
# scripts/check-linux-runtime-deps.sh --print-deb-packages), so the binaries load as they do
# for a user, plus qemu-user. ROSETTA_ADVERTISE_AVX=1 is passed so the native step also works
# under Docker Desktop's amd64 emulation on an Apple Silicon Mac; an x86 host ignores it.
#
# ⚠ Bookworm, not bullseye, because the emulator is the instrument. bullseye's qemu-user 5.2
# runs the VEX encodings of SSE operations under `-cpu Nehalem` instead of refusing them:
# measured 2026-11-05, `vpaddd`, `vmovdqu` and `vxorps` on ymm registers all exit 0 there, and
# only instructions with no SSE twin (vpbroadcastd, vfmadd231ps, vcvtph2ps, shlx) give SIGILL.
# A vectorized initializer is usually made of the first kind, so step 2 could not fail on it.
# bookworm's qemu-user 7.2 refuses all of them and still runs SSE4.2. The glibc floor is not
# this script's job: rust.yml boots the same binaries, self test included, on debian:bullseye.
#
# --self-test compiles stand-in programs in the same container that answer the three kinds of
# invocation the way a correct build would, and four broken ones (an AVX2 static initializer;
# a self test that skips the processor check; a status that ignores the processor), and
# requires the check to pass, then to fail at step 2, 3 and 4. They are not Biorouter.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
IMAGE="debian:bookworm-slim"
NEHALEM="qemu-x86_64 -cpu Nehalem"

usage() {
  echo "usage: $0 [--no-status] [BIN_DIR] | --self-test" >&2
  exit 2
}

# ── Inside the container ──────────────────────────────────────────────────────

# prepare_container <packages...>: install packages in the container, with one retry for a
# mirror that answered badly (the image's own lists; bookworm's archives are all served).
prepare_container() {
  printf 'Acquire::Retries "5";\n' >/etc/apt/apt.conf.d/99retries
  if apt-get update -q >/dev/null &&
    DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends "$@" >/dev/null; then
    return 0
  fi
  echo "check-dictation-isa: the package install failed; retrying once"
  sleep 10
  apt-get update -q >/dev/null
  DEBIAN_FRONTEND=noninteractive apt-get install -y -q --no-install-recommends "$@" >/dev/null
}

step_failed() { # step_failed <n> <sentence>
  echo "::error::check-dictation-isa step $1: $2"
  return 1
}

# run_checks <bin dir> <1 to check status, 0 to skip it>: steps 1 to 4, stopping at the first
# failure. Every result is checked explicitly, because callers run this in a condition, where
# bash ignores errexit.
run_checks() {
  local bin="$1" status="$2" fixtures=/w/vendor/whisper.cpp/source b rc home
  for b in biorouter biorouterd; do
    [ -x "$bin/$b" ] || { step_failed 1 "$bin/$b is missing or not executable"; return 1; }
  done

  echo "== step 1: native positive control"
  for b in biorouter biorouterd; do
    rc=0
    "$bin/$b" --version || rc=$?
    [ "$rc" -eq 0 ] || { step_failed 1 "$b --version exited $rc on this machine's own processor"; return 1; }
  done
  bash /w/scripts/dictation-self-test.sh --fixtures "$fixtures" "$bin/biorouter" "$bin/biorouterd" \
    || { step_failed 1 "the dictation self test fails on this machine's own processor, so the steps below would prove nothing"; return 1; }

  echo "== step 2: --version under $NEHALEM"
  for b in biorouter biorouterd; do
    rc=0
    # shellcheck disable=SC2086 # NEHALEM is a command and its arguments
    $NEHALEM "$bin/$b" --version || rc=$?
    if [ "$rc" -ne 0 ]; then
      step_failed 2 "$b --version exited $rc on a processor without AVX$([ "$rc" -eq 132 ] && echo ' (SIGILL)'). Code that runs before main, such as a static initializer, uses an instruction an old processor lacks; every user of such a processor would lose all of Biorouter, not only dictation (DD-12)"
      return 1
    fi
  done

  echo "== step 3: the self test under $NEHALEM answers with MSG-CPU"
  bash /w/scripts/dictation-self-test.sh --fixtures "$fixtures" --expect-cpu-refusal --prefix "$NEHALEM" \
    "$bin/biorouter" "$bin/biorouterd" \
    || { step_failed 3 "the self test on a processor without AVX2 did not exit 20 with the MSG-CPU sentence (DD-12)"; return 1; }

  if [ "$status" -eq 1 ]; then
    echo "== step 4: dictation status under $NEHALEM reports unsupported_cpu"
    home="$(mktemp -d)"
    rc=0
    # shellcheck disable=SC2086
    HOME="$home" $NEHALEM "$bin/biorouter" dictation status --format json >/tmp/dictation-status.json || rc=$?
    [ "$rc" -eq 0 ] || { step_failed 4 "biorouter dictation status --format json exited $rc under Nehalem"; return 1; }
    python3 -I /w/scripts/check-dictation-status-json.py --expect-state unsupported_cpu \
      --expect-missing-feature avx2 /tmp/dictation-status.json \
      || { step_failed 4 "dictation status under Nehalem does not report unsupported_cpu with avx2 missing"; return 1; }
  fi
  echo "check-dictation-isa: both binaries start on a processor without AVX and refuse dictation with a sentence"
}

inside() { # inside <bin dir> <status>
  # DEB_PACKAGES is a space separated list; word splitting is the point.
  # shellcheck disable=SC2086
  if [ "$2" -eq 1 ]; then
    prepare_container qemu-user python3 ${DEB_PACKAGES:-}
  else
    prepare_container qemu-user ${DEB_PACKAGES:-}
  fi
  run_checks "$1" "$2"
}

# The stand-in programs for the self test, one C source with switches.
write_fixture_source() {
  cat >"$1" <<'EOF'
#include <cpuid.h>
#include <stdio.h>
#include <string.h>

#ifdef AVX2_CTOR
/* What a compiler that vectorizes a static initializer produces: AVX2 before main. */
__attribute__((constructor)) static void vectorized_initializer(void) {
  __asm__ volatile("vpaddd %%ymm0, %%ymm0, %%ymm0" ::: "xmm0");
}
#endif

static const char *ids[4] = {"avx2", "fma", "f16c", "bmi2"};
static const char *names[4] = {"AVX2", "FMA", "F16C", "BMI2"};

static int missing_features(int out[4]) {
  unsigned a, b, c, d;
  int have[4] = {0, 0, 0, 0}, n = 0, i;
  if (__get_cpuid(1, &a, &b, &c, &d)) { have[1] = (c >> 12) & 1; have[2] = (c >> 29) & 1; }
  if (__get_cpuid_count(7, 0, &a, &b, &c, &d)) { have[0] = (b >> 5) & 1; have[3] = (b >> 8) & 1; }
  for (i = 0; i < 4; i++) if (!have[i]) out[n++] = i;
  return n;
}

static void print_ids(const int *m, int n) {
  int i;
  for (i = 0; i < n; i++) printf("%s\"%s\"", i ? "," : "", ids[m[i]]);
}

int main(int argc, char **argv) {
  int m[4], n, i;
  if (argc == 2 && strcmp(argv[1], "--version") == 0) {
    puts("biorouter 0.0.0 (check-dictation-isa stand-in, not Biorouter)");
    return 0;
  }
  if (argc >= 3 && strcmp(argv[1], "__br-dictation-worker") == 0 && strcmp(argv[2], "self-test") == 0) {
    n = missing_features(m);
#ifdef NO_GATE
    n = 0;
    __asm__ volatile("vpaddd %%ymm0, %%ymm0, %%ymm0" ::: "xmm0");
#endif
    if (n) {
      fputs("This processor does not support the instructions local dictation needs (", stderr);
      for (i = 0; i < n; i++)
        fprintf(stderr, "%s%s", i == 0 ? "" : (i == n - 1 ? " and " : ", "), names[m[i]]);
      fputs("). Your system's own dictation still works, without your Biorouter dictionary.\n", stderr);
      printf("{\"selftest\":\"failed\",\"stage\":\"cpu\",\"exit\":20,\"missing\":[");
      print_ids(m, n);
      puts("]}");
      return 20;
    }
    printf("{\"selftest\":\"ok\",\"whisper\":\"%s\",\"ggml\":\"0.28.0\",\"isa\":\"x86-64-v3\","
           "\"backend\":\"cpu\",\"elapsed_ms\":1}\n", PIN);
    return 0;
  }
  if (argc == 5 && strcmp(argv[1], "dictation") == 0 && strcmp(argv[2], "status") == 0 &&
      strcmp(argv[3], "--format") == 0 && strcmp(argv[4], "json") == 0) {
    n = missing_features(m);
#ifdef STATUS_IGNORES_CPU
    n = 0;
#endif
    printf("{\"schema\":1,\"engine\":{\"compiled\":true,\"state\":\"%s\",\"backend\":%s,"
           "\"device\":null,\"threads\":4,\"missing_cpu_features\":[",
           n ? "unsupported_cpu" : "stopped", n ? "null" : "\"cpu\"");
    print_ids(m, n);
    printf("],\"rss_bytes\":null,\"build\":{\"whisper\":\"%s\",\"ggml\":\"0.28.0\",\"isa\":\"x86-64-v3\"}},"
           "\"models\":[],\"active_model\":null,\"language\":\"en\",\"policy\":{\"managed\":false,"
           "\"enabled\":{\"allowed\":true,\"reason\":null},\"capture\":{\"allowed\":true,\"reason\":null},"
           "\"install\":{\"allowed\":true,\"reason\":null},\"external_engine\":{\"allowed\":true,\"reason\":null},"
           "\"dictionary\":{\"allowed\":true,\"reason\":null},\"agent_writes\":{\"allowed\":true,\"reason\":null},"
           "\"allowed_models\":null},\"capture\":{\"available\":false,\"reason\":\"not_compiled\"},"
           "\"account\":\"fixture\",\"host\":\"fixture\",\"models_dir\":\"/tmp/models\"}\n", PIN);
    return 0;
  }
  return 2;
}
EOF
}

self_test_inside() {
  local pin src out variant rc failures=0 cases=0 log
  prepare_container gcc libc6-dev qemu-user python3
  pin="$(sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([0-9a-f]\{8\}\)[0-9a-f]*".*/\1/p' /w/vendor/whisper.cpp/pin.json | head -n 1)"
  [ -n "$pin" ] || { echo "::error::cannot read the engine pin from vendor/whisper.cpp/pin.json"; return 1; }
  src=/tmp/isa-fixture.c
  write_fixture_source "$src"
  for variant in good AVX2_CTOR NO_GATE STATUS_IGNORES_CPU; do
    out="/tmp/isa-fixtures/$variant"
    mkdir -p "$out"
    if [ "$variant" = good ]; then
      gcc -O1 -DPIN="\"$pin\"" -o "$out/biorouter" "$src"
    else
      gcc -O1 -DPIN="\"$pin\"" -D"$variant" -o "$out/biorouter" "$src"
    fi
    cp "$out/biorouter" "$out/biorouterd"
  done

  expect() { # expect <variant> <pass | step N> <case name>
    local variant="$1" want="$2" name="$3"
    cases=$((cases + 1))
    log="/tmp/isa-fixtures/$variant.log"
    rc=0
    run_checks "/tmp/isa-fixtures/$variant" 1 >"$log" 2>&1 || rc=$?
    if { [ "$want" = pass ] && [ "$rc" -eq 0 ]; } \
      || { [ "$want" != pass ] && [ "$rc" -ne 0 ] && grep -q "check-dictation-isa $want:" "$log"; }; then
      echo "ok   $name"
    else
      echo "FAIL $name (exit $rc)"
      sed 's/^/     /' "$log"
      failures=$((failures + 1))
    fi
  }
  expect good pass "a build that checks the processor passes every step"
  expect AVX2_CTOR "step 2" "an AVX2 static initializer fails step 2 (--version under Nehalem)"
  expect NO_GATE "step 3" "a self test that skips the processor check fails step 3"
  expect STATUS_IGNORES_CPU "step 4" "a status that ignores the processor fails step 4"
  if [ "$failures" -ne 0 ]; then
    echo "::error::$failures of $cases check-dictation-isa self test case(s) did not behave as expected"
    return 1
  fi
  echo "check-dictation-isa self test: $cases cases behaved as expected"
}

# ── On the host ───────────────────────────────────────────────────────────────

need_docker() {
  command -v docker >/dev/null 2>&1 || { echo "::error::check-dictation-isa needs docker" >&2; exit 2; }
  docker info >/dev/null 2>&1 || { echo "::error::the docker daemon is not reachable" >&2; exit 2; }
}

host_check() { # host_check <bin dir> <status>
  local bin_dir="$1" status="$2" deb_packages
  need_docker
  for b in biorouter biorouterd; do
    [ -f "$bin_dir/$b" ] || { echo "::error::missing binary $bin_dir/$b; run the cross build first" >&2; exit 2; }
  done
  bin_dir="$(cd "$bin_dir" && pwd)"
  deb_packages="$("$SCRIPT_DIR/check-linux-runtime-deps.sh" --print-deb-packages "$bin_dir")"
  docker run --rm --platform linux/amd64 -e DEB_PACKAGES="$deb_packages" -e ROSETTA_ADVERTISE_AVX=1 \
    -v "$ROOT":/w:ro -v "$bin_dir":/under-test:ro "$IMAGE" \
    bash /w/scripts/check-dictation-isa.sh --inside /under-test "$status"
}

host_self_test() {
  need_docker
  docker run --rm --platform linux/amd64 -e ROSETTA_ADVERTISE_AVX=1 -v "$ROOT":/w:ro "$IMAGE" \
    bash /w/scripts/check-dictation-isa.sh --self-test-inside
}

STATUS=1
BIN_DIR=""
case "${1:-}" in
  --inside)
    [ "$#" -eq 3 ] || usage
    inside "$2" "$3"
    exit $?
    ;;
  --self-test-inside)
    [ "$#" -eq 1 ] || usage
    self_test_inside
    exit $?
    ;;
  --self-test)
    [ "$#" -eq 1 ] || usage
    host_self_test
    exit $?
    ;;
esac
while [ "$#" -gt 0 ]; do
  case "$1" in
    --no-status) STATUS=0; shift ;;
    -*) usage ;;
    *) [ -z "$BIN_DIR" ] || usage; BIN_DIR="$1"; shift ;;
  esac
done
cd "$ROOT"
host_check "${BIN_DIR:-target/x86_64-unknown-linux-gnu/release}" "$STATUS"
