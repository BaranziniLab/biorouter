#!/usr/bin/env bash
# The macOS backends declare macOS 12.0 as their minimum (DD-11 in
# docs/history/local-dictation-2026-11/implementation-plan.md).
#
# Usage:
#   scripts/check-macos-deployment.sh [--require-imports] BINARY...
#   scripts/check-macos-deployment.sh --self-test
#
# For every architecture slice of every BINARY:
#
#   1. The minimum macOS must be exactly 12.0. It is read with `vtool -show-build`
#      (`otool -l` when vtool is missing), from LC_BUILD_VERSION's `minos` or, for
#      an older x86_64 link, LC_VERSION_MIN_MACOSX's `version`. Older means the
#      floor in .cargo/config.toml did not reach the link (rustc's own default is
#      11.0 on arm64 and 10.12 on x86_64); newer means dyld refuses the binary on a
#      macOS the app still supports.
#
#   2. No symbol that needs a macOS newer than 12.0 is imported strongly. A
#      strong import of such a symbol makes dyld refuse to start the whole binary
#      on older macOS, before any of its code runs. The check knows two kinds of
#      symbol:
#        - the pinned table below, for symbols measured in the engine spike;
#        - any Objective-C class, whose minimum macOS is read from the
#          `API_AVAILABLE(macos(N))` annotation on its `@interface` in the
#          active SDK's framework headers.
#      No binary and no SDK stub records when a C function appeared, so a C
#      function outside the table is not caught here; the compiler's
#      -Wunguarded-availability warnings are the check for those.
#      Each slice's imports are read with `xcrun dyld_info -imports`, and with
#      `nm -m -u` when dyld_info cannot read the slice. dyld_info refuses some
#      binaries that rustc's `strip` touched ("mis-aligned LINKEDIT string pool";
#      whether it does depends on the string pool's size, so any `strip = ...`
#      profile, `release` included, can produce one), while nm still reads their
#      symbol table, where every import stays. nm prints a weak import as
#      `(undefined) weak external SYM (from LIB)` and a strong one without
#      `weak`. When neither tool reads a slice, the step is skipped for it with
#      a note, unless --require-imports is given, which makes an unreadable
#      import table a failure. CI and scripts/release.sh both pass it.
#      CHECK_MACOS_DEPLOYMENT_IMPORTS=dyld_info or =nm limits the check to one
#      reader (the self test uses it to exercise each one).
#
# Exit codes: 0 every binary passed; 1 a binary failed; 2 a usage error, a
# missing file or a missing tool.
#
# Plain bash 3.2 (macOS's /bin/bash) and the Xcode command line tools; nothing else.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SELF="$ROOT/scripts/check-macos-deployment.sh"

# The floor. .cargo/config.toml declares the same value; the self test fails when
# the two differ.
MACOS_FLOOR="12.0"

# Symbols measured to need a newer macOS than the floor. One rule per line:
#   <shell glob over the symbol name>|<macOS it needs>|<what it is>
PINNED_RULES='*$NEWLAPACK*|13.3|an Accelerate BLAS or LAPACK interface from ACCELERATE_NEW_LAPACK
_OBJC_CLASS_$_MTLResidencySet*|15.0|a Metal residency set class
_OBJC_METACLASS_$_MTLResidencySet*|15.0|a Metal residency set class'

err() { printf '::error::%s\n' "$*" >&2; }

# Prints -1, 0 or 1 as dotted version $1 is older than, equal to or newer than $2.
version_cmp() {
  local IFS=.
  # shellcheck disable=SC2206 # splitting on dots is the point
  local -a a=($1) b=($2)
  local i x y
  for i in 0 1 2; do
    x=${a[$i]:-0}
    y=${b[$i]:-0}
    if [ "$x" -gt "$y" ]; then echo 1; return; fi
    if [ "$x" -lt "$y" ]; then echo -1; return; fi
  done
  echo 0
}

is_version() {
  printf '%s' "$1" | grep -Eq '^[0-9]+(\.[0-9]+){0,2}$'
}

