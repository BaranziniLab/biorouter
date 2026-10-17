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

**What CI runs.** `rust.yml`'s `test` job runs `--lib --bins` on all three OSes and, on
ubuntu only, every `tests/*.rs` integration binary **except** the exclusion table inside its
`cargo test (integration binaries, no network)` step. That step runs in a loopback-only network
namespace and never passes `--ignored`. So a new integration binary is covered the day it lands,
and a test that reaches the network fails there rather than passing while the third party is up.
If a binary genuinely cannot run offline, put its live tests behind `#[ignore]`, or give the binary
a line in that table saying why. Before 2026-09, `rust.yml` ran only the sandbox's integration
target, so a green run said nothing about the rest of `tests/`.

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
measured 2026-09-11**, and again on the redesign branch on 2026-10-11. That
meets the precondition this note used to set for
wiring `format:check` into `lint:check` or CI — whether to do it is a
maintainer's call — but nothing holds the count at zero, so re-measure right
before wiring it in: a gate that fails on arrival gets disabled rather than
obeyed. Until then, run `format:check` yourself before pushing frontend
changes.

⚠ **The retired-name guard** (`scripts/check-no-goose-block.sh`, R-31) fails
when any tracked file's text **or path** names the former upstream project or
its publisher. `NOTICE` is the only file allowed to (apart from the guard itself,
which has to spell what it looks for), because it keeps the license attribution
verbatim (owner ruling, 2026-10-11; `LICENSE` no longer carries a publisher
copyright line). It reads the tracked tree through
`git grep -I`, so ignored build output and binary files never trip it, and it
matches the publisher only by its company name, possessive, logo file names and
repository URLs, never the ordinary word "block". It runs in
`just check-everything`, in `rust.yml`'s `guards` job and in
`scripts/verify-docs.sh`. When it fires, remove the mention and write "the
former upstream" or "the legacy folder" instead; never widen its allowlist. The
one value the code still has to spell (the Windows legacy folder's author,
below in Configuration) is declared once per language, `LEGACY_APP_AUTHOR` in
`app_folder/mod.rs` and in `ui/desktop/src/utils/appFolder.ts`; everything else
says "the legacy folder" or builds the path from the constant.

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
scripts/release.sh mac-backends 1.80.1 # mac arm64/x64 + signed helpers; adopt-ci supplies Linux/Windows
# scripts/release.sh backends 1.80.1   # optional: build all four targets locally
scripts/release.sh linux-backend 1.80.1 # just the linux x86_64 backend (re-runnable)
scripts/release.sh mac-arm64 1.80.1    # sign + notarize
scripts/release.sh mac-intel 1.80.1
scripts/release.sh windows 1.80.1
scripts/release.sh linux 1.80.1        # GUI deb + rpm; run LAST (corrupts node_modules)
scripts/release.sh cli-linux 1.80.1    # CLI-only deb + rpm, no GUI (biorouter + biorouterd + the web bundle)
scripts/release.sh mac-manifest 1.80.1 # latest-mac.yml for electron-updater
scripts/release.sh verify 1.80.1
scripts/release.sh draft 1.80.1        # draft GitHub release with all 11 assets + notes
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
- **Release assets — exactly 11**: the 6 GUI artifacts `Biorouter-{ver}-arm64.dmg`, `Biorouter-{ver}-x64.dmg`, `biorouter_{ver}_amd64.deb`, `Biorouter-{ver}-1.x86_64.rpm`, `Biorouter-win32-x64-{ver}.zip`, `Biorouter-Setup-{ver}.exe`; the 2 **CLI-only** Linux packages (no GUI) `biorouter-cli_{ver}_amd64.deb` and `biorouter-cli-{ver}-1.x86_64.rpm` (`biorouter` + `biorouterd` + the web interface at `/usr/share/biorouter/web`, built by `scripts/build-cli-linux-packages.sh` → `dist/cli/`, smoke-tested in clean Debian/Rocky containers); and the 3 **macOS auto-update** artifacts `Biorouter-darwin-arm64-{ver}.zip`, `Biorouter-darwin-x64-{ver}.zip`, and `latest-mac.yml`. The two darwin zips are the signed+notarized `maker-zip` app archives (Squirrel.Mac format, `Biorouter.app` at root); `latest-mac.yml` (generated by `ui/desktop/scripts/generate-update-manifests.js`, run from the `mac-manifest` phase, which `draft` invokes) lists both with base64 SHA-512 + size so `electron-updater` can do the in-app one-click "Restart & Update" on macOS — **without the zips + yml, clients 404 and drop to the assisted GitHub-download fallback**. electron-updater 6.x picks the arch by the `arm64`/`x64` token in the zip filename, so both clients share one manifest. Don't also upload the unversioned `Biorouter.zip` / `Biorouter_intel_mac.zip` from `out/<platform>/` — they're build intermediates, not release artifacts. ⚠ **Windows now ships a Squirrel installer** (`Biorouter-Setup-{ver}.exe`, from `maker-squirrel` in `forge.config.ts`), and the assisted updater downloads *that* in preference to the zip — running it over an existing install upgrades in place and keeps the shortcuts. The filename is agreed in two places that never import each other (`WINDOWS_SETUP_EXE` in `forge.config.ts`, the candidate list in `githubUpdater.ts`); `ui/desktop/src/utils/windowsInstallerAsset.test.ts` pins them together, because a drift there still builds and still uploads while the updater silently matches nothing. It is NOT electron-updater: that is the electron-builder/NSIS world, and this project packages with forge. Windows is still not a silent one-click restart — the user runs the installer — and the zip remains as a fallback for releases cut before the installer existed. Linux (deb/rpm) still has no in-place installer and keeps using the assisted-download fallback. See `docs/releases/auto-update-test-checklist.md`.
- **`verify` checks more than file existence**: alongside the 9 on-disk artifacts it runs `scripts/check-brand-consistency.sh` and `scripts/smoke-test-release-artifacts.sh` (whose `smoke_serve` installs the CLI package, runs `biorouter serve` and drives the whole browser contract), and fails the phase if any of them does. `just check-everything` runs the brand check too — it asserts `"productName": "Biorouter"` and the BR-monogram brand assets, which is why every packaged artifact name above is `Biorouter`, lowercase `r`.
- **Linux glibc baseline**: the linux backend is cross-compiled on `rust:1.92-bullseye` (glibc 2.31), NOT rolling `rust:latest` (now trixie, glibc 2.39, which yields binaries that fail to start on Debian 12 / Ubuntu 22.04 / RHEL-Rocky 9). The pin lives in `LINUX_RUST_IMG` in `scripts/release.sh`. ⚠ **This line used to say `cli-linux`'s smoke test catches a regression here. It cannot, and that was measured on 2026-08-20**: those containers are `debian:bookworm` (glibc 2.36) and `rockylinux:9` (glibc 2.34), so a floor raised from 2.31 to 2.34 passes every smoke test in the repo and breaks only on the user's older machine. What actually enforces it is `assert_glibc_floor` in `scripts/release.sh`, called from `cmd_linux-backend` — a symbol-table read against `GLIBC_MAX`, which also fails when it can read no symbol at all rather than passing vacuously. `linux-backend` `rm -rf`s the target dir first to force a from-scratch compile against the pinned glibc (cached objects keep stale symbol versions).
- ⚠ **A draft pins the commit the tag will get.** `draft` runs
  `gh release create v{ver} --draft --target "$RELEASE_SOURCE_SHA"`, and
  `require_remote_main_exact` sets that variable from `HEAD` only after asserting
  `HEAD` equals a freshly fetched `origin/main` — so the target is the exact commit
  the artifacts were built from, not whatever `main` happens to be. It used to be
  `--target main`, which recorded `targetCommitish` = whatever `main` was *then*.
  **No tag exists yet** — a draft shows as `untagged-…` — and `publish` creates the
  tag from that recorded target, not from today's `main`. Since 1.91.0 `publish`
  also reads `targetCommitish` back and refuses unless it equals the source commit,
  because the pin guaranteed it only by construction: a draft made any other way (a
  hand-run `gh release create`, or a script from before the pin) would have
  published a wrong tag with every other check passing. So a release that is drafted, then fixed,
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
  - `scripts/release.sh draft <ver>` runs `mac-manifest`, asserts all 11 assets exist (dying on the first missing one), then `gh release create v{ver} --draft --target "$RELEASE_SOURCE_SHA" --title "Biorouter v{ver}" --notes-file docs/releases/notes/v{ver}.md <11 assets>`.
  - `scripts/release.sh publish <ver>` re-runs `verify`, refuses unless `v{ver}` already exists **as a draft** whose `targetCommitish` is the source commit, refuses unless `gh run list --workflow release-artifact-smoke.yml` shows a successful run titled `Release artifact smoke v{ver}`, and only then `gh release edit v{ver} --draft=false`.
  - So `all` stops at the draft and prints *"draft created; run the native Windows smoke workflow, then: scripts/release.sh publish {ver}"*. Hand-rolling a `gh release create` bypasses the Windows smoke gate — don't. Verify macOS with `xcrun stapler validate <app>` and `codesign -dv <app>` as well.
- **Release notes** live at `docs/releases/notes/v{ver}.md`, one file per version.

## Architecture

### Rust Workspace (`crates/`)

| Crate | Binary | Purpose |
|-------|--------|---------|
| `biorouter` | — | Core agent library: main agent loop, LLM providers, MCP extension manager, session/conversation state, workflow execution, scheduling |
| `biorouter-server` | `biorouterd` | Axum REST API + WebSocket server; routes in `src/routes/`; OpenAPI spec generated via utoipa |
| `biorouter-cli` | `biorouter` | Interactive CLI; subcommands in `src/commands/` |
| `biorouter-mcp` | — | Built-in MCP servers (Developer, Biorouter Copilot, Web & Documents, Memory, Auto Visualiser, Knowledge, Agent Drafter, DataSQL, Files, Compute). Also hosts `active_work.rs`, which is *not* a server but the process-global registry of long-running work (background shell jobs + running subagents) that `GET /active_work` reads |
| `biorouter-sandbox` | — | Capability-scoped sandboxed execution (`docker.rs`, `seatbelt.rs`, `local.rs`, `environment.rs`, `shell_sandbox/`); a leaf crate with no engine deps |
| `biorouter-acp` | — | Agent Communication Protocol for multi-agent orchestration |
| `biorouter-bench` | — | Benchmarking harness |
| `biorouter-test` | — | Integration tests |

Only five of `biorouter-mcp`'s servers are spawnable as **subprocesses** via
`biorouter mcp <name>` — `autovisualiser`, `computercontroller`, `webdocuments`,
`developer`, `memory` (the `McpCommand` enum in `mcp_server_runner.rs`).
⚠ `computercontroller` is the **Biorouter Copilot** server: the extension key kept its
legacy spelling so existing `config.yaml` entries and per-chat tool overrides keep
working, and only the display name changed. The web, document and cache tools that
used to live under that key — `web_scrape`, `xlsx_tool`, `docx_tool`, `pdf_tool`,
`cache` — moved to `webdocuments`, so a `computercontroller__docx_tool` written
anywhere now names nothing. Three shipped office skills did exactly that.
`agent_drafter` (as `appcontrol`), `datasql`, `files_server` and `compute_server`
are injected **in-process** by `configure_agent` in `routes/apps.rs` via
`add_inprocess_server`, and have no subprocess name; their absence from that enum
is not dead code.

### Core Agent Library (`crates/biorouter/src/`)

