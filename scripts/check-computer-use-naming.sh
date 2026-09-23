#!/usr/bin/env bash
#
# The capability is called "Biorouter Copilot" wherever a person can read it. It
# is NOT "Computer Use" -- that was its previous name -- it is not "Computer
# Controller" -- that was the previous, removed capability -- and it is not
# "Codex Computer Use" or "Open Computer Use", which are upstream's.
#
# The file is still named check-computer-use-naming.sh, and the workflow step
# that runs it is still matched by that path in AGENTS.md, BUILDING*.md, the
# Justfile and .github/workflows/rust.yml. Renaming the file is a separate,
# coordinated change; renaming the capability is this one.
#
# What this check does NOT police, deliberately:
#
#   * the internal identifier `computercontroller`. It is the MCP server name
#     (`biorouter mcp computercontroller`), the builtin registry key and a config
#     key, so renaming it would break existing configurations and command lines.
#     It is already paired with the user-facing label "Biorouter Copilot"
#     (ui/desktop/src/components/settings/capabilities/capabilities.ts).
#   * the snake_case and kebab-case identifiers built on the old name:
#     `computer_use_*` error codes, the `computer-use` installed payload
#     directory, `COMPUTER_USE_*` constants, `X-Computer-Use-Key`,
#     `--computer-use-approval`, `supports_computer_use` (a vendor API field)
#     and the `computer_use::` module path. Rules 5 and 6 match the spaced,
#     Title-Case phrase only, so none of them can be caught by accident.
#   * the generic industry term in lower case ("allow computer use with
#     {model}"). Same reason: the phrase is matched case-sensitively.
#   * "BioRouter Computer Use.app", the macOS helper bundle. macOS keys
#     Accessibility and Screen Recording grants on the bundle identifier and
#     path, so renaming it would silently revoke every existing user's grants.
#   * upstream's own name. `vendor/computer-use/`, the vendored
#     `OpenComputerUse*` sources and the NOTICE file that attributes them are the
#     upstream project's identity, and the licence requires that attribution.
#   * migration copy that names an OLD name AS old, e.g. "This legacy Computer
#     Controller tool was removed" or "formerly Computer Use". Telling a user
#     what their old thing was called is the entire point of that message, so
#     the rules are narrow: an old name is allowed only when the same line marks
#     it legacy or former.
#   * historical records and migration prose. docs/history/, docs/releases/notes/
#     and the integration plan describe what the product USED to be called; a
#     release note is a record of what that version said, not a live product
#     string. This check polices code and agent-facing prompts, not historical prose.
#   * Rust test modules, `crates/*/tests/*.rs` and `*.test.ts(x)` files. One test
#     deliberately feeds "Computer Controller" as an impersonation attempt
#     (external_servers_cannot_impersonate_the_computer_use_approval_namespace);
#     failing on a test's own adversarial input would make the gate unfixable.
#     A test module is recognised by a cfg(test)-family attribute -- including
#     `#[cfg(all(test, unix))]`, which both Biorouter Copilot modules use -- that
#     is immediately followed by a `mod`. It is NOT recognised by the attribute
#     alone: in 25+ files the first one sits on a test-only `use` or `const` in
#     the import block, and stripping from there to EOF blanked entire production
#     files (agent.rs: 23,434 of 23,460 lines). The body is then blanked by
#     BRACE COUNTING rather than to EOF, because a nested test module mid-file
#     otherwise hides every production line after it.

set -euo pipefail
cd "$(dirname "$0")/.."

status=0

# Sources a person's or agent's words can reach. third_party/ is upstream,
# src/web/ is a built bundle, and *.test.* files carry adversarial input.
scratch=$(mktemp -d)
trap 'rm -rf "$scratch"' EXIT
sources=$(git ls-files 'crates/*.rs' 'ui/desktop/src/*.ts' 'ui/desktop/src/*.tsx' \
  crates/biorouter/src/prompts crates/biorouter/src/agents/builtin_skills biorouter-self-test.yaml \
  | grep -v '^third_party/' \
  | grep -v '^ui/desktop/src/web/' \
  | grep -vE '^crates/[^/]+/tests/' \
  | grep -vE '\.test\.tsx?$')
if [ -z "$sources" ]; then
  echo "Biorouter Copilot naming: no sources matched -- the gate would pass vacuously." >&2
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
    /^[[:space:]]*#\[cfg\((all\()?test[,)]/ && depth == 0 { held = $0; next }
    held != "" {
      # Only a module turns it into a test block. Anything else (a use, a const)
      # was a test-only item, so put the attribute back and carry on scanning.
      if ($0 ~ /^[[:space:]]*(pub[[:space:]]+)?mod[[:space:]]/) {
        # ⚠ Blank the module BODY, by counting braces, not everything to EOF.
        # Blanking to EOF read the FIRST test module as the last thing in the
        # file, which is the common Rust shape but not a rule. A NESTED
        # `#[cfg(test)] mod` mid-file then hid every production line after it:
        # agent.rs:609 blanked 21,609 of 23,468 lines, so the gate could not see
        # 92% of the file it exists to police, and a live model-visible string
        # put back to "Computer Use" passed it.
        inmod = 1
        depth = 0
        opened = 0
        # TWO blanks: one standing in for the held attribute line, which was
        # consumed without printing, and one for this `mod` line. One output
        # line per input line, or every reported line number shifts.
        print ""
        print ""
        body = $0
        # The brace may open on this line or a later one.
        n = gsub(/{/, "{", body); depth += n; if (n > 0) opened = 1
        n = gsub(/}/, "}", body); depth -= n
        if (opened && depth <= 0) { inmod = 0 }
        held = ""
        next
      }
      print held
      held = ""
    }
    inmod == 1 {
      body = $0
      n = gsub(/{/, "{", body); depth += n; if (n > 0) opened = 1
      n = gsub(/}/, "}", body); depth -= n
      print ""
      if (opened && depth <= 0) { inmod = 0 }
      next
    }
    { print }
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
  echo "Biorouter Copilot naming: 'Computer Controller' is the REMOVED capability's name." >&2
  echo "Use 'Biorouter Copilot', or mark the line legacy if it is migration copy:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 2. Vendor-qualified spellings. The capability is Biorouter's, whoever supplies
