# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

**BioRouter** is an AI-powered integrated research environment for biomedical discovery built by UCSF's Baranzini Lab. It unifies multiple LLM providers, AI agents, MCP-based extensions, and customizable workflows into a single extensible tool. The architecture has three layers: Interface (Electron GUI or CLI) → Agent (reasoning loop with session state) → Extensions (pluggable MCP servers providing tools).

## Tech Stack

- **Backend:** Rust workspace (`crates/`) with Rust 1.92 (see `rust-toolchain.toml`)
- **Frontend:** Electron + React 19 + TypeScript, built with Vite and packaged via Electron Forge
- **Task runner:** `just` (see `Justfile` for all available tasks)
- **Node requirement:** Node 24+

## Key Commands

### Development

```bash
source bin/activate-hermit      # Activate hermit environment (run first)
cargo build                     # Debug build of Rust backend
just install-deps               # Install npm/Yarn deps (run once)
just run-ui                     # Build backend + frontend and launch GUI
just run-dev                    # Build debug (not release) backend + launch GUI
just run-server                 # Run REST API server only (biorouterd)
just run-ui-only                # Run frontend without rebuilding backend
just debug-ui                   # Run frontend against external backend
just debug-server               # Run server with secret=test (pairs with debug-ui)
just debug-ui-main-process      # Run UI with Chrome DevTools on localhost:9229
```

### Testing

```bash
cargo test                                      # Run all Rust tests
cargo test -p biorouter-mcp                     # Run tests for a single crate
cargo test --test mcp_integration_test          # Run MCP integration tests
just record-mcp-tests                           # Re-record MCP test cassettes
cd ui/desktop && npm run test:run               # Run frontend unit tests (Vitest)
cd ui/desktop && npm run test-e2e               # Run Playwright E2E tests
```

### Code Quality

```bash
cargo fmt                           # Format Rust code
./scripts/clippy-lint.sh            # Run clippy linter
cd ui/desktop && npm run lint:check   # tsc + ESLint + themes/contrast/tokens — NOT Prettier
cd ui/desktop && npm run format:check # Prettier; nothing else runs it, CI included
just check-everything               # Run all style/lint checks
```

⚠ **Frontend formatting is not enforced anywhere.** `lint:check` resolves to
`typecheck && eslint && check:themes && check:contrast && check:tokens` — no
Prettier — and no workflow under `.github/workflows/` invokes Prettier either.
Two things look like they do and do not: `just check-everything` prints
"Checking UI code formatting..." and then runs `lint:check`; and the
`lint-staged` block in `ui/desktop/package.json` runs `prettier --write` but
nothing triggers it — `prepare` runs `husky` from `ui/desktop`, husky 9 exits
early when the current directory has no `.git` (`npm ci` prints `.git can't be
found`), and `.githooks/` holds only `commit-msg`. This line used to claim
`lint:check` was an "ESLint + Prettier check"; it never was, and the drift is
measurable: five files under `ui/desktop/src` failed `format:check` on `main`
on 2026-09-02, and four on 2026-09-11. Those four were reformatted that day
(formatting only), after which `format:check` passed with **0 failing files,
measured 2026-09-11**. That meets the precondition this note used to set for
wiring `format:check` into `lint:check` or CI — whether to do it is a
maintainer's call — but nothing holds the count at zero, so re-measure right
before wiring it in: a gate that fails on arrival gets disabled rather than
obeyed. Until then, run `format:check` yourself before pushing frontend
changes.

### Build & Release

```bash
just release-binary     # macOS ARM64 release build
just release-intel      # macOS Intel release build
just make-ui            # Package Electron app (macOS ARM64)
just make-ui-linux      # Package for Linux (requires Docker)
just make-ui-windows    # Package for Windows (requires Docker)
just generate-openapi   # Regenerate OpenAPI spec from server routes
```

**Cargo build profiles** (defined in the root `Cargo.toml`):
- `release` — the default for `cargo build --release`, the Justfile, and the whole
  `scripts/release.sh` pipeline. Now sets `strip = true`, which removes the symbol
  table from shipped binaries (~13% smaller: biorouterd 124→108 MB, biorouter
  138→120 MB) with no runtime change. Debug info was already off. Every shipped
  artifact benefits automatically — no pipeline change needed.
- `release-dist` — `cargo build --profile release-dist`. Inherits `release` (so
  stripped) and adds thin LTO + 16 codegen-units for the smallest/fastest binary.
  Slower to compile, so it is opt-in. Intended as the final distribution profile;
  wiring `just release-binary`/`scripts/release.sh` to it is a safe follow-up once
  the macOS notarized packaging path has had a smoke test. **Measured 2026-08-20:
  still nothing invokes it** — `just release-binary` and all four `cargo build`
  lines in `scripts/release.sh` pass a plain `--release`. The comment in
  `Cargo.toml` claimed otherwise until this date; grep before believing either.
- `quick` — `cargo build --profile quick`. opt-level 1 + max codegen-units +
  incremental, for fast iteration on optimized builds. Everyday dev still uses the
  debug `cargo build` / `cargo check`.

### Releasing (cross-platform)

**Automated path (preferred): `scripts/release.sh` and the `release` workflow.**
The pipeline — version bump → compile all 4 backends → sign + **notarize** both
macOS dmgs → package Windows zip + Linux deb/rpm + CLI-only deb/rpm →
verify → **draft** the GitHub release — is encoded in
[`scripts/release.sh`](scripts/release.sh). Note where it stops: `all` ends at a
*draft*, not a published release, because publication is gated on a native
Windows smoke run (see the Publish bullet below). It bakes in every
hard-won invariant below (Node-24 dmg maker, winpthread + `LZMA_API_STATIC`
cross-compile fixes, one-platform-at-a-time staging, Linux-last node_modules
order, auto-installing the `appdmg` dmg dep, notarization creds read from
`notarization/APPLE_DEVELOPER_NOTES.md`).

```bash
scripts/release.sh all 1.80.1          # every build/verify phase, ending at the draft
scripts/release.sh all patch           # same, version resolved from the current Cargo version
# or one phase at a time (resumable):
scripts/release.sh bump 1.80.1
scripts/release.sh bump patch          # major | minor | patch also accepted
scripts/release.sh backends 1.80.1     # mac arm64/x64 + windows + linux (docker)
scripts/release.sh linux-backend 1.80.1 # just the linux x86_64 backend (re-runnable)
scripts/release.sh mac-arm64 1.80.1    # sign + notarize
scripts/release.sh mac-intel 1.80.1
scripts/release.sh windows 1.80.1
scripts/release.sh linux 1.80.1        # GUI deb + rpm; run LAST (corrupts node_modules)
scripts/release.sh cli-linux 1.80.1    # CLI-only deb + rpm, no GUI (biorouter + biorouterd + the web bundle)
scripts/release.sh mac-manifest 1.80.1 # latest-mac.yml for electron-updater
scripts/release.sh verify 1.80.1
scripts/release.sh draft 1.80.1        # draft GitHub release with all 10 assets + notes
scripts/release.sh publish 1.80.1      # flip the draft live (gated on the Windows smoke run)
```

**Version arguments** (`bump` and `all` only):
- `major` / `minor` / `patch` are resolved against the current `Cargo.toml`
  version. `minor-minor` is an accepted alias for `patch`.
- A bump that would move the version **backwards** dies with *"refusing to bump
  BACKWARDS"* — electron-updater compares versions, so a regression would strand
  clients. Pass the literal version again if the move is deliberate.
- The keywords are rejected on every phase after `bump`, which needs the explicit
  version the tree is **already at**: `bump` has rewritten the version, so `minor`
  would now resolve against the new value and name a release that does not exist.

For an agent-orchestrated run (each phase as a verified subagent that stops on
the first failure), use the **`release` workflow** in
[`.claude/workflows/release.js`](.claude/workflows/release.js):
`Workflow({ name: 'release', args: { version: '1.80.1' } })`. After a release,
restore a mac-native node_modules: `cd ui/desktop && rm -rf node_modules && npm ci` (**`ci`, not
`install`** — see the bullet below).

The detailed manual steps and the reasoning behind each invariant follow.

- **Version bump**: edit 6 files — `Cargo.toml`, `ui/desktop/package.json`, `ui/desktop/package-lock.json` (2 occurrences), `ui/desktop/openapi.json`, and `README.md` (the badge URL `badge/version-X.Y.Z-tan.svg` **and** the `alt="Version X.Y.Z"` text — two substitutions). Then `cargo check` to refresh `Cargo.lock`. (`scripts/release.sh bump <ver>` does all of it.)
- **One version, three binaries (CLI = daemon = GUI)**: the version lives in exactly one source of truth — `[workspace.package].version` in `Cargo.toml`. The CLI (`biorouter`), the daemon (`biorouterd`), and the core library all use `version.workspace = true`, so the three Rust binaries can **never** disagree at build time — they are compiled from the same workspace version (surfaced via `env!("CARGO_PKG_VERSION")`). The desktop GUI keeps its own copy in `ui/desktop/package.json` (+ two in `package-lock.json`, one in `openapi.json`), and `README.md` carries a version badge; `release.sh bump` rewrites all of them in lockstep. `scripts/check-version-consistency.sh` (run by `just check-everything` / `just check-versions`) is the guard that fails CI if any desktop JSON **or the README badge** drifts from the Cargo version, or if a crate ever hardcodes its own `version` instead of inheriting the workspace one. The badge was brought under the tooling late, and the reason is instructive: unguarded, it had silently drifted to 1.87.2 while the tree was on 1.88.6 — three releases stale on the first thing a repo visitor reads. **Do not hand-edit a single version file** — always use `scripts/release.sh bump <ver>` so all six stay in sync.
- **Runtime CLI-vs-app drift** (the "Biorouter CLI Update 1.20.0 → 1.85.1" prompt): the GUI bundles a `biorouterd` that always matches the app, but the user's terminal `biorouter` is a *separately installed* binary that can lag. This is an install-state mismatch, not a source-version bug — the in-app "Biorouter CLI Update" card re-installs/symlinks the matching CLI (in a dev tree it points `~/.local/bin/biorouter` at `target/debug/biorouter`). The source versions are already linked; only the on-PATH install can be stale.
- **macOS dmg maker needs hermit's Node *and* hermit's npm**: the `macos-alias` / `appdmg` native modules only build under hermit's Node (v24), not a newer Homebrew Node — run all packaging under `source bin/activate-hermit`. ⚠ **The npm version matters as much as the node version**, which this line said nothing about until a 1.89.5 phase died on it: `appdmg` is an *optional* dependency of `electron-installer-dmg`, npm skips an optional dep whose install fails rather than failing the install, and a newer npm blocks native install scripts by default — so a **green `npm ci` does not imply `appdmg` is present**. Check `npm --version` is hermit's (11.x), not the system npm. If the dmg maker dies with `Cannot find module 'appdmg'` or a `NODE_MODULE_VERSION` mismatch: `(cd ui/desktop && npm ci && npm rebuild macos-alias ds-store)` — **`ci`, not `install`**, for the same lockfile reason as the bullet below; this line recommended `install` for a long time and was wrong.
- **Cross-compile link fixes** (windows-gnu / linux-gnu, in the Justfile + `release.sh`): `aws-lc-sys` needs winpthread appended *after* the rlibs on the mingw link line (linker wrapper); `lzma-sys` (via `xz2`, the `.brkb` path) needs `LZMA_API_STATIC=1` so it statically builds bundled liblzma instead of the host one. Run the docker cross builds with the system docker (hermit does **not** shadow it).
- **macOS sign + notarize**: set `APPLE_ID` and `APPLE_APP_SPECIFIC_PASSWORD` on the `npm run bundle:default` / `bundle:intel` invocation. Signing identity is the UCSF Developer ID Application (team `F3YYBXAFJ8`).
- **Intel macOS requires `just release-intel` first**. `bundle:intel` does NOT cross-compile the Rust backend — it repackages whatever is in `ui/desktop/src/bin/`. Without `target/x86_64-apple-darwin/release/{biorouter,biorouterd}`, `prepare-platform-binaries.js` falls through to the arm64 build and ships an Intel dmg that crashes on Intel Macs with "bad CPU type." Always run `just release-intel` (or have a recent `target/x86_64-apple-darwin/release/` build) immediately before `npm run bundle:intel`. Verify with `file ui/desktop/out/Biorouter-darwin-x64/Biorouter.app/Contents/Resources/bin/biorouter` — must say `x86_64`, not `arm64`. Same rule applies symmetrically: `bundle:default` needs `target/release/` to be the arm64 build (`just release-binary` or `just copy-binary`).
- **Build platforms one at a time** — every bundle writes to `ui/desktop/src/bin/` and clobbers the others. After any non-mac build, run `just release-binary` (or `just copy-binary`) to restore the local arm64 binary.
- **After Linux/Windows Docker builds**, the on-disk `ui/desktop/node_modules` is Linux-flavored — the macOS bundle then fails with `@rollup/rollup-darwin-arm64` missing, and so does `npx vitest`, with a `MODULE_NOT_FOUND` inside `rollup/dist/native.js` that looks nothing like a platform problem. Fix: `cd ui/desktop && rm -rf node_modules && npm ci`.
  ⚠ **`npm ci`, never `npm install`.** `install` rewrites `package-lock.json`, and the next cross build runs `npm ci` inside the container, which refuses a lockfile that disagrees with `package.json` — so the quick fix here breaks the next Linux/Windows build instead. This line said `npm install` for a long time; it was wrong.