# Prints every macOS minimum the slice declares, one per line; fails when there is none.
min_os_of() { # <binary> <arch>
  local out
  # CHECK_MACOS_DEPLOYMENT_READER=otool lets the self test exercise the fallback.
  if [ "${CHECK_MACOS_DEPLOYMENT_READER:-vtool}" = vtool ] && command -v vtool >/dev/null 2>&1; then
    out=$(vtool -arch "$2" -show-build "$1" 2>/dev/null) || return 1
  else
    out=$(otool -arch "$2" -l "$1" 2>/dev/null) || return 1
  fi
  # vtool names the platform (MACOS), otool numbers it (1). A zippered binary also
  # carries a Mac Catalyst LC_BUILD_VERSION; only the macOS one is the floor.
  printf '%s\n' "$out" | awk '
    $1 == "cmd" { cmd = $2; plat = ""; next }
    cmd == "LC_BUILD_VERSION" && $1 == "platform" { plat = $2; next }
    cmd == "LC_BUILD_VERSION" && $1 == "minos" && (plat == "MACOS" || plat == "1") { print $2; found = 1; next }
    cmd == "LC_VERSION_MIN_MACOSX" && $1 == "version" { print $2; found = 1; next }
    END { if (!found) exit 1 }'
}

SDK_PATH=""
sdk_path() {
  if [ -z "$SDK_PATH" ]; then
    SDK_PATH=$(xcrun --sdk macosx --show-sdk-path 2>/dev/null || true)
    [ -n "$SDK_PATH" ] || SDK_PATH="none"
  fi
  printf '%s' "$SDK_PATH"
}