- **`agents/agent.rs`** — Main agent loop: LLM interaction, tool dispatch, context management
- **`agents/extension_manager.rs`** — MCP extension lifecycle and tool registration
- **`providers/`** — 45+ provider modules (Anthropic, OpenAI, Azure, AWS Bedrock, Databricks, Ollama, etc.); `factory.rs` creates providers, `base.rs` defines the abstract interface
- **`session/`** — Session persistence (SQLite via sqlx)
  - ⚠ **`sessions.updated_at` is ACTIVITY, and opening a chat is not activity (R-30).**
    Everything sorted by activity reads it: the sidebar's Last activity, History's date
    buckets, `GET /sessions`, `biorouter session list` and `--resume` with no id. A door
    that opens a chat runs its setup inside `session::activity::opening`, and a
    machine-wide sweep (uninstalling an extension prunes every saved roster) inside
    `session::activity::housekeeping`, the same task-local scope under a second name
    (`session/activity.rs`). Inside it, and only there, four **setup** writes leave
    `updated_at` alone: the provider bind (`bind_provider_if_allowed`), the
    extension-data write, a `SessionUpdateBuilder::apply` that sets `extension_data`
    and nothing else, and the empty-chat working-directory write. A message, a turn's
    token accounting, a rewrite or truncation and a rename stamp whether or not they run
    inside a scope, and outside one every write stamps, so a forgotten scope costs a chat
    that jumps on open (the old bug), never one that stays put while a turn runs. The
    provider bind also stamps only when the provider or `model_name` differs from what
    the row records, at every door. What counts as activity is coordinator ruling 16: a
    message or turn, a rename, an explicit switch to a different model, a composer
    extension or skill toggle, `workspace_set_tools` aimed at the chat, a Crew grant on
    it. ⚠ The scope does not follow a `tokio::spawn`, so a door awaits its setup writes
    inline, and `only_the_opening_and_housekeeping_doors_enter_a_scope` counts every door
    that enters one (six files, measured 2026-10-11): wrapping a deliberate change in a
    scope fails that census rather than quietly freezing a chat. The renderer shows a
    person's own rename at once: the name channel carries `activityAt`, which raises the
    row (`laterActivity` in `utils/sessionNameSync.ts`), and the sidebar releases its
    pointer hold for that one move. Full list of doors and tests:
    [`docs/desktop-ui/sidebar-chat-list.md`](docs/desktop-ui/sidebar-chat-list.md)
    ("What moves a chat").
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
- **`components/knowledge/`**: the Knowledge route, under the sidebar's
  Components disclosure (between Skills and Built apps). Provides KB selector
  with cmd-K-style palette, ingest panel (dropzone / paste text with URL extraction / staged
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
- **Holding the daemon secret does not make a caller the user.** That was the premise behind
  leaving the `/knowledge/*` read routes, `GET /sessions` and `DELETE /sessions/{id}` ungated. A
  public chat's shell recovers the secret with `ps eww`, and QA used it to read a private base and
  delete a private chat (H2/M1/M2/F0, 2026-09-10). Every HTTP route that names a chat or a
  knowledge base now asks `routes::session_reach`'s one decision: a private target needs the
  user-action proof or a stated private capability. The rules that follow:
  - A route that names one chat calls `session_reach`, and refuses with its exact plain text.
  - Listings filter through `HttpCaller::lists_session`.
  - Running work is the same rule reached through the row's chat. `GET /active_work` filters
    through `HttpCaller::lists_work`, and its cancel asks `work_reach` before anything stops. A
    row that names no chat is treated as a private chat's, so a registrant that knows its chat must
    set `ActiveWorkItem::session_id`. The shell's rows take it from the `_meta` session id.
  - A schedule is gated on its WORK, through `schedule_reach`: every write (create, run now,
    pause, resume, re-time, delete, and `POST /workflows/schedule`) is refused to an unproven
    public caller when a chat the schedule names is private, or when the model its runs bind is —
    resolved by `scheduler::scheduled_run_provider_name`, the run's own resolution. Public work
    stays open. ⚠ The daemon follows `schedule.json`, so the file is not a boundary.
    ⚠ **Write-time gating alone is not enough: a run re-resolves its model when it starts.** Each
    arming door (create, re-time, resume, `/workflows/schedule`) records its caller's standing
    through the scheduler's `_armed` methods (`ScheduledJob::armed_with_private_reach`), and
    `execute_job` refuses a private model on a public-only record (`scheduled_run_refusal`).
    Without it, deleting a public creator chat moved a secret-only caller's schedule onto the
    private default. ⚠ **And on NO record, when the private default stands in for a creator chat
    that is gone** (`RunModelSource::DefaultInPlaceOfCreator`): `/loop`, `/schedule` and
    `manage_schedule` jobs record `true` only from a private chat, so a public chat's schedule
    needed no arming request at all — one secret-only `DELETE /sessions/<creator>` (QA, 2026-09-14).
    Never record `false` from a chat: a person who later moves that chat private would be refused.
    The trait's `_armed` defaults DROP the standing — only for test doubles.
    Renderer schedule writes go through `ui/desktop/src/schedule.ts` alone; a source guard in
    `schedule.userProof.test.ts` fails on any other importer of a generated schedule function.
    ⚠ **The `/schedule` and `/loop` verbs are a second door** (a public chat takes `/reply` from any
    secret holder, and `execute_command` runs before the model): they ask `slash_verb_reaches` on
    ONE sampled `CallCapability` — a private chat manages anything, any other chat only work
    `scheduler::schedule_work` calls public. That one function is also what `schedule_reach` asks;
    never re-spell "private work" at a door. A refused tick is a `ScheduledRunRefused` and gives
    its `max_runs` firing back; `/schedule run` asks `scheduled_run_preflight` before requesting
    a background run and acknowledges the request without claiming execution has started.
  - **A restore binds a chat's FIRST model** when its row names none: `restart`,
    `update_working_dir` and a workspace turn into a cold chat go through
    `Agent::restore_provider_from_session(session, DefaultBind)`, and the door computes the
    standing with `routes::agent::RestoreStanding` (= `new_chat_bind_decision` on a private
    default: proof on a keyed daemon, SD-12 on a keyless one) and asks `refuse_before_restoring`
    before it changes anything. Ungated, a secret-only restart wrote `versa_azure` onto a public
    chat and made the `(None, CreatorChat)` run-time cell trust a bind no person made (QA,
    2026-09-14).
  - `GET /sessions/changes` shows a change only through `lists_session` — on the tier it was
    recorded with AND the chat's row as it is now (`visible_changes`) — observes only those rows,
    and a withheld change must never answer a poll early — a poll that returns when a private row
    moves times it.
  - Every `/knowledge/bases/{id}` route sits in `knowledge::router`'s `base_routes`, behind
    `gate_knowledge_base`. Put any new `{id}` route there.
  - ⚠ **The renderer must send `userActionHeaders()` on every such call.** A missing proof is not an
    error: private rows silently vanish, and the Knowledge view's prune effects then read them as
    deleted.
  - A `biorouter serve` browser gets its operator's tier on listings and knowledge bases only
    (SD-10).
  - The wiring census (`crates/biorouter/tests/privacy_guard_wiring.rs`) counts every call site.
- **Affiliation is a third axis** (DR-26, plan Phase 6): tier asks *how sensitive*, affiliation asks
  *whose*. HIPAA compliance does not transfer between institutions, so a UCSF model reaching another
  institution's private connector is warned/refused even though both endpoints are Private.
- **The master switch** lives in its own record beside `config.yaml`, **not in it** and **not in an
  env var** — the agent has `developer__shell`, so a switch it can edit is not a switch. Loaded once
  per process; a load error resolves to ON. ⚠ The record is still agent-writable (DR-17), so an OFF
  answer is **announced, never prevented**: one `WARN` at load, `BIOROUTER_PRIVACY_TIERS_RECORD` on
  `/config` (served from memory), and `PrivacyTiersOffNote` above every composer. Each door stamps
  the record with the value it wrote; OFF with no matching stamp reads *turned off outside the app*.
  The stamp is forgeable — a signal, not a barrier; see `privacy/master_switch.rs` and §10.6.
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

### Secret guard (BR-23, rebuilt for QA-C H1)

The always-on floor that keeps credential files (`~/.aws/credentials`, SSH private keys,
`secrets.yaml`, `.env`) away from the model in every chat, mode and tier. Reference:
[`docs/security/secret-guard.md`](docs/security/secret-guard.md).

- **Arguments are judged after resolution, never as raw tokens.** `crates/biorouter-mcp/src/secret_guard/`
  (`lex` → `expand` → `resolve`) reads a command the way the shell will — `~`, `$VAR`, globs
  against the real directory, `cd`/`pushd`, nested `sh -c`/`eval`/here-documents, symlinks,
  case folded — and is shared by the dispatch scan (`secret_guard_denial`), `developer__shell`
  (`validate_shell_command`, resolved from the directory the command really runs in).
- ⚠ **Fail closed: a match is a refusal whether or not the file exists.** The old `exists()` gate
  was asked of the *unexpanded* token, which is exactly how `cat ~/.aws/credentials` reached a
  public model. Do not restore it to quiet a false positive — the refusal message and a
  `.biorouterignore` negation are the escape hatch.
- **Output is scanned too.** `guardrails/secret_output.rs` withholds private keys, AWS keys and
  provider-store values from every tool result and error inside `dispatch_tool_call`'s future —
  the one place the coding-agent bridge's results pass (they skip `guard_tool_result`).
- **Tests:** `cargo test -p biorouter-mcp --lib -- secret_guard h1_` and
  `cargo test -p biorouter --lib -- secret_output extension_manager`. The H1 tables use a fake
  HOME; never point a test at the operator's real `~/.aws`, `~/.ssh` or `~/.config/biorouter`.

### Knowledge feature

The Knowledge feature (built across Plans 1-6 in `docs/history/knowledge-base-buildout/*`) provides personal, LLM-maintained knowledge bases backed by markdown trees + git history.

- **Backend module:** `crates/biorouter-mcp/src/knowledge/` (types, store, git, graph, credibility, convert/, macros/, subagent/loop_, MCP server).
- **HTTP routes:** `crates/biorouter-server/src/routes/knowledge.rs` covers `/knowledge/bases`, `/ingest` (SSE), `/graph`, `/history`, `/preview`, `/restore`, `/page`, `/active`, `/export`, `/import`.
- **Frontend:** `ui/desktop/src/components/knowledge/` (view shell, KB selector, ingest panel, force-graph + change-log drawer). There is no chat-side KB chip any more: a chat's knowledge bases are the Knowledge list of the composer's Tools popover (`KnowledgePanel` and `useKnowledgePicker` in `ui/desktop/src/components/bottom_menu/BottomMenuKnowledgeSelection.tsx`, mounted by `bottom_menu/ToolsChip.tsx`; see "Desktop shell geometry").
- **Storage layout:** `~/.config/biorouter/knowledge/<kb-id>/` with `raw/`, `knowledge/`, `index.md`, `log.md`, `schema.md`, and a hidden `.git/`.
- **One axis, one pointer.** A session's knowledge bases are the *visible* set — everything not in `.hidden-kbs` (machine-wide) or `.hidden-kb-sessions/<sha256(session_id)>` (per session; an empty `[]` means "hide nothing", not "inherit"). KB-less search spans this set with per-hit `kb_id` attribution. Its **primary** is the write target, default single-base read target and Knowledge view's subject. Soul is the product default when the user has expressed no preference. A missing `.active-kb` / `.active-kb-sessions/<digest>` inherits; a bare id pins a choice; a blank file explicitly chooses no primary and must not fall back to Soul. KB-less writes with that explicit no-primary state fail with the candidate list. The primary must remain visible; the daemon repairs selection when its base is hidden or deleted — and the two repairs differ (D2): hiding *promotes* to the first remaining base, deleting *clears* every pointer that named the base to that explicit blank, and a chat that only inherited the pointer is left inheriting. A blank `.active-kb` after a delete is the repair, not its absence. ⚠ The renderer adopts these repairs by re-reading; it never writes one (no `clear_primary` after a delete, no prune against its own base list), and every selection read from the desktop carries `userActionHeaders()`, because the gate refuses a private chat's selection without it. `kb_set_active` changes the primary without narrowing search. Set-only edits send neither `primary_kb` nor `clear_primary`; see [`docs/knowledge-base/multi-kb-implementation-plan.md`](docs/knowledge-base/multi-kb-implementation-plan.md).
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
settings provider grid, `biorouter configure`). Why the catalog, the window
and the request policy look the way they do: a 2026-10 report that local models
"kept returning JSON instead of doing the task" traced to a prompt cut to ~2k
tokens (see the Ollama block below), and once a model saw the whole ~31-40k-token
prompt both catalog models made structured tool calls reliably.

