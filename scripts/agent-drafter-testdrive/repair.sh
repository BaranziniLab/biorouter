#!/usr/bin/env bash
# Repair a test-drive app IN Biorouter, using the fixed platform's own agent.
#
# This is the end-to-end proof of the remediation: the same Application Drafter that
# produced these broken apps is pointed at one of them and asked to fix it — with
# no hand-editing. Everything it needs to succeed is now a tool or a rejection,
# not a paragraph of prose it can ignore:
#
#   * `list_platform_catalog` lets it SEE that `phenotype-defs` does not exist;
#   * the write boundary REJECTS the invented id if it tries to keep it;
#   * `requires` gives it an honest way to record the unmet need;
#   * `declare_surface` lets it declare `state_initial` in one typed call instead
#     of guessing at a whole-manifest rewrite; and
#   * lint tells it, by name, that its drag surface is unreachable and that
#     `br.dnd.catalog` is the fix.
#
# Usage: repair.sh <app-id>
set -euo pipefail

APP="${1:?usage: repair.sh <app-id>}"
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"

source "$ROOT/bin/activate-hermit" >/dev/null
# Provider credentials. 0600, never echoed — the daemon logs its whole spawn env.
source /tmp/br-testdrive.env

# NOTE: BIOROUTER_PATH_ROOT is deliberately NOT set.
#
# Before the Wave 0.1 path fix, `agent_drafter::default_root()` ignored
# BIOROUTER_PATH_ROOT and resolved through XDG, so this corpus lives at
# $XDG_CONFIG_HOME/biorouter/agent_drafter. Now that BIOROUTER_PATH_ROOT is honoured
# (it must be — that is the whole point of the fix, and a cross-crate test pins it to
# `biorouter::config::Paths`), setting it would point the daemon at
# <root>/config/agent_drafter instead: a different, EMPTY directory. The agent would
# see zero apps.
#
# XDG_CONFIG_HOME alone gives the same isolation and resolves identically under both
# the old and the new resolver. Anyone with an existing BIOROUTER_PATH_ROOT-based
# store must move it from <root>/config/biorouter/ to <root>/config/.
export XDG_CONFIG_HOME="$ROOT/.br-testdrive/runtime/config"
export BIOROUTER_PROVIDER=versa_azure
export BIOROUTER_MODEL=gpt-5.5-2026-04-24
export BIOROUTER_DISABLE_KEYRING=true
export BIOROUTER_ESBUILD_BIN="$ROOT/ui/desktop/node_modules/.bin/esbuild"

read -r -d '' PROMPT <<EOF || true
Repair the Biorouter app "$APP". It was authored against an older SDK and has
four defects. Fix all of them, then rebuild.

1. It configures a knowledge base that does not exist on this machine. Call
   \`list_platform_catalog\` FIRST to see what is actually installed. Then use
   \`configure_app\` to clear the invented \`knowledge_base\` (pass an empty
   string) and record the unmet need in \`requires\` instead, e.g.
   [{"kind":"knowledge_base","id":"<the id it wanted>","reason":"<why>"}].

2. Its bound elements render blank on first load because the manifest declares no
   initial shared state. Use \`declare_surface\` with \`merge: true\` to add a
   \`state_initial\` document whose keys cover every \`data-br-bind\` pointer the
   markup uses. Read the app's index.html first to find them.

3. Its drag interaction is hand-rolled HTML5 drag-and-drop, which no keyboard,
   touch, or automated pointer can drive. Replace it in src/main.ts with the
   \`br.dnd.catalog({ source, target, signal, onDrop })\` primitive, marking the
   draggable items \`data-br-item="<id>"\` and the drop zones
   \`data-br-zone="<id>"\` in index.html. Have it emit the app's declared
   \`chip_dropped\` signal.

4. Finally call \`build_app\` and then \`lint_app\`. Report the remaining findings.

Work only on this app. Do not create a new one. Use ONLY the agent_drafter tools —
do not shell out, do not read Biorouter's source, do not create symlinks. If a tool
rejects something, read the rejection: it names what is actually installed and what
to do instead.
EOF

echo "=== repairing $APP with the fixed platform's own agent ==="
/tmp/br-testdrive-target/debug/biorouter run \
  --with-builtin agent_drafter \
  --text "$PROMPT"