# Prints the macOS an Objective-C class needs, from its @interface in the SDK
# headers of the framework it is imported from; prints nothing when unknown.
class_min_macos() { # <class> <framework>
  local sdk dir hit file line start
  sdk=$(sdk_path)
  [ "$sdk" != "none" ] || return 0
  dir="$sdk/System/Library/Frameworks/$2.framework/Headers"
  [ -d "$dir" ] || return 0
  hit=$(grep -n -E "@interface[[:space:]]+$1[[:space:]]*(:|<|$)" "$dir"/*.h 2>/dev/null | head -1 || true)
  [ -n "$hit" ] || return 0
  file=${hit%%:*}
  hit=${hit#*:}
  line=${hit%%:*}
  start=$((line > 4 ? line - 4 : 1))
  # Only the lines after the previous declaration or comment belong to this one.
  sed -n "${start},${line}p" "$file" \
    | awk '/@end|;|\*\// { buf = ""; next } { buf = buf " " $0 } END { print buf }' \
    | grep -oE 'macos(x)?\([0-9]+(\.[0-9]+)*\)' | tail -1 | grep -oE '[0-9]+(\.[0-9]+)*' || true
}

# The two readers below print "<arch>\t<symbol>\t<from>" for every strong import
# of one slice; keep_candidates keeps the ones that could need a newer macOS (the
# symbols the rules look at).

# From `xcrun dyld_info -arch <arch> -imports` output, which marks a weak import
# `[weak-import]`.
strong_imports_dyld_info() { # <arch> <dyld_info output>
  printf '%s\n' "$2" | awk -v arch="$1" '
    / \[[A-Za-z0-9_]+\]:$/ || /-imports:/ { next }
    NF == 0 { next }
    {
      if (index($0, "[weak-import]") > 0) next
      sym = $1
      if (sym ~ /^0x[0-9A-Fa-f]+$/) sym = $2
      from = ""
      if (match($0, /\(from [^)]*\)/)) from = substr($0, RSTART + 6, RLENGTH - 7)
      print arch "\t" sym "\t" from
    }'
}

# From `nm -m -u -arch <arch>`; fails when nm cannot read the slice. The symbol is
# the word after `external`; a `weak` before it marks a weak import. (nm prints
# the same word for a reference to a weakly defined symbol, which the Objective-C
# classes and Accelerate interfaces the rules look at are not.)
strong_imports_nm() { # <binary> <arch>
  local out
  out=$(nm -m -u -arch "$2" "$1" 2>/dev/null) || return 1
  printf '%s\n' "$out" | awk -v arch="$2" '
    index($0, "(undefined)") == 0 { next }
    {
      sym = ""
      weak = 0
      for (i = 1; i < NF; i++) {
        if ($i == "weak") weak = 1
        if ($i == "external") { sym = $(i + 1); break }
      }
      if (weak || sym == "") next
      from = ""
      if (match($0, /\(from [^)]*\)/)) from = substr($0, RSTART + 6, RLENGTH - 7)
      print arch "\t" sym "\t" from
    }'
}

keep_candidates() {
  awk -F '\t' 'index($2, "$NEWLAPACK") > 0 || $2 ~ /^_OBJC_(META)?CLASS_\$_/'
}

# Prints "<version>|<reason>" when the symbol needs a newer macOS than the floor.
newer_than_floor() { # <symbol> <from>
  local sym="$1" from="$2" rule glob version reason class need
  while IFS= read -r rule; do
    [ -n "$rule" ] || continue
    glob=${rule%%|*}
    rule=${rule#*|}
    version=${rule%%|*}
    reason=${rule#*|}
    # shellcheck disable=SC2254 # the rule is a glob on purpose
    case "$sym" in
      $glob)
        if [ "$(version_cmp "$version" "$MACOS_FLOOR")" = 1 ]; then
          printf '%s|%s\n' "$version" "$reason"
          return 0
        fi
        ;;
    esac
  done <<EOF
$PINNED_RULES
EOF
  case "$sym" in
    _OBJC_CLASS_\$_* | _OBJC_METACLASS_\$_*)
      class=${sym#_OBJC_CLASS_\$_}
      class=${class#_OBJC_METACLASS_\$_}
      need=$(class_min_macos "$class" "$from")
      if [ -n "$need" ] && [ "$(version_cmp "$need" "$MACOS_FLOOR")" = 1 ]; then
        printf '%s|%s\n' "$need" "the Objective-C class $class (API_AVAILABLE in $from's SDK headers)"
      fi
      ;;
  esac
  return 0
}

check_binary() { # <binary> <require imports: 0|1>
  local bin="$1" require="$2" archs arch mins min fail=0 imports rc reason line sym from hit count
  local mode="${CHECK_MACOS_DEPLOYMENT_IMPORTS:-auto}" read_by slice candidates="" read_any=0
  if ! archs=$(lipo -archs "$bin" 2>/dev/null) || [ -z "$archs" ]; then
    err "$bin is not a Mach-O binary"
    return 1
  fi
  for arch in $archs; do
    if ! mins=$(min_os_of "$bin" "$arch"); then
      err "$bin [$arch] declares no macOS minimum (neither LC_BUILD_VERSION nor LC_VERSION_MIN_MACOSX)"
      fail=1
      continue
    fi
    for min in $mins; do
      if ! is_version "$min"; then
        err "$bin [$arch] declares an unreadable macOS minimum '$min'"
        fail=1
      elif [ "$(version_cmp "$min" "$MACOS_FLOOR")" != 0 ]; then
        err "$bin [$arch] declares macOS $min as its minimum, not $MACOS_FLOOR. .cargo/config.toml sets MACOSX_DEPLOYMENT_TARGET = \"$MACOS_FLOOR\" for every cargo build; a value already in the environment wins over it, so look for a stray export, or a build that did not run cargo from this checkout."
        fail=1
      else
        echo "$bin [$arch]: minimum macOS $min"
      fi
    done
  done

  for arch in $archs; do
    read_by=""
    reason=""
    slice=""
    if [ "$mode" != nm ]; then
      rc=0
      imports=$(xcrun dyld_info -arch "$arch" -imports "$bin" 2>&1) || rc=$?
      if [ "$rc" -eq 0 ] && printf '%s\n' "$imports" | grep -q -- '-imports:'; then
        read_by="xcrun dyld_info -imports"
        slice=$(strong_imports_dyld_info "$arch" "$imports")
      else
        reason="dyld_info: $(printf '%s' "$imports" | tail -1 | sed 's/^ *//')"
        [ "$rc" -eq 0 ] || reason="$reason (exit $rc)"
      fi
    fi
    if [ -z "$read_by" ] && [ "$mode" != dyld_info ]; then
      if slice=$(strong_imports_nm "$bin" "$arch"); then
        read_by="nm -m -u"
      else
        reason="${reason:+$reason; }nm -m -u could not read it either"
      fi
    fi
    if [ -z "$read_by" ]; then
      if [ "$require" = 1 ]; then
        err "$bin [$arch]: cannot read its import table, and --require-imports was given ($reason)"
        fail=1
      else
        echo "$bin [$arch]: import table not read ($reason), so only the minimum was checked"
      fi
      continue
    fi
    echo "$bin [$arch]: imports read with $read_by"
    read_any=1
    candidates="$candidates$(printf '%s\n' "$slice" | keep_candidates)
"
  done
  [ "$read_any" = 1 ] || return "$fail"
  count=0
  while IFS= read -r line; do
    [ -n "$line" ] || continue
    arch=$(printf '%s' "$line" | cut -f1)
    sym=$(printf '%s' "$line" | cut -f2)
    from=$(printf '%s' "$line" | cut -f3)
    hit=$(newer_than_floor "$sym" "$from")
    if [ -n "$hit" ]; then
      err "$bin [$arch] imports $sym from $from strongly; it needs macOS ${hit%%|*} (${hit#*|}), so dyld would refuse the binary on macOS $MACOS_FLOOR. Declare it weak (an availability check in the C or Objective-C source, built with MACOSX_DEPLOYMENT_TARGET=$MACOS_FLOOR) or stop using it."
      fail=1
      count=$((count + 1))
    fi
  done <<EOF
$candidates
EOF
  if [ "$count" -eq 0 ]; then
    echo "$bin: no strong import needs a macOS newer than $MACOS_FLOOR"
  fi
  return "$fail"
}

check() { # <require imports> <binary>...
  local require="$1" bin failed=0
  shift
  [ "$#" -gt 0 ] || { echo "usage: $0 [--require-imports] BINARY... | --self-test" >&2; return 2; }
  for tool in lipo xcrun; do
    command -v "$tool" >/dev/null 2>&1 || { err "$tool is missing; install the Xcode command line tools"; return 2; }
  done
  if ! command -v vtool >/dev/null 2>&1 && ! command -v otool >/dev/null 2>&1; then
    err "neither vtool nor otool is installed; install the Xcode command line tools"
    return 2
  fi
  case "${CHECK_MACOS_DEPLOYMENT_IMPORTS:-auto}" in
    auto | dyld_info | nm) ;;
    *)
      err "CHECK_MACOS_DEPLOYMENT_IMPORTS is '$CHECK_MACOS_DEPLOYMENT_IMPORTS'; it must be auto, dyld_info or nm"
      return 2
      ;;
  esac
  for bin in "$@"; do
    [ -f "$bin" ] || { err "missing binary $bin"; return 2; }
  done
  for bin in "$@"; do
    check_binary "$bin" "$require" || failed=1
  done
  if [ "$failed" -ne 0 ]; then
    echo "The macOS backends must declare macOS $MACOS_FLOOR (DD-11); see the errors above." >&2
    return 1
  fi
  echo "OK: every binary declares macOS $MACOS_FLOOR"
}