- **`macos-alias` `NODE_MODULE_VERSION` mismatch** during forge `make`: `cd ui/desktop && npm rebuild macos-alias`.
- **Unmount any stale `/Volumes/Biorouter*` mounts before the dmg step** — leftover mounts cause `cp: Operation not permitted` and abort `electron-forge maker-dmg`.
- **Do not hand-roll the dmg via `hdiutil create`** — it skips the `Applications` symlink and the background-image layout that `electron-forge maker-dmg` adds. If `bundle:default` fails at the dmg step, fix the underlying cause (usually a stale `/Volumes` mount) and re-run, don't `hdiutil` over it.
- **Release assets — exactly 10**: the 5 GUI artifacts `Biorouter-{ver}-arm64.dmg`, `Biorouter-{ver}-x64.dmg`, `biorouter_{ver}_amd64.deb`, `Biorouter-{ver}-1.x86_64.rpm`, `Biorouter-win32-x64-{ver}.zip`; the 2 **CLI-only** Linux packages (no GUI) `biorouter-cli_{ver}_amd64.deb` and `biorouter-cli-{ver}-1.x86_64.rpm` (`biorouter` + `biorouterd` + the web interface at `/usr/share/biorouter/web`, built by `scripts/build-cli-linux-packages.sh` → `dist/cli/`, smoke-tested in clean Debian/Rocky containers); and the 3 **macOS auto-update** artifacts `Biorouter-darwin-arm64-{ver}.zip`, `Biorouter-darwin-x64-{ver}.zip`, and `latest-mac.yml`. The two darwin zips are the signed+notarized `maker-zip` app archives (Squirrel.Mac format, `Biorouter.app` at root); `latest-mac.yml` (generated by `ui/desktop/scripts/generate-update-manifests.js`, run from the `mac-manifest` phase, which `draft` invokes) lists both with base64 SHA-512 + size so `electron-updater` can do the in-app one-click "Restart & Update" on macOS — **without the zips + yml, clients 404 and drop to the assisted GitHub-download fallback**. electron-updater 6.x picks the arch by the `arm64`/`x64` token in the zip filename, so both clients share one manifest. Don't also upload the unversioned `Biorouter.zip` / `Biorouter_intel_mac.zip` from `out/<platform>/` — they're build intermediates, not release artifacts. Windows (plain zip) and Linux (deb/rpm) have no electron-updater in-place installer and keep using the assisted-download fallback. See `docs/releases/auto-update-test-checklist.md`.
- **`verify` checks more than file existence**: alongside the 9 on-disk artifacts it runs `scripts/check-brand-consistency.sh` and `scripts/smoke-test-release-artifacts.sh` (whose `smoke_serve` installs the CLI package, runs `biorouter serve` and drives the whole browser contract), and fails the phase if any of them does. `just check-everything` runs the brand check too — it asserts `"productName": "Biorouter"` and the BR-monogram brand assets, which is why every packaged artifact name above is `Biorouter`, lowercase `r`.
- **Linux glibc baseline**: the linux backend is cross-compiled on `rust:1.92-bullseye` (glibc 2.31), NOT rolling `rust:latest` (now trixie, glibc 2.39, which yields binaries that fail to start on Debian 12 / Ubuntu 22.04 / RHEL-Rocky 9). The pin lives in `LINUX_RUST_IMG` in `scripts/release.sh`. ⚠ **This line used to say `cli-linux`'s smoke test catches a regression here. It cannot, and that was measured on 2026-08-20**: those containers are `debian:bookworm` (glibc 2.36) and `rockylinux:9` (glibc 2.34), so a floor raised from 2.31 to 2.34 passes every smoke test in the repo and breaks only on the user's older machine. What actually enforces it is `assert_glibc_floor` in `scripts/release.sh`, called from `cmd_linux-backend` — a symbol-table read against `GLIBC_MAX`, which also fails when it can read no symbol at all rather than passing vacuously. `linux-backend` `rm -rf`s the target dir first to force a from-scratch compile against the pinned glibc (cached objects keep stale symbol versions).
- ⚠ **A draft pins the commit the tag will get, so DRAFT LAST.** `draft` runs
  `gh release create v{ver} --draft --target main`, which records
  `targetCommitish` = whatever `main` is *then*. **No tag exists yet** — a draft
  shows as `untagged-…` — and `publish` creates the tag from that recorded
  target, not from today's `main`. So a release that is drafted, then fixed,
  then **rebuilt and re-uploaded** ships artifacts from a commit its tag has
  never contained. Measured on v1.89.8: drafted 06:35 at `2889ad52`, published
  18:56 from artifacts built at `97140878` — 8 commits apart, and the gap held
  the provider credential fixes the release existed to deliver. Nothing catches
  it: `verify` compares the provenance `source_sha` against `HEAD`, which is the
  right check for the build and silent about the tag.
  ⚠ **Before publishing, check the draft's TARGET, not the tag** — `git
  ls-remote --tags` is empty for a draft and reads as a mismatch when nothing is
  wrong:
  ```bash
  gh release view v<ver> --json targetCommitish --jq .targetCommitish
  awk -F'\t' '$1=="source_sha"{print $2}' dist/release-build-<ver>.tsv
  ```
- **Publishing is two steps, and the gate between them is not optional.**
  - `scripts/release.sh draft <ver>` runs `mac-manifest`, asserts all 10 assets exist (dying on the first missing one), then `gh release create v{ver} --draft --target main --title "Biorouter v{ver}" --notes-file docs/releases/notes/v{ver}.md <10 assets>`.
  - `scripts/release.sh publish <ver>` re-runs `verify`, refuses unless `v{ver}` already exists **as a draft**, refuses unless `gh run list --workflow release-artifact-smoke.yml` shows a successful run titled `Release artifact smoke v{ver}`, and only then `gh release edit v{ver} --draft=false`.
  - So `all` stops at the draft and prints *"draft created; run the native Windows smoke workflow, then: scripts/release.sh publish {ver}"*. Hand-rolling a `gh release create` bypasses the Windows smoke gate — don't. Verify macOS with `xcrun stapler validate <app>` and `codesign -dv <app>` as well.
- **Release notes** live at `docs/releases/notes/v{ver}.md`, one file per version.

## Architecture

### Rust Workspace (`crates/`)

| Crate | Binary | Purpose |
|-------|--------|---------|
| `biorouter` | — | Core agent library: main agent loop, LLM providers, MCP extension manager, session/conversation state, workflow execution, scheduling |
| `biorouter-server` | `biorouterd` | Axum REST API + WebSocket server; routes in `src/routes/`; OpenAPI spec generated via utoipa |
| `biorouter-cli` | `biorouter` | Interactive CLI; subcommands in `src/commands/` |
| `biorouter-mcp` | — | Built-in MCP servers (Developer, Computer Controller, Memory, Auto Visualiser, Knowledge, Agent Drafter, DataSQL, Files, Compute). Also hosts `active_work.rs`, which is *not* a server but the process-global registry of long-running work (background shell jobs + running subagents) that `GET /active_work` reads |
| `biorouter-sandbox` | — | Capability-scoped sandboxed execution (`docker.rs`, `seatbelt.rs`, `local.rs`, `environment.rs`, `shell_sandbox/`); a leaf crate with no engine deps |
| `biorouter-acp` | — | Agent Communication Protocol for multi-agent orchestration |
| `biorouter-bench` | — | Benchmarking harness |
| `biorouter-test` | — | Integration tests |

Only four of `biorouter-mcp`'s servers are spawnable as **subprocesses** via
`biorouter mcp <name>` — `autovisualiser`, `computercontroller`, `developer`,
`memory` (the `McpCommand` enum in `mcp_server_runner.rs`).
`agent_drafter` (as `appcontrol`), `datasql`, `files_server` and `compute_server`
are injected **in-process** by `configure_agent` in `routes/apps.rs` via
`add_inprocess_server`, and have no subprocess name; their absence from that enum
is not dead code.

### Core Agent Library (`crates/biorouter/src/`)

- **`agents/agent.rs`** — Main agent loop: LLM interaction, tool dispatch, context management
- **`agents/extension_manager.rs`** — MCP extension lifecycle and tool registration
- **`providers/`** — 45+ provider modules (Anthropic, OpenAI, Azure, AWS Bedrock, Databricks, Ollama, etc.); `factory.rs` creates providers, `base.rs` defines the abstract interface
- **`session/`** — Session persistence (SQLite via sqlx)
- **`workflow/`** — Workflow definition (YAML/JSON), Jinja-style templating (minijinja), and execution
- **`context_mgmt/`** — Token counting (tiktoken-rs) and context window pruning
- **`security/`** — Permission modes, `.biorouterignore` handling
- **`privacy/`** — Privacy tiers (issue #56): the capability/classification lattices, the eight enforcement gates, declassification, the master switch, and institutional affiliation. See the section below.
- **`scheduler.rs`** — Cron-based job scheduling (tokio-cron-scheduler)
- **`knowledge/`** — Personal knowledge base: storage, git history, file
  conversion (HTML/PDF/DOCX/CSV), credibility classification
  (Crossref/OpenAlex), graph derivation, **macros (ingest / query / lint)
  backed by a bounded sub-agent loop**, BM25 search, per-KB concurrency
  mutex, and an active-KB state for session-scoped tool defaulting. The
  shared service backs the `knowledge` MCP extension and HTTP routes under
  `/knowledge/*` via `biorouter-server` (with SSE-streamed macros and
  `.brkb` export/import). (Module lives in `biorouter-mcp`; re-exported as
  `biorouter::knowledge`.)

  Note: the macros (`ingest`, `query`, `lint`) and the agentic credibility
  fallback take a `Box<dyn Completer>` argument rather than a `Provider`
  directly, to avoid a circular dependency on `biorouter`. The HTTP routes
  wrap a real `biorouter::providers::Provider` in a `ProviderCompleter`
  adapter (`crates/biorouter/src/knowledge/provider_completer.rs`).

### Frontend (`ui/desktop/src/`)

- **`main.ts`** — Electron main process: spawns `biorouterd`, manages windows, IPC
- **`preload.ts`** — IPC bridge between renderer and main process
- **`api/`** — TypeScript API client auto-generated from OpenAPI spec (do not hand-edit)
- **`components/`** — 75+ modular React UI components
- **`contexts/`** — React Context for global state
- **`workflow/`** — Workflow builder UI
- **`components/knowledge/`** — Top-level Knowledge route in the sidebar
  (between Skills and Settings). Provides KB selector with cmd-K-style
  palette, ingest panel (dropzone / paste text with URL extraction / staged
  list), and live SSE-streamed digestion progress via `useIngestStream`.

### Privacy tiers (issue #56)

A conversation that has touched a **private** model or a **private** data source may never reach a
model hosted outside the user's institution. Design + status:
[`docs/security/privacy-tiers.md`](docs/security/privacy-tiers.md) — **read its "What shipped, and
what did not" section first**; the rest of that document is the design, not the running system.

- **Two lattices, one crossing.** `ProviderTier` is CAPABILITY (what a session may *do*; a pure
  function of the provider bound right now, reduced with `least` over lead/worker, never stored).
  `SessionClassification` is CLASSIFICATION (how sensitive the contents are; reduced with `max` over
  time, stored in `sessions.privacy_tier`, a **permanent ratchet**). They do not interconvert —
  `privacy::floor` is the only crossing and a repo-grep test asserts its caller count. Invariant:
  `capability(S) >= classification(S)`.
- **Eight gates.** A — bind (`Agent::update_provider`). B — turn (top of `Agent::reply`;
  repair-first, and where the ratchet fires). C — dispatch
  (`ExtensionManager::dispatch_tool_call`, the one choke point every tool call passes through, plus
  its resource/prompt siblings). D — `chatrecall` SEARCH (a SQL predicate in both builders) and
  LOAD. E — discovery (`filter_tools`). F — the two extension channels that are not tool calls.
  G — cross-session conversation ingest. H — the alternate-provider construction sites.
- **`CallCapability` is sampled once per call and threaded.** A gate on a tool-call path must ask
  `cap.enforced()` / `cap.tier()`, **never** re-read `privacy_tiers_enabled()` — a second read is
  the race the type exists to close. Gates that are not on a tool-call path (bind, turn, spawn,
  search, route) read the flag directly.
- **A spawn never changes the tier** (`subagent_tool.rs`): public parent → private child is refused,
  private parent → public child is refused, and a public child silently *drops* private or
  cross-institution extensions with a note back to the parent rather than failing the spawn.
- **Knowledge bases ratchet too.** A base takes the tier of the most sensitive session that wrote to
  it (four write choke points), is refused to a public caller at the read choke points, and a
  refusal names what it refused. `biorouter-mcp/src/knowledge/tier*.rs`.
- **Affiliation is a third axis** (DR-26, plan Phase 6): tier asks *how sensitive*, affiliation asks
  *whose*. HIPAA compliance does not transfer between institutions, so a UCSF model reaching another
  institution's private connector is warned/refused even though both endpoints are Private.
- **The master switch** lives in its own record beside `config.yaml`, **not in it** and **not in an
  env var** — the agent has `developer__shell`, so a switch it can edit is not a switch. Loaded once
  per process; a load error resolves to ON.
- **Lineage is NOT a boundary; the tier is the only one.** `may_write` ⇔ `may_read` ⇔ `VIS`, so an
  agent may inject a prompt into any conversation it can see — a child, a sibling, an unrelated
  chat. R6's old "steer what you spawned, read everything else" rule is retired and
  `Lineage`/`lineage_of` are deleted. Two *other* one-hop rules survive and are constantly
  confused with it: `McpMeta::workspace_child_scope_only` (which confines an auto-injected
  supervision grant's read/close/watch to direct children) and the flat refusal of every
  `workspace_*` tool to a `SessionType::SubAgent`. A private→public write raises a
  **first-crossing approval showing the payload**, once per (caller, target) pair, in every
  permission mode — `agents/workspace_inspector.rs` asks, `privacy/crossing.rs` remembers,
  `handle_send_prompt`/`handle_set_tools` record only once the write lands.
- **Known gaps, do not assume otherwise.** The general filesystem read-deny (§9.5, DR-14) is
  DEFERRED — a public chat with a shell still reads ordinary files, which is why the
  non-private-model disclosure ships. ⚠ This bullet also claimed §7's cross-session matrix was
  "written but wired to nothing" and that `workspace_read_conversation` checked only
  `session_type == Hidden`; **both were already false** — the read gate is
  `visibility::refuse_unless_readable` and the write gate composes it with `may_write`. Grep the
  symbol rather than trusting a summary, including this one.
- **Tests:** `cargo test -p biorouter --lib privacy::` and
  `cargo test -p biorouter-mcp --lib knowledge::tier`, plus five integration binaries that are
  **spread across three crates** — `-p biorouter`: `--test privacy_toggle`,
  `--test privacy_capability`, `--test privacy_disclosure_toggle`; `-p biorouter-server`:
  `--test privacy_toggle_config`; `-p biorouter-mcp`: `--test privacy_toggle_export`. Naming any of
  the last two under `-p biorouter` fails with "no test target named …", because the master switch
  is exercised where each surface lives rather than from one crate. Several enforcement points are
  held in place by repo-grep assertions — if you add a second call site for `raise_privacy`, `floor`
  or `.call_tool(`, a test will tell you, and the right fix is usually not to update the count.

### Knowledge feature

The Knowledge feature (built across Plans 1-6 in `docs/history/knowledge-base-buildout/*`) provides personal, LLM-maintained knowledge bases backed by markdown trees + git history.

- **Backend module:** `crates/biorouter-mcp/src/knowledge/` (types, store, git, graph, credibility, convert/, macros/, subagent/loop_, MCP server).
- **HTTP routes:** `crates/biorouter-server/src/routes/knowledge.rs` covers `/knowledge/bases`, `/ingest` (SSE), `/graph`, `/history`, `/preview`, `/restore`, `/page`, `/active`, `/export`, `/import`.
- **Frontend:** `ui/desktop/src/components/knowledge/` (view shell, KB selector, ingest panel, force-graph + change-log drawer). The chat-side KB chip lives at `ui/desktop/src/components/bottom_menu/BottomMenuKnowledgeSelection.tsx`.
- **Storage layout:** `~/.config/biorouter/knowledge/<kb-id>/` with `raw/`, `knowledge/`, `index.md`, `log.md`, `schema.md`, and a hidden `.git/`.
- **One axis, one pointer.** A session's knowledge bases are the *visible* set — everything not in `.hidden-kbs` (machine-wide) or `.hidden-kb-sessions/<sha256(session_id)>` (per session; an empty `[]` means "hide nothing", not "inherit"). KB-less search spans this set with per-hit `kb_id` attribution. Its **primary** is the write target, default single-base read target and Knowledge view's subject. Soul is the product default when the user has expressed no preference. A missing `.active-kb` / `.active-kb-sessions/<digest>` inherits; a bare id pins a choice; a blank file explicitly chooses no primary and must not fall back to Soul. KB-less writes with that explicit no-primary state fail with the candidate list. The primary must remain visible; the daemon repairs selection when its base is hidden or deleted. `kb_set_active` changes the primary without narrowing search. Set-only edits send neither `primary_kb` nor `clear_primary`; see [`docs/knowledge-base/multi-kb-implementation-plan.md`](docs/knowledge-base/multi-kb-implementation-plan.md).
- **Sub-agent loop:** `crates/biorouter-mcp/src/knowledge/subagent/loop_.rs` drives ingest / query / lint macros. Mutating tools accept an optional `txn` so a macro's tool calls commit as one logical change.

When working on the Knowledge feature:
- Run `cargo test -p biorouter-mcp --lib knowledge::` and `cargo test -p biorouter-server --test knowledge_routes` for backend changes. A count written here must be *measured*, not approximate — a stale figure is worse than none, because a "pre + N" assertion against it reads a shortfall as a pass. The `knowledge::` figure carried here had drifted, so it was deleted rather than guessed: measure it in your own run. The `knowledge_routes` figure that used to sit here had drifted the same way (it read 38 against a suite of 58) and is gone for the same reason. The ingest stream's terminal-frame contract lives in its own binary, `cargo test -p biorouter-server --test knowledge_ingest_stream` (1 test), because it sets `BIOROUTER_KNOWLEDGE_TEST_MODE` and would race the un-mocked provider tests next door.
- After touching `routes/knowledge.rs`, regenerate the TS client with `just generate-openapi && cd ui/desktop && npm run generate-api`.
- Graph derivation lives in `graph.rs` and depends on the sub-agent emitting `[[knowledge-link]]` markers in page bodies; the default `schema_default.md` reinforces this. If a graph has nodes but no edges, the underlying pages likely lack `[[…]]` cross-references.

**The Open Knowledge Format (OKF), and BioOKF as a strict profile.** A knowledge
base now declares a `format` in its `manifest.yaml`: the informal LLM-wiki page
shape it always had, OKF v0.2 (`crates/biorouter-mcp/src/knowledge/okf/`), or
BioOKF v0.5 (`.../knowledge/biookf/`) — a *strict* biomedical profile with a
controlled vocabulary, typed domain/range constraints and its own lint. Only the
latter two can be **created** (`KbFormatChooser.tsx`; `POST /knowledge/bases`
with `format: "legacy"` is refused, and names the two that work). `legacy` is a
state a base is *found* in, never one it is put into — and it is not a migration
the user is dragged through: **a legacy manifest keeps working**, the
`format` key it gains on re-save is inert, and the migration ladder runs for a
base that has none of the stage-3 keys rather than skipping it
(`knowledge/manifest.rs` pins all three in tests). Design, decision records and
the live progress tracker: [`docs/knowledge-base/okf-migration/`](docs/knowledge-base/okf-migration/README.md).

- **Links are typed now, and the old grammar still parses.** `knowledge/links.rs`
  reads both the legacy `[[bare-slug]]` and OKF's typed edge, so the same page
  yields edges under either format. ⚠ **A reader that only understands the
  bracket grammar silently produces a graph with nodes and no edges** — that was
  a real bug in lint, and the shape it takes is exactly the "nodes but no edges"
  symptom the line above blames on missing cross-references. Check which grammar
  the reader handles before concluding the pages are at fault.
- **Two bases can be merged** (`knowledge/merge.rs`), and the merge is a write
  choke point, so the tier ratchet in `knowledge/tier*.rs` applies: the result
  takes the more sensitive of the two.
- **Lint is reachable from a chat**, not only from the Knowledge view, and a
  *private* model may run a read-only lint on its own private base — the
  read-only qualifier is what makes that safe, so do not generalise it to the
  mutating macros.

### Llama Server (bundled llama.cpp local models)

The "Llama Server" provider (`llamacpp`) gives zero-setup local models: the
desktop app bundles a pinned llama.cpp `llama-server` binary and manages it as
a sidecar process. It is ranked first among Local Models (before Ollama), and
Local ranks before Institutional/Commercial everywhere (GUI onboarding,
settings provider grid, `biorouter configure`).

- **Provider:** `crates/biorouter/src/providers/llamacpp.rs` — OpenAI-compat
  HTTP to the sidecar; curated `MODEL_CATALOG` of **Gemma 4 and Qwen3.6**
  models mirrored from the Ollama library, with Google's QAT GGUFs as the
  Hugging Face fallback. `default_model_name()` is memory-tiered:
  `gemma4-12b` at ≥64 GiB, `gemma4` (E4B) below. Unlisted models are accepted
  as raw `owner/repo:QUANT` HF specs. ⚠ This block claimed a Qwen3.5 catalog
  defaulting to `qwen3.5-4b` until 2026-08-23; there has never been a
  `qwen3.5-4b` entry — read `MODEL_CATALOG` rather than trusting a summary.
- **Sidecar manager:** `crates/biorouter/src/providers/llamacpp_sidecar.rs` —
  binary discovery (`BIOROUTER_LLAMACPP_BIN` → `<exe dir>/llamacpp/` → dev
  repo path → PATH), spawn with `-hf` (models download from Hugging Face into
  the Ollama model store on first use), `/health` readiness, status snapshots,
  restart on model switch. Defaults: port 11543, q8_0 KV cache, and a
  **memory-tiered context window** — `default_context_size()` returns 128k at
  ≥64 GiB of GPU-addressable memory, 64k at ≥16 GiB, else 32k. It is **not**
  `--ctx-size 0`: a model's native window can be 262k, and allocating that KV
  cache is slow-to-impossible on a laptop. `LLAMACPP_CONTEXT_SIZE` pins it
  (`0` means "auto"), and the window the server really allocated is read back
  from `/props` so the gauge matches reality. Thinking is **off** by default
  (`--reasoning off`), so short warm-up completions spend their budget on
  visible content rather than hidden reasoning; `LLAMACPP_ENABLE_THINKING=true`
  turns it on. `LLAMACPP_EXTRA_ARGS` for anything else, `LLAMACPP_EXTERNAL_HOST`
  to use an unmanaged server.
  **Speed:** `--spec-type ngram-simple` is passed by default —
  self-speculative decoding that drafts from the context, so it needs no draft
  model, no download and no extra VRAM. Measured on an M4 Max with
  gemma-4-E4B: **79.7 → 354.4 tok/s** on repetition-heavy agentic generation
  (quoting tool output, rewriting a file) with free text unchanged at ~82.
  `ngram-mod` is faster still but costs 2% on free text; `ngram-map-k4v` loses
  16% and is rejected. `LLAMACPP_SPEC_TYPE=none` disables it. Only Metal/Gemma
  has been measured — re-benchmark before assuming it holds elsewhere.
  **Self-heal:** three ordered retries on a dead child (drop optional flags
  after an argv error → step down the auto context after an OOM → Hugging Face
  fallback / resume a partial download). While any of them is deciding,
  `status()` reports `Starting`, **not** `Error` — a terminal status there made
  the GUI's 1500 ms poller abort startups that were about to recover.
  **Orphan reaping:** statics never drop, so `kill_on_drop` cannot cover
  process exit; spawns are recorded in `<data>/llamacpp/run/<ppid>.pid` and
  the next `ensure()` in any Biorouter process kills children of dead parents.
- **Pinned build:** `LLAMA_SERVER_BUILD` in the sidecar must match
  `LLAMA_BUILD` in `ui/desktop/scripts/fetch-llama-server.js`, which downloads
  per-platform archives at package time into `src/bin/llamacpp/` (mac = Metal
  ~10 MB, win = Vulkan ~37 MB with CPU fallback via ggml dynamic backends,
  linux = CPU ~15 MB; CUDA stays opt-in via `BIOROUTER_LLAMACPP_BIN`). Bump
  the pin deliberately and smoke-test — llama.cpp releases multiple times a
  day with no semver.
- **Linux floor:** llama-server needs glibc ≥ 2.35-ish plus `libssl3` and
  `libgomp1` (declared in the deb/rpm maker configs in `forge.config.ts`) —
  i.e. Debian 12+/Ubuntu 22.04+. Debian 11 runs the app but not local models.
- **HTTP routes:** `/llamacpp/status|ensure|stop` in
  `crates/biorouter-server/src/routes/llamacpp.rs` (status includes the
  catalog; ensure is async — poll status for download progress).
- **Frontend:** `LlamaServerInlineCard.tsx` (the first row of the Local tab,
  open by default in onboarding), provider ordering in `providerOrdering.ts`,
  tab + section order in `settings/providers/ProviderCatalog.tsx` (`ProviderGrid`
  is deleted). "Local ranks first everywhere" still holds for tab ORDER, which is
  fixed at Local · Institutional · Public. ⚠ The tab that opens *selected* is a
  different question and is computed, not fixed: a route hint (`?tab=public`)
  wins, then the tab holding `BIOROUTER_PROVIDER`, then Public when a coding
  agent reports `signed_in_subscription`, then Local — and never an empty tab.
  So a machine with a signed-in `claude` and nothing configured opens on Public,
  which is not a regression of the Local-first ranking. See
  [`docs/desktop-ui/provider-catalog.md`](docs/desktop-ui/provider-catalog.md).
- **Tests:** unit tests in both modules; route tests
  `cargo test -p biorouter-server --test llamacpp_routes`; live end-to-end
  (real server + tiny Qwen3.5 0.8B, ~0.5 GB one-time download):
  `BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server cargo test -p biorouter --test llamacpp_integration -- --ignored --test-threads=1`

### Coding-agent providers (Claude Code / Codex)

Two providers that run inference on the user's **own vendor subscription** by
driving a coding-agent CLI they already installed and signed in to: `claude_code`
(shown **Claude Code**) and `codex` (shown **Codex**). BioRouter never sees a
credential — there is no base URL and no API key. Full reference:
[`docs/providers/coding-agents/`](docs/providers/coding-agents/README.md), whose
compliance page is required reading before research data goes near either.

- **Both are `ProviderTier::Public`, deliberately.** A consumer Pro/Max or
  ChatGPT Plus plan carries no BAA and no zero-data-retention agreement, so PHI
  must never reach them. Being Public is not an oversight — it is what makes the
  privacy bind gate (Gate A) keep them out of a clinical session.
- **The child is a whole agent, so its own tools are switched off and replaced.**
  `crates/biorouter/src/providers/coding_agent/` holds the shared machinery
  (`discovery` finds the binary without spawning it, `appserver` speaks the
  vendor protocol, `transcript` flattens the conversation into one prompt,
  `effort` maps the reasoning-effort setting onto each vendor's own ladder).
  The isolation flags are security-relevant, not hygiene: `--setting-sources ""`
  stops a hook in the cwd executing, `--strict-mcp-config` closes a measured MCP
  leak, and `--bare` must never be passed.
- **BioRouter's own extensions reach the child over one MCP relay**
  (`coding_agent/bridge.rs` + `routes/tool_bridge.rs`). MCP is the only channel
  that returns a result into a live turn, and the capability **rides the URL**
  (`/tool_bridge/{nonce}`) because Codex sends no auth header — which is why that
  one route is exempt from the secret-key middleware and absent from the OpenAPI
  spec. Tools still execute on BioRouter's side, behind its inspectors,
  permission mode, `.biorouterignore`, vault and privacy gates.
- **Install/sign-in state is reported, not guessed:** `/coding_agents/status`
  (`routes/coding_agents.rs`) backs the onboarding card
  `onboarding/CodingAgentInlineCard.tsx`, wired beside `LlamaServerInlineCard` in
  `ProviderGuard.tsx`. `CLAUDE_CODE_COMMAND` / `CODEX_COMMAND` override discovery.
- **Tests:** `cargo test -p biorouter --lib providers::coding_agent`,
  `cargo test -p biorouter-server --test tool_bridge_routes`, and the vitest
  suite for the onboarding card. The live end-to-end tests need the real vendor
  CLIs installed and signed in.

### Artifact side panel (desktop)

The right-hand panel previews **anything the agent creates**, not just
visualizations, and it is the **only** surface any of it is ever displayed on. In
a transcript an artifact is a click-to-open card and nothing else — there is no
inline frame and no second "expand" destination. In a live chat the panel opens
automatically on the newest artifact; a saved or shared transcript opens nothing
until the reader clicks. Full rule, and the record of the inline renderer that
was removed to make it true:
[`docs/desktop-ui/artifact-display-surfaces.md`](docs/desktop-ui/artifact-display-surfaces.md).

- **Components:** `ui/desktop/src/components/artifacts/` — `ArtifactViewer.tsx`
  (the panel), `useArtifactPanel.ts` (its geometry + open/close state machine),
  `artifactUtils.ts` (detection + parsing helpers), `artifactTypes.ts`.
  Collection lives in `collectArtifactsFromMessages` in `components/BaseChat.tsx`.
- **Three surfaces mount it, from one hook.** `BaseChat.tsx`,
  `sessions/SessionHistoryView.tsx` (History + a schedule's run detail) and
  `sessions/SharedSessionView.tsx`. ⚠ **`onOpenArtifact` is REQUIRED down the
  whole transcript chain** (`ProgressiveMessageList` → `BioRouterMessage` →
  `ToolCallWithResponse` → `MCPUIResourceRenderer`), so a transcript surface with
  nowhere to put an artifact does not compile. That is deliberate: the split this
  replaced existed *only* because two call sites omitted an optional prop, and an
  optional callback made the divergence invisible. Auto-open and auto-repair stay
  chat-only — a read-only surface passes no `onRenderError`, so the panel never
  installs the repair listener.
- **What reaches the panel:**
  1. `ui://` embedded resources from tool responses — Auto Visualiser figures and
     reports, and Agent Drafter app preview cards (`create_app`, `configure_app`,
     `update_app`, `build_app`, `launch_app`, `preview_app` all return one).
  2. Files a tool call created, read off the call's **arguments** —
     `text_editor` (`write`/`str_replace`/`insert`/`undo_edit`, never `view`; ⚠ this
     line listed `create` and `diff` as commands until 2026-09-01 — `create` is not a
     command at all and `diff` is a *parameter* on `str_replace`, and `undo_edit` was
     missing),
     `write_file`/`create_file`/`edit_file`/…, and `shell` redirect / `-o`
     `--output` targets. Relative paths resolve against the session working dir.
     Only successful tool responses count. See `fileArtifactPathsFromToolCall`.
  3. File paths the assistant mentions in its prose (the original behaviour).
- **How each kind renders:** HTML → sandboxed `srcdoc` iframe; images → inline;
  directories → `DirectoryTreePreview`, a left rail holding a filter box and an
  expandable `role="tree"` (branch chip and per-entry status dots on a
  `gitDirectory`) beside a live preview of the selected entry;
  `.md`/`.Rmd`/`.qmd` → rendered prose with a
  Preview/Raw toggle; `.csv`/`.tsv` → a real table (quoted fields honoured,
  capped at 500 rows) with a Table/Raw toggle; everything else → syntax-
  highlighted, line-numbered code with a language chip and Copy.
- **Syntax highlighting** follows the app theme *and* the theme family. The one
  palette lives in `ui/desktop/src/styles/codeTheme.ts` and is selected as
  `codeThemesByFamily[useThemeFamily()][useResolvedTheme()]` — the identical
  expression in the panel, chat's markdown code blocks, `ToolCallWithResponse`
  and `NotebookPreview`, so they can never drift. The older `codeThemes.light` /
  `codeThemes.dark` export is Parchment-only back-compat with no consumers left;
  reaching for it pins code to Parchment under the other two families. Leaf
  components read the mode with `useResolvedTheme()` from `contexts/ThemeContext`
  — it falls back to `light` outside a provider instead of throwing like
  `useTheme()`.
  - **Never combine `wrapLongLines` with `showLineNumbers`** in
    `react-syntax-highlighter`: it then sets `display: flex` on every line
    (`highlight.js:106`), turning each token into a flex item and shredding long
    lines across the panel. The panel keeps line numbers and lets long lines scroll
    horizontally. Guarded by a test in `ArtifactViewer.test.tsx` — a short-line
    fixture will not catch this.
  - **Prism token classes are unprefixed and collide with Tailwind utilities.**
    A markdown table in a code block emits `<span class="token table">`, which
    Tailwind's `.table { display: table }` turned into a table box — one source
    line became a vertical stack of cells, orphaning the line numbers. `main.css`
    carries an unlayered `code [class~='token'] { display: inline; }` that wins
    over Tailwind's `utilities` layer regardless of specificity. *Unlayered* is
    the load-bearing part, not its position — it no longer sits at the end of the
    file, so grep the selector rather than reading the tail. jsdom does not apply
    Tailwind, so only a real browser catches this class of bug; sweep the panel
    with the harness (`.artifact-harness`) across md / csv / json / yaml / xml /
    R / py / sql / css / sh / toml / rs / ts after touching the code view.
- **No delete control on in-chat artifact cards.** Deleting an Agent Drafter app
  is destructive (removes files from disk) and lives only in the **Applications**
  tab (`ApplicationsView.tsx`) — never on the in-chat card in `MCPUIResourceRenderer`,
  where a stray click would nuke an app. The card opens the panel, and that is all
  it does.
- **Auto-repair of a broken artifact only resumes a *live* conversation.** When an
  artifact iframe posts `biorouter-viz-render-error`, `handleArtifactRenderError`
  in `BaseChat.tsx` feeds it back to the agent to fix — but only if
  `shouldAutoRepairArtifact(chatState, lastAgentActiveAt, now)` is true: a turn is
  running, or one finished within `ARTIFACT_REPAIR_ACTIVE_GRACE_MS` (15 s). Once the
  chat has been idle longer than that (or is merely reloading a saved session), a
  failure surfacing now was introduced by the user managing the artifact afterwards
  — reopening an old figure, editing an app's code, deleting it — and must NOT
  silently resume a finished conversation. Pure and unit-tested in
  `BaseChat.artifacts.test.ts`.
- **Browser harness:** `ui/desktop/.artifact-harness/` mounts the real
  `ArtifactViewer` in a plain browser against fixtures produced by the real Rust
  tools, so the panel can be checked without launching Electron:

  ```bash
  PREVIEW_FIXTURE_DIR=/tmp/fx cargo test -p biorouter-mcp --test preview_fixture_dump -- --ignored
  AUTOVIS_DUMP=/tmp/fx/dashboard.html cargo test -p biorouter-mcp --lib \
    autovisualiser::tests::dump_sample_dashboard -- --ignored
  cd ui/desktop && PREVIEW_FIXTURE_DIR=/tmp/fx npx vite --config .artifact-harness/vite.config.mts --port 5199
  ```

- **Driving the real dev GUI:** `BIOROUTER_NO_HMR=1` freezes the renderer (no vite
  watching, no hot reload). Without it, any save anywhere under `ui/desktop/src/`
  full-reloads the page and destroys the chat session under test — which makes
  agent-browser/Playwright runs fail in ways that look like app bugs. Combine with
  the sandboxing and CDP port from `just agent-browser-ui`. It has a second
  consequence that reads as a completely different bug — it also blinds Tailwind's
  class scanner, so a *newly written* utility never reaches the stylesheet; see
  "Desktop shell geometry" below.

- **Launching the GUI from an agent shell — read
  [`docs/desktop-ui/launching-the-dev-gui.md`](docs/desktop-ui/launching-the-dev-gui.md) first.**
  `just run-dev` works at a human terminal and *cannot* survive a shell without a
  TTY. Five distinct failures there produce symptoms that read as application
  bugs, and three of them look identical from the outside:
  - `ELECTRON_RUN_AS_NODE=1` (commonly exported in agent shells) makes Electron
    exit instantly with no window and no error — always `env -u ELECTRON_RUN_AS_NODE`.
  - `electron-forge start` reads stdin for its `rs` command, so `< /dev/null`
    hands it EOF and it takes the app down with it. Wrapping it in `script` to
    fake a pty does not help; run the Electron binary directly against
    `.vite/build/main.js` instead.
  - a bare `npx vite` does **not** load `vite.renderer.config.mts`, so Tailwind
    never runs and the app renders as unstyled serif HTML that is fully
    functional — it looks like a broken app, it is a broken launcher. Always
    pass `--config vite.renderer.config.mts`.
  - verify with a **CDP screenshot** (`--remote-debugging-port`, then
    agent-browser), never `screencapture` of the whole screen: the app window
    sits behind the editor, raising it is unreliable, and a full-screen grab
    captures the user's mail and browser history.

  Ruled out with evidence, so don't re-diagnose: Electron *can* open a window
  from an agent shell (a minimal app fires `ready` and stays alive), and the
  staged `ui/desktop/src/bin/` binaries are usually fine — check with `file`
  before suspecting them.

### Theme families (Parchment / Alma Mater / Roche Limit)

Two orthogonal axes: light/dark mode (a `.dark` class on `<html>`) and **theme
family** (a `data-theme` attribute). Three families ship — Parchment (default;
warm ink + coral, not a warm ground — see "one neutral set" below), Alma Mater
(UCSF navy + teal), Roche Limit (JupyterLab-inspired). Themes are **baked into
the app by decision**; they are not user-installable.

**A theme is ONE file.** `ui/desktop/themes/<id>.theme.mjs` is the source of
truth; `npm run themes` generates everything else:

- `src/styles/main.css` — the `:root[data-theme=X]` / `.dark[data-theme=X]` token
  blocks (inside a `THEMES:GENERATED` marker region)
- `src/styles/themes.generated.ts` — syntax palettes, terminal ANSI palettes,
  brand-mark inks, the family manifest and `THEME_FAMILY_IDS`
- `index.html` — the pre-hydration family list and the boot-splash CSS

Do **not** hand-edit those regions. `npm run themes -- --check` runs inside
`lint:check` and fails CI if they are stale.

Key invariants, each learned from a real bug:

- **`@theme inline` is load-bearing.** It emits `.bg-sidebar { background-color:
  var(--sidebar) }` and keeps `--color-sidebar` out of the cascade, so utilities
  are late-bound to the semantic token. Plain `@theme` would freeze values at
  build time and break scoped theming with no test failing.
- **Light block must precede dark.** `:root[data-theme=X]` and
  `.dark[data-theme=X]` have identical specificity (0,2,0) — only source order
  separates them. Reversing the pair renders light tokens in dark mode while
  every contrast ratio still passes. `check-contrast.mjs` asserts the ordering.
- **One neutral set, three inks.** All three families now wear the SAME neutrals
  — every surface, grey and structural border is Roche Limit's, in both modes,
  including dark's ladder (canvas darkest, cards a step up; Parchment dark and
  Alma Mater dark both used to invert it, and one shared set cannot carry two
  contradictory orders). What still varies is a family's *ink*, its *accent*
  (plus the accent-derived heat ramp) and its status hues — measured, 28 of 58
  light tokens and 30 of 58 dark ones differ, and not one of them is a surface.
  **Nothing enforces the sharing:** `check-contrast.mjs` audits each family on
  its own, so a diverged neutral passes every assertion. Roche Limit's file is
  the reference set; moving a neutral means editing all three `*.theme.mjs`
  files *and* `main.css`'s hand-authored `:root`/`.dark` base block together.
- **`terminalGround` is now the same token in all three** — `--background-muted`,
  in both modes. The warning that used to sit here was right and still is, aimed
  at a changed fact: Parchment dark really did paint `--background-code`, and
  that was a real difference while the two tokens held different per-family
  values. So read each family's `terminalGround` field rather than assuming
  either way — the agreement is a measurement (the generator resolves every
  family's ANSI palette against its own ground), not a rule, and it can change
  back.
- **Derived, never authored:** terminal/code/splash grounds, picker label and
  swatch, the family list. These are the values that historically drifted.
- **`check-contrast.mjs` discovers families** from the stylesheet; a new family
  is audited with zero edits to it (330 assertions at three families, measured
  2026-08-08 — re-measure rather than trusting this figure).

Design docs: [`docs/design/theming/theme-system-architecture.md`](docs/design/theming/theme-system-architecture.md)
(architecture + the decisions and their reasons), plus one per family —
[`design.md`](design.md) at the **repo root** (Parchment; note it self-reports an
older version than the tree), and `docs/design/theming/alma-mater-theme-tokens.md`
and `docs/design/theming/roche-limit-theme.md`.

### Desktop startup path (issue #88)

The app used to freeze for seconds on launch, and the cause generalises past the
one bug: **the Electron main thread was running synchronous `spawn`s** — version
probes for the dependency check, then the updater — so the window could not paint
until every probe returned. Fixed in `ui/desktop/src/main.ts` +
`utils/startupSchedule.ts`; the write-up worth reading before adding anything to
launch is
[`docs/desktop-ui/startup-freeze-and-main-thread-blocking.md`](docs/desktop-ui/startup-freeze-and-main-thread-blocking.md).

- **Nothing synchronous goes on the main thread at startup.** `runProbe` in
  `utils/dependencyChecker.ts` is the async replacement for `spawnSync` that
  every dependency check now goes through, and `utils/startupSchedule.ts`
  decides what is allowed to run before first paint. The updater no longer runs
  immediately either — it was heating the machine right after launch. `runProbe`
  has its own test binary (`utils/runProbe.test.ts`) because its mapping of a
  child's outcome onto `{ ok, code, timedOut }` is what decides whether the user
  is told "failed", "timed out" or "not installed".
- **`utils/mainThreadWatchdog.ts` reports a blocked main thread** instead of
  leaving the next freeze to be guessed at, and
  `ui/desktop/scripts/measure-startup-freeze.mjs` is how you measure one.
- **A failed dependency is actionable, not a dead end.** `DependencySetupModal`
  offers one-click installs (serialized — one installer at a time), and every
  dependency surface carries a "Debug with Biorouter" escape hatch
  (`utils/launchDependencyDebug.ts`, `utils/dependencyDebugPrompt.ts`) that hands
  the failure to the agent. The terminal half is
  **`biorouter doctor --fix [DEP]`** (`commands/doctor.rs`).
- **Background extension-update failures are reported** (`ExtensionUpdateReporter`);
  they used to fail silently.
- ⚠ **On Windows an absolute path must not be routed through `cmd.exe`** — doing
  so was one of the four defects an adversarial review of this branch found.

### Desktop shell geometry (layout tokens in `main.css`)

Not per family — these live in `:root` and are the same under every
`data-theme`. Each is the fix for a drift that had already happened, so the
value's *location* is as load-bearing as the number.

- **`--chrome-height: 44px` is wired, not a target.** All three top bands read
  `h-chrome`: the sidebar's titlebar band (`BioRouterSidebar/AppSidebar.tsx`),
  the chat header (`BaseChat.tsx`) and the artifact tab strip
  (`artifacts/ArtifactViewer.tsx`). They were 52px written three ways, which is
  exactly what let them drift at the seam they share. ⚠ **The three move
  together or not at all** — they meet at one continuous top edge, so a band
  that shrinks beside a stationary one reads as misalignment rather than as a
  tighter app. That is why the value lives in `:root` and in none of the three
  files. `--tab-height` (32px) does the same job for `.br-tab`, drawn identically
  in the chat header, the artifact panel and the terminal dock: a fixed height
  *centred* in its band, because the bands differ and a band-filling tab would be
  a different size in each one.
- **The sidebar is a RANGE, not a width** — `components/ui/sidebarWidth.ts`,
  216–360px with a 288px default, dragged from `SidebarResizeHandle` and stored
  per user in `localStorage` (clamped **on read**, so a value from an older build
  can never land outside today's bounds). Two consequences that are easy to get
  wrong. ⚠ **The window's `minWidth` derives from the DEFAULT** (288 + 760 =
  1048, `main.ts`). It was briefly taken from the *minimum* on the argument that
  216 is the only width in the range that is a property of the app rather than of
  a preference; that runs backwards, because `216 + 760 = 976` leaves the shipped
  288px sidebar a `688px` column — under the measure the floor exists to protect.
  ⚠ **The wide end is closed by an
  identity, not by a clamp** — `SIDEBAR_MAX_WIDTH + 760 = 1120 =
  SIDEBAR_COMPACT_WIDTH`, rung 1 of the yield ladder — so raising the max without
  moving the ladder silently starts eating the measure. `styles/measures.test.ts`
  pins both. The arithmetic lives in a React-free module because jsdom computes
  no layout: a test that mounts the sidebar cannot see how wide it got, and the
  handle's affordance (hover hairline + `col-resize`) is authored CSS in
  `main.css` for the same reason the composer's focus edge is.
- **`--measure-chat` is a flat `760px`.** It was briefly widened into a clamp and
  that was wrong for this measure specifically — a 1180px composer is a line of
  prose the eye has to track back across. `styles/measures.test.ts` asserts the
  whole string, because every loose matcher (`/760px/`) is satisfied by
  `clamp(760px, 78%, 1180px)`, the value being ruled out.
- **The composer's card is the input, and nothing else** (`ChatInput.tsx`). Three
  rows — context above, the card holding the prose *and* Send, controls below —
  with the outer two directly on the canvas, so the only boxed thing on screen is
  the one thing the user acts on. The focus edge is that card's own 1px border
  turning `--border-accent` (not `--accent-muted`, which at 2px reads as a thick
  warm-grey band), and it is **authored CSS** —
  `.biorouter-composer-card:has(textarea:focus)` in `main.css` — never a Tailwind
  `has-[...]` variant at the call site.
  - ⚠ **The reason generalises far past the composer: a newly written Tailwind
    class can silently fail to generate.** Under `BIOROUTER_NO_HMR` the renderer
    runs `watch: { ignored: ['**'] }` (`vite.renderer.config.mts`), which is the
    same signal Tailwind's scanner uses to notice new class strings. Three
    spellings were each measured in the running app — the class on the element, a
    focused `textarea` descendant, and no matching rule anywhere in the
    stylesheet. Clearing `node_modules/.vite` changed nothing. Nothing
    load-bearing should depend on class-scanning having worked; author the rule.
  - `styles/composerFocus.test.ts` asserts the declaration **at the source**, and
    has to: jsdom has no layout engine, never runs Tailwind and does not evaluate
    `:has()`, so a component test that focuses the textarea and reads
    `borderColor` sees the resting value and passes whether the rule exists or
    not.
- **Long messages fold** (`utils/messageClamp.ts`): above 10 lines *or* 600
  characters, clipped to 200px behind a fade with an expand control. The
  arithmetic is deliberately free of React and the DOM — `UserMessage.tsx` owns
  the clipping, the fade and the control and none of the thresholds — because a
  threshold you can only exercise by rendering a component is one nobody
  re-tests. It fires on length alone; there is no paste-origin signal.
- **Toasts sit in the top-right corner**, at `--toast-inset-top =
  --titlebar-drag-height + 12px`. It was 144px, derived by measuring the lowest
  ink in the tallest page header so a toast could never cover a page title —
  which worked, and put the extension-load report halfway down the chat pane.
  ⚠ **The floor is not negotiable.** Anything above `--titlebar-drag-height`
  overlaps the `-webkit-app-region: drag` rect `App.tsx` paints, and Electron
  folds drag rects in **DOM order** — the drag div lives inside App's main tree,
  *later* than the toast container — so a later drag rect eats clicks on a
  higher-z control no matter what the z-index says (issue #74). A toast's × and
  "View details" would look present and be dead. `styles/toastLayer.test.ts`.

### Auto Visualiser feature

The Auto Visualiser (`autovisualiser`) built-in MCP server turns structured data
into self-contained interactive HTML figures, returned as `ui://…` resources.
A figure is shown as a click-to-open card in the transcript and rendered in the
**artifact side panel** (a sandboxed `srcdoc` iframe the panel builds itself) —
never inline. The `/mcp-ui-proxy` route that served the old inline iframe is
gone, along with its exemption from the secret-key middleware; see
[`docs/desktop-ui/artifact-display-surfaces.md`](docs/desktop-ui/artifact-display-surfaces.md).

- **Module:** `crates/biorouter-mcp/src/autovisualiser/` — `mod.rs` (router +
  the 8 original tools), `common.rs` (shared infra), `tools_extra.rs` (Mermaid
  wrappers), `tools_charts.rs` (Chart.js), `tools_d3.rs` (D3), `tools_geo.rs`
  (Leaflet), `tools_dashboard.rs` (the composite report), `tests.rs` +
  `tests_extra.rs` + `tests_dashboard.rs`. The `tools_*.rs` files are
  `include!`d into `mod.rs`; each defines a `#[tool_router(router = …)]` impl
  block, combined in `new()` via `ToolRouter` `+`.
- **Shared pipeline (`common.rs`):** validate → JSON-encode safely (`js_data`
  neutralises `</script>` breakout) → `assemble` template with `{{ASSETS}}` +
  `{{COMMON}}` (the shared `templates/_common.js`: theme, palette, auto-resize,
  global error card) → base64 `ui://` blob (`finish`). Every tool also enforces
  size limits + semantic checks and returns a friendly `INVALID_PARAMS` message
  instead of producing a broken figure.
- **Tools: 3 advertised, 32 metadata-only.** ⚠ This line read "Tools (33)" until
  2026-09-01 and listed the 32 figure tools as individually callable. They are **not**:
  they live in a non-dispatchable `figure_router` behind `render_figure` (a `kind` plus
  that kind's arguments) and `describe_figure`, with `render_dashboard` the third
  advertised tool. `exactly_three_tools_are_advertised` in `tests_dashboard.rs:1249` pins
  it. The 32 names below are the **kinds**, not callable tool names — an error message
  that hands one of them to a model is naming a call it cannot make (#150).
  Kinds: charts (`show_chart`, `render_histogram`, `render_boxplot`,
  `render_bubble`, `render_area`, `render_radar`, `render_donut`, `render_gauge`);
  scientific (`render_volcano`, `render_manhattan`, `render_kaplan_meier`,
  `render_forest`); relationships/hierarchies (`render_network`, `render_sankey`,
  `render_chord`, `render_heatmap`, `render_treemap`, `render_sunburst`,
  `render_dendrogram`, `render_wordcloud`, `render_calendar_heatmap`); diagrams
  (`render_mermaid` + typed wrappers `render_flowchart`/`gantt`/`sequence`/
  `mindmap`/`timeline`/`er_diagram`/`state_diagram`/`class_diagram`); geo
  (`render_map`, `render_choropleth`); composite (`render_dashboard`).
- **`render_dashboard` (combining figures):** takes `title`, `subtitle`,
  `summary`, `footer` and either `panels` or grouped `sections`, where each panel
  names any other Auto Visualiser tool plus that tool's exact arguments, with a
  `title`/`caption`/`notes` and `width: full|half`. It renders one scrollable
  report artifact — masthead, contents, section prose, numbered figure captions,
  collapsible notes — instead of N separate artifacts the user must open one at a
  time. The server instructions tell the model to reach for it whenever an answer
  needs more than one figure.
  - Panels are rendered by calling the real single-figure tools inside
    `common::render_fragment`, which swaps `asset_html` for an
    `<!--AUTOVIS_ASSETS-->` sentinel and records which libraries the figure asked
    for (a `tokio::task_local` sink). The report stores each library's source
    **once** and its own JS splices it into every panel's `srcdoc` at render
    time; panels hydrate lazily via `IntersectionObserver`. Without this, three
    Mermaid panels would carry three 3.3 MB copies of Mermaid.
  - A panel whose arguments are invalid becomes an error card inside the report
    (naming the tool and the problem) rather than failing the whole call; the
    assistant-audience text lists every failed figure so the model can fix it.
    All panels failing *is* a tool error.
  - Panels post `ui-size-change` to the report (which grows that iframe) and the
    report reports a **capped** height to the host, so a long report scrolls
    internally instead of adding thousands of pixels to the chat transcript.
  - An embed stylesheet strips each figure's standalone chrome (its own card,
    background and title banner) so panels sit flush in the report's cards.
  - **A report always inlines its libraries, ignoring `BIOROUTER_AUTOVIS_CDN`.**
    The desktop app sets that flag to `1` by default (`ui/desktop/src/biorouterd.ts`),
    so CDN mode is the normal GUI path. A *standalone* figure survives it only
    because the Electron main process rewrites the figure's `<script src=…>` back
    into an inline script before display — the renderer's CSP is
    `script-src 'self' 'unsafe-inline'`, so a remote script never loads. A report
    keeps its library tags inside base64 asset/panel blobs, where that rewriter
    cannot reach them, so a CDN report rendered blank figures ("Chart is not
    defined"). Dedup already caps the cost at one copy per library.
    Guarded by `crates/biorouter-mcp/tests/autovis_dashboard_cdn.rs`.
  - Models generalise from the other 32 tools, which all take a single `data`
    argument, and wrap the whole report in one (`{"data": {"title": …}}`) — GPT-5.5
    does, then retries identically after a rejection. `normalize_dashboard_args`
    unwraps a `data`/`dashboard`/`report` envelope and parses stringified
    `sections`/`panels`, in the same spirit as `common::de_flexible`.
- **Assets:** libraries (D3, Chart.js, Leaflet, Mermaid) are inlined by default
  for offline use. `BIOROUTER_AUTOVIS_CDN=1` switches to pinned CDN tags, which
  shrinks the persisted/reloaded blob from megabytes to a few KB (recommended if
  large Mermaid diagrams fail to re-render on chat reopen), **and is the desktop
  default** (`ui/desktop/src/biorouterd.ts`).
  `BIOROUTER_AUTOVIS_DEBUG=1` (or debug builds) dumps generated HTML to the app
  cache dir (`<cache>/autovisualiser/<name>-<pid>.html`).
  - ⚠ **CDN mode does not mean the figure fetches anything.** Every artifact is
    displayed under `default-src 'none'`, so a remote reference is dead on
    arrival; what makes CDN mode work is the Electron main process pre-fetching
    each URL and splicing the source in as an inline `<script>` before the CSP
    applies (`ui/desktop/src/utils/artifactCdnAssets.ts`). Two invariants follow,
    and Mermaid shipped violating **both** — its URL was absent from the desktop
    list, and it was emitted as `<script type="module">import … '/+esm'</script>`,
    a shape the `<script src=…>` rewriter can never match: (1) every URL the tools
    emit in CDN mode is in `ARTIFACT_CDN_ASSETS`; (2) each is emitted as a
    `src=`/`href=` tag referencing a **classic** (non-ESM) build, because the
    replacement produces a classic script. Both are asserted from Rust, against
    the real desktop source, in
    `crates/biorouter-mcp/tests/autovis_cdn_desktop_contract.rs` — the only place
    that can see both halves. Full write-up:
    [`docs/desktop-ui/artifact-cdn-assets.md`](docs/desktop-ui/artifact-cdn-assets.md).
- **Tests:** `cargo test -p biorouter-mcp --lib autovisualiser` (happy paths,
  edge cases, escaping, lenient enum parsing).

### Agent Drafter (BioRouter apps) — agent-driven UI + export

`agent_drafter` builds **BioRouter apps**: a TypeScript front-end wired to a real
per-app agent over `GET /apps/<id>/agent`. Full design in
[`docs/agent-drafter/apps-platform-design.md`](docs/agent-drafter/apps-platform-design.md).

- **Apps SDK v2** — the typed two-way surface (app contract, shared state doc,
  catalog + `ui_patch`, `br.kb`/`br.model`, theme packs, archetypes, standalone
  export). Human-facing reference: [`docs/apps-sdk/sdk-reference.md`](docs/apps-sdk/sdk-reference.md)
  (every `br.*` signature, the manifest schema, the frame tables); the v2 map sits
  atop `docs/agent-drafter/apps-platform-design.md`. Partial in this build: the SDK emits
  `ui_error` frames the daemon doesn't consume, and `ui_suggest` has no MCP tool.
  Multi-agent worker profiles (`orchestration.agents` → `br.agent(name)` +
  `consult` + `ready.profiles`) are **actively landing** in this branch —
  serialized cross-profile turns (parallel is a stretch goal); treat the code as
  authoritative. New test gates: the SDK v2 harness self-test
  `node scripts/agent-drafter/ui-control-harness.mjs` (real `sdk.ts` in jsdom vs a
  mock daemon; needs esbuild + jsdom) and `cargo test -p biorouter-server --lib
  routes::apps` (frames, KB grants, provider-class routing). The test count that
  used to sit here had roughly doubled while the line stood still, so it was
  removed rather than guessed — measure it in your own run before asserting
  "pre + N".

- **The agent drives the app, it doesn't just answer in it.** A per-session
  in-process MCP server (`agent_drafter/control.rs`, injected as `appcontrol` by
  `configure_agent` exactly like `datasql`/`files`/`compute`) exposes `ui_*`
  tools — `ui_describe`, `ui_panel`, `ui_render`, `ui_chart`, `ui_graph`,
  `ui_highlight`, `ui_theme`, `ui_layout`, `ui_notify`, `ui_state`, `ui_ask`.
  Each pushes a `{"type":"ui","cmd":…}` frame down the app's own WebSocket, which
  `templates/sdk.ts` (`class UiRuntime`) applies to the DOM.
- **`ui_ask` blocks the tool call** until the browser sends `ui_reply`, so the
  agent branches on the user's answer inside one turn. That is why
  `handle_agent_socket` **splits the socket** and `select!`s over three sources
  (agent events / UI commands / inbound frames). It also made `cancel` work
  mid-turn for the first time.
- **`UiBridge` is rebindable.** `get_agent` caches one agent per session and
  `add_inprocess_server` is idempotent by name, so a reconnecting browser reuses
  the same `AppControlServer`. The `UI_BRIDGES` registry (keyed by session id)
  hands back the same bridge; `attach()` re-points it at the new socket and
  replays `ui_state`, `detach()` unblocks any parked `ui_ask`. Without this every
  reload would leave the `ui_*` tools writing into a dead channel.
- **`capabilities.ui` defaults to ON** (unlike the deny-by-default
  `files`/`data`/`compute`/`vault`): its blast radius is the app's own page.
  `{"ui":{"enabled":false}}` for a text-only app; `allow_theme`/`allow_layout`/
  `allow_ask` are individually revocable.
- **Authors expose render targets** with `<section data-br-region="results">`;
  the agent finds them via `ui_describe` and writes to `@region:results`. Panels
  need no region — the SDK always provides a `.br-dock` drawer.
- **Apps vendor their own `src/sdk.ts`**, so `manifest.sdk_hash` fingerprints the
  SDK a bundle was built from. `build_app` refreshes the vendored copy and stamps
  it; `serve_index` rebuilds on drift. Otherwise an app built before a protocol
  addition silently ignores the new frames forever.
- **Export is directly runnable.** `export_app` writes `manifest.json` (without
  it the daemon 404s the app), leaves the endpoint unset so the SDK derives it
  from the page origin (the desktop app starts `biorouterd` on an *ephemeral*
  port, so the old hardcoded `:3000` never worked), ships `biorouter-launch.sh` +
  `run.sh`/`run.command` (locate `biorouterd`, install, start, verify, open — no
  Node needed) and a `serve.mjs` that proxies `/apps/**` incl. the WS upgrade and
  binds loopback only. `.vault/` is excluded. The daemon's port env var is
  **`BIOROUTER_PORT`**, not `BIOROUTER_SERVER__PORT` (only the secret key uses
  the `__` form — `Settings` is a flat struct).
- **Tests:** `cargo test -p biorouter-mcp --lib agent_drafter::`,
  `cargo test -p biorouter-mcp --test ui_example_apps`,
  `cargo test -p biorouter-server --lib routes::apps`. Browser-level:
  `scripts/agent-drafter/ui-control-harness.mjs` (mock daemon, real SDK) and
  `ui/desktop/scripts/appcheck/check-ui-app.mjs` (real agent; asserts `ui` frames
  arrive). Examples: `scripts/agent-drafter-apps/examples/ui/` +
  `install-examples.sh`.

### Skills: one catalog, one importer (#113, #115)

Two rules govern every skill surface, and both replaced a set of parallel
implementations that had drifted. Reference:
[`docs/extensions/skill-catalog.md`](docs/extensions/skill-catalog.md) and
[`docs/extensions/skill-packages.md`](docs/extensions/skill-packages.md).

**The daemon owns the catalog, and nothing else scans.**
`crates/biorouter/src/agents/skill_catalog.rs` enumerates the roots *with
provenance* (`roots()` — the ONE definition; `SkillsClient::get_default_skill_directories`
is its paths-only view), discovers skills, derives bundles from the discovery
result, and composes machine-wide with per-session state in `compose_state` —
which `skills_extension`'s own filter also calls, so the model's view and the
interface's switch cannot disagree. Served over `GET /skills/catalog`; the
renderer's `loadSkillsFromDirs`/`ALL_SKILL_DIRS` scanner is **deleted**, and
Settings, the composer picker, the `@`-mention list and the workflow picker all
read `useSkillCatalog`.

- **The catalog is a process-global snapshot every `SkillsClient` reads
  through**, not a constructor-time map, so a skill installed mid-conversation
  is loadable in that conversation. Staleness = the root set (an extension
  install *adds a root*) + the mtime of every root and every **bundle**
  directory (creating `<bundle>/<child>/` bumps the bundle's mtime, not the
  root's). ⚠ mtime has a one-second window; in-process writers must call
  `skill_catalog::invalidate()`/`refresh()` rather than rely on it.
- **Per-chat state is `workspace_skills/v1` on the session row, never
  `skills-config.json`** — that file is machine-wide and shared with
  `biorouter skill enable/disable`. `POST /skills/session` persists first and
  reads the catalog back *after* the write, so the interface renders confirmed
  state; a refusal restores the previous catalog and raises an **error** toast.
  Precedence: skill `add` > skill `remove` > **bundle** `add` > bundle `remove`
  > machine-wide. The bundle arms are load-bearing — without them a per-chat
  bundle toggle writes a name no skill matches.
- ⚠ **A refreshed catalog is not an enabled skill, and an install must not say
  it is.** Both install handlers hard-coded `"usableInThisConversation": true`,
  so a package reinstalled into a chat that had switched it off was announced as
  ready and then filtered out of every model-facing list. The answer is composed
  by `SkillCatalog::view`, never re-decided — and the obvious hand-rolled test
  ("is this name in `over.remove`?") is wrong for exactly the case that matters,
  because a per-chat **bundle** toggle persists the bundle's name and no
  member's. Reading the post-install catalog rather than the `ImportPlan` also
  settles which shape landed: installed `individual`, the components have no
  bundle, so a revocation naming the bundle genuinely stops applying.
  `session_skills::forget` prunes on uninstall, but that is hygiene, not the
  fix: another chat may hold the same revocation, and `workspace_set_tools`
  writes overrides into sessions other than the caller's, so no prune on any
  remove path can ever be complete.
- ⚠ `CatalogSkill.builtin` answers "did Biorouter put this here?", and
  `CatalogBundle.builtin` answers it for a bundle **row**, which is a different
  control over a different directory. That second field is defence in depth —
  the one shipped bundle is a Context and `pickerBundles` strips Contexts before
  a row renders — so do not mistake it for the thing that closes the
  delete-succeeds-then-reverts hole; that is `refuse_shipped`, below. The
  hand-synced `skillUtils.BUILTIN_SKILL_NAMES` copy is gone; `contexts.test.ts`
  now asserts it has not come back. The single refusal that makes "no seeded
  skill can be deleted" true on **every** surface — GUI Trash and
  `biorouter skill remove` alike — is `refuse_shipped` in
  `skill_package/install.rs`, scoped to Biorouter's own skills root and called
  from **both** `remove` and `install`. ⚠ Guarding only `remove` is worse than
  guarding neither: an install that shadows a shipped name renames the seeded
  directory aside and deletes it, the seeder then writes `SKILL.md` back inside
  the user's package, and `remove` refuses to uninstall the result.
- **A Context row may name a bundle, not a skill.** The four knowledge-format
  skills plus `update-soul` are seeded into `<skills root>/knowledge-bases/` and
  offered as *one* switch, "Knowledge". Three consequences, each of which was a
  bug in the shape it prevents: `compose_state` and `enabled_skill_entries` test
  a skill's **bundle** against the hidden-Context set as well as its own name
  (`skills_extension::context_ids` is the list, and it holds `KNOWLEDGE_BUNDLE`,
  not five skill names); the renderer's filters take the bundle too
  (`isContextSkill(name, bundle)`, `isContextBundle(name)`, and the shared
  `pickerBundles(view)` that the composer, the `@`-mention list and the workflow
  picker all read); and every surface that enumerates *directory entries* rather
  than skills — `count_user_skills`, the CLI status line — must ask
  `is_shipped_entry_name`, because `is_builtin_skill_name` does not know the
  bundle directory and reads it as one skill the user installed.
  ⚠ `ensure_soul_skill` and `ensure_builtin_skills` both **move** — not delete
  — the flat directories a pre-bundle install left behind, and both do the move
  *before* seeding. Two reasons, and flattening them is how the first draft got
  it wrong. Relocating at all is not tidiness: discovery keys by frontmatter
  `name`, so a flat copy and a bundled copy are two candidates for one map key
  and `read_dir` order decides. Moving rather than deleting is because the
  seeder writes exactly one file per skill, `SKILL.md` — every *other* file in
  that directory is a supporting file the user may have put there, which
  `find_supporting_files` serves to the model, and which `remove_dir_all` would
  take with it. A failed move leaves the old copy alone: a duplicate row is
  visible and fixable, a deleted file is not.

**One import pipeline** — `crates/biorouter/src/agents/skill_package/`, reached
from Add Skill (URL or `.zip`), the marketplace, the `importSkillPackage` MCP
tool, `biorouter skill install <path-or-url>`, and `/skills/packages/*`. The two
depth-counting archive parsers in `routes/shell.rs` and `commands/skill.rs` are
deleted, not kept in step.

- **Metadata beats shape**, and the ladder **merges** rather than picking one
  rung: `biorouter-package.json` → `.codex-plugin/plugin.json` →
  `.claude-plugin/plugin.json` → `skills-manifest.json` → structure. HyperFrames
  ships all three plugin files and only the Codex one carries `skills`.
- **A shared name prefix is never the detector** — `media-use`, `slideshow`,
  `product-launch-video` and `faceless-explainer` carry none — and every
  component keeps its declared name exactly.
- **Ambiguity is a question**: components loose at an archive root with no
  manifest could be one package or a zipped folder, so the plan says so and
  install refuses until answered. A named parent directory is not ambiguous, and
  neither is a manifest. Over HTTP the question is a **200** `needsChoice`, not
  a 4xx — an agent seeing an error retries instead of asking.
- **A partial write never lands**: staged in a sibling directory, verified by
  reading the tree back, swapped in with two renames.
- ⚠ **Run the live test after touching `manifest.rs` or `plan.rs`:**
  `cargo test -p biorouter --test skill_package_live -- --ignored`. It downloads
  the real HyperFrames repository. A complete synthetic fixture matrix passed
  while **three** real defects survived — `skills-manifest.json`'s `skills` is a
  *map* not an array (which failed the whole document and refused the import),
  the display name lives in the plugin manifest's `interface` block, and package
  identity was resolved before the single-skill collapse. A fixture is a
  statement about what you *think* the world looks like.

Tests: `cargo test -p biorouter --lib -- skill`,
`cargo test -p biorouter-server --lib -- routes::skills`,
`cargo test -p biorouter-cli --lib` (**needs an isolated `HOME` — and both `CARGO_HOME`
and `RUSTUP_HOME` kept pointing at the real ones, as LITERAL paths**: cargo and rustup
both derive their homes from `$HOME`, so a bare `HOME=$(mktemp -d) cargo test …` loses
the crate registry and dies with `error[E0463]: can't find crate for \`biorouter\``. That
surfaces as **rc=101 with zero failing tests**, which reads as a flake rather than as
nothing having compiled — and a harness that greps for `test result:` reports the
PREVIOUS build's count.

⚠ **Do not write `CARGO_HOME="$HOME/.cargo"` in the same command.** This line said exactly
that until 2026-09-01 and it is wrong in zsh: the assignments are applied left to right,
so `$HOME` expands to the **new temp dir**, not the real one. The failure is not an error
— it silently redirects `RUSTUP_HOME` as well, so cargo re-downloads the whole toolchain
and every target recompiles from scratch (measured: `installing component 'rustc'` mid-run,
and 11m38s for a single crate that normally takes seconds). Use literals:

```bash
HOME=$(mktemp -d) CARGO_HOME=/Users/wgu/.cargo RUSTUP_HOME=/Users/wgu/.rustup cargo test -p biorouter-cli
```
), and `cd ui/desktop && npm run test:run`.

### Workspace control (several conversations at once)

Workspace control (BR-71) is the agent's tool surface over *other* conversations:
running several chats side by side, delegating to subagents you can watch, and
reaching into another chat to read it, steer it, or fix its setup. User-facing
guides: [`docs/agent-loop/workspace-control.md`](docs/agent-loop/workspace-control.md)
and the per-tool reference
[`docs/agent-loop/workspace-control-tools.md`](docs/agent-loop/workspace-control-tools.md);
delegation is covered by [`docs/agent-loop/subagents.md`](docs/agent-loop/subagents.md)
and the capability itself by [`docs/extensions/built-in/workspace.md`](docs/extensions/built-in/workspace.md).

- **The `workspace` capability** (represented internally by the legacy platform-extension type) —
  `crates/biorouter/src/agents/workspace_extension.rs`, registered
  **`default_enabled: true`** (`agents/extension.rs:93`, pinned by
  `workspace_is_a_default_on_capability_granting_its_whole_surface`). ⚠ This line said
  `false` until 2026-09-01, and `workspace_extension.rs`'s own module doc still does —
  grep the registry, not the doc. It ships ON as a capability (#76) and grants the FULL
  surface (`available_tools` empty), including reading and steering other conversations.
  The delegation gate excludes it by name so it cannot satisfy condition 5 by itself.
  Supporting modules: `agents/workspace_inspector.rs`,
  `agents/workspace_summary.rs`, and `crates/biorouter/src/workspace_services.rs`
  (the shared service the extension and the HTTP layer both call).
- **HTTP surface:** `crates/biorouter-server/src/routes/workspace.rs`.
- **CLI parity:** `crates/biorouter-cli/src/commands/workspace_parity.rs` — the
  terminal reaches the same capabilities the GUI tabs expose.
- **Subagent delegation:** `agents/subagent_tool.rs`,
  `agents/subagent_execution_tool/`, `agents/subagent_handle.rs`.
- **GUI tab/pane layer:** `ui/desktop/src/components/chatGroups/`.

**The delegation gate** (`Agent::subagents_enabled`, `agents/agent.rs`) has
**five** conditions. The generic `subagent` tool is offered only when all of them
hold:

1. `subagent_tool_enabled` — Agent-Drafter apps with declared worker profiles set
   this false, so `consult` stays the one delegation mechanism.
2. **Completely Autonomous** mode (`BioRouterMode::Auto`).
3. The bound model's name does **not** start with `gemini`.
4. The session is not itself a `SessionType::SubAgent` — a subagent cannot spawn
   its own.
5. **At least one non-injected extension is loaded**
   (`ExtensionManager::has_non_injected_extensions`).

⚠ **Condition 5 is the one that gets missed, and it is deliberately
self-excluding.** `ensure_spawn_extension` puts `workspace` into the extension
map, so counting it would make one turn's `true` the reason for the next turn's
`true` — an agent that removed its last real extension would keep delegating
forever off a grant it derived from itself. The consequence to recognise: a chat
with **every** extension disabled has no `subagent` tool, which looks exactly
like a registration bug and is not one.

If the tool is missing, check all five before suspecting registration.

⚠ **The gate lives in the agent's turn loop, so the HTTP API cannot be used to
test it.** Three separate attempts to exercise it over HTTP all measured nothing,
each for a different reason, and each looked like a working probe:

- `GET /agent/tools` lists what the **ExtensionManager** holds. The advertising
  gate is applied by the agent when it builds the model's tool list, so
  `workspace__subagent` appears here even in a session where delegation is off.
  Its presence is not evidence the model was offered it.
- `POST /agent/call_tool` goes straight to `ExtensionManager::dispatch_tool_call`
  and never passes the `is_spawn_tool_call` check at `agent.rs:3847`, so a spawn
  invoked that way is not refused whatever the mode. That route is user-initiated
  (secret key + `X-User-Action`) and privacy **Gate C still applies** to it, so
  the privacy barrier is intact — what is bypassed is the model-facing capability
  gate, not a privacy one. Do not "fix" this by duplicating the gate into the
  route without first deciding whether a user calling the tool deliberately
  should be bound by a gate that exists to constrain the *model*.
- Condition 2 reads `self.config.biorouter_mode`, a **snapshot taken when the
  Agent was constructed**. Flipping `BIOROUTER_MODE` through `/config/upsert` on
  a running daemon does not reach an agent that already exists, so a live-toggle
  probe reports "no change" even where the gate is wired correctly.

Test the gate where it is: the unit tests in `agents/agent.rs`
(`subagents_enabled_injects_the_workspace_extension_with_the_spawn_tool_only`,
`an_explicit_workspace_entry_still_hides_the_spawn_tool_when_delegation_is_off`,
`subagents_disabled_injects_nothing`), via
`cargo test -p biorouter --lib -- subagent` (102 tests).

### Browser access (`biorouter serve`)

`biorouter serve` (alias `headless`) starts `biorouterd`, points it at the built interface and
prints a URL. The daemon serves the SPA **on its own origin**, so nothing is proxied. This
replaced a standalone `biorouter-headless` binary and its Linux tarball, both deleted
2026-08-23; release assets went 11 → 10. Design and reasoning:
[`docs/deployment/serve-decisions.md`](docs/deployment/serve-decisions.md) (SD-1..SD-8),
[`serve-architecture.md`](docs/deployment/serve-architecture.md),
[`browser-access.md`](docs/deployment/browser-access.md).

- **A browser session cannot change its model or provider, deliberately** (SD-1).
  `set_config_provider` still 409s, and `commands/serve.rs` spawns the daemon with
  `Stdio::null()` so no proof-of-user digest is installed — the property is enforced by the
  process model, not by a check. The tier implied by the operator's `biorouter configure`
  choice then holds for every session in that daemon. This closes open question 23 in
  `docs/security/privacy-tiers-execution-plan.md`. ⚠ The refusal text is written for an
  **agent**, so every surface that writes a capability key asks `isBrowserSurface()`
  (`ui/desktop/src/utils/surface.ts`) and explains *before* the user can reach the 409. Do not
  "fix" browser mode by weakening the refusal.
- **A control that can never work here says so, before it is touched** (SD-8). The same
  `Stdio::null()` that closes SD-1 means NO approval carrying `requires_user_proof` can ever
  be granted on a `serve` daemon — for anyone, always. So `confirm_tool_action` answers a
  refusal with `reason: "unproven"` or `"noKeyInstalled"` and a sentence written for a person;
  the approval card reads `isBrowserSurface()` up front and renders that in place of its
  buttons; and the roster withholds the tools whose only path runs through such an approval
  (skills' three mutations, the extension manager's install and delete). Browsing, searching
  and `manage_extensions` stay — the last one's approval is a fallback for an
  operator-pinned-off extension and its ordinary path needs none. ⚠ Not a security change:
  nothing that was refused becomes permitted. The availability flag is sampled ONCE per roster
  and threaded, so a roster can never half-believe a person is reachable.
- **Proof of a person is checked at the resolution choke point, not at one route.** Every door
  that answers a parked decision — the HTTP route, an Agent Drafter app's WebSocket, ACP, the
  CLI prompt, the TUI modal, an ancestor agent's relay — passes a `DecisionAuthority` into
  `PendingUserActions::resolve_matching`, which refuses an *allow* on a proof-backed
  `ToolApproval` from a surface that cannot prove a person acted. It is a newtype with a
  private field, not an enum, because an enum variant is a literal any crate could write; a
  repo-walk test in `pending_user_action.rs` asserts the two privileged constructors appear
  only in `routes/action_required.rs` and the two CLI surfaces. ⚠ The gate keys on
  `is_allowed()` and the request's own flag, never on the variant — denials and cancellations
  land from anywhere, and a gate on `ToolApproval(_)` would kill every bridged coding-agent
  approval on the desktop. A refusing door must also immediately deny: a refusal leaves the
  caller parked by design, and nothing re-raises an ephemeral card.
- **The serving path.** `Settings.serve_ui` (`BIOROUTER_SERVE_UI`) →
  `routes::web_ui::attach`, called **after** `check_token` in `commands/agent.rs` so the shell
  and bundle sit *structurally outside* that middleware rather than being exempted by path.
  The document is gated by a browser token exchanged once for an `HttpOnly; SameSite=Strict`
  cookie; the cookie authenticates **the document only** — API routes still take
  `X-Secret-Key`, so there is no CSRF surface and `check_token` needed no change.
- **`routes::shell`** holds the 16 `/headless/*` endpoints (path kept deliberately; the
  renderer builds `origin + '/headless'`). They had **no authentication at all** on the old
  binary and `fs_read` had no path validation; the port confines every filesystem handler to
  an allowlist and refuses credential stores by name.
- **WebSocket origins**: `routes::origin_matches_host` compares `Origin` to the request's own
  `Host`. That is a same-origin test, not a wildcard, and it is what lets a browser reach the
  daemon at a LAN address `is_local_origin` has never heard of.
- ⚠ **Three traps.** The app uses a **HashRouter**, so its routes live in the fragment and
  never reach the daemon — that is the only reason `/sessions/{id}` (a real API route) does
  not collide with the app's own; a history router would break pages silently. The bundle must
  be **root-base** (`npm run build:web` → `ui/desktop/src/web`), because Forge forces a
  *relative* base and a relative bundle served at `/` breaks deep links while the landing page
  looks fine. And `<exe>/../web` resolves for the packaged app and Windows zip but **not** for
  deb/rpm (`/usr/bin/../web` = `/usr/web`), hence `/usr/share/biorouter/web`.
- **Tests:** `cargo test -p biorouter-server --lib routes::web_ui routes::shell`,
  `cargo test -p biorouter-cli --lib commands::serve`, the `serve` job in
  `.github/workflows/rust.yml`, and `smoke_serve` in
  `scripts/smoke-test-release-artifacts.sh`.

### Communication Flow

```
CLI (offline)  → calls biorouter crate APIs / the on-disk session store directly
CLI (live)     → HTTP + SSE to a running biorouterd
GUI            → Electron main spawns biorouterd → React renderer communicates
                 via HTTP/WebSocket (type-safe client from generated OpenAPI)
```

The CLI is **two** things, and the split is the commonest CLI failure:

- `session list` / `export` / rename / remove touch only the on-disk session store
  and need nothing running.
- `session send` / `watch` / `attach` / `cancel` require a **reachable
  `biorouterd`**, because live turns are process state, not files.

The trap: the desktop app starts its daemon on an *ephemeral* port under a
per-launch random secret (`ui/desktop/src/biorouterd.ts`, `main.ts`), and writes
neither of them anywhere the CLI can read. **Having the app open does not satisfy
the requirement.** Starting your own `biorouterd agent` gives you a *second* daemon
that shares the session store (a directory) but not live turns (process memory),
so `session watch` will report nothing running while the GUI is visibly working.
To make both halves share one process, use the external-backend setup in
[`docs/agent-loop/workspace-control.md`](docs/agent-loop/workspace-control.md).

After changing server routes, always run `just generate-openapi` to regenerate the TypeScript client.

## Configuration

- User config: `~/.config/biorouter/config.yaml` (providers, API keys, extensions)
- Session history: `~/.config/biorouter/sessions/` (SQLite)
- Workflows/skills: `~/.config/biorouter/workflows/` and `~/.config/biorouter/skills/`
- Secrets: OS credential store (macOS Keychain / Windows Credential Manager / Linux Secret Service) via the `keyring` crate, read once per process and cached in memory so macOS shows at most one Keychain authorization prompt per run (tell users to click "Always Allow"). `BIOROUTER_DISABLE_KEYRING=true` switches to plaintext `secrets.yaml`; headless Linux falls back to it automatically. On Windows the secrets blob is chunked across credentials (2560-byte cap each). `just copy-binary` re-signs dev binaries with the Developer ID (when present) so Keychain grants survive rebuilds. Logic in `crates/biorouter/src/config/base.rs`; see `docs/security/secret-storage.md`.

Key environment variables:
- `ALPHA=true` — Enable alpha features
- `BIOROUTER_EXTERNAL_BACKEND=true` — Use external backend (for UI dev)
- `BIOROUTER_EXTERNAL_PORT` — Backend port (default 3000)
- `BIOROUTER_SERVER__SECRET_KEY` — Server auth key (default `test` in debug-server mode); uses `__` for nested config keys
- `BIOROUTER_RECORD_MCP=1` — Re-record MCP integration test cassettes (VCR-style)
- `BIOROUTER_UPDATE_FEED_URL` — Override the auto-update feed (GitHub by default) with a generic static-file feed (`latest-mac.yml` + per-arch app zips). For self-hosted/enterprise update mirrors and for the one-click-update swap test (`ui/desktop/scripts/notarized-swap-test.sh`). See `docs/releases/auto-update-test-checklist.md`.
- `BIOROUTER_UPDATE_AUTO_INSTALL=1` — Test-only: auto-trigger `quitAndInstall` on download (gated behind `BIOROUTER_UPDATE_FEED_URL`, so it never fires in production).
- `BIOROUTER_SKIP_NOTARIZE=1` — Build a signed-but-not-notarized macOS app (forge.config.ts), for fast local/test builds. Release builds leave it unset (fully notarized + stapled).

**macOS packaging gotchas (learned building the one-click-update test):** package under **hermit Node 24** (`source bin/activate-hermit`), not Homebrew Node 26 — under Node 26 `electron-forge package` silently no-ops (exits 0 with no `.app`). And ensure `ui/desktop/src/bin/{biorouter,biorouterd}` are the macOS **arm64** Mach-O binaries (`just copy-binary` or restage from `target/release/`), not the Linux ELF binaries a prior Linux release leaves behind — otherwise the bundled backend can't exec and the app quits on launch. Build only `--targets @electron-forge/maker-zip` to skip the DMG maker's `appdmg` native dep when you just need the auto-update zip.

## Documentation

**All prose documentation goes under `docs/`. There is no other documentation folder — do not create one.** No `proposals/`, `plans/`, `notes/`, `rfcs/`, or a stray Markdown file at the repo root. A `proposals/` folder was created in July 2026, drifted into holding a stale duplicate of a document already migrated into `docs/`, and was deleted on 2026-07-26; that is the failure mode this rule exists to prevent. The only Markdown outside `docs/` is a **closed** list of root files plus per-package READMEs that ship next to the code they install (for example `integrations/jupyter-ai/README.md`, `landing/`). The root list, in full:

- **What GitHub and the project's governance expect:** `README.md`, `CONTRIBUTING.md`, `SECURITY.md`, `BUILDING.md` / `BUILDING_DOCKER.md` / `BUILDING_LINUX.md`, `GOVERNANCE.md`, `MAINTAINERS.md`, `ACCEPTABLE_USAGE.md`, `RELEASE.md`.
- **AI-contributor steering files:** `CLAUDE.md` (this file), `AGENTS.md`, `HOWTOAI.md`. These must sit at the repo root because the tools that read them look there — moving one under `docs/` would silently stop it being loaded.
- **`design.md`** — the Parchment design system, cited by the theme-families section above.

The list being closed is the point: adding a root Markdown file means amending this list, not appending quietly.

Two docs govern the tree and both are binding:

- [`docs/organization.md`](docs/organization.md) — **where** a document goes. Living documentation is filed by subsystem at the top level; records of finished work go to `docs/history/<campaign>/`. A new subsystem folder needs a `README.md` index, a row in `docs/README.md`'s by-topic table, and a row in `docs/organization.md` §2.
- [`docs/contributing/documentation-style.md`](docs/contributing/documentation-style.md) — **what it looks like inside**. Every file opens with the context header (`> **What this is.** / **Status:** / **Audience:**`), uses sentence-case headings, kebab-case filenames, and closes with `## Related documentation`.

A proposal or design is not an exception: write it in the subsystem folder it belongs to with `Status: Proposed`/`Current`, and move it to `docs/history/` when the work concludes.

## Code Review Standards

From `.github/copilot-instructions.md`: Reviews focus on **security, correctness, and architecture patterns** — not style (handled by CI) or refactoring suggestions. Flag issues only with >80% confidence. Security-sensitive code (auth, permissions, credential handling) requires human review regardless of AI assistance.

From `HOWTOAI.md`: Avoid using AI-generated code for security logic, complex business rules, or schema migrations without thorough human review. Always get human review for MCP protocol implementations and async/concurrency logic.

Use `.biorouterhints` to guide BioRouter's coding style (patterns, error handling, tests) and `.biorouterignore` to protect sensitive files from being read by the agent.

## Connected Repositories & Ecosystem

BioRouter is the hub of a small ecosystem of Baranzini Lab / UCSF repositories: the app itself, its public website + marketplace, a shareable-skills repo, and a set of installable MCP "agent" extensions and skill packs. This section maps every related repo/resource so a future session knows how the pieces fit together. All GitHub URLs below were verified to return HTTP 200 on 2026-06-20 unless noted.

### Core repos

| Repo | Purpose | Local path | GitHub |
|------|---------|------------|--------|
| **biorouter** (this repo) | The main app: Rust workspace (CLI `biorouter`, daemon `biorouterd`, core agent lib, MCP servers) + Electron/React desktop GUI. | `/Users/wgu/Desktop/biorouter` | https://github.com/BaranziniLab/biorouter |
| **landing site** (now in this repo) | Public website + docs, deployed at **http://biorouter.ucsf.edu/** (custom-domain GitHub Pages, `CNAME` = `biorouter.ucsf.edu`). Hosts the BAAM marketplace (`baam.html`), `download.html`, `docs.html`, `skills.html`, and the machine-readable **`registry.json`** (the authoritative marketplace catalog of extensions + skills). **As of the 2026-06-29 consolidation it lives in this repo under [`landing/`](landing/)** and is published by [`.github/workflows/deploy-landing.yml`](.github/workflows/deploy-landing.yml) (Pages source = "GitHub Actions") — so the site ships and versions with the app. The `biorouter.ucsf.edu` custom domain was **cut over** to this repo (released from `biorouter-landing`, claimed on `biorouter`); the live site now serves from `landing/`. The standalone `biorouter-landing` repo was **deleted** (remote + local) on 2026-06-29 after verification; its full history was archived to a git bundle (`~/Desktop/biorouter-landing-archive-2026-06-29.bundle`) kept as insurance. This `landing/` folder is now the single source of truth for the site. See [`landing/MIGRATION.md`](landing/MIGRATION.md). | `/Users/wgu/Desktop/biorouter/landing` | https://github.com/BaranziniLab/biorouter (was: https://github.com/BaranziniLab/biorouter-landing) |
| **biorouter-skills** | Shareable Biorouter skills. Each skill ships as a GitHub Release asset `skill-<id>` → `<id>.zip`. Local clone has two remotes: `baranzini` → BaranziniLab (canonical) and `origin` → Broccolito (dev fork). | `/Users/wgu/Desktop/biorouter-skills` | https://github.com/BaranziniLab/biorouter-skills (fork: https://github.com/Broccolito/biorouter-skills) |

### Marketplace & docs (BAAM)

**BAAM** = the Biorouter Agent/extension/skill marketplace, served from the landing site. The durable, machine-readable source of truth is `registry.json` in the landing repo (`"source": "https://biorouter.ucsf.edu/baam"`), which the app reads to list and one-click-install extensions (`.brxt` bundles) and skills (`.zip`).

- Marketplace page: http://biorouter.ucsf.edu/baam.html
- Registry (catalog JSON): https://biorouter.ucsf.edu/registry.json (source: `landing/registry.json` in this repo, generated by `landing/scripts/build-registry.mjs`)
- Docs: http://biorouter.ucsf.edu/docs.html · Skills gallery: http://biorouter.ucsf.edu/skills.html · Downloads: http://biorouter.ucsf.edu/download.html
- Baranzini Lab: https://baranzinilab.ucsf.edu/ · UCSF: https://www.ucsf.edu

### Extension agents (installable MCP servers, `.brxt` bundles)

These are pluggable MCP extensions distributed via the marketplace; each lives in its own repo and publishes a `.brxt` bundle as a GitHub Release asset.

| Agent | Purpose | GitHub | Notes |
|-------|---------|--------|-------|
| **SPOKEAgent** | Cypher queries on the SPOKE biomedical knowledge graph (diseases, genes, proteins, drugs, pathways); bundles a `spoke-knowledge-graph` skill. | https://github.com/BaranziniLab/SPOKEAgent | Needs `SPOKEAGENT_PASSCODE` (UCSF wiki credentials page). |
| **UCSFOMOPAgent** | Natural-language SQL over the UCSF OMOP de-identified clinical database (OMOP CDM EHR data, read-only). | https://github.com/BaranziniLab/UCSFOMOPAgent | Requires UCSF credentials. |
| **CDWAgent** | Multimodal natural-language access to the UCSF Clinical Data Warehouse (cohorts, labs, imaging, notes/NLP); read-only. | https://github.com/BaranziniLab/CDWAgent | Requires UCSF network credentials (`CAMPUS\username`). |
| **PlaywrightAgent** | Browser automation via Microsoft's `@playwright/mcp` (navigate, extract, fill forms); no vision model needed, needs Node.js. | https://github.com/BaranziniLab/PlaywrightAgent | |
| **CodeGraphAgent** | Pre-indexed code knowledge graph ("who calls X?", "what breaks if I change Z?") across 23 languages incl. R/Julia/MATLAB/Perl; tree-sitter fork of CodeGraph. | https://github.com/Broccolito/CodeGraphAgent | Broccolito org. |
| **BiorOffice** | Create/read/edit Word/Excel/PowerPoint from chat (built on OfficeCLI, no MS Office needed); ships four bundled office skills. | https://github.com/Broccolito/BiorOffice | Broccolito org. |

### Skill packs (`biorouter-skills`, ~85 skills)

All skills are published as releases of **`BaranziniLab/biorouter-skills`** (asset path `releases/download/skill-<id>/<id>.zip`). Grouped by category in `registry.json`:

- **Core** (9): `scientific-research`, `taste-skill` (frontend design), `anti-ai-writing`, `ggplot-visualization`, `r-scripting`, `python-scripting`, `ralph`, `superpowers`, `ucsf-hpc`.
- **Developer** (8): `code-review`, `code-simplifier`, `commit-commands`, `skill-creator`, `hookify`, `code-modernization`, `playground`, `frontend-design`.
- **Biomedical** (~68): genomics/omics + clinical bioinformatics skills, e.g. `single-cell`, `variant-calling`, `differential-expression`, `pathway-analysis`, `spatial-transcriptomics`, `proteomics`, `metabolomics`, `chip-seq`, `atac-seq`, `crispr-screens`, `phylogenetics`, `clinical-biostatistics`, `clinical-databases`, `multi-omics-integration`, `workflows`, etc. See `registry.json` for the full list.

### Workflows

- **biorouter-workflows** — shareable workflow YAML definitions (e.g. `ehr-diabetes-dashboard.yaml`, referenced from `baam.html` via `raw.githubusercontent.com`). GitHub: https://github.com/BaranziniLab/biorouter-workflows (the `Broccolito/biorouter-workflows` URL in `baam.html` 301-redirects to the BaranziniLab repo).

> Maintenance note: the authoritative, always-current catalog of extensions and skills is `landing/registry.json` in this repo (formerly the `biorouter-landing` repo). When agents/skills are added or versions change, that file (not this section) is the source of truth — re-derive this section from it if it drifts.
