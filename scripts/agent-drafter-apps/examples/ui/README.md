# Agent-driven UI example apps

Each folder is a Biorouter app whose **agent drives the interface** with the
`ui_*` tools (see `crates/biorouter-mcp/src/agent_drafter/control.rs`) instead of
only writing text into a chat log.

Layout (per app):

    <id>/
      manifest.json     # agent config; `capabilities.ui` on, system_prompt says WHEN to call ui_*
      index.html        # the shell, declaring `data-br-region="…"` render targets
      src/main.ts       # app logic; `src/sdk.ts` is copied in at install time

## Install + run

    scripts/agent-drafter-apps/install-examples.sh          # copy into the local store
    biorouter apps open <id>                                # start a daemon and open the app

An app's address opens only in a browser that followed the one-time link
`biorouter apps open` (or the desktop app) hands it; the address alone answers
"Open this app from Biorouter".

## Verify

Deterministic (no LLM) — runs in CI:

    cargo test -p biorouter-mcp --test ui_example_apps

Live (drives a real agent and asserts it emitted `ui` command frames):

    BIOROUTER_SERVER__SECRET_KEY=<that daemon's secret> \
      node ui/desktop/scripts/appcheck/check-ui-app.mjs http://127.0.0.1:3000 <id> \
      "<prompt>" --expect=panel,chart

Controls that intentionally update only local presentation state must declare
`data-br-local`; the executing smoke harness otherwise requires a prompt, call,
or signal frame to reach the agent.
