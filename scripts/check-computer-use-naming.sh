#!/usr/bin/env bash
#
# The capability is called "Computer Use" (or "Biorouter Computer Use") wherever
# a person can read it. It is NOT "Computer Controller" -- that was the previous,
# removed capability -- and it is not "Codex Computer Use" or "Open Computer Use".
#
# What this check does NOT police, deliberately:
#
#   * the internal identifier `computercontroller`. It is the MCP server name
#     (`biorouter mcp computercontroller`), the builtin registry key and a config
#     key, so renaming it would break existing configurations and command lines.
#     It is already paired with the user-facing label "Computer Use"
#     (ui/desktop/src/components/settings/capabilities/capabilities.ts).
#   * upstream's own name. `vendor/computer-use/`, the vendored
#     `OpenComputerUse*` sources and the NOTICE file that attributes them are the
#     upstream project's identity, and the licence requires that attribution.
#   * migration copy that names the OLD capability AS legacy, e.g. "This legacy
#     Computer Controller tool was removed. Use the Computer Use tools". Telling a
#     user what their old thing was called is the entire point of that message, so
#     the rule is narrow: "Computer Controller" is allowed only when the same line
#     marks it legacy.
#   * historical records and migration prose. docs/history/, docs/releases/notes/
#     and the integration plan describe what the product USED to be called; a
#     release note is a record of what that version said, not a live product
#     string. This check therefore polices CODE -- the strings a person reads out
#     of the running app and CLI -- and leaves prose to review.
#   * Rust test modules, `crates/*/tests/*.rs` and `*.test.ts(x)` files. One test
#     deliberately feeds "Computer Controller" as an impersonation attempt
#     (external_servers_cannot_impersonate_the_computer_use_approval_namespace);
#     failing on a test's own adversarial input would make the gate unfixable.
#     A test module is recognised by a cfg(test)-family attribute -- including
#     `#[cfg(all(test, unix))]`, which both Computer Use modules use -- that is
#     immediately followed by a `mod`. It is NOT recognised by the attribute
#     alone: in 25+ files the first one sits on a test-only `use` or `const` in
#     the import block, and stripping from there to EOF blanked entire production
#     files (agent.rs: 23,434 of 23,460 lines).

set -euo pipefail
cd "$(dirname "$0")/.."

status=0

# Sources a person's words can reach. third_party/ is upstream, src/web/ is a
# built bundle, and *.test.* files carry deliberate adversarial input.
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
sources=$(git ls-files 'crates/*.rs' 'ui/desktop/src/*.ts' 'ui/desktop/src/*.tsx' \
  | grep -v '^third_party/' \
  | grep -v '^ui/desktop/src/web/' \
  | grep -vE '^crates/[^/]+/tests/' \
  | grep -vE '\.test\.tsx?$')
if [ -z "$sources" ]; then
  echo "Computer Use naming: no sources matched -- the gate would pass vacuously." >&2
  exit 2
fi

# Rust keeps its tests in-file, so a trailing test module is blanked before
# matching. The attribute alone is NOT the signal -- it must be followed by a
# `mod`, or a test-only `use` in the import block blanks the whole file. Line
# numbers are preserved so the report still points at the real line.
shipped=$scratch/shipped
mkdir -p "$shipped"
for file in $sources; do
  # A tracked file deleted but not yet committed is the normal mid-refactor
  # state; without this awk aborts the whole gate with an unattributed error.
  [ -f "$file" ] || continue
  mkdir -p "$shipped/$(dirname "$file")"
  awk '
    # A cfg(test) family attribute: cfg(test), cfg(all(test, ...)), indented.
    # Held, not printed, so the decision costs no line: exactly one output line
    # per input line, or every reported line number would be shifted.
    /^[[:space:]]*#\[cfg\((all\()?test[,)]/ && !stop { held = $0; next }
    held != "" {
      # Only a module turns it into a test block. Anything else (a use, a const)
      # was a test-only item, so put the attribute back and carry on scanning.
      if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]/) { stop = 1; print "" }
      else { print held }
      held = ""
    }
    { print (stop ? "" : $0) }
    END { if (held != "") print held }
  ' "$file" > "$shipped/$file"
done
sources=$(printf '%s\n' $sources | sed "s|^|$shipped/|")
strip_prefix() { sed "s|$shipped/||"; }

# 1. The removed capability's name, in any case, unless the SAME PHRASE is
#    marked legacy. Matching a bare "legacy" anywhere on the line is too loose:
#    `const LEGACY_LABEL: &str = "Computer Controller"` would silence itself.
offenders=$(printf '%s\n' "$sources" | xargs grep -ni 'computer controller' 2>/dev/null \
  | grep -viE 'legacy[[:space:]]+computer controller' || true)
if [ -n "$offenders" ]; then
  echo "Computer Use naming: 'Computer Controller' is the REMOVED capability's name." >&2
  echo "Use 'Computer Use', or mark the line legacy if it is migration copy:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 2. Vendor-qualified spellings. The capability is Biorouter's, whoever supplies
#    the helper underneath it.
offenders=$(printf '%s\n' "$sources" | xargs grep -ni 'Codex Computer Use\|OCU Computer Use' 2>/dev/null || true)
if [ -n "$offenders" ]; then
  echo "Computer Use naming: the capability is 'Computer Use', never vendor-qualified:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 3. Upstream's name presented as ours. Attribution lives in the NOTICE file and
#    under third_party/, both excluded above.
offenders=$(printf '%s\n' "$sources" | xargs grep -n 'Open Computer Use' 2>/dev/null \
  | grep -vi 'includes\|upstream\|third.party\|attribut\|pinned\|based on' || true)
if [ -n "$offenders" ]; then
  echo "Computer Use naming: 'Open Computer Use' is upstream's name; attribute it, do not adopt it:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 4. The pairing that keeps the internal key legible: the `computercontroller`
#    row must carry the user-facing label, so the identifier never surfaces on
#    its own. Asserted as a PAIR -- a bare label grep passes even if the row is
#    renamed away, which is the only thing this rule exists to catch.
capabilities=ui/desktop/src/components/settings/capabilities/capabilities.ts
if [ ! -f "$capabilities" ]; then
  echo "Computer Use naming: $capabilities not found -- has it moved?" >&2
  status=1
elif ! grep -A4 "key: 'computercontroller'" "$capabilities" | grep -qE "label: ['\"]Computer Use['\"]"; then
  echo "Computer Use naming: the 'computercontroller' capability row does not carry the" >&2
  echo "label 'Computer Use', so the internal identifier can reach the user." >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "Computer Use naming is consistent"
fi
exit "$status"
