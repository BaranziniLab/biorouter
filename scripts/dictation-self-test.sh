#!/usr/bin/env bash
# Run the local dictation engine's offline self test in shipped binaries (DD-12).
#
# Usage:
#   scripts/dictation-self-test.sh [--fixtures DIR] [--prefix "WORDS"] [--expect-backend metal|cpu]
#                                  [--expect-cpu-refusal] [--cpu] [--timeout SECONDS] BINARY...
#
# For each BINARY it runs
#   [PREFIX] BINARY __br-dictation-worker self-test --model DIR/models/for-tests-ggml-base.en.bin
#       --vad DIR/models/for-tests-silero-v6.2.0-ggml.bin --wav DIR/samples/jfk.wav [--cpu]
# and requires exit 0 and exactly one line on stdout, containing "selftest":"ok" (and
# "backend":"<b>" with --expect-backend). With --expect-cpu-refusal it requires instead exit 20,
# stderr beginning with the MSG-CPU sentence, and the "stage":"cpu" failure line on stdout: the
# answer an old processor must get in place of a crash.
#
# The self test loads whisper.cpp's vocabulary-only test model and the voice detector from the
# vendored tree, runs the detector on jfk.wav and decodes it, all offline in about a second. It
# is how every package smoke proves the engine in that package loads and runs on its target.
#
#   --fixtures DIR   the vendored whisper.cpp source folder (default <repo>/vendor/whisper.cpp/source).
#                    When DIR/../pin.json exists, the line's "whisper" field must match its commit.
#   --prefix WORDS   split on spaces and put before the binary, for example
#                    "env ROSETTA_ADVERTISE_AVX=1 arch -x86_64" or "qemu-x86_64 -cpu Nehalem".
#   --cpu            pass --cpu: force the CPU backend.
#   --timeout SECS   kill a run that takes longer (default 300; emulated runs are slow).
#
# Success lines go to stdout and every failure detail to stderr, so a caller that pipes stdout
# into `grep -q` still shows why a run failed. Works in bash 3.2 (macOS), Linux, and Git Bash on
# Windows.
set -euo pipefail

MSG_CPU_PREFIX='This processor does not support the instructions local dictation needs ('
WORKER_MARKER='__br-dictation-worker'

usage() {
  echo "usage: $0 [--fixtures DIR] [--prefix \"WORDS\"] [--expect-backend metal|cpu] [--expect-cpu-refusal] [--cpu] [--timeout SECONDS] BINARY..." >&2
  exit 2
}

# The repository root, in the form the binaries under test can open: on Git Bash `pwd -W`
# prints D:/a/... (a native program does not understand /d/a/...); elsewhere it is not a
# valid option and plain pwd answers.
repo_root() {
  local here
  here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
  (cd "$here" && pwd -W 2>/dev/null) || printf '%s\n' "$here"
}

FIXTURES=""
PREFIX_WORDS=""
EXPECT_BACKEND=""
EXPECT_REFUSAL=0
CPU_FLAG=0
TIMEOUT_SECS=300
BINARIES=()
while [ "$#" -gt 0 ]; do
  case "$1" in
    --fixtures) [ "$#" -ge 2 ] || usage; FIXTURES="$2"; shift 2 ;;
    --prefix) [ "$#" -ge 2 ] || usage; PREFIX_WORDS="$2"; shift 2 ;;
    --expect-backend)
      [ "$#" -ge 2 ] || usage
      case "$2" in metal | cpu) EXPECT_BACKEND="$2" ;; *) usage ;; esac
      shift 2
      ;;
    --expect-cpu-refusal) EXPECT_REFUSAL=1; shift ;;
    --cpu) CPU_FLAG=1; shift ;;
    --timeout)
      [ "$#" -ge 2 ] || usage
      case "$2" in '' | *[!0-9]*) usage ;; esac
      TIMEOUT_SECS="$2"; shift 2
      ;;
    --) shift; while [ "$#" -gt 0 ]; do BINARIES+=("$1"); shift; done ;;
    -*) usage ;;
    *) BINARIES+=("$1"); shift ;;
  esac