- **Catalog: exactly two models** (`MODEL_CATALOG` in
  `crates/biorouter/src/providers/llamacpp.rs`): `qwen3.8-27b` — Qwen3.8 27B,
  `ggml-org/Qwen3.8-27B-GGUF:Q4_K_M` (the llama.cpp project's own conversion;
  Qwen publishes no GGUF), 19 GB, **the default on every memory tier** — and
  `gemma4-31b` — Gemma 4 31B, Google's own QAT `google/gemma-4-31B-it-qat-q4_0-gguf:Q4_0`,
  18 GB. Both dense, 262k native window, 32 GiB minimum / 48 GiB recommended.
  Each entry pins its GGUF file name, byte size and SHA-256 (measured
  2026-10-09), its Ollama tags (the bare manifest-list tag plus the GGUF-only
  `qwen3.8:27b-q4_K_M` / `gemma4:31b-it-q4_K_M`), its thinking control, vendor
  sampling and `--spec-type`. Unlisted models are still accepted as raw
  `owner/repo:QUANT` HF specs (strictly validated). ⚠ This block described a
  Gemma 4/Qwen3.6 catalog defaulting to `gemma4-12b`/E4B until 2026-10-10, and
  a Qwen3.5 one before that — read `MODEL_CATALOG` rather than trusting a summary.
- **Retired names still resolve, two different ways** (`RETIRED_MODELS`:
  `gemma4`, `gemma4-e2b`, `gemma4-12b`, `gemma4-26b`, `qwen3.6`, `qwen3.6-27b`
  and the v1.85 `qwen3.5-*` / `gemma-4-*` names). For **chat**
  (`chat_model_name` / `resolve_chat_model_source`: configs, sessions,
  workflows, the `/llamacpp/ensure|warmup` routes, `biorouter models local pull`)
  a retired name runs `qwen3.8-27b`, logged once per name. For **cache
  deletion** (`resolve_model_source`: `/llamacpp/delete`, `models local rm`) it
  keeps its OWN former spec, so deleting `gemma4` frees E4B's files, not Qwen's.
  On `LLAMACPP_EXTERNAL_HOST` a name passes through verbatim — that server is
  someone else's and may route on it.
- **Sidecar manager:** `crates/biorouter/src/providers/llamacpp_sidecar.rs` —
  binary discovery (`BIOROUTER_LLAMACPP_BIN` → `<exe dir>/llamacpp/` → dev
  repo path → PATH), `/health` readiness, status snapshots, restart on model
  switch. **Launch order** (`launch_source_in_dir`): the catalog's HF file
  already in the cache, started with `-m` so a launch needs no network (its size
  is checked against the pin, with a warning on a mismatch); then the GGUF
  `image.model` layer of an Ollama pull (GGUF magic checked, manifest lists
  followed — an Apple Silicon pull of the bare tag is MLX safetensors, which is
  not loadable and counts as not downloaded); only then `-hf`, which downloads
  into the Ollama model store (`LLAMA_CACHE` = `OLLAMA_MODELS` or
  `~/.ollama/models`). Port 11543, q8_0 KV cache.
- **Context window:** `default_context_size()` is 128k at ≥48 GiB of
  GPU-addressable memory (unified memory on Apple Silicon, VRAM on a discrete
  GPU, system RAM on a CPU-only machine), else 64k. **There is no 32k tier**:
  Biorouter's instructions plus tool schemas alone are ~31-40k tokens, so a 32k
  window failed the first request of every chat. The out-of-memory step-down
  stops at 64k (`next_lower_auto_context_size`) and then reports "not enough
  memory" rather than starting a server that cannot answer. It is **not**
  `--ctx-size 0` (a 262k KV cache is slow-to-impossible on a laptop).
  `LLAMACPP_CONTEXT_SIZE` pins it (`0` = auto); the window the server really
  allocated is read back from `/props`, and `from_env` clamps the reported
  window to the server's before the first request.
- **One request at a time:** `--parallel 1` (`LLAMACPP_PARALLEL` overrides).
  llama-server's default 4 slots share one KV cache, and two concurrent
  bootstrap-sized requests in a 64k window both failed (b9611). A request holds
  a `SidecarLease` — the switch read lock plus one permit per server slot — so a
  second request waits in-process instead of on a silent socket that the idle
  timeout would cut, and a model switch waits for in-flight leases. A streamed
  reply releases its lease when generation ends, never across a tool approval.
- **Security:** a fresh random API key per launch, written 0600 to
  `<data>/llamacpp/run/<pid>.key` and passed with `--api-key-file` after the
  extra args; the provider sends it as Bearer and `/health` stays public (the
  loopback bind alone let any web page drive the model, since llama-server
  reflects any `Origin`). `--no-slots`, `--no-ui`, and the loopback
  `--host`/`--port` re-asserted last. `LLAMACPP_EXTRA_ARGS` is an **allowlist**
  (`ALLOWED_EXTRA_FLAGS`, plus `--spec-*` minus the draft-model/download flags),
  matched the way llama.cpp parses (`_` → `-` in `--` flags, `--flag=value`
  split, argument arity and value-token shape checked); it replaced a blocklist
  that missed `--tools`. Missing or flag-shaped required values are rejected,
  and `--flash-attn` values get an enum check. Most numeric and range constraints
  remain for llama-server to validate.
  The child's environment loses every `LLAMA_*` variable and HF token; only
  `LLAMACPP_HF_TOKEN` reaches it, as `HF_TOKEN`.
- **Requests:** streamed requests to the managed sidecar send
  `return_progress: true`, so a cold prefill's `prompt_progress` frames keep the
  idle timer fed. `LLAMACPP_TIMEOUT` is an IDLE bound — `0`, the declared
  default, means 600 s, or 3600 s on a CPU-only machine — and a timeout is never
  retried (`TimeoutRetry::GiveUp`). A request with no `max_tokens` gets 16384
  (32768 with thinking); vendor sampling is filled in unless the caller set it.
  Replies, streamed and blocking, pass through the same guarded text recovery
  and argument normalisation as Ollama's (see the Ollama block's tool-call
  bullet).
- **Thinking:** the server starts with `--reasoning off` (`on` with
  `LLAMACPP_ENABLE_THINKING=true`), but every Biorouter request states it in
  `chat_template_kwargs.enable_thinking`, which overrides the flag both ways on
  b11429: Quick off, Normal follows `LLAMACPP_ENABLE_THINKING`, Deep on. Qwen3.8
  also gets `reasoning_effort` `medium` (Normal) or `xhigh` (Deep) — **never
  `high`**, which its template rejects with HTTP 500; sending no level is
  byte-identical to `xhigh`, so Normal and Deep would not differ. Warm-up and
  titles use `complete_without_thinking`. llama-server's reasoning streams in
  250 ms batches, but the agent holds them and shows one Thinking block per
  phase when the phase ends (`reply_parts::HeldThinking`), because no client
  merges live thinking yet. The desktop does not render Thinking blocks at all;
  the CLI prints them only with `BIOROUTER_CLI_SHOW_THINKING`.
- **Suitability** (`/llamacpp/status`, per model): `suitable`,
  `above_recommendation`, `below_minimum` (under 32 GiB — still offered with a
  warning, because there is no smaller model), `cpu_only` (an Intel Mac, whose
  x64 build has no Metal, and the bundled Linux CPU build; budgeted by system
  RAM and never "recommended") and `unknown_resources`.
- **Speed** (measured 2026-10-10, M4 Max, b11429, 128k window, q8_0 KV, thinking
  off, GPU uncontended): Qwen3.8 27B prefilled a cold ~31k-token prompt in
  156 s (~197 tok/s), wrote free text at 18.3 tok/s and rewrote a file at
  37.0 tok/s; Gemma 4 31B took 238 s for ~32k tokens (~136 tok/s), 16.9 and
  37.8 tok/s. These are cold-prefill timings without a reusable prompt cache;
  a shared cached prefix can shorten the wait in later chats.
  `--spec-type ngram-simple` (self-speculative decoding: no draft model, no
  download, no extra VRAM) is each catalog model's own default, re-measured on
  b11429 because Qwen3.8's linear attention and Gemma 4's sliding window force
  checkpoint-based speculation: paired against `none` (contended, so only the
  ratios count) it is 2-4x faster on file rewrites and 4-10% slower on free
  text. `LLAMACPP_SPEC_TYPE=none` disables it. The older 79.7 → 354.4 tok/s
  figure was Gemma 4 E4B on b9611.
  **Self-heal:** ordered retries on a dead child (drop optional flags after an
  argv error → step down the auto context after an OOM → Hugging Face fallback
  / resume a partial download). While any of them is deciding, `status()`
  reports `Starting`, **not** `Error` — a terminal status there made the GUI's
  1500 ms poller abort startups that were about to recover. A GPU compute
  backend that fails mid-request (Metal out of memory: every later request 500s
  while `/health` stays green) is discarded (`discard_failed_backend`) so the
  next request starts a fresh server.
  **Orphan reaping:** statics never drop, so `kill_on_drop` cannot cover
  process exit; spawns are recorded in `<data>/llamacpp/run/<ppid>.pid` and
  the next `ensure()` in any Biorouter process kills children of dead parents.
- **Pinned build:** `LLAMA_SERVER_BUILD = "b11429"` in the sidecar must match
  `LLAMA_BUILD` in `ui/desktop/scripts/fetch-llama-server.js`, which downloads
  per-platform archives at package time into `src/bin/llamacpp/` and checks
  each against its pinned SHA-256 (`ASSET_SHA256`, plus the LICENSE) before
  extracting — an asset with no pinned digest is refused, so a bump must add the
  digests from `gh api repos/ggml-org/llama.cpp/releases/tags/<build>`. Builds:
  mac arm64 = Metal, mac x64 = CPU only, win = Vulkan with CPU fallback via ggml
  dynamic backends, linux = CPU; CUDA stays opt-in via
  `BIOROUTER_LLAMACPP_BIN`. b11429 (2026-10-05) is the first build with every
  fix the two models need (Qwen3 tool parser, `reasoning_effort` in templates,
  the Gemma 4 required-tool grammar fix, `-hf` skipping draft files). Bump the
  pin deliberately and smoke-test — llama.cpp releases multiple times a day
  with no semver.
- **Linux floor:** llama-server needs glibc ≥ 2.35-ish plus `libssl3` and
  `libgomp1` (declared in the deb/rpm maker configs in `forge.config.ts`) —
  i.e. Debian 12+/Ubuntu 22.04+. Debian 11 runs the app but not local models.
