#!/usr/bin/env bash
# The local dictation test fixtures stay inside a fixed size budget (DD-95).
#
# Usage:
#   scripts/check-dictation-fixtures.sh              check (CI `guards`, `just check-everything`)
#   scripts/check-dictation-fixtures.sh --self-test  prove the check fails when it must
#
# Four of whisper.cpp's own test files are committed inside the vendored tree (two
# vocabulary-only models, the voice detector and a speech sample, 2,399,463 B in all), and
# every later fixture must fit beside them. Real speech models are hundreds of megabytes
# and live in release assets; a test that wants one is #[ignore]d and downloads it. This
# guard is what stops a convenient copy of one landing in the repository instead.
#
# It sums every file git would commit (tracked, plus untracked and not ignored) under:
#   vendor/whisper.cpp/source/models/
#   vendor/whisper.cpp/source/samples/
#   crates/biorouter-dictation/tests/fixtures/
# and counts *.wav, *.flac, *.mp3, *.m4a and *.ogg as audio. The limits:
#   all fixtures   2.75 MiB  (2,883,584 B)
#   one file       1 MiB     (1,048,576 B)
#   audio          0.75 MiB  (786,432 B)
#
# Plain bash 3.2 (macOS's /bin/bash) and git; nothing else.
set -euo pipefail
cd "$(dirname "$0")/.."

TOTAL_MAX=2883584
FILE_MAX=1048576
AUDIO_MAX=786432
FIXTURE_DIRS="vendor/whisper.cpp/source/models vendor/whisper.cpp/source/samples crates/biorouter-dictation/tests/fixtures"
REAL_FIXTURES="vendor/whisper.cpp/source/models/for-tests-ggml-base.en.bin vendor/whisper.cpp/source/models/for-tests-ggml-tiny.bin vendor/whisper.cpp/source/models/for-tests-silero-v6.2.0-ggml.bin vendor/whisper.cpp/source/samples/jfk.wav"
REAL_TOTAL=2399463

is_audio() {
  local lower
  lower=$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')
  case "$lower" in
    *.wav | *.flac | *.mp3 | *.m4a | *.ogg) return 0 ;;
    *) return 1 ;;
  esac
}

check() {
  if [ "$(git rev-parse --is-inside-work-tree 2>/dev/null)" != "true" ]; then
    echo "::error::$(pwd) is not a git work tree; the fixture budget reads what git would commit" >&2
    return 1
  fi
  local total=0 audio=0 count=0 largest=0 largest_path="" fail=0 size path
  # shellcheck disable=SC2086 # FIXTURE_DIRS is a space separated list of fixed paths
  while IFS= read -r -d '' path; do
    [ -f "$path" ] || continue
    size=$(wc -c < "$path" | tr -d '[:space:]')
    count=$((count + 1))
    total=$((total + size))
    if [ "$size" -gt "$largest" ]; then
      largest=$size
      largest_path=$path
    fi
    if is_audio "$path"; then
      audio=$((audio + size))
    fi
    if [ "$size" -gt "$FILE_MAX" ]; then
      echo "::error::$path is $size B, over the 1 MiB ($FILE_MAX B) limit for one fixture file" >&2
      fail=1
    fi
  done < <(git ls-files -z --cached --others --exclude-standard -- $FIXTURE_DIRS | sort -zu)

  if [ "$total" -gt "$TOTAL_MAX" ]; then
    echo "::error::the dictation fixtures come to $total B in all, over the 2.75 MiB ($TOTAL_MAX B) budget" >&2
    fail=1
  fi
  if [ "$audio" -gt "$AUDIO_MAX" ]; then
    echo "::error::the dictation fixtures hold $audio B of audio, over the 0.75 MiB ($AUDIO_MAX B) budget" >&2
    fail=1
  fi
  if [ "$fail" -ne 0 ]; then
    echo "A fixture this large belongs in a release asset that an #[ignore]d test downloads, or in a signal the test generates." >&2
    return 1
  fi
  echo "dictation fixtures: $count files, $total B of $TOTAL_MAX B; audio $audio B of $AUDIO_MAX B; largest $largest B of $FILE_MAX B${largest_path:+ ($largest_path)}"
}

