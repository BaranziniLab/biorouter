#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
CLI="/tmp/br-testdrive-target/debug/biorouter"
STORE="$ROOT/.br-testdrive/runtime/config/biorouter/agent_drafter"
LOGS="$ROOT/docs/agent-drafter-testdrive-100/layout-probes/authoring-logs"

mkdir -p "$LOGS"
export BIOROUTER_PATH_ROOT="$ROOT/.br-testdrive/runtime"
export XDG_CONFIG_HOME="$ROOT/.br-testdrive/runtime/config"
export BIOROUTER_PROVIDER="versa_azure"
export BIOROUTER_MODEL="gpt-5.5-2026-04-24"
export BIOROUTER_DISABLE_KEYRING="true"
export BIOROUTER_ESBUILD_BIN="$ROOT/ui/desktop/node_modules/.bin/esbuild"
export CARGO_TARGET_DIR="/tmp/br-testdrive-target"
: "${VERSA_AZURE_API_KEY:?source /tmp/br-testdrive.env before running layout probes}"

run_probe() {
  local id="$1"
  local archetype="$2"
  local theme="$3"
  local layout="$4"
  local look="$5"
  local cli_args=(
    run
    --with-builtin agent_drafter
    -n "testdrive-$id"
    --max-turns 80
    -i -
  )
  if [[ -f "$STORE/$id/manifest.json" ]]; then
    cli_args+=(--resume)
  fi

  local prompt
  prompt="Build the controlled Application Drafter layout-diversity probe below.

Use exact app id: $id
Use exact non-chat archetype: $archetype
Use exact theme pack: $theme

You must author every file only through create_app, update_app, configure_app,
and build_app. Do not use shell/file tools. Build and fix every lint error.
Keep the app in the Application Drafter store; do not export it.

LAYOUT CONTRACT:
$layout
The app must not contain a persistent left sidebar, right sidebar, rail, or
inspector column. Do not fall back to the common left/center/right/bottom shell.
Put data-layout-probe=\"$id\" and data-layout-family=\"$archetype\" on <body> so
the audit can identify the authored geometry.

VISUAL CONTRACT:
$look
Make the hierarchy, control placement, geometry, and responsive behavior
specific to this probe, not a recolored starter.

FUNCTIONAL CONTRACT:
- kind=agentic; createApp({ autoChat:false }); any composer is secondary.
- Declare/register action activate_probe(mode:string, intensity:number).
- Declare/emit signal probe_adjusted(value:number).
- Provide a visible direct-manipulation slider that updates the primary visual
  locally and emits probe_adjusted.
- Provide one primary button that calls activate_probe and visibly changes the
  authored surface.
- Declare worker profiles layout_critic and interaction_auditor. The main agent
  must consult both, then call activate_probe and narrate the change.
- Main, both workers, and every route must use only provider versa_azure and
  model gpt-5.5-2026-04-24. Never use or mention any other provider/model.
- ui_describe once per turn, subscribe before relying on signals, no repeated
  unchanged tool calls, and only the main agent may control the UI.
- Use a useful surface.state_schema and reactive data-br-bind paths.
- Make the page responsive without turning its transient drawer/sheet/popover
  into persistent sidebars at desktop or narrow widths.

First create a valid starter without orchestration, read it, then replace the
starter composition and extend its manifest while preserving required metadata.
Finish by reporting the exact id and lint result."

  printf '%s\n' "$prompt" | "$CLI" "${cli_args[@]}" >"$LOGS/$id.log" 2>&1

  if rg -q 'The IP Address is invalid|Authentication failed.*403 Forbidden' "$LOGS/$id.log"; then
    echo "provider blocked while authoring $id" >&2
    return 75
  fi
  echo "authored $id"
}

run_probe \
  "layout-probe-kpi-mosaic" \
  "dashboard" \
  "clinical" \
  "An asymmetric full-page KPI mosaic under a top command ribbon, with a horizontal story band and a bottom narrative drawer that opens over content." \
  "Editorial clinical command center: oversized numerals, offset tile spans, white/steel ground, coral only for active state."

run_probe \
  "layout-probe-centered-wizard" \
  "wizard" \
  "journal" \
  "One centered vertical stepper in a generous page field. Progressive disclosure replaces navigation columns; completion expands into a full-width result canvas." \
  "Bookish, calm, warm paper with a strong centered reading measure and clearly different step silhouettes."

run_probe \
  "layout-probe-radial-canvas" \
  "canvas" \
  "midnight" \
  "A full-bleed radial canvas. Tools orbit the focal object as floating petals; details appear only in a modal bottom sheet and disappear when dismissed." \
  "Deep-space black, luminous rings, sparse labels, cyan/violet motion traces, no rectangular dashboard chrome."

run_probe \
  "layout-probe-tabletop-workbench" \
  "workbench" \
  "lab-notebook" \
  "A full-width dense tabletop beneath a horizontal filter ribbon. Selected-row detail expands as a bottom drawer spanning the viewport rather than a side panel." \
  "Graph-paper lab notebook, ruled row groups, ink annotations, amber selection wash, tactile drawer handle."

run_probe \
  "layout-probe-constellation" \
  "explorer" \
  "terminal" \
  "A full-bleed constellation network with a centered command-palette overlay. Node dossiers use anchored transient popovers and never reserve an inspector column." \
  "Phosphor terminal constellation: black ground, fine green vectors, compact popovers, bright keyboard-focus states."