# ── self test ────────────────────────────────────────────────────────────────
ST_FAILED=0
ST_LOG=""
ST_DIR=""

# Runs the check on its own arguments, then compares the exit code with the one
# expected and, when a fragment is given, looks for it in what the check printed.
expect() { # <expected exit code> <fragment or ""> <description> <check arguments...>
  local want="$1" fragment="$2" what="$3" got=0
  shift 3
  "$SELF" "$@" >"$ST_LOG" 2>&1 || got=$?
  if [ "$got" != "$want" ]; then
    echo "FAILED: $what (exit $got, expected $want); the check printed:" >&2
    sed 's/^/    /' "$ST_LOG" >&2
    ST_FAILED=1
  elif [ -n "$fragment" ] && ! grep -qF -- "$fragment" "$ST_LOG"; then
    echo "FAILED: $what (exit $got as expected, but the output lacks '$fragment'); the check printed:" >&2
    sed 's/^/    /' "$ST_LOG" >&2
    ST_FAILED=1
  else
    echo "ok: $what"
  fi
}

fixture_failed() { # <what could not be built>
  echo "FAILED: could not build $1:" >&2
  sed 's/^/    /' "$ST_LOG" >&2
  ST_FAILED=1
}

rust_target_installed() { # <target>
  local libdir
  libdir=$(rustc --print target-libdir --target "$1" 2>/dev/null) || return 1
  ls "$libdir"/libstd-*.rlib >/dev/null 2>&1
}

slice_name() { # <rust target> -> the architecture name lipo and vtool use
  case "$1" in
    aarch64-*) echo arm64 ;;
    *) echo "${1%%-*}" ;;
  esac
}

# Links a C fixture at the floor with clang; returns clang's status.
import_fixture() { # <name> <C source>
  printf '%s\n' "$2" >"$ST_DIR/$1.c"
  xcrun clang -mmacosx-version-min="$MACOS_FLOOR" -framework Metal -framework Accelerate \
    -o "$ST_DIR/$1" "$ST_DIR/$1.c" >"$ST_LOG" 2>&1
}

