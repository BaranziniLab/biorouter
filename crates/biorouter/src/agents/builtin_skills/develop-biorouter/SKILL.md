---
name: develop-biorouter
description: "Guide for working on the Biorouter application itself: the Rust workspace and Electron/React desktop layout, where the agent loop, providers, extensions, sessions and UI code live, the build/test/lint commands, the OpenAPI regeneration step that server changes require, and the checks CI gates on. Load this skill whenever the user wants to build, change, debug, test, or review Biorouter's own source code, as opposed to authoring a skill (develop-biorouter-skill) or packaging an extension (develop-biorouter-extension)."
user-invocable: true
---

# Developing Biorouter

This is the orientation for changing **Biorouter itself**: the Rust workspace in
`crates/` and the Electron + React desktop app in `ui/desktop/`. It is not about
writing a skill (see `develop-biorouter-skill`) or packaging a `.brxt` extension
(see `develop-biorouter-extension`), both of which live outside the app's source
tree.

> Read the repository's own steering files first when they exist:
> `CLAUDE.md`, `AGENTS.md` and `.biorouterhints` at the repo root, plus
> `CONTRIBUTING.md`. They carry invariants that were each learned from a real
> failure, and they win over anything here if they disagree.

## Activate the toolchain before anything else

```bash
source bin/activate-hermit    # puts the pinned Rust and Node 24 on PATH
```

Hermit supplies both toolchains. `rust-toolchain.toml` pins the Rust channel and
`ui/desktop/package.json` declares `"engines": { "node": "^24.0.0" }`. A newer
Node is not a substitute: packaging silently produces no `.app`, and the macOS
dmg maker's native modules only build under Node 24. Symptoms of the wrong Node
read as application bugs, not version problems.

After a fresh clone, install the frontend dependencies once:

```bash
just install-deps             # npm ci in ui/desktop (landing/video is a second,
                              # unrelated JS tree with its own install)
```

## Repository layout

| Path | What it is |
|---|---|
| `crates/biorouter` | Core agent library: agent loop, LLM providers, extension manager, session state, workflows, scheduler, privacy, context management |
| `crates/biorouter-server` | Axum REST + WebSocket server, binary `biorouterd`; routes in `src/routes/` |
| `crates/biorouter-cli` | Interactive CLI, binary `biorouter`; subcommands in `src/commands/` |
| `crates/biorouter-mcp` | Built-in MCP servers (developer, computer controller, memory, auto visualiser, knowledge, agent drafter, datasql, files, compute) |
| `crates/biorouter-sandbox` | Capability-scoped sandboxed execution (docker, seatbelt, local) |
| `crates/biorouter-acp` | Agent Communication Protocol for multi-agent orchestration |
| `crates/biorouter-authprompt` | macOS auth prompt helper the background service cannot raise itself |
| `crates/biorouter-bench`, `crates/biorouter-test` | Benchmarks and integration test harness |
| `ui/desktop` | Electron main process, preload bridge, React renderer |
| `docs/` | All prose documentation. There is no other documentation folder |
| `landing/` | The public site at biorouter.ucsf.edu, deployed from this repo |
| `Justfile` | Every build, run, package and check task |

The workspace is `members = ["crates/*"]`, and every crate inherits
`version.workspace = true`, so the three Rust binaries can never disagree on a
version at build time.

## Where the interesting code lives

Inside `crates/biorouter/src/`:

- `agents/agent.rs` is the main loop: prompt assembly, LLM call, tool dispatch,
  context management. Start here for anything about how a turn runs.
- `agents/extension_manager.rs` owns MCP extension lifecycle and tool
  registration. `dispatch_tool_call` is the single choke point every tool call
  passes through, which is why enforcement gates sit on it.
- `agents/*_extension.rs` are the platform extensions compiled into the agent
  (skills, todo, chat recall, code execution, workspace, extension manager).
- `agents/subagent_tool.rs` plus `agents/subagent_execution_tool/` implement
  delegation.
- `providers/` holds one module per LLM provider; `base.rs` defines the trait
  every provider implements and `factory.rs` constructs them by name.
- `session/` persists sessions to SQLite through sqlx.
- `workflow/` parses and executes workflow YAML/JSON with minijinja templating.
- `context_mgmt/` does token counting and context-window pruning.
- `security/` holds permission modes and `.biorouterignore` handling;
  `privacy/` holds the tier lattices and their enforcement gates.