- **HTTP routes:** `/llamacpp/status|ensure|warmup|delete|stop` in
  `crates/biorouter-server/src/routes/llamacpp.rs` (status includes the
  catalog and per-model suitability; ensure is async — poll status for download
  progress). The CLI's `biorouter models local list|pull|rm` resolve names the
  way these routes do: `pull` like `ensure`, `rm` like `delete`, and `list`
  labels the copy the sidecar would launch (HF cache before Ollama).
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
- **Tests:** `cargo test -p biorouter --lib providers::llamacpp` (both
  modules), `cargo test -p biorouter-server --test llamacpp_routes`, and live
  end-to-end against the default `qwen3.8-27b` (a ~19 GB one-time download, or
  reuse `ollama pull qwen3.8:27b-q4_K_M`; `LLAMACPP_TEST_MODEL=gemma4-31b` or a
  raw spec switches it):
  `BIOROUTER_LLAMACPP_BIN=ui/desktop/src/bin/llamacpp/llama-server cargo test -p biorouter --test llamacpp_integration -- --ignored --test-threads=1`
  (11 tests, 450 s for Qwen3.8 on 2026-10-10). ⚠ Its tool tests (and the
  survey's) accept any `ToolRequest`, and replies pass through text recovery
  first, so a whole-reply JSON call passes too. The binary leaves its last
  sidecar running at exit — statics never drop — and the next Biorouter
  process's `ensure()` reaps it. Checklist:
  [`docs/providers/llama-server/model-catalog-qa-checklist.md`](docs/providers/llama-server/model-catalog-qa-checklist.md).

### Ollama provider

`crates/biorouter/src/providers/ollama.rs` (what to send) and
`formats/ollama.rs` (the wire format). User-facing reference:
[`docs/providers/ollama/`](docs/providers/ollama/README.md).

- **Native `POST /api/chat`, not `/v1`.** Until 2026-10 the provider used
  `/v1/chat/completions`, which cannot set the context window. An Ollama whose
  default window was 4096 tokens (every CPU-only or low-VRAM machine) silently
  kept the first 4 and the last ~2046 tokens of the ~31-40k-token prompt; the
  model saw only the tail of the tool list and wrote invented tool calls as
  JSON text, while Biorouter believed it had 128k (a guess from the model's
  name) and compaction trusted the truncated usage. That was the "returns JSON
  instead of doing the task" report.
- **`options.num_ctx` on every request**, resolved once per (instance, model):
  `OLLAMA_NUM_CTX` → a context limit the user set (`BIOROUTER_CONTEXT_LIMIT`,
  or a limit that differs from the name-based guess) → 128k at ≥64 GiB of
  GPU-addressable memory (note: 48 GiB for Llama Server), else 64k — capped at
  the model's trained window from `/api/show` (`<general.architecture>.context_length`).
  The same number goes on turns, blocking calls and titles, because a different
  `num_ctx` reloads the model, and `get_model_config()` reports it so
  compaction and the gauge see the real window. A custom `ollama`-engine
  provider uses `BIOROUTER_CONTEXT_LIMIT`, not the built-in `OLLAMA_*` keys.
- **Post-load memory check:** native replies probe `GET /api/ps` once per
  observed window for the matching model's `size`, `size_vram`, `context_length`.
  The loaded context may be per-request or a whole multiple across parallel
  slots; compaction still sees the per-request window. Derived automatic
  windows stay automatic through provider rebuilds.
  An automatic window above 64k drops to 64k for later requests when
  `size_vram < size`, when the loaded allocation exceeds 75% of local system
  RAM, or when it takes at least 25% and less than 20% of RAM remains available.
  RAM checks are loopback-only; remote servers use their own GPU-spill figure.
  The cap is process-wide per host/model, including fresh providers serving the
  context gauge and reasoning-effort rebuilds. Turns, corrective retries,
  summaries and titles use it; the next request costs one reload. At the 64k
  floor it warns once rather than shrinking below the full agent prompt.
  Explicit `OLLAMA_NUM_CTX`/`BIOROUTER_CONTEXT_LIMIT` and recognised user model
  limits are preserved. A per-model limit identical to the name default has
  no provenance and remains automatic;
  use either explicit setting to pin that value. Restore bindings preserve the
  construction config rather than the reported live window. Server settings are
  never changed. Warnings name `OLLAMA_NUM_PARALLEL` (which multiplies the cache)
  and `OLLAMA_NUM_CTX`. Failed/missing probes are best-effort and retried later;
  a load failure before a reply cannot be repaired by this post-load check.
- **No silent cuts.** `truncate: false` turns an over-window prompt into a 400,
  mapped to `ContextLengthExceeded` in the canonical
  `utils::context_overflow_message` sentence (so the agent can tell when
  compaction cannot help) naming the setting to raise. For servers that cut
  anyway (Ollama 0.4.0-0.12.5, or the `/v1` fallback) `TurnCheck` discards a
  reply whose prompt count is implausibly small for the request's bytes (more
  than 12 bytes per token over 20 KB; 6 when the count has the shape of
  Ollama's cut, half a power-of-two window plus 1-8, because lfm2's 16,387 of
  32,768 ran at ~9.8): no text shown, no tool run. Before 0.4.0
  the count excludes the cache, so nothing is judged.
- **`/v1` fallback** only when `/api/chat` answers 404 with anything other
  than "model not found" (a non-Ollama server behind a custom provider);
  sticky per instance, text held until the size check passes.
- **Every request streams**, blocking ones collected, so `OLLAMA_TIMEOUT`
  (600 s) bounds silence, never a whole generation; a timeout is never retried
  (`TimeoutRetry::GiveUp`), a connection failure still is. One before the reply
  begins is `ProviderError::IdleTimeout`, naming the seconds, the prompt's rough
  size and the setting, and the agent does not re-send it either (it used to,
  as a network blip: diagnostics 20261006_2). An unpinned turn gets
  `num_predict` 16384, or 32768 when the model may think.
- **Thinking:** `think` only to models whose `/api/show` capabilities include
  `thinking` (others answer `think:true` with a 400). Quick sends `false`;
  Normal sends nothing unless `OLLAMA_THINK` is set; Deep sends the deepest
  declared level — `xhigh` for qwen3.8, never `high`, which its renderer
  rejects — or `true` for a boolean-only model such as gemma4. A pinned output
  budget under 1024 tokens sends `false` unless Deep: `biorouter configure`'s
  50-token probe otherwise came back as reasoning only (measured, Ollama 0.40.2).
  Titles: `think:false`, `num_predict` 128.
- **`keep_alive`** `OLLAMA_KEEP_ALIVE`, default `"30m"` (a number is seconds,
  `-1` keeps the model loaded), so a 27-31B model is not unloaded and its
  prompt re-processed after Ollama's 5-minute default.
- **Models:** `OLLAMA_DEFAULT_MODEL = "qwen3.8:27b"`; `OLLAMA_RECOMMENDED_MODELS`
  is that and `gemma4:31b` (a one-time warning below 32 GiB). The model list is
  what the user pulled, minus tags whose `/api/show` capabilities lack `tools`
  (the configured model always stays); `biorouter configure` puts the
  recommended pair (and its quantisation variants) first and cloud tags LAST,
  labelled. ⚠ `gemma4:31b-cloud` matches the variant prefix `gemma4:31b-`, so
  the variant match must exclude cloud tags or Enter saves an ollama.com model
  as "Recommended: Gemma 4 31B" (`commands/configure.rs` calls the provider's
  own `utils::is_ollama_cloud_model`). `OLLAMA_HOST` resolves exactly as Ollama's
  own client reads it (`resolve_ollama_base_url`: no scheme → 11434,
  `http://` → 80).
- **HTTP routes + desktop** (`crates/biorouter-server/src/routes/ollama.rs`):
  `/ollama/recommended` serves the recommended pair, a memory suitability
  rating and the resolved base URL to the onboarding card (`utils/ollamaDetection.ts`,
  which hard-coded `gpt-oss:20b` until 2026-10); `/ollama/context_window?model=`
  answers with the `num_ctx` a chat on that model would send, built through
  `OllamaProvider::from_env` + `context_window_for`, and
  `utils/liveContextWindow.ts` feeds it to the gauge in place of the
  name-pattern guess ("qwen" read as 128k) that hid the original bug. These two
  files and the route are the single sources of truth for the recommendation
  and the gauge's local window. `/ollama/recommended` also returns `tier`, the
  provider's own `tier()` for the default model: the onboarding card decides
  "this computer" and private from it, never from its own host list (`0.0.0.0`
  is Public, `::1` is Private), and keeps reachability (the CSP allows only
  `127.0.0.1`) as a separate question. Memory is `ollama::sizing_memory()`,
  shared by the route and the provider's `num_ctx` tier: on Linux an NVIDIA
  GPU's VRAM (`llamacpp_sidecar::discrete_vram_gib`), never system RAM, because
  the sidecar's `cpu_only` figure there describes its own bundled CPU build and
  Ollama ships CUDA. Deleting a Llama Server row that came from Ollama removes
  the tags whose `/api/show` FROM line names the listed blob, never
  `ollama_name`, because on Apple Silicon the bare tag is the MLX build.
- **Prompt clock:** for a local engine (`prompt_manager::is_local_engine`) the
  system prompt states the date, not the hour. The clock sits near the top of
  the prompt, so an hourly change re-prefilled everything after it: measured on
  qwen3.8:27b, 8,191 of 30,745 tokens reused and 107 s of prefill on the first
  request after each hour boundary. The MOIM still carries the minute.
- **Tier: a cloud model is Public even on a loopback host.** `tier()` is
  `self_hosted_tier(base URL)` (loopback → Private) EXCEPT when the model is
  served remotely: a tag that is `cloud` or ends in `-cloud`
  (`utils::is_ollama_cloud_model`), or one whose `/api/show` carries
  `remote_host`/`remote_model` (a cloud model renamed with `ollama cp`).
  Ollama forwards those to ollama.com, so they are Public with no affiliation.
  This makes the model NAME a tier input in both directions, as `OLLAMA_HOST`
  is: a local tag to a cloud tag lowers the tier, and back raises it, which is
  why `BIOROUTER_MODEL` is in `CAPABILITY_CONFIG_KEYS`. A request for such a
  model from an instance that already told the gates "Private" is refused
  before anything is sent.