done
[ "${#BINARIES[@]}" -gt 0 ] || usage
if [ "$EXPECT_REFUSAL" -eq 1 ] && [ -n "$EXPECT_BACKEND" ]; then
  echo "--expect-cpu-refusal and --expect-backend cannot both hold" >&2
  exit 2
fi
[ -n "$FIXTURES" ] || FIXTURES="$(repo_root)/vendor/whisper.cpp/source"

MODEL="$FIXTURES/models/for-tests-ggml-base.en.bin"
VAD="$FIXTURES/models/for-tests-silero-v6.2.0-ggml.bin"
WAV="$FIXTURES/samples/jfk.wav"
for fixture in "$MODEL" "$VAD" "$WAV"; do
  if [ ! -f "$fixture" ]; then
    echo "::error::the self test fixture $fixture is missing (is --fixtures the vendored whisper.cpp source folder?)" >&2
    exit 2
  fi
done

# The engine pin the binaries must report, when the pin file sits beside the fixtures.
WANT_WHISPER=""
if [ -f "$FIXTURES/../pin.json" ]; then
  WANT_WHISPER="$(sed -n 's/.*"commit"[[:space:]]*:[[:space:]]*"\([0-9a-f]\{8\}\)[0-9a-f]*".*/\1/p' "$FIXTURES/../pin.json" | head -n 1)"
fi

PREFIX=()
if [ -n "$PREFIX_WORDS" ]; then
  read -r -a PREFIX <<<"$PREFIX_WORDS"
fi
EXTRA=()
[ "$CPU_FLAG" -eq 0 ] || EXTRA+=(--cpu)

SCRATCH="$(mktemp -d "${TMPDIR:-/tmp}/dictation-self-test.XXXXXX")"
# shellcheck disable=SC2064 # expand now: the path is fixed for this run
trap "rm -rf '$SCRATCH'" EXIT

# run_bounded <seconds> <stdout file> <stderr file> <command...>: run the command and return
# its exit status, or kill it after <seconds> and return 124. The watchdog's own output goes
# to /dev/null, so a caller reading this script's stdout through a pipe is never held open
# by a sleeping watchdog.
run_bounded() {
  local secs="$1" out="$2" err="$3" pid dog rc=0
  shift 3
  "$@" >"$out" 2>"$err" </dev/null &
  pid=$!
  (
    s=""
    trap 'if [ -n "$s" ]; then kill "$s" 2>/dev/null; fi; exit 0' TERM
    sleep "$secs" &
    s=$!
    wait "$s" || true
    kill -9 "$pid" 2>/dev/null || true
    touch "$out.timed-out"
  ) >/dev/null 2>&1 </dev/null &
  dog=$!
  wait "$pid" || rc=$?
  kill "$dog" 2>/dev/null || true
  wait "$dog" 2>/dev/null || true
  if [ -e "$out.timed-out" ]; then
    return 124
  fi
  return "$rc"
}

show_failure() { # show_failure <binary> <reason> <exit> <stdout file> <stderr file>
  {
    echo "::error::dictation self test of $1: $2"
    echo "  command: ${PREFIX[*]+${PREFIX[*]} }$1 $WORKER_MARKER self-test --model $MODEL --vad $VAD --wav $WAV${EXTRA[*]+ ${EXTRA[*]}}"
    echo "  exit status: $3"
    echo "  stdout:"
    sed 's/^/    /' "$4"
    echo "  stderr (last 40 lines):"
    tail -n 40 "$5" | sed 's/^/    /'
  } >&2
}