- `knowledge/` re-exports the knowledge base service that lives in
  `biorouter-mcp`.
- `prompts/` holds the system prompt templates.

Inside `ui/desktop/src/`:

- `main.ts` is the Electron main process: it spawns `biorouterd`, manages
  windows and owns IPC. `preload.ts` is the renderer bridge.
- `api/` is the TypeScript client generated from the OpenAPI spec. **Never
  hand-edit it**, and never hand-edit `ui/desktop/openapi.json` either.
- `components/` holds the React UI, one folder or file per surface; chat lives
  in `BaseChat.tsx` and `ChatInput.tsx`, settings under `components/settings/`.
- `styles/main.css` carries the generated theme token blocks and the layout
  tokens; `themes/<id>.theme.mjs` is the single source for a theme family and
  `npm run themes` regenerates everything derived from it.

Entry points worth bookmarking: `crates/biorouter-cli/src/main.rs`,
`crates/biorouter-server/src/main.rs`, `ui/desktop/src/main.ts`, and
`crates/biorouter/src/agents/agent.rs`.

## Where a new feature goes

Implement non-trivial behavior **in the `biorouter` crate**, then expose it
twice:

1. **CLI:** add or extend a subcommand in `crates/biorouter-cli/src/commands/`
   that calls the library.
2. **Desktop:** add a route under `crates/biorouter-server/src/routes/`, run
   `just generate-openapi`, then call the generated client from TypeScript.

Do not reimplement logic in the CLI or the server. Both are thin surfaces over
the same library, and a behavior that exists in only one of them is a bug
report waiting to happen.

A new built-in MCP tool belongs in `crates/biorouter-mcp/src/<server>/`. A new
platform tool that the agent should always have belongs in
`crates/biorouter/src/agents/` beside the other `*_extension.rs` modules.

## Everyday commands

```bash
# Build
cargo check                       # fastest signal that it compiles
cargo build                       # debug binaries in target/debug/
cargo build --release             # release binaries in target/release/

# Run
just run-dev                      # debug backend + launch the desktop GUI
just run-ui                       # release backend + launch the desktop GUI
just run-ui-only                  # frontend only, against an already built backend
just run-server                   # biorouterd alone
just debug-server                 # biorouterd with secret=test, pairs with `just debug-ui`
./target/debug/biorouter session  # the CLI against your local build

# Test
cargo test                                   # the whole workspace
cargo test -p biorouter-mcp                  # one crate
cargo test -p biorouter --lib -- <filter>    # one module's unit tests
cargo test -p biorouter-server --test <name> # one integration binary
cd ui/desktop && npm run test:run            # vitest, single pass
cd ui/desktop && npm run test-e2e            # Playwright

# Quality
cargo fmt
./scripts/clippy-lint.sh
cd ui/desktop && npm run lint:check
just check-everything             # every style/lint gate in one command
```

`just check-everything` is the single precommit entry point for **style and
lint** — fmt, clippy, `npm run lint:check`, the OpenAPI schema check, and the
version, brand, cross-drift and BAAM-registry checks. It runs **no tests**. Run
`cargo test` and `npm run test:run` yourself as well before you claim a change
is done.

`npm test` with no arguments is **watch mode and never exits**. Use
`npm run test:run` in any non-interactive context.

## The seam that bites: server changes need regenerated clients

After touching anything under `crates/biorouter-server/src/routes/`:

```bash
just generate-openapi
```

That runs the `generate_schema` binary to rewrite `ui/desktop/openapi.json` and
then regenerates `ui/desktop/src/api/`. Skipping it leaves the TypeScript client
calling an endpoint shape that no longer exists, and
`./scripts/check-openapi-schema.sh` (part of `just check-everything`) fails the
build.

## Testing expectations

- **Put tests where the thing lives.** Unit tests go in a `#[cfg(test)] mod
  tests` beside the code; cross-cutting tests go in the crate's `tests/` folder.
  Integration binaries are per crate, so a test that exercises the server is
  named under `-p biorouter-server`, not under `-p biorouter`.
- **Run the crate you touched, then the workspace.** `cargo test -p <crate>` for
  the fast loop, `cargo test` before you claim it is done.
- **`npm run test:run` is required to merge.** It is the `Unit tests (vitest)`
  status check and it runs on every pull request whether or not you touched
  `ui/desktop`.