self_test() {
  local tmp host other config_value targets t slice reader default
  if [ "$(uname -s)" != "Darwin" ]; then
    err "the self test builds Mach-O fixtures and needs macOS"
    return 2
  fi
  for tool in rustc cargo lipo xcrun; do
    command -v "$tool" >/dev/null 2>&1 || { err "the self test needs $tool"; return 2; }
  done
  # rust-toolchain.toml and .cargo/config.toml are found from the working directory.
  cd "$ROOT"
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/check-macos-deployment.XXXXXX")
  # shellcheck disable=SC2064 # expand now: the path is fixed
  trap "rm -rf '$tmp'" EXIT
  ST_DIR="$tmp"
  ST_LOG="$tmp/check.log"

  # The floor this script holds is the one .cargo/config.toml sets.
  config_value=$(sed -n 's/^MACOSX_DEPLOYMENT_TARGET[[:space:]]*=[[:space:]]*"\([^"]*\)".*/\1/p' .cargo/config.toml 2>/dev/null | head -1)
  if [ "$config_value" = "$MACOS_FLOOR" ]; then
    echo "ok: .cargo/config.toml sets MACOSX_DEPLOYMENT_TARGET = \"$MACOS_FLOOR\", the floor this check holds"
  else
    echo "FAILED: .cargo/config.toml sets MACOSX_DEPLOYMENT_TARGET to '${config_value}', not \"$MACOS_FLOOR\"" >&2
    ST_FAILED=1
  fi

  printf 'fn main() {}\n' >"$tmp/main.rs"
  mkdir -p "$tmp/fixture/src"
  cp "$tmp/main.rs" "$tmp/fixture/src/main.rs"
  printf '[package]\nname = "fixture"\nversion = "0.0.0"\nedition = "2021"\npublish = false\n\n[workspace]\n' \
    >"$tmp/fixture/Cargo.toml"

  case "$(uname -m)" in
    arm64) host=aarch64-apple-darwin other=x86_64-apple-darwin ;;
    *) host=x86_64-apple-darwin other=aarch64-apple-darwin ;;
  esac
  targets=$host
  if rust_target_installed "$other"; then
    targets="$host $other"
  else
    echo "note: the Rust standard library for $other is not installed; its fixtures are skipped"
  fi

  for t in $targets; do
    slice=$(slice_name "$t")
    # Through cargo from this checkout, with the variable absent from the
    # environment: only .cargo/config.toml can supply the floor.
    if env -u MACOSX_DEPLOYMENT_TARGET cargo build --offline --quiet \
      --manifest-path "$tmp/fixture/Cargo.toml" --target-dir "$tmp/target" --target "$t" >"$ST_LOG" 2>&1; then
      cp "$tmp/target/$t/debug/fixture" "$tmp/config-$t"
    else
      fixture_failed "the $t fixture through cargo"
    fi
    # The variable cleared: rustc falls back to its own default.
    if ! env -u MACOSX_DEPLOYMENT_TARGET rustc --target "$t" -o "$tmp/cleared-$t" "$tmp/main.rs" >"$ST_LOG" 2>&1; then
      fixture_failed "the cleared $t fixture"
    fi
    case "$t" in
      aarch64-*) default=11.0 ;;
      *) default=10.12 ;;
    esac
    # Both readers: vtool, and the otool fallback for a machine without vtool.
    for reader in vtool otool; do
      if [ -f "$tmp/config-$t" ]; then
        CHECK_MACOS_DEPLOYMENT_READER=$reader \
          expect 0 "[$slice]: minimum macOS $MACOS_FLOOR" \
          "$reader passes a $t binary that cargo built through .cargo/config.toml" "$tmp/config-$t"
      fi
      if [ -f "$tmp/cleared-$t" ]; then
        CHECK_MACOS_DEPLOYMENT_READER=$reader \
          expect 1 "declares macOS $default as its minimum, not $MACOS_FLOOR" \
          "$reader fails a $t binary built with MACOSX_DEPLOYMENT_TARGET cleared (rustc's default, $default)" "$tmp/cleared-$t"
      fi
    done
    if [ -f "$tmp/config-$t" ]; then
      expect 0 "no strong import needs a macOS newer than $MACOS_FLOOR" \
        "passes the same binary with --require-imports (unstripped)" --require-imports "$tmp/config-$t"
    fi
  done

  if MACOSX_DEPLOYMENT_TARGET=13.0 rustc -o "$tmp/newer" "$tmp/main.rs" >"$ST_LOG" 2>&1; then
    expect 1 "declares macOS 13.0 as its minimum, not $MACOS_FLOOR" \
      "fails a binary whose minimum is newer than the floor (13.0)" "$tmp/newer"
  else
    fixture_failed "the 13.0 fixture"
  fi
  # Stripped the way the release profile strips (`strip = true`).
  if MACOSX_DEPLOYMENT_TARGET="$MACOS_FLOOR" rustc -C strip=symbols -o "$tmp/stripped" "$tmp/main.rs" >"$ST_LOG" 2>&1; then
    expect 0 "no strong import needs a macOS newer than $MACOS_FLOOR" \
      "reads the imports of a stripped binary under --require-imports" --require-imports "$tmp/stripped"
    if ! xcrun dyld_info -imports "$tmp/stripped" 2>/dev/null | grep -q -- '-imports:'; then
      # dyld_info refuses this one ("mis-aligned LINKEDIT string pool").
      expect 0 "imports read with nm -m -u" \
        "falls back to nm on a stripped binary dyld_info cannot read" --require-imports "$tmp/stripped"
      CHECK_MACOS_DEPLOYMENT_IMPORTS=dyld_info \
        expect 0 "import table not read" \
        "passes on the minimum alone when no reader can read the imports" "$tmp/stripped"
      CHECK_MACOS_DEPLOYMENT_IMPORTS=dyld_info \
        expect 1 "cannot read its import table, and --require-imports was given" \
        "fails under --require-imports when no reader can read the imports" --require-imports "$tmp/stripped"
    else
      echo "note: dyld_info reads this rustc's stripped fixture, so the fallback to nm is exercised only by the nm cases below"
    fi
  else
    fixture_failed "the stripped fixture"
  fi
  # A stripped Rust binary that imports a macOS 15 class strongly, as an engine
  # linked into biorouterd could.
  cat >"$tmp/residency.rs" <<'RS'
