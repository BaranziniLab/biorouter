#!/usr/bin/env bash
# Stage the Linux x86_64 Crew broker, with its manifest, for a package that carries it (OW-9).
#
# The daemon's Crew setup service installs `biorouter-crew` into a server account by sending
# it over the SSH session, so every desktop build and the CLI packages carry the exact Linux
# broker beside `biorouterd` and need no network to find it:
#
#   <dir of biorouterd>/crew-broker/linux-x86_64/biorouter-crew
#   <dir of biorouterd>/crew-broker/linux-x86_64/manifest.json
#
# The manifest is what the setup service trusts: it checks the bytes it sends against `sha256`
# and `size`, and compares `version` with its own. So it is WRITTEN HERE, from the bytes being
# staged, and never by hand:
#
#   {"schema":1,"name":"biorouter-crew","version":"<Cargo.toml>","target":"x86_64-unknown-linux-gnu",
#    "glibc_max":"<highest GLIBC_x.y the binary imports>","sha256":"<hex>","size":<bytes>,
#    "protocol":1,"capabilities":[...],"source_sha":"<git HEAD>","source_dirty":<bool>}
#
# `capabilities` and `protocol` come from `biorouter-crew --version --output-format json` when
# the broker answers that with a JSON object; a broker that prints only its version line gets
# `[]` and protocol 1. `source_dirty` follows `git describe --dirty`: tracked changes only.
#
# Usage: scripts/stage-crew-broker.sh <linux-release-dir-or-binary> <dest-dir>
#   <linux-release-dir-or-binary>  a directory holding biorouter-crew (for example
#                                  target/x86_64-unknown-linux-gnu/release) or the binary itself
#   <dest-dir>                     the directory to (re)create, normally
#                                  ui/desktop/src/bin/crew-broker/linux-x86_64
#   Relative paths resolve from the caller's directory.
#
# Each of these is refused before anything lands at <dest-dir>:
#   * a file that is not an x86-64 ELF executable (e_machine 0x3E);
#   * a broker that needs a glibc newer than the floor (scripts/check-glibc-floor.sh);
#   * a broker built without join-by-name (scripts/check-crew-broker-join.sh);
#   * a broker whose `--version` is not this tree's Cargo.toml version;
#   * a <dest-dir> that holds anything other than an earlier staging.
# The checks run on the staged copy, not the source, and the copy replaces <dest-dir> with two
# renames, so a failed or interrupted run leaves the previous staging in place.
#
# Needs Docker (the glibc read, and running the broker on a host that is not Linux x86_64,
# inside the glibc 2.31 baseline image) and python3.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BROKER=biorouter-crew
TARGET=x86_64-unknown-linux-gnu

log() { printf '\033[1;36m[crew-broker]\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m[crew-broker] ERROR:\033[0m %s\n' "$*" >&2; exit 1; }

SOURCE="${1:?usage: stage-crew-broker.sh <linux-release-dir-or-binary> <dest-dir>}"
DEST="${2:?usage: stage-crew-broker.sh <linux-release-dir-or-binary> <dest-dir>}"
[ -d "$SOURCE" ] && SOURCE="$SOURCE/$BROKER"
[ -f "$SOURCE" ] || die "missing Crew broker $SOURCE — build it first (just stage-crew-broker, or scripts/release.sh linux-backend <ver>)"
[ -s "$SOURCE" ] || die "Crew broker $SOURCE is empty"
command -v python3 >/dev/null 2>&1 || die "python3 is required to write the manifest"
command -v docker >/dev/null 2>&1 || die "docker is required to read the broker's glibc requirement"

VERSION="$(perl -0ne 'print $1 if /\[workspace\.package\].*?version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"/s' "$ROOT/Cargo.toml")"
[ -n "$VERSION" ] || die "could not read the workspace version from $ROOT/Cargo.toml"