- **Tool-call protocols:** every model's template decides what Ollama returns
  for a tool call (built-in Go renderer/parser, Harmony, a Go template's tag
  parser, the GGUF's Jinja template, or no tool support), so the same request
  yields structured calls, calls written as text, or an empty reply depending
  on the model. 12 models were probed on the wire (2026-10-10): `qwen2.5-coder:7b`
  and `phi4-mini` made 0 structured calls, `deepseek-r1:8b` fabricates tool
  output, `mistral-small3.2` returns empty replies on any argument named `name`.
  Handling, all measured against those captures:
  - **Text recovery** (`formats/text_tool_calls.rs`, wrapped around both local
    providers' streams — Ollama and Llama Server): TIER1 only, i.e. the WHOLE
    reply is one call object, an array/sequence of them, or phi4-mini's
    `type`-named array, optionally fenced, or one of those wrapped whole in a
    `<tool_call>` tag, closing tag optional (L8, seen live from
    qwen2.5-coder:7b) — never in `<tool_response>`, the RESULT tag, where the
    same model echoed and invented tool output. Guards: no structured call in the
    turn, finish reason not `length`, a closed key set (`name` + `arguments` or
    `parameters`, optional `id`/`type`), every name byte-identical to a tool in
    THIS request, argument objects with required fields and basic/direct types
    checked, all or nothing. Checks include nested properties/items and
    `additionalProperties: false` to depth 16. They do not resolve `$ref` or
    evaluate enum, composed schemas (`anyOf`/`oneOf`), or other JSON Schema
    constraints; those remain for the tool's normal argument handling. Text
    that could still be such a payload is held back while it
    streams (thinking is not). A recovered call carries `recovered_from_text`
    metadata and goes through the same approval path as any other. Prose plus a
    call (TIER2: ≤400 characters of prose, then one block that may itself be an
    array or sequence of calls) is recovered only by the toolshim, which is
    opt-in (`BIOROUTER_TOOLSHIM`) but runs on ANY provider, reads the reply only
    after it streamed, and has no finish reason to check.
    ⚠ Do not add parsers for `[TOOL_CALLS]`, Harmony or pythonic lists, nor
    Hermes tags beyond the whole-reply `<tool_call>` wrapper, "for robustness":
    none of them ever reached Biorouter as text — Ollama parses or swallows
    them — so such a parser would only ever see a model QUOTING one. Never
    recover from `<tool_response>`, the RESULT tag. A call inside prose is never
    acted on, not even by a retry: a quote cannot be told from an intention.
  - **Argument normalisation** (R16) on every call, structured or recovered:
    Cohere's `{tool_name, parameters}` wrapper is unwrapped, a stringified
    object/array is decoded and `"10"`/`"true"` coerced where the schema asks
    for a number/boolean (never where it allows a string); an enum violation is
    left for the tool to report, so the model sees the error.
  - **One corrective retry per turn**, Ollama's native path only (never `/v1`),
    shared by three triggers: (1) a `tool '<x>' not found` error (never read as
    "model not found"), re-asked with the tool list; (2) an empty reply to a
    request with tools whose tokens may be a call Ollama's parser swallowed —
    with no reasoning, more than 8 output tokens; with reasoning, always,
    because measured reasoning ran 2.4-4.7 characters per token and qwen3:1.7b's
    swallowed call fell inside that band (`TurnGuard::empty_reply`). The error
    then says only what the numbers show: "could not parse as a tool call" only
    beyond a ceiling for the reasoning (characters/2 + 16 tokens) plus 8, else
    "returned reasoning only (N output tokens)"; a reply cut at `length`, or
    one with no reasoning and at most 8 tokens, fails at once; (3) a WHOLE reply
    that is a call recovery declined (`text_tool_calls::attempted_calls`: a name
    without its `extension__` prefix, arguments that do not fit, the
    `<tool_response>` tag; several objects or an array, each one naming an
    offered tool), whose name stands for an offered tool. That one
    asks rather than orders, is skipped when the call echoes the conversation
    (made by an earlier turn, or quoted in a tool result or earlier assistant
    reply; `echoes_conversation`), and is best-effort: if it cannot be sent,
    fails or comes back empty, the turn ends with the reply already shown. A
    retry after shown text starts a new paragraph.
  - **A reply of calls nothing ran is SAID, never run** (diagnostics
    20261006_2): when a turn that offered tools ends on a reply made only of
    call objects written as text (`whole_reply_call_names`), `agents::reply_parts`
    adds one user-only inline notice after it ("Nothing ran: ..."), for every
    provider. It is stored, so a reload shows it, and every wire format drops it
    because it is not agent-visible. Calls Code Execution hides (`shell`,
    `text_editor`) still get no retry: the provider cannot see them.
  - **Tool roster for templates that hide tools** (R8): when `/api/show`'s
    template shows tools only next to the last user message (llama3.1,
    mistral-small3.2, mistral-nemo; `ModelFacts::tools_only_near_last_user`),
    `tool_roster` appends one line per tool (name, params, first sentence;
    ≤64 tools) to the system prompt on EVERY request for that model, so the
    prefix stays cacheable, plus one warning per model.
  - **No tool support** (`/api/show` capabilities lack `tools`, e.g.
    `gemma3:4b`, which 400s any request with tools): refused before sending,
    and hidden from the model list.

  Per-model matrix, models to avoid, and the guards:
  [`docs/providers/ollama/tool-call-protocols.md`](docs/providers/ollama/tool-call-protocols.md).
- **Tests:** `cargo test -p biorouter --lib ollama`, and live against a real
  Ollama with the two models pulled:
  `cargo test -p biorouter --test ollama_live -- --ignored --test-threads=1`
  (`OLLAMA_TEST_MODELS` replaces the list; it runs the real configure probe).

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
  (`routes/coding_agents.rs`) backs the AI agents rows of the provider catalog's Public tab
  (`settings/providers/ProviderCatalog.tsx`; the state machine and controls are
  `onboarding/codingAgentControls.tsx`), on first run and in Settings, beside the Local tab's
  `LlamaServerInlineCard`. `CLAUDE_CODE_COMMAND` / `CODEX_COMMAND` override discovery.
- **"Configured" means the key is saved AND the CLI resolves.** `check_provider_configured`
  (`routes/utils.rs`) asks `discovery::resolve_configured` — the lookup the status probe
  uses — so a command key naming a missing CLI is served `is_configured: false` with
  `unavailable_reason`, and `SwitchModelModal` shows that row disabled with the reason rather
  than offering a bind `from_env` would refuse. Sign-in is deliberately NOT part of it: learning
  it spawns the CLI, and `GET /config/providers` runs for every provider. See
  [`docs/desktop-ui/provider-catalog.md`](docs/desktop-ui/provider-catalog.md).
- **Tests:** `cargo test -p biorouter --lib providers::coding_agent`,
  `cargo test -p biorouter-server --test tool_bridge_routes`, and
  `onboarding/codingAgentControls.test.tsx`. The live end-to-end tests need the real vendor
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
- **Every kind sits under one 32px preview bar** (`artifacts/PreviewBar.tsx`: name, meta,
  Copy, view switch), drawn from the artifact's metadata before the content loads. The content
  portals its count and actions into it (`PreviewBarMeta` / `PreviewBarActions`), and a layer
  that is fading out is wrapped in `PreviewBarGate` so it puts nothing into the bar. A tab that
  has been seen never shows a loading state again (`previewCacheRef`); a first load shows
  nothing for 200ms, then `PreviewSkeleton`'s three lines. A `ui://` artifact's meta is its
  transcript card's own word (Figure, Report, App; `artifactCardKind`), so the two never name
  one thing differently. In the header, a file gets a `⋯` menu (Quote selection, Send a region to
  the chat, Reveal in Finder); a figure, report, app or page would get a menu of one, so "Send a
  region to the chat" is a Camera icon button in the menu's place (pressed while a region is
  being picked, and a second press cancels). The one-item menu stays only where the action cannot
  work, a `biorouter serve` browser (SD-8), because its visible reason is what says why. The tab
  strip's edge fade is 32px.
- **How each kind renders:** HTML → sandboxed `srcdoc` iframe; images → inline;
  directories → `DirectoryTreePreview`, a left rail holding a filter box and an
  expandable `role="tree"` (branch chip and per-entry status dots on a
  `gitDirectory`) beside a live preview of the selected entry;
  `.md`/`.Rmd`/`.qmd` → rendered prose with a
  Preview/Raw toggle; `.csv`/`.tsv` → a real table (quoted fields honoured,
  capped at 500 rows) with a Table/Raw toggle; everything else → syntax-
  highlighted, line-numbered code with a language chip and Copy.
  **Every text preview sits on "paper"** — `.br-paper`, painting
  `--background-default`, the exact ground an Auto Visualiser chart paints
  (`artifactPaper.test.ts` pins the two together) — with a 760px column
  (`.br-preview-measure`, `max-width: var(--measure-chat)` inside the scroller
  with responsive gutters; the provisional `.br-paper-measure` is gone) and one
  left edge for prose, code and tables. Fenced blocks, inline code and notebook
  source sit in the shared `--background-well` token, not `--background-code` (which in dark *is*
  the page). Selection is renderer-wide Biorouter orange from `--selection-hue`
  / `--selection-alpha` — literals in a bare `:root`/`.dark` pair that no family
  may re-declare, because Alma Mater re-points the coral scale to teal. All of
  it is authored CSS in `main.css` ("Preview paper"), placed right after the
  `code [class~='token']` rule, never Tailwind strings. The pieces live in
  `MarkdownDocument.tsx`, `DelimitedTable.tsx`, `CodeBlock` in
  `ArtifactViewer.tsx`, and `styles/prismGrammars.ts` (raw CSV/TSV grammars,
  R/Python call tokens, log refinements).
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
  - verify native desktop behavior with Biorouter Copilot after per-request consent.
    Inspect the target app and capture its window through the native helper;
    avoid collecting unrelated desktop content. The removed script/capture
    routes must never be used as a fallback.

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
  The two fill neutrals of the 2026-10 redesign are the exception that IS
  enforced: `--background-fill` and `--background-fill-strong` (with
  `--border-control` and `--control-track-off`) are `STRUCTURAL_TOKENS` in
  `ui/desktop/scripts/lib/theme-contract.mjs`, declared as a hex once per mode in
  the base block and nowhere in a family, which `styles/themeNeutrals.test.ts` asserts;
  the only other declaration is the dark popover's re-pointing (see "Fills
  before lines" below). Since the redesign every family's `sidebar-icon` is its
  muted label ink (it was the accent in Alma Mater and Roche Limit, the full ink
  in Parchment), so a nav icon is one ink with its label and the accent is held
  for the active row's rail and the lock badge; it still differs per family
  because the ink does.
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
  is audited with zero edits to it (1148 assertions at three families, measured
  2026-10-11 with `npm run check:contrast`; it was 330 on 2026-08-08, and the
  redesign's fill and dark-popover rows are added per family too. Re-measure
  rather than trusting this figure).

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
- **Sidebar rows are 28px on a 30px pitch, at every width** (owner message 12;
  `BioRouterSidebar/sidebar.css`, `--row-height-nav: 28px` in `main.css`): a 2px
  gap between rows, 13px labels (`--text-secondary`), the 16px icon at x=16 and
  the label at x=40 whatever the sidebar's width (an 8px list inset plus 8px row
  padding, then 16 + 8), muted ink at rest, and a 2px accent rail on the current
  row. Nav rows, chat rows, the Chats header, folder headers, Show more and a row
  being renamed (a 24px input in the same 28px box) share the box and both
  columns. `sidebarGeometry.browser.test.ts`
  measures it in Chromium at 216, 288 and 360px, because jsdom lays nothing out.
  The rules are authored and unlayered, so they beat the cva string
  `SidebarMenuButton` still carries; ⚠ a background set there also beats the
  D-15 focus fill in `@layer base`, so every rule that paints a row restates the
  focus fill and the 2px `--border-focus` edge for `:focus-visible`. Reference:
  [`docs/desktop-ui/sidebar-chat-list.md`](docs/desktop-ui/sidebar-chat-list.md).
- **The Chats header's View options menu** (owner message 3) groups the list by
  Date, Folder or None and sorts it by Last activity, Created or Name (defaults:
  Date, Last activity), as pure functions in `BioRouterSidebar/sidebarChatView.ts`.
  The choice is per viewer, in `localStorage` key `biorouter:sidebar-chat-view`,
  clamped field by field on read and synced across windows by the `storage` event
  (`useSidebarChatView.ts`); collapsed folders live in
  `biorouter:sidebar-collapsed-folders` (at most 200). The row menu is one
  component shared with History and the tab strip (`chats/ChatRowContextMenu.tsx`:
  Rename, Open in new tab, Open in new window, Diverge, Export…, Copy chat ID,
  Delete chat…), and **Rename is in place** (owner message 4): from the menu, `F2`
  on a focused row, or `⌘⌥R` (`Ctrl+Alt+R`) for the current chat, committed
  through `renameSessionOptimistically` (`utils/sessionNameSync.ts`), the one
  function every rename surface calls. ⚠ `⌘⌥R` reaches the renderer only because
  the `before-input-event` handler in `main.ts` skips its `⌘R` reload while Alt is
  held; drop that check and the shortcut reloads the window instead.