#[link(name = "Metal", kind = "framework")]
extern "C" {
    #[link_name = "OBJC_CLASS_$_MTLResidencySetDescriptor"]
    static RESIDENCY_SET_DESCRIPTOR: u8;
}
#[allow(unused_unsafe)]
fn main() {
    let p = unsafe { std::ptr::addr_of!(RESIDENCY_SET_DESCRIPTOR) };
    std::process::exit((p as usize & 1) as i32);
}
RS
  if MACOSX_DEPLOYMENT_TARGET="$MACOS_FLOOR" rustc -C strip=symbols -o "$tmp/residency-stripped" "$tmp/residency.rs" >"$ST_LOG" 2>&1; then
    for reader in auto nm; do
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 1 "it needs macOS 15.0 (a Metal residency set class)" \
        "fails a stripped Rust binary that imports MTLResidencySetDescriptor strongly (reader $reader)" \
        --require-imports "$tmp/residency-stripped"
    done
  else
    fixture_failed "the stripped MTLResidencySetDescriptor fixture"
  fi
  if [ -f "$tmp/config-$host" ] && [ -f "$tmp/cleared-$host" ]; then
    expect 1 "cleared-$host [$(slice_name "$host")] declares macOS" \
      "fails when one of the binaries given fails" "$tmp/config-$host" "$tmp/cleared-$host"
  fi
  if [ "$targets" != "$host" ] && [ -f "$tmp/config-$host" ] && [ -f "$tmp/config-$other" ]; then
    if lipo -create -output "$tmp/universal-ok" "$tmp/config-$host" "$tmp/config-$other" >"$ST_LOG" 2>&1; then
      expect 0 "[$(slice_name "$other")]: minimum macOS $MACOS_FLOOR" \
        "passes a universal binary whose slices both declare $MACOS_FLOOR" "$tmp/universal-ok"
    else
      fixture_failed "the universal fixture"
    fi
    if [ -f "$tmp/cleared-$other" ] &&
      lipo -create -output "$tmp/universal-bad" "$tmp/config-$host" "$tmp/cleared-$other" >"$ST_LOG" 2>&1; then
      expect 1 "universal-bad [$(slice_name "$other")] declares macOS" \
        "fails a universal binary when one slice was built with the variable cleared" "$tmp/universal-bad"
    else
      fixture_failed "the mixed universal fixture"
    fi
  fi

  # Strong and weak imports, linked by clang at the floor, through each reader.
  import_fixture residency-strong 'extern char OBJC_CLASS_$_MTLResidencySetDescriptor;