# ── Where the staging happens ─────────────────────────────────────────────────
# Beside DEST, so the final step is a rename within one directory, never a copy.
mkdir -p "$(dirname "$DEST")"
DEST="$(cd "$(dirname "$DEST")" && pwd)/$(basename "$DEST")"
if [ -e "$DEST" ] || [ -L "$DEST" ]; then
  [ -d "$DEST" ] && [ ! -L "$DEST" ] || die "refusing to replace $DEST: it is not a directory"
  # DEST is replaced wholesale, so a mistyped path (src/bin itself, say) must not be emptied.
  for entry in "$DEST"/* "$DEST"/.[!.]*; do
    [ -e "$entry" ] || [ -L "$entry" ] || continue
    case "$(basename "$entry")" in
      "$BROKER"|manifest.json) ;;
      *) die "refusing to replace $DEST: it holds $(basename "$entry"), which no staging writes" ;;
    esac
  done
fi
STAGING="$(mktemp -d "$(dirname "$DEST")/.$(basename "$DEST").staging.XXXXXX")"
# mktemp makes it 0700, and it becomes DEST: a package would install a directory only its
# owner (root) can enter, so the setup service of every other account would find no broker.
chmod 0755 "$STAGING"
HOLD=""
cleanup() {
  # Put the previous staging back if the swap stopped half way.
  if [ -n "$HOLD" ] && [ -d "$HOLD/previous" ] && [ ! -e "$DEST" ]; then
    mv "$HOLD/previous" "$DEST" || true
  fi
  rm -rf -- "$STAGING" ${HOLD:+"$HOLD"}
}
trap cleanup EXIT

STAGED="$STAGING/$BROKER"
cp "$SOURCE" "$STAGED"
chmod 0755 "$STAGED"

# ── 1. An x86-64 ELF executable ───────────────────────────────────────────────
# e_ident: magic 7f 'E' 'L' 'F', class 2 (64-bit), data 1 (little-endian, which is how the
# two-byte fields below are read); e_type at 16 is EXEC (2) or DYN (3, a PIE); e_machine at
# 18 is 0x3E (x86-64). A macOS or aarch64 build fails here rather than on someone's server.
read -r -a header <<<"$(od -An -v -tx1 -N20 "$STAGED" | tr '\n' ' ')"
[ "${#header[@]}" -eq 20 ] || die "$SOURCE is too short to be an ELF executable"
[ "${header[0]}${header[1]}${header[2]}${header[3]}" = 7f454c46 ] || die "$SOURCE is not an ELF file"
[ "${header[4]}" = 02 ] || die "$SOURCE is not a 64-bit ELF file"
[ "${header[5]}" = 01 ] || die "$SOURCE is not a little-endian ELF file"
case "${header[16]}${header[17]}" in
  0200|0300) ;;
  *) die "$SOURCE is an ELF file but not an executable (e_type 0x${header[17]}${header[16]})" ;;
esac
[ "${header[18]}${header[19]}" = 3e00 ] \
  || die "$SOURCE is built for ELF machine 0x${header[19]}${header[18]}, not x86-64 (0x3e)"
log "ELF x86-64 executable: $SOURCE"

# ── 2. The glibc floor, read from the symbol table ────────────────────────────
GLIBC_OUT="$STAGING/.glibc-max"
bash "$ROOT/scripts/check-glibc-floor.sh" --max-out "$GLIBC_OUT" "$STAGING" "$BROKER" \
  || die "the Crew broker failed the glibc floor check (output above)"
GLIBC_MAX="$(cat "$GLIBC_OUT")"
rm -f "$GLIBC_OUT"

# ── 3. Joining by invitation ──────────────────────────────────────────────────
bash "$ROOT/scripts/check-crew-broker-join.sh" "$STAGED" \
  || die "the Crew broker was built without the join-by-name feature"

# ── 4. Its own word on its version and capabilities ───────────────────────────
# Natively on Linux x86_64; elsewhere in the glibc 2.31 baseline the Linux packages are built
# on, which also proves the broker starts on the floor it was checked against.
run_broker() { # <args...>
  if [ "$(uname -s)" = Linux ] && [ "$(uname -m)" = x86_64 ]; then
    "$STAGED" "$@"
  else
    local image
    image="$(python3 -I -c 'import json, sys; print(json.load(open(sys.argv[1]))["image"])' \
               "$ROOT/ui/desktop/scripts/linux-native-baseline.json")"
    docker run --rm --platform linux/amd64 --network none -v "$STAGING":/broker:ro \
      "$image" "/broker/$BROKER" "$@"
  fi
}
reported="$(run_broker --version)" || die "the Crew broker did not run: $SOURCE --version failed"
[ "$reported" = "$BROKER $VERSION" ] \
  || die "the Crew broker reports '$reported' but this tree is $VERSION: it was built from another version. Rebuild it (just stage-crew-broker, or scripts/release.sh linux-backend $VERSION)."
DESCRIPTION="$(run_broker --version --output-format json)" \
  || die "the Crew broker did not run: $SOURCE --version --output-format json failed"

# ── 5. Provenance ─────────────────────────────────────────────────────────────
SOURCE_SHA="$(git -C "$ROOT" rev-parse HEAD 2>/dev/null || true)"
SOURCE_DIRTY=""
if [ -n "$SOURCE_SHA" ]; then
  if [ -n "$(git -C "$ROOT" status --porcelain --untracked-files=no)" ]; then
    SOURCE_DIRTY=true
  else
    SOURCE_DIRTY=false
  fi
else
  log "warning: $ROOT is not a git checkout, so the manifest records no source commit"
fi

# ── 6. The manifest, from the staged bytes ────────────────────────────────────
if ! python3 -I - "$STAGED" "$STAGING/manifest.json" "$VERSION" "$TARGET" "$GLIBC_MAX" \
  "$SOURCE_SHA" "$SOURCE_DIRTY" "$DESCRIPTION" <<'PY'
import hashlib
import json
import sys

binary, out, version, target, glibc_max, source_sha, source_dirty, description = sys.argv[1:]
data = open(binary, "rb").read()

protocol, capabilities = 1, []
try:
    described = json.loads(description)
except ValueError:
    described = None  # a broker that prints only its version line
if isinstance(described, dict):
    if described.get("version", version) != version:
        raise SystemExit(f"the broker describes itself as version {described['version']!r}, not {version}")
    protocol = described.get("protocol", protocol)
    capabilities = described.get("capabilities", capabilities)
    if not isinstance(protocol, int) or isinstance(protocol, bool) or protocol < 1:
        raise SystemExit(f"the broker reports an invalid protocol {protocol!r}")
    if not isinstance(capabilities, list) or not all(isinstance(c, str) and c for c in capabilities):
        raise SystemExit(f"the broker reports invalid capabilities {capabilities!r}")

manifest = {
    "schema": 1,
    "name": "biorouter-crew",
    "version": version,
    "target": target,
    "glibc_max": glibc_max,
    "sha256": hashlib.sha256(data).hexdigest(),
    "size": len(data),
    "protocol": protocol,
    "capabilities": capabilities,
    "source_sha": source_sha or None,
    "source_dirty": {"true": True, "false": False}.get(source_dirty),
}
with open(out, "w", encoding="utf-8") as handle:
    json.dump(manifest, handle, indent=2)
    handle.write("\n")
PY
then
  die "could not write the manifest (reason above)"
fi
chmod 0644 "$STAGING/manifest.json"

# ── 7. Swap it in ─────────────────────────────────────────────────────────────
if [ -d "$DEST" ]; then
  HOLD="$(mktemp -d "$(dirname "$DEST")/.$(basename "$DEST").previous.XXXXXX")"
  mv "$DEST" "$HOLD/previous"
fi
mv "$STAGING" "$DEST"
log "staged $BROKER $VERSION at $DEST"
log "manifest: $(tr -d '\n' <"$DEST/manifest.json" | tr -s ' ')"