#    the helper underneath it.
offenders=$(printf '%s\n' "$sources" | xargs grep -ni 'Codex Computer Use\|OCU Computer Use\|Codex Copilot' 2>/dev/null || true)
if [ -n "$offenders" ]; then
  echo "Biorouter Copilot naming: the capability is 'Biorouter Copilot', never vendor-qualified:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 3. Upstream's name presented as ours. Attribution lives in the NOTICE file and
#    under third_party/, both excluded above.
offenders=$(printf '%s\n' "$sources" | xargs grep -n 'Open Computer Use' 2>/dev/null \
  | grep -vi 'includes\|upstream\|third.party\|attribut\|pinned\|based on' || true)
if [ -n "$offenders" ]; then
  echo "Biorouter Copilot naming: 'Open Computer Use' is upstream's name; attribute it, do not adopt it:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 4. The pairing that keeps the internal key legible: the `computercontroller`
#    row must carry the user-facing label, so the identifier never surfaces on
#    its own. Asserted as a PAIR -- a bare label grep passes even if the row is
#    renamed away, which is the only thing this rule exists to catch.
capabilities=ui/desktop/src/components/settings/capabilities/capabilities.ts
if [ ! -f "$capabilities" ]; then
  echo "Biorouter Copilot naming: $capabilities not found -- has it moved?" >&2
  status=1
elif ! grep -A4 "key: 'computercontroller'" "$capabilities" | grep -qE "label: ['\"]Biorouter Copilot['\"]"; then
  echo "Biorouter Copilot naming: the 'computercontroller' capability row does not carry" >&2
  echo "the label 'Biorouter Copilot', so the internal identifier can reach the user." >&2
  status=1
fi

# 5. The PREVIOUS name, still spelled as ours. Matched case-sensitively and with
#    a space, so `computer_use_*`, `computer-use`, `COMPUTER_USE_*`,
#    `X-Computer-Use-Key` and the lower-case generic term are all out of scope by
#    construction -- see the header for why each of those stays. What remains is
#    the exemption list, and each entry is a literal that cannot move:
#      * "Open Computer Use" and "Codex Computer Use": rules 2 and 3 own them.
#      * "BioRouter Computer Use.app" (and the bare bundle name it is built
#        from): macOS grants are keyed on it.
#      * the name marked as former, by a marker word ADJACENT to it.
#        ⚠ The marker must sit next to the phrase, not merely somewhere on the
#        line. Matching it anywhere is the same hole rule 1 narrows itself to
#        avoid (see its comment): one incidental "legacy" or "renamed" in a long
#        sentence then excuses a live "Computer Use" elsewhere on that line, and
#        the longer the line the likelier it is to happen by accident.
offenders=$(printf '%s\n' "$sources" | xargs grep -n 'Computer Use' 2>/dev/null \
  | grep -v 'Open Computer Use' \
  | grep -v 'Codex Computer Use' \
  | grep -v 'BioRouter Computer Use' \
  | grep -viE -v '(formerly|previously|used to be|renamed|was called|legacy|before the rename)[[:space:]]+(the[[:space:]]+)?"?Computer Use|Computer Use"?[[:space:]]*[,(]?[[:space:]]*(which[[:space:]]+)?(was[[:space:]]+)?(formerly|previously|renamed|legacy)' || true)
if [ -n "$offenders" ]; then
  echo "Biorouter Copilot naming: 'Computer Use' is the capability's PREVIOUS name." >&2
  echo "Use 'Biorouter Copilot', or mark the line as naming the former name:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# 6. The one spelling. The product is "Biorouter Copilot" -- lower-case r, and
#    never "Copilot" on its own in a sentence a person reads. Only the
#    capitalisation is policed here: "Copilot" alone is a real word in this
#    repository (.github/copilot-instructions.md is GitHub's), and a rule
#    against it would fire on correct lines.
offenders=$(printf '%s\n' "$sources" | xargs grep -n 'BioRouter Copilot' 2>/dev/null || true)
if [ -n "$offenders" ]; then
  echo "Biorouter Copilot naming: the product name is 'Biorouter Copilot', lower-case r:" >&2
  printf '%s\n' "$offenders" | strip_prefix >&2
  status=1
fi

# Agent-facing prose must not teach the former name or expose the compatibility
# identifier. The self-test YAML keeps that identifier in config, so its prose
# is covered by the phrase checks above rather than this stricter rule.
prompt_sources=$(git ls-files crates/biorouter/src/prompts crates/biorouter/src/agents/builtin_skills)
offenders=$(printf '%s\n' "$prompt_sources" | xargs grep -niE 'computer[[:space:]-]*controller|open[[:space:]]+computer[[:space:]]+use' 2>/dev/null || true)
if [ -n "$offenders" ]; then
  echo "Biorouter Copilot naming: agent-facing prompts must name the capability only as Biorouter Copilot:" >&2
  printf '%s\n' "$offenders" >&2
  status=1
fi

if [ "$status" -eq 0 ]; then
  echo "Biorouter Copilot naming is consistent"
fi
exit "$status"