int main(void) { return (int)((long)&OBJC_CLASS_$_MTLResidencySetDescriptor & 1); }' ||
    fixture_failed "the strong MTLResidencySetDescriptor fixture"
  import_fixture residency-weak 'extern char OBJC_CLASS_$_MTLResidencySetDescriptor __attribute__((weak_import));
int main(void) { return (int)((long)&OBJC_CLASS_$_MTLResidencySetDescriptor & 1); }' ||
    fixture_failed "the weak MTLResidencySetDescriptor fixture"
  import_fixture lapack-strong 'extern void cblas_sgemm$NEWLAPACK$ILP64(void);
int main(int argc, char **argv) { (void)argv; if (argc > 99) cblas_sgemm$NEWLAPACK$ILP64(); return 0; }' ||
    fixture_failed "the NEWLAPACK fixture"
  import_fixture logstate-strong 'extern char OBJC_CLASS_$_MTLLogStateDescriptor;
int main(void) { return (int)((long)&OBJC_CLASS_$_MTLLogStateDescriptor & 1); }' ||
    fixture_failed "the MTLLogStateDescriptor fixture"
  import_fixture compileoptions-strong 'extern char OBJC_CLASS_$_MTLCompileOptions;
int main(void) { return (int)((long)&OBJC_CLASS_$_MTLCompileOptions & 1); }' ||
    fixture_failed "the MTLCompileOptions fixture"
  for reader in dyld_info nm; do
    if [ -f "$tmp/residency-strong" ]; then
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 1 "it needs macOS 15.0 (a Metal residency set class)" \
        "fails a strong import of MTLResidencySetDescriptor (macOS 15.0, pinned; reader $reader)" \
        --require-imports "$tmp/residency-strong"
    fi
    if [ -f "$tmp/residency-weak" ]; then
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 0 "no strong import needs" \
        "passes a weak import of MTLResidencySetDescriptor (reader $reader)" \
        --require-imports "$tmp/residency-weak"
    fi
    if [ -f "$tmp/lapack-strong" ]; then
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 1 "it needs macOS 13.3" \
        "fails a strong import of cblas_sgemm\$NEWLAPACK\$ILP64 (macOS 13.3, pinned; reader $reader)" \
        --require-imports "$tmp/lapack-strong"
    fi
    if [ -f "$tmp/logstate-strong" ]; then
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 1 "it needs macOS 15.0 (the Objective-C class MTLLogStateDescriptor" \
        "fails a strong import of MTLLogStateDescriptor (macOS 15.0, read from the SDK headers; reader $reader)" \
        --require-imports "$tmp/logstate-strong"
    fi
    if [ -f "$tmp/compileoptions-strong" ]; then
      CHECK_MACOS_DEPLOYMENT_IMPORTS=$reader \
        expect 0 "no strong import needs" \
        "passes a strong import of MTLCompileOptions (macOS 10.11, read from the SDK headers; reader $reader)" \
        --require-imports "$tmp/compileoptions-strong"
    fi
  done
  CHECK_MACOS_DEPLOYMENT_IMPORTS=otool \
    expect 2 "it must be auto, dyld_info or nm" "refuses an unknown import reader" "$tmp/does-not-exist"

  printf 'not a binary\n' >"$tmp/text"
  expect 1 "is not a Mach-O binary" "fails a file that is not a Mach-O binary" "$tmp/text"
  expect 2 "missing binary" "refuses a binary that does not exist" "$tmp/does-not-exist"
  expect 2 "usage:" "refuses to run with no binary" --require-imports

  if [ "$ST_FAILED" -ne 0 ]; then
    echo "check-macos-deployment self test FAILED" >&2
    return 1
  fi
  echo "check-macos-deployment self test passed"
}

case "${1:-}" in
  --self-test)
    [ "$#" -eq 1 ] || { echo "usage: $0 --self-test" >&2; exit 2; }
    self_test
    ;;
  --require-imports)
    shift
    check 1 "$@"
    ;;
  -h | --help)
    sed -n '2,/^set -euo pipefail$/p' "$0" | sed '$d' | sed 's/^# \{0,1\}//'
    ;;
  *)
    check 0 "$@"
    ;;
esac