failures=0
index=0
for bin in "${BINARIES[@]}"; do
  index=$((index + 1))
  out="$SCRATCH/$index.out"
  err="$SCRATCH/$index.err"
  if [ ! -f "$bin" ]; then
    echo "::error::dictation self test: $bin does not exist" >&2
    failures=$((failures + 1))
    continue
  fi
  rc=0
  run_bounded "$TIMEOUT_SECS" "$out.raw" "$err.raw" \
    ${PREFIX[@]+"${PREFIX[@]}"} "$bin" "$WORKER_MARKER" self-test \
    --model "$MODEL" --vad "$VAD" --wav "$WAV" ${EXTRA[@]+"${EXTRA[@]}"} || rc=$?
  if [ ! -f "$out.raw" ] || [ ! -f "$err.raw" ]; then
    scratch_present=0
    stdout_present=0
    stderr_present=0
    [ ! -d "$SCRATCH" ] || scratch_present=1
    [ ! -f "$out.raw" ] || stdout_present=1
    [ ! -f "$err.raw" ] || stderr_present=1
    echo "::error::dictation self test output missing after binary $index: scratch=$scratch_present stdout=$stdout_present stderr=$stderr_present child_exit=$rc" >&2
    exit 1
  fi
  tr -d '\r' <"$out.raw" >"$out"
  tr -d '\r' <"$err.raw" >"$err"
  lines="$(grep -c . "$out" || true)"
  line="$(grep -m 1 . "$out" || true)"

  if [ "$rc" -eq 124 ]; then
    show_failure "$bin" "no answer within $TIMEOUT_SECS s" "$rc" "$out" "$err"
    failures=$((failures + 1))
    continue
  fi

  if [ "$EXPECT_REFUSAL" -eq 1 ]; then
    first_err="$(grep -m 1 . "$err" || true)"
    if [ "$rc" -ne 20 ]; then
      show_failure "$bin" "expected exit 20 (unsupported processor), got $rc" "$rc" "$out" "$err"
      failures=$((failures + 1))
    elif [ "${first_err#"$MSG_CPU_PREFIX"}" = "$first_err" ]; then
      show_failure "$bin" "exit 20, but stderr does not begin with the MSG-CPU sentence" "$rc" "$out" "$err"
      failures=$((failures + 1))
    elif [ "$lines" -ne 1 ] || [[ "$line" != *'"selftest":"failed"'* ]] || [[ "$line" != *'"stage":"cpu"'* ]]; then
      show_failure "$bin" "exit 20, but stdout is not one \"selftest\":\"failed\" line with \"stage\":\"cpu\"" "$rc" "$out" "$err"
      failures=$((failures + 1))
    else
      echo "refused as expected: $bin: $first_err"
    fi
    continue
  fi

  if [ "$rc" -ne 0 ]; then
    show_failure "$bin" "expected exit 0, got $rc" "$rc" "$out" "$err"
    failures=$((failures + 1))
  elif [ "$lines" -ne 1 ]; then
    show_failure "$bin" "expected exactly one line on stdout, got $lines" "$rc" "$out" "$err"
    failures=$((failures + 1))
  elif [[ "$line" != *'"selftest":"ok"'* ]]; then
    show_failure "$bin" "the stdout line does not contain \"selftest\":\"ok\"" "$rc" "$out" "$err"
    failures=$((failures + 1))
  elif [ -n "$EXPECT_BACKEND" ] && [[ "$line" != *"\"backend\":\"$EXPECT_BACKEND\""* ]]; then
    show_failure "$bin" "expected \"backend\":\"$EXPECT_BACKEND\"" "$rc" "$out" "$err"
    failures=$((failures + 1))
  elif [ -n "$WANT_WHISPER" ] && [[ "$line" != *"\"whisper\":\"$WANT_WHISPER\""* ]]; then
    show_failure "$bin" "expected \"whisper\":\"$WANT_WHISPER\", the commit in $FIXTURES/../pin.json" "$rc" "$out" "$err"
    failures=$((failures + 1))
  else
    echo "ok: $bin: $line"
  fi
done

if [ "$failures" -ne 0 ]; then
  echo "::error::$failures of ${#BINARIES[@]} dictation self test(s) failed" >&2
  exit 1
fi