self_test() {
  local here sandbox repo failures=0 cases=0
  here=$(pwd)
  sandbox=$(mktemp -d "${TMPDIR:-/tmp}/dictation-fixtures-selftest.XXXXXX")
  # shellcheck disable=SC2064 # expand now: the path is fixed for this run
  trap "rm -rf '$sandbox'" EXIT
  repo="$sandbox/repo"
  # Hermetic git: no system or personal config and no personal ignore file.
  while IFS= read -r name; do unset "$name"; done < <(env | sed -n 's/^\(GIT_[A-Za-z_]*\)=.*/\1/p')
  export HOME="$sandbox/home" XDG_CONFIG_HOME="$sandbox/xdg" GIT_CONFIG_NOSYSTEM=1

  local fixture
  for fixture in $REAL_FIXTURES; do
    if [ ! -f "$here/$fixture" ]; then
      echo "::error::self test needs the real fixture $fixture" >&2
      return 1
    fi
  done

  fresh() {
    rm -rf "$repo"
    mkdir -p "$repo/scripts"
    cp "$here/scripts/check-dictation-fixtures.sh" "$repo/scripts/"
    git init -q "$repo"
    for fixture in $REAL_FIXTURES; do
      mkdir -p "$repo/$(dirname "$fixture")"
      cp "$here/$fixture" "$repo/$fixture"
    done
  }

  dummy() { # dummy <path under the scratch repo> <bytes>
    mkdir -p "$repo/$(dirname "$1")"
    head -c "$2" /dev/zero > "$repo/$1"
  }

  expect() { # expect <pass|fail> <case name> <words the output must contain>
    local want=$1 name=$2 needle=$3 output code good=0
    cases=$((cases + 1))
    set +e
    output=$(bash "$repo/scripts/check-dictation-fixtures.sh" 2>&1)
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

  fresh
  expect pass "the real four fixtures pass at $REAL_TOTAL B" "dictation fixtures: 4 files, $REAL_TOTAL B"

  fresh
  dummy vendor/whisper.cpp/source/models/for-tests-dummy.bin 1153434
  expect fail "a 1.1 MiB dummy fixture fails" "over the 1 MiB"

  fresh
  dummy crates/biorouter-dictation/tests/fixtures/too-big.bin 1153434
  expect fail "a 1.1 MiB dummy in the crate's fixtures fails" "over the 1 MiB"

  fresh
  dummy vendor/whisper.cpp/source/samples/extra.WAV 500000
  expect fail "audio over 0.75 MiB fails (extension case ignored)" "of audio, over"

  fresh
  dummy crates/biorouter-dictation/tests/fixtures/a.bin 300000
  dummy crates/biorouter-dictation/tests/fixtures/b.bin 300000
  expect fail "fixtures over 2.75 MiB in all fail" "in all, over"

  fresh
  dummy crates/biorouter-dictation/tests/fixtures/fits.bin 400000
  expect pass "a fixture that fits the budget passes" "dictation fixtures: 5 files"

  fresh
  printf '*.local\n' > "$repo/.gitignore"
  dummy vendor/whisper.cpp/source/models/scratch.local 2000000
  expect pass "an ignored file is not a fixture" "dictation fixtures: 4 files"

  if [ "$failures" -ne 0 ]; then
    echo "::error::$failures of $cases self test case(s) did not behave as expected" >&2
    return 1
  fi
  echo "check-dictation-fixtures self test: $cases cases behaved as expected"
}

case "${1:-}" in
  "") check ;;
  --self-test) self_test ;;
  *)
    echo "usage: $0 [--self-test]" >&2
    exit 2
    ;;
esac