- **Only the lines near the viewport are rows** (R-28; `sliceSidebarLines`,
  `sidebarLinesInView` and `nextSidebarDrawRange` in `sidebarChatView.ts`). The
  whole list is held and arranged, so its order is always complete, but every line
  outside the lines in view plus `SIDEBAR_RENDER_OVERSCAN` (12) on each side is
  folded into one `.br-chat-row-gap` placeholder of exactly its height, so the
  scroll height and every drawn row's place are unchanged. The pitch is read off a
  drawn row (`measuredLinePitch` in `RecentChats.tsx`), never assumed: the Text
  size preference scales the 28px row and its 2px gap (34.5px a line under Larger
  text), which is also why the placeholder scales its own 2px. Kept drawn whatever
  the range: the focused line (row, folder header or Show more), the current chat
  (so the list's single Tab stop stays on it), a row being renamed or just renamed,
  and a line whose right-click or `⋯` menu is open, so focus, an open menu or the
  editor is never unmounted under the person. ⚠ No row mounts a Radix menu root:
  the list has ONE non-modal right-click menu that opens on the row the event came
  from, and a `⋯` is a plain button (`LazyMenuButton`) until it is focused or
  pressed, because every mounted root adds document `keydown` listeners that each
  key press and view switch then paid for, row by row.
- **One sidebar request may carry the whole list** (R-28, owner message 14).
  `GET /sessions/sidebar` clamps `limit` to `MAX_SIDEBAR_SESSION_LIMIT`, 2,000
  (`routes/session.rs`; it was 50), so a non-default view, which needs every chat
  before it can arrange them because the server pages in last-activity order only,
  reads the rest in one request (`useSidebarSessions({ loadAll: true })`, capped at
  `SIDEBAR_SESSION_LOAD_ALL_CAP`, shown once when the read completes): 3.0 s to
  0.2 s for 2,000 chats against a debug daemon, per the constant's own comment. The
  page is a keyset over `ORDER BY s.updated_at DESC, s.id ASC` that walks
  `idx_sessions_updated` (the type filter is written `+s.session_type` to keep the
  planner off `idx_sessions_type`; `the_summary_listing_walks_the_updated_index`
  pins the plan), with `EXISTS` and correlated `COUNT(*)` probes in place of the old
  `JOIN messages … GROUP BY`. History's `GET /sessions` shares the shape
  (`full_listing_sql`, `the_full_listing_walks_the_updated_index`). ⚠ Privacy
  filtering did not change: it is still a SQL predicate
  (`AND s.privacy_tier = 'public'`, which fails closed, for a caller whose
  `lists_session` refuses a private chat), the cursor names the last row the page
  RETURNED, and the 2,000 bound is not a privacy knob. Every page the renderer
  asks for carries `userActionHeaders()`, or private chats silently drop out of a
  re-sorted list.
- **`--measure-chat` is a flat `760px`.** It was briefly widened into a clamp and
  that was wrong for this measure specifically — a 1180px composer is a line of
  prose the eye has to track back across. `styles/measures.test.ts` asserts the
  whole string, because every loose matcher (`/760px/`) is satisfied by
  `clamp(760px, 78%, 1180px)`, the value being ruled out.
- **The Chat summary docks as rung 0, before anything else yields** (`summaryRailFit` in
  `Layout/yieldLadder.ts`; [`docs/desktop-ui/chat-summary-rail.md`](docs/desktop-ui/chat-summary-rail.md)).
  Once a chat has had a turn, a 240 to 280px card sits to the right of the conversation under the chat
  header, but only where the conversation keeps its full measure: a 816px transcript cell (760 plus
  the 56px of `px-1` and `paddingX={6}` around it) plus the rail, so `available ≥ 1056` (1068 after
  a measured hide), width `clamp(available − 816, 240, 280)`; `available` is the pane minus a side
  preview, and a stacked preview hides it. Elsewhere the header's `PanelRight` button opens the same
  content as a 280px popover. ⚠ The 56 is derived from two literals in `BaseChat.tsx`, which
  `styles/summaryRail.test.ts` pins; moving either moves the seam. The grid lives in
  `components/summary/summaryRail.css`, not `main.css`, keyed on
  `[data-summary-rail]:not([data-preview-layout])`, and it snaps: only the conversation's FLIP
  (`hooks/useColumnGlide.ts`) and the card itself animate.
- **Fills before lines** (R-27, owner message 13). In-page containers and
  controls are a subtle solid fill with no resting outline: `--background-fill`
  (`#f4f4f2` light, `#232320` dark) at rest and `--background-fill-strong`
  (`#eaeae6`, `#2c2c29`) for hover and press, two neutrals declared once per mode
  in the base blocks and shared by every family. Write
  `bg-background-fill border border-transparent`, plus
  `hover:bg-background-fill-strong` where the box answers the pointer. ⚠ Keep the
  1px border, transparent, never `border-0`: no geometry moves between the
  bordered and the filled look, and forced colours repaint the fill as Canvas and
  draw the transparent border in the system ink, which is the only edge such a
  box has there (`styles/fillTokens.browser.test.ts`). ⚠ **A fill never sits on a
  fill.** Inside a filled box (`Card`, every `Note`, a dialog's inner panel and
  row, a washed list row, or anything given `.br-fill-surface`) controls rest one
  step back on `--background-default`; on a recessed ground (`.br-ground-recessed`:
  the sidebar, the `--background-muted` grounds, the page band) they rest one step
  in, on the strong step, because the rest fill IS the ground there. Those grounds
  set three private variables the primitives read (`--br-control-ground`,
  `--br-control-hover`, `--br-track-ground`); never set them at a call site. A row
  list that reads apart by air and the hover wash drops its hairlines with
  `.br-rows-flat` (forced colours bring the line back). **Lines stay only where
  they are structural**: the 44px band's bottom edge, pane dividers and resize
  handles, table rules inside a filled table, a dialog footer's scroll edge, and
  floating surfaces (menus, popovers, tooltips, dialogs and toasts keep a hairline
  and a shadow so they hold up in dark). Checkboxes, radios and the switch keep
  `--border-control` at 3:1. ⚠ **Text fields have no resting border**: a field is
  found by its fill, its label or placeholder and its place, and draws
  `--border-focus` while focused or `--border-danger` while invalid. That
  accessibility trade-off is recorded in [design.md](design.md) 9.11 (the rule,
  the contrast floors `check-contrast.mjs` holds); the per-control table and the
  three variables are in
  [`docs/desktop-ui/shared-controls.md`](docs/desktop-ui/shared-controls.md)
  ("Fills before lines"). A dark popover re-points the strong step one step deeper
  (`.dark .biorouter-popover-surface`), so a field there rests as a quiet well and
  still shows hover. Crew's own `crew-*` recipes keep their lines; the shared
  primitives Crew renders take the fill like everywhere else (ruling 12). The
  composer card (a chat's, Home's and the message editor's) is the one exception
  (R-32, next bullet).
- **The composer is one card and one muted footer line** (`ChatInput.tsx`, spec
  3.7; `ChatInput.structure.test.tsx` pins the shape). The card holds the queue,
  notices, chips and the text, then one 32px controls row: `+`, the Tools chip,
  the model and effort chip, and Send. Under the card, a 12px muted footer
  (`bottom_menu/ComposerFooter.tsx`): the working folder on the left, the context
  ring and the chat's cost on the right, only the folder on Home. The card's
  classes live in one module, `composer/composerCard.ts`, and Home renders the
  same `ChatInput`, so the two never differ. Its surface,
  `composerCardSurfaceClassName`, is also what the in-place message edit card in
  `UserMessage.tsx` wears, because editing a message is writing in a composer
  (coordinator ruling 21). ⚠ **The composer card is the one box the fill rule
  does not reach** (R-32, owner message 17, 2026-10-11, superseding R-27 for this
  card only, in a chat, on Home and in the message editor): it is the page
  ground (white in light) with its pre-R-27 resting 1px hairline, not the grey
  fill, and it carries no `.br-fill-surface`, so what sits in it rests as on any
  page ground. Do not
  "fix" it under the fill rule. The focus edge is that same 1px border turning
  `--border-accent` (the family accent, coral in Parchment; not `--accent-muted`,
  which at 2px reads as a thick warm-grey band), and it is **authored CSS**:
  `.biorouter-composer-card:has(textarea:focus)` in `main.css`, unlayered so it
  beats the card's border utility, and never a Tailwind `has-[...]` variant at
  the call site. The working sweep rides the same border
  (`.biorouter-composer-card[data-working='true']`). Crew's composer writes the
  same class and the same resting pair, so it shares the hairline and the focus
  edge; only `ChatInput` sets `data-working`, so the sweep is the chat's alone.
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
  - **The Tools popover's switcher never moves** (R-25, owner message 13).
    `bottom_menu/ToolsChip.tsx` opens one popover above the chip holding one list
    at a time (a search field, a scope line saying where a change applies with
    Enable all or Disable all, the switch rows) and the Extensions | Skills |
    Knowledge `SegmentedControl` as its LAST child, so the control rides the
    popover's bottom edge, 6px above the chip (`sideOffset`): Radix places a `side="top"` popover by its bottom, so a
    longer or shorter list grows or shrinks it upward. ⚠ `avoidCollisions={false}`
    is load-bearing (collision handling would flip the popover below the chip or
    slide it along it); the room above caps the height instead, through Radix's
    available-height variable in `.br-picker-popover`'s `max-height`
    (`pickers.css`) and `TOOLS_POPOVER_COLLISION_PADDING`, whose 52px top keeps
    the search field off the 44px drag band. DOM order is the visual order, so
    Tab walks top to bottom. `toolsPopover.browser.test.tsx` measures the control
    14px above the chip in every list at two window heights.
  - **Effort is a three-stop slider** (R-26): `bottom_menu/EffortSection.tsx` in
    the model menu (`settings/models/bottom_bar/ModelsBottomBar.tsx`, opened from
    `bottom_menu/ModelEffortChip.tsx`), Quick, Normal and Deep, applying at once
    through the same store and scope the radio rows used
    (`setReasoningEffort(scope, …)`, `store/reasoningEffort.ts`); the level
    descriptions sit in an InfoTip the thumb is described by. The primitive is
    `components/ui/slider.tsx`, built in house because `@radix-ui/react-slider` is
    not a direct dependency and nothing may be installed: discrete stops only, the
    THUMB is the `role="slider"` with `aria-valuetext` set to the stop's label,
    arrows, Home, End and Page keys (mirrored in RTL), and a press that snaps and
    drags stop to stop; its geometry and colour are authored `.br-slider*` rules in
    `main.css`. Tests: `ui/slider.test.tsx`, `ui/slider.browser.test.tsx`,
    `bottom_menu/effortSection.browser.test.tsx`.
- **Icons: one registry, one glyph per concept** (owner messages 7, 9 and 11;
  [design.md](design.md) 9.8). `components/icons/app-icons.tsx` is Lucide through
  `light()`, which pins stroke 1.5 and `currentColor` after the prop spread, plus
  hand-drawn glyphs under the same contract (`ChatBubble`, `CrewChat`, `Agent`,
  `KnowledgeGraph`, `Skill`, `Copilot`). `ENTITY_ICONS` (`icons/entity-icons.ts`)
  gives each entity one glyph (workflow, knowledge, extension, skill,
  application, schedule, crew, agent, chat, model, folder). Tool rows read their
  family glyphs from it (`utils/toolGlyph.ts`), and the chat kinds import the same
  glyphs by name, which `chatKind.test.ts` asserts. Chat kinds are
  `chats/chatKind.ts` (chat, Crew task, branch, sub-agent, app, scheduled run, terminal, and a workflow run that
  stays dormant until the listing carries an `origin`), drawn by `ChatKindIcon`.
  ⚠ **Private is a shape, not a tint**: kind and privacy are separate axes composed
  by `chatIconFor`, and `withPrivateBadge` puts a lock badge on any kind's glyph,
  the badge in the family accent (`.br-icon-lock-badge`) over a body in the row's
  ink, so an all-private list is not a wall of coral. Tool rows use
  `utils/toolGlyph.ts`'s seven-rung ladder: specific glyphs only for Biorouter's
  own extensions (`BUILT_IN_EXTENSION_KEYS`, which `toolGlyph.test.ts` checks
  against the crates), the Puzzle for every external or installed extension
  whatever its tools are called, and the wrench for a tool nothing recognises.
  ⚠ Never change the glyph behind an export name Crew imports (57 files under
  `components/crew/` import `icons/app-icons`, measured 2026-10-11); add a new
  name instead, as `NewChat` (SquarePen) was added so `Plus` keeps meaning add.
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
  it; `serve_index` rebuilds on drift. The bundle also records the SDK's source
  version, so an older daemon preserves a newer SDK across rebuilds. The desktop
  ships pinned, integrity-checked esbuild beside the daemon; CLI packages stage
  the same binary. Builds publish JavaScript and imported CSS together.
- **Browser launch uses the daemon origin.** The desktop's shared-daemon proxy
  requires its secret, so an external browser cannot use that proxy's address.
  `POST /apps/{id}/launch` returns the daemon's own loopback URL and a one-time
  access link; desktop launch and managed preview use that origin. Browser-open
  failures are shown to the person. Build-chat buttons check session metadata
  before opening a tab, and show a disabled reason when the chat is gone.
- **Export is directly runnable.** `export_app` writes `manifest.json` (without
  it the daemon 404s the app), leaves the endpoint unset so the SDK derives it
  from the page origin (the desktop app starts `biorouterd` on an *ephemeral*
  port, so the old hardcoded `:3000` never worked), ships `biorouter-launch.sh` +
  `run.sh`/`run.command` (locate `biorouterd`, install, start, verify, open — no
  Node needed) and a `serve.mjs` that proxies `/apps/**` incl. the WS upgrade and
  binds loopback only. `.vault/` is excluded. The daemon's port env var is
  **`BIOROUTER_PORT`**, not `BIOROUTER_SERVER__PORT` (only the secret key uses
  the `__` form — `Settings` is a flat struct).
- **Exports have one implementation:** `agent_drafter/export.rs`. The desktop
  downloads the `POST /apps/{id}/export` zip, preserves executable modes, and
  safely extracts it into the person's chosen folder. Launcher, Full and bundled
  daemon options reach the exporter; Full filters shipped skills and privacy
  gates knowledge payloads. Exported HTML stays raw for daemon assembly, binary
  assets are preserved, and working files, secrets and build-chat ids are omitted.
  Launchers honor `BIOROUTER_PATH_ROOT` for an isolated recipient store and retain
  backups before replacing installed contents. Full launchers start or reuse the
  authenticated daemon before importing knowledge archives through its existing
  multipart API. Import receipts prevent duplicate imports on retries; only
  matching app declarations are rebound to allocated knowledge ids. Payload
  consent and archive privacy/owner floors remain enforced.
- **App pages require a document navigation** on browser credentials in a secure
  browser context (loopback or HTTPS). Plain HTTP on a network address refuses
  app surfaces and the secret-bearing serve shell, because Fetch Metadata and
  opener protections do not apply there. Forwarded HTTPS on a network host also
  requires browser-controlled Fetch Metadata, so a spoofed proxy header alone
  cannot bypass that refusal. Authored inline scripts and handlers run under a response-specific CSP;
  runtime-injected handlers remain blocked. Figure frames have a separate policy
  and are admitted by the desktop preview only as the same app's subframe.
  Deleting an app retires its sockets and tokens; a replacement has a new
  `app_uid` and a separate durable conversation. Per-app origins and private-app
  classification remain the separately documented platform followups.
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
`cargo test -p biorouter-cli --lib` (**the isolated `HOME` is no longer needed** — run it
plain. `crates/biorouter-cli/src/test_sandbox.rs` points `BIOROUTER_PATH_ROOT` at a
throwaway dir in a `#[ctor]`, before `main`, and freezes `Config::global()` and the session
store's root there, so the DEFAULT invocation — what a developer types, and what an
editor's test runner emits — reaches nothing real. Measured on this machine: a plain
`cargo test -p biorouter-cli` is 503 passed in 35s with `~/.config/biorouter` and
`~/.local/share/biorouter` byte-for-byte unchanged, where the same command on `main` at
5a404ecd wrote `<HOME>/.config/biorouter/config.yaml`.

⚠ If you prefix ANY cargo command with `HOME=$(mktemp -d)` for some other reason, keep both
`CARGO_HOME` and `RUSTUP_HOME` pointing at the real ones as LITERAL paths. Cargo and rustup
both derive their homes from `$HOME`, so a bare `HOME=$(mktemp -d) cargo test …` loses the
crate registry and dies with `error[E0463]: can't find crate for \`biorouter\``. That
surfaces as **rc=101 with zero failing tests**, which reads as a flake rather than as
nothing having compiled — and a harness that greps for `test result:` reports the
PREVIOUS build's count. And **do not write `CARGO_HOME="$HOME/.cargo"` in the same
command**: this line said exactly that until 2026-09-01 and it is wrong in zsh, because the
assignments are applied left to right, so `$HOME` expands to the **new temp dir**, not the
real one. The failure is not an error — it silently redirects `RUSTUP_HOME` as well, so
cargo re-downloads the whole toolchain and every target recompiles from scratch (measured:
`installing component 'rustc'` mid-run, and 11m38s for a single crate that normally takes
seconds). Use literals:

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
`cargo test -p biorouter --lib -- subagent` (**198 tests, measured 2026-09-12** — this
line said 102 for long enough that a "pre + N" assertion against it would have read a
shortfall of ninety-six as a pass; re-measure rather than trusting the figure, which
moved 197 → 198 between this line being written and the branch carrying it landing).

### Steering a running turn

A steer (`POST /interrupt`, "Add this message to the current turn") is accepted into the agent's
soft-interrupt queue and must always reach the loop or be stored as unanswered. Reference:
[`docs/agent-loop/steering-a-running-turn.md`](docs/agent-loop/steering-a-running-turn.md).

- **The queue opens before the prologue and closes only at the commit point.** The runner
  (`workspace/turn.rs` `prepare_turn`) opens it before the extension wait; the loop drains it
  immediately before the `break` that really ends a turn (`commit_turn_exit_or_continue`), never
  before the done gate, self-critique or Stop hooks. A `409` names its reason
  (`no_turn` / `not_accepting_yet` / `turn_closing`); the 403 shapes are untouched (SD-11).
- **A steer leaves the queue only after its row is stored** (`consume_soft_interrupts_at_boundary`,
  `land_live_ack`), and every row a turn carries past an ending that did not read it is marked
  `steer_outcome: unanswered`. A steer the person typed resets `max_turns` / `max_tool_calls` /
  the stall stop; a spend cap does not.
- ⚠ **Every new wait in the reply loop needs a steer arm, and it goes in a `Box::pin`'d helper**
  (`next_batch_wake`, `next_gate_wake`, `next_approval_wake`, `open_provider_or_steer`,
  `next_native_supervision_claim`). State lives on `Agent` — `loop_phase`, `live_acks`,
  `turn_exit`, `steer_arrivals` — never as generator locals (the stack cliff). Several waits can
  listen at once, which is why they use the `steer_arrivals` watch and not `notify_one`.
- **The renderer owns a steer until the daemon answers** (retry with one idempotency key), retires
  its chip by message id, treats every frame including `Ping` as a heartbeat, and reconciles a
  silent or orphaned running state with `/agent/resume`.
- **Observers are capped at two** (`MAX_LIVE_OBSERVER_STREAMS`): 2 observers + 2 renderer
  long-polls + 1 `/reply` must leave one of Chromium's six connections free.
- Tests: `cargo test -p biorouter --test steer_always_lands`, the three stack-margin binaries
  (`soft_interrupt_agent_loop`, `turn_abort_tests`, `subagent_delegation`), and
  `ui/desktop/src/hooks/chatStreamStore.steerLands.test.tsx`.

### Browser access (`biorouter serve`)

`biorouter serve` (alias `headless`) starts `biorouterd`, points it at the built interface and
prints a URL. The daemon serves the SPA **on its own origin**, so nothing is proxied. This
replaced a standalone `biorouter-headless` binary and its Linux tarball, both deleted
2026-08-23; release assets went 11 → 10. Design and reasoning:
[`docs/deployment/serve-decisions.md`](docs/deployment/serve-decisions.md) (the `SD-n` records),
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
- **A new chat starts on the configured model without a proof, and nothing else does** (SD-12).
  Until it, a `serve` daemon with a private provider configured refused EVERY `/agent/start`
  (the 2026-09-10 QA's F1): the new-chat bind asked for a proof a keyless daemon cannot check.
  Four pieces, each load-bearing — measured by removing it: on a keyless daemon
  `new_chat_bind_decision` (`routes/agent.rs`) lets the configured default bind, while a keyed
  daemon still refuses a proof-less private first bind; `raise_baseline` makes a keyless
  daemon's `/agent/update_provider` measure every move onto a private model from Public, or the
  exemption would carry sideways to a private model nobody configured; the browser states the
  host's model as `X-Caller-Provider` (`userActionHeaders()` on `isBrowserSurface()`), without
  which a chat's first reply ratcheted it private and its next request 403'd; and
  `biorouter_server::launch` **pins the exemption to the configuration the daemon was launched
  with**. Tests:
  `cargo test -p biorouter-server --test new_chat_no_user_key` (its own binary: the digest is a
  process-global `OnceLock`).
  ⚠ **The exemption's first justification was FALSE and the fix is that pin.** It rested on
  *"`/agent/start` binds `BIOROUTER_PROVIDER`, a key only a proven person may write"*. The HTTP
  doors are shut, but `config.yaml` is not an HTTP resource: DR-14's filesystem deny is DEFERRED,
  the agent holds `developer__shell`, and `Config`'s cache is keyed on a `FileStamp` it re-`stat`s
  per read — so a model with a shell on a keyless daemon configured PUBLIC could write a private
  provider into the file and get a 200 with Private capability where `main` answers 409 (measured
  2026-09-12). The pinned set is `privacy::CAPABILITY_CONFIG_KEYS` verbatim, which since 2026-10-10 includes `BIOROUTER_MODEL`;
  pinning the provider NAME alone is not enough, because flipping `OLLAMA_HOST` to loopback moves
  `ollama`'s tier with the name untouched. ⚠ And `NoKeyInstalled` is **not** the same thing as
  "this is `serve`" — a desktop spawn satisfies it when `userActionKey` is undefined or the
  daemon's bounded 2s stdin read times out. That case is a repairable fault, so the desktop
  launcher declares its intent in `BIOROUTER_USER_ACTION_EXPECTED` and such a daemon keeps
  `main`'s refusal plus a startup `ERROR`. ⚠ Since 2026-10-10 `BIOROUTER_MODEL` is a capability
  key, because `ollama`'s `tier()` reads the model name (a cloud tag is Public through a loopback
  host); an unproven upsert or remove is refused like `OLLAMA_HOST`'s, and
  `the_model_key_is_a_capability_key_exactly_while_a_tier_reads_the_model` ties the classification
  to that read. Until then it was an integrity key in `NOT_CAPABILITY_CONFIG_KEYS`, pinned here by
  hand. ⚠ **Still unreachable in a browser, and out of SD-12's scope:**
  `/agent/cancel` and `/interrupt` require the proof unconditionally, so Stop and mid-turn
  steering cannot work on a keyless daemon. ⚠ `privacy_ar15_is_retired.rs`'s closure scan took
  the FIRST `TierRaiseNeedsUser` in `routes/agent.rs`, which from `eb594ded` was the new-chat gate
  and not AR-15's — it now starts at `update_agent_provider`.
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
- **Stop answers to the reach gate on a keyless daemon; steering does not** (SD-11).
  `/agent/cancel` and the two `/agent/continuation/*` routes take the proof on a daemon that
  holds a key, and on one that holds none gate through `authorize_agent_control` — the *same
  call* `/agent/stop` makes — via `reply.rs::authorize_turn_control`. Tightening that gate
  tightens who may press Stop in a browser. ⚠ **`/interrupt` is NOT one of them.** It keeps the
  proof on both kinds of daemon: the keyless arm's whole argument is that the caller already
  reaches the same effect through `/agent/stop` and `/reply`, and `/reply` is refused `409` by
  the BR-33 single-turn lock in the exact state where a steer lands — so admitting it would add
  silent mid-turn injection into a turn already in flight, which nothing else there can do
  (`reply.rs::steer_refusal`). Its keyless refusal carries `STEER_NO_KEY` and is **never an
  empty 403**, because an empty turn-control 403 is how `biorouter session attach` recognises a
  daemon that holds a key and asks the person for it. A subagent's tab stays refused throughout.
  ⚠ Keyless behaviour can only be tested in its own binary (the digest is a process-global
  `OnceLock`): `cargo test -p biorouter-server --test turn_control_no_user_key`.
  ⚠ **The CLI reads the refusal's shape, not its status.** `biorouter session cancel` / `attach`
  / `send` cannot ask a daemon whether it holds a key, so they send without the proof and ask the
  person for the key only on turn control's **empty** 403 — the keyed `Unproven` arm; every
  keyless refusal carries a sentence and is printed instead (`key_verdict` in
  `commands/session_watch.rs`). A sentence added to `Unproven`, or an empty keyless refusal,
  breaks the terminal silently — one never prompts on the desktop's daemon, the other prompts a
  `serve` user for a key that does not exist. Pinned from both sides.
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
  The document is gated by a browser token exchanged for an `HttpOnly; SameSite=Strict`
  cookie; the cookie authenticates **the document only** — API routes still take
  `X-Secret-Key`, so there is no CSRF surface and `check_token` needed no change. ⚠ The
  exchange does **not** consume the token — it is honoured as often as it is presented until the
  daemon stops, and the cookie's value *is* the token. Deliberate, not an oversight: SD-9 in
  `serve-decisions.md` records why single use was rejected. It used to be called "spent".
- **`routes::shell`** holds the 16 `/headless/*` paths — 17 handlers, since `/headless/settings`
  answers both GET and POST (path kept deliberately; the renderer builds
  `origin + '/headless'`). They had **no authentication at all** on the old
  binary and `fs_read` had no path validation; the port confines every filesystem handler to
  an allowlist and refuses credential stores by name.
- **WebSocket origins**: both socket gates (`/ui/workspace`, `/apps/{id}/agent`) ask
  `routes::UpgradeOrigin::is_this_daemons` — the `Origin` must match the request's own `Host`
  in **scheme, host and port** (`origin_matches_host`), which is what lets a browser reach the
  daemon at a LAN address nobody enumerated. The scheme is `http` unless a proxy in front says
  `X-Forwarded-Proto: https`, so a TLS proxy must forward both `Host` and that header. Beside it,
  the gates admit the origin of the address `biorouter serve --public-url` declares
  (`BIOROUTER_PUBLIC_URL`, `routes::PublicUrl`), which is how a proxy chain that rewrites `Host`
  or appends `http` to `X-Forwarded-Proto` (JupyterHub's configurable-http-proxy, issue #402)
  still reaches the live views. Chrome sends no Fetch Metadata on a WebSocket handshake (measured
  2026-10-16), so the browser cannot vouch for a socket the way it does for a page. The one
  other origin admitted is a renderer the launcher declares in `BIOROUTER_RENDERER_ORIGIN`:
  `main.ts` declares the dev renderer's vite origin (loopback `http`) or, packaged, the literal
  `file://`, and `just debug-server` declares vite's default. ⚠ **`file://` is admitted by NAME
  but only where it was DECLARED**, and only on the workspace gate — an Electron `file:` page is
  the one client that opens that socket, while an app's page is served by the daemon over http
  and so is same-origin with its own. It used to be admitted by name on *every* daemon, which
  meant a local `.html` opened in a browser cleared the origin gate on a `biorouter serve` host,
  where `/ui/workspace` is one of only two paths exempt from `check_token`; `serve` now strips
  the variable from the daemon it spawns. ⚠ Until QA-D F7 (2026-09-11) the gates also took
  `is_local_origin` — any
  loopback port, scheme ignored — so every local page's socket passed as the daemon's own.
  `is_local_origin` is the **CORS** rule now and nothing else; do not hand it back to a socket
  gate.
- **`serve` owns its daemon's lifetime**, because the daemon honours the token and serves the
  shell carrying its secret for as long as it runs: stopping `serve` is the only revocation.
  Two layers. Every exit path after the spawn goes through `stop_daemon` (SIGTERM, a 10 s grace,
  then SIGKILL and reap), with SIGINT/SIGTERM listeners installed *before* the spawn; and on
  Unix the daemon is started with `biorouterd agent --exit-with-parent <serve pid>`, which
  covers a SIGKILLed or crashed `serve`. ⚠ Until 2026-09 neither held — the `Child` was moved
  into a `spawn_blocking` wait, so only a terminal's process-group Ctrl-C ever reached the
  daemon and `kill <pid of serve>` orphaned it on the port. ⚠ The flag is opt-in and compares
  `getppid()` with the pid `serve` named, **not with 1**: an orphan is re-parented to the
  nearest subreaper (`systemd --user`, a container init), so `== 1` never fires there. Never
  pass it from the desktop.
- ⚠ **Three traps.** The app uses a **HashRouter**, so its routes live in the fragment and
  never reach the daemon — that is the only reason `/sessions/{id}` (a real API route) does
  not collide with the app's own; a history router would break pages silently. The bundle is
  **relative** (`npm run build:web` passes `--base ./` → `ui/desktop/src/web`), and that is safe
  only because `routes::web_ui` writes `<base href>` (`mount_href`: `./`, `../`, …) as the first
  element of the shell's head: without it a path that falls through to the shell asks for
  `/a/assets/...` and gets the shell back. It used to be **root-base** for exactly that reason,
  which made every reverse proxy path prefix 404 its assets (issue #402, SD-15 supersedes SD-4);
  do not go back, and do not drop the `<base>`. The renderer derives its API base from the same
  base URI, so nothing in the served page may assume it lives at `/`. And `<exe>/../web` resolves
  for the packaged app and Windows zip but **not** for deb/rpm (`/usr/bin/../web` = `/usr/web`),
  hence `/usr/share/biorouter/web`.
- **A proxied address counts as secure on the browser's word** (SD-15).
  `auth::reached_over_a_secure_context` admits a non-loopback `Host` when the request carries
  `Sec-Fetch-Mode` and `Sec-Fetch-Dest`, which a browser sends only to an https or loopback URL
  and script cannot forge. It no longer asks `X-Forwarded-Proto`: requiring it beside the metadata
  added nothing and refused JupyterHub, whose chain reports `https,http`. A plain-HTTP LAN
  address is still refused, because Chrome sends no Fetch Metadata there.
- **Tests:** `cargo test -p biorouter-server --lib -- routes::web_ui routes::shell` (the `--`
  is required: cargo takes one filter before it and rejects a second with a usage error),
  `cargo test -p biorouter-cli --lib commands::serve`, the `serve` job in
  `.github/workflows/rust.yml`, and `smoke_serve` in
  `scripts/smoke-test-release-artifacts.sh`. The lifecycle has its own binary,
  `cargo test -p biorouter-cli --test serve_lifecycle`, which runs the real `serve` and
  signals it by pid; ⚠ it needs a `biorouterd` from the same tree beside `biorouter`, which
  `cargo test -p biorouter-cli` does not build — run `cargo build -p biorouter-cli -p
  biorouter-server` first. CI runs it in the `serve` job, because the workspace test job is
  `--lib --bins` and never runs an integration binary.

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
- Windows: everything above sits under `%APPDATA%\UCSF\biorouter` (`config\`, and `data\`, which also holds state and logs because etcetera's Windows strategy has no state dir), the cache under `%LOCALAPPDATA%\UCSF\biorouter`. The author segment is `APP_AUTHOR = "UCSF"` in `crates/biorouter-mcp/src/app_folder/mod.rs`, deliberately not the product name, because Electron's own user-data folder on Windows is `%APPDATA%\Biorouter` and the two must stay apart; macOS and Linux use XDG, which never reads the author. Earlier releases used another author folder, **the legacy folder**, whose name is spelled only by `LEGACY_APP_AUTHOR` (once in Rust, once in `ui/desktop/src/utils/appFolder.ts`; never write it anywhere else, see the retired-name guard).
  - **Resolution, on every call, in every process:** the legacy folder while it is present (a check that fails counts as present) and the current one is known absent; otherwise the current one, which wins when both exist (the legacy one is then left alone and the desktop app says so at start).
  - ⚠ **Only the daemon moves it.** `biorouterd agent` calls `app_folder::move_for_this_process()` before logging (`biorouter-server/src/main.rs`); the CLI, `biorouterd mcp` children and the desktop main process never move anything. It moves only when the legacy folder is present, the current one absent (both known) and no other Biorouter process holds the legacy folder's `.biorouter-folder.lock` (`MOVE_LOCK`): every process that resolves the legacy folder holds that lock shared for life (through `platform_dirs`), because `Config::global`, the session store and the log directory freeze the path. The move drops `.folder-move-pending` (`MOVE_MARKER`) into the legacy folder first, then makes one rename inside one base, so it is atomic and never a copy.
  - **The rewrite** (`app_folder/rewrite.rs`) then fixes stored absolute paths that pointed into the legacy folder, the ones that break silently otherwise: the per-chat `uv run --directory` snapshots of `.brxt` extensions, the knowledge base list, Crew's `daemon-profile.json` and a marketplace extension's provenance `install_dir`. It edits each file's text, matching both the environment's and `fs::canonicalize`'s spelling of the base, and writes only when re-parsing the new text equals rewriting the old parse; transcripts, logs, `*.bak*` and venvs are left alone. The marker travels with the tree, so a crash between the rename and the rewrite is finished on the next start.
  - ⚠ **The desktop resolves by the same rule in `appFolder.ts`, and the two must agree.** Both run one table of cases, `app_folder/resolution-cases.json`, and `the_desktop_constants_match_these` (`app_folder/tests.rs`) reads `appFolder.ts` and pins `APP_AUTHOR`, `APP_NAME` and `LEGACY_APP_AUTHOR` equal across the two languages. User-facing steps: [`docs/troubleshooting/common-problems-and-fixes.md`](docs/troubleshooting/common-problems-and-fixes.md).
- Shared daemon user-action key (macOS and Linux): `~/.local/state/biorouter/daemon/user-action-key.json`, mode 0600, written by whoever starts the daemon (desktop app or `biorouter crew`), read by every later client, removed by the daemon on exit. Nobody types an approval secret; a daemon whose key is missing or refused is stopped and replaced.
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

### Native Biorouter Copilot

**Upstream is VENDORED, not cloned.** `vendor/computer-use/source/` is a
complete copy of the MIT-licensed upstream at `pin.json`'s commit, committed here, and a
build reads it with **no network** — an upstream repository that disappears or changes
cannot affect a BioRouter build. The tree is *pristine*; `patches/` is applied to a
throwaway copy at build time, which is what keeps an upstream update tractable.
`source-manifest.json` hashes every file and the build verifies it (both directions)
before the bytes become build input. `scripts/check-vendored-computer-use.sh` is the CI
guard and also asserts every vendored file is **committable** — repo-wide `.gitignore`
rules matched 15 of them on the first attempt. Ten upstream files are deliberately
excluded as third-party extracted artwork that MIT does not cover; they are docs, not
build inputs, and the build is byte-identical without them.
⚠ The staged copy gets its own `git init`, and that line is load-bearing: `git apply`
resolves against the repository containing the working directory, and the staging
directory lives inside BioRouter's, where `target/` is ignored. Without it the edits to
tracked files landed while every `new file mode` hunk was silently dropped, and the build
happily produced a helper 28,672 bytes smaller from effectively unpatched upstream.
`scripts/check-computer-use-upstream.py` reports what changed upstream and, more usefully,
which of those files our patches also touch.


`computercontroller` — the extension key Biorouter Copilot kept for config
compatibility — exposes exactly ten native tools: `list_apps`, `get_app_state`, `click`,
`perform_secondary_action`, `scroll`, `drag`, `type_text`, `press_key`, `set_value`,
`screen_capture`. The native helper owns all desktop observation/control; Developer has no
capture or control tool. `webdocuments` owns web/document/cache utilities. Old scripting,
control and capture implementations and aliases are removed, not migrated or hidden.

The host obtains one Biorouter Copilot grant per task/chat/model/target before observation. Respect
Stop/revoke, avoid retry loops and script fallbacks, refresh stale state, and verify outcomes.
Private/public chats retain separate observations and consent. Pause before private-to-public
handoff because the physical desktop is shared. See `docs/design/computer-use-integration-plan.md`
and the implementation-status ledger for all acceptance gates, including real-app validation.