- **A test must fail against a plausible wrong implementation.** A test that
  passes whether or not the change is present is worse than no test, because it
  reads as coverage. Verify a new test fails before your fix and passes after.
- **jsdom cannot see layout, Tailwind, or `:has()`.** Anything about geometry,
  generated utility classes, drag regions or computed color has to be asserted
  at the source (read the CSS or the token file in the test) or checked in a
  real browser. A component test that reads `getComputedStyle` in jsdom passes
  whether the rule exists or not.
- **Do not weaken a failing guard to make it green.** Several tests in this repo
  assert counts or call-site counts deliberately, so that adding a second call
  site forces a conversation. If one fires, the usual right answer is to change
  the code, not the number.
- **Measure counts, never estimate them.** If you write "N tests" in a comment
  or a doc, run the suite and read the number. A stale count turns a "pre + N"
  assertion into a silent pass.

## Conventions CI enforces

- `cargo fmt` and `./scripts/clippy-lint.sh` must both be clean.
- `npm run lint:check` is typecheck plus eslint with zero warnings plus the
  theme codegen, contrast and token checks.
- **Never hand-edit a version file.** Six files move together; use
  `scripts/release.sh bump <version>` and let `just check-versions` verify it.
- **All prose documentation goes under `docs/`.** Do not create `proposals/`,
  `plans/`, `notes/` or a stray markdown file at the repo root; the root list is
  closed. Every doc opens with the context header and closes with
  `## Related documentation` (see `docs/contributing/documentation-style.md`).
- The user-facing brand spelling is **Biorouter**, capital B and lowercase r.
  `./scripts/check-brand-consistency.sh` enforces it.
- **Commit messages** in this repo reject AI co-author trailers. The
  `check-commits` workflow greps every commit message in the push or pull
  request for a `Co-Authored-By:` line naming anthropic, claude, openai,
  chatgpt, gemini or copilot, and fails the job on a hit. Only commit messages
  are inspected — PR bodies are not — but do not put one there either.

## Code style the reviewers apply

- Errors are `anyhow::Result` in the library; add context only when it says
  something the underlying error does not already say.
- Prefer clear names to comments. Comment the *why* for a non-obvious decision
  or a hard-won invariant, never the *what* of self-evident code.
- Do not make a field optional that does not need to be, and let the compiler
  enforce invariants rather than writing defensive runtime checks.
- Reviews focus on security, correctness and architecture. Security-sensitive
  code (auth, permissions, credential handling), MCP protocol work and
  async/concurrency logic need human review regardless of who wrote them.

## Debugging the running app

- The desktop app spawns its own `biorouterd` on an **ephemeral port** under a
  per-launch random secret, and writes neither anywhere the CLI can read. Having
  the app open therefore does not let `biorouter session watch` see its live
  turns. For one shared process, use the external-backend setup:
  `just debug-server` plus `just debug-ui`.
- `BIOROUTER_EXTERNAL_BACKEND=true` and `BIOROUTER_EXTERNAL_PORT` point the UI
  at a backend you started yourself.
- `BIOROUTER_NO_HMR=1` freezes the renderer, so a save elsewhere in the tree
  does not reload the page and destroy the chat session under test. It also
  blinds Tailwind's class scanner, so a newly written utility class may never
  reach the stylesheet; author the CSS rule instead of relying on the scan.
- `ALPHA=true` enables alpha features. `BIOROUTER_SERVER__SECRET_KEY` sets the
  server auth key (the `__` form is only for nested config keys; the port is
  plain `BIOROUTER_PORT`).
- Verify a GUI change with a screenshot over the Chrome DevTools protocol rather
  than a full-screen capture, and read
  `docs/desktop-ui/launching-the-dev-gui.md` before launching the app from a
  shell without a TTY.

## Change checklist

```
[ ] source bin/activate-hermit before building anything
[ ] Behavior implemented in the biorouter crate, surfaced through the CLI
    and/or a server route rather than duplicated
[ ] cargo fmt clean
[ ] cargo check, then cargo test -p <crate> for what you touched
[ ] ./scripts/clippy-lint.sh clean
[ ] just generate-openapi if any server route changed (never hand-edit
    openapi.json or ui/desktop/src/api/)
[ ] cd ui/desktop && npm run test:run and npm run lint:check for UI changes
[ ] New tests fail without the change and pass with it
[ ] No guard relaxed to make a suite green
[ ] Docs, if any, under docs/ with the standard header
[ ] just check-everything before opening the pull request
```
