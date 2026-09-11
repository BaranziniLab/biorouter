# Justfile

# list all tasks
default:
  @just --list

# Run all style checks and formatting (precommit validation)
check-everything:
    @echo "🔧 RUNNING ALL STYLE CHECKS..."
    @echo "  → Formatting Rust code..."
    cargo fmt --all
    @echo "  → Running clippy linting..."
    ./scripts/clippy-lint.sh
    @echo "  → Checking UI code formatting..."
    cd ui/desktop && npm run lint:check
    @echo "  → Validating OpenAPI schema..."
    just check-openapi-schema
    @echo "  → Checking CLI/daemon/GUI version consistency..."
    ./scripts/check-version-consistency.sh
    @echo "  → Checking Biorouter name and logo consistency..."
    ./scripts/check-brand-consistency.sh
    @echo "  → Checking cross-compile recipes have not drifted (glibc floor pin)..."
    ./scripts/check-no-cross-drift.sh
    @echo "  → Checking the BAAM registry generator still refuses what it must..."
    just check-registry
    @echo "  → Checking the BAAM private set agrees in all three committed copies..."
    just check-privacy-registry
    @echo ""
    @echo "✅ All style checks passed!"

# Assert the CLI, daemon, and GUI all report the same version (source of truth:
# Cargo [workspace.package].version). Guards against hand edits drifting one of
# the desktop JSON files out of sync with the Rust workspace version.
check-versions:
    ./scripts/check-version-consistency.sh

# The BAAM registry generator writes three copies of one catalog, one of them a
# compiled-in Rust security baseline (crates/biorouter/src/privacy/
# registry_private.rs). Its refusals are the only thing standing between a
# mis-tagged card and a private extension silently classified Public, so they
# need a gate of their own. No npm install: node:test ships with the runtime.
check-registry:
    node --test landing/scripts/build-registry.test.mjs
    node landing/scripts/build-registry.mjs --check

# The set of private extensions is committed in three places at once: the
# published landing/registry.json, the snapshot bundled in the desktop app, and
# the Rust baseline compiled into the CLI and the daemon. `check-registry` above
# asks whether they are byte-for-byte what baam.html generates; this asks the
# question that survives a generator change — do all three name the same private
# set, and does that set have anything in it at all. An extension missing from
# the compiled-in copy is not a build error, it is silently classified Public.
#
# The mutant suite runs FIRST, and for the same reason `check-registry` has one:
# a checker earns its invocation only by failing when it should. The live
# `--check` below passes against a `checkDocsPrivacy` rewritten to `return []` —
# the suite is what refuses that, by requiring 20 ways of being wrong to each
# produce a failure. No npm install: node:test ships with the runtime.
check-privacy-registry:
    node --test landing/scripts/check-docs-privacy.test.mjs
    node landing/scripts/check-consistency.mjs --check

# The BAAM shelf's privacy badges and Private/Public facet, in a real browser.
# Half of what it asserts is layout — a chip that does not fit the 22px
# `overflow: hidden` tag row is not cramped, it is deleted from view — and jsdom
# returns zeros for every rect, so it cannot stand in. NOT part of
# `check-everything`: it resolves Playwright out of ui/desktop/node_modules and
# needs a chromium, neither of which a bare checkout has. CI runs it as its own
# job, and the landing deploy is gated on it.
check-shelf:
    node --test landing/scripts/baam-privacy-facet.test.mjs

# Default release command
release-binary:
    @echo "Building release version..."
    cargo build --release
    @just copy-binary
    @echo "Generating OpenAPI schema..."
    cargo run -p biorouter-server --bin generate_schema

# Build Windows executable (x86_64-pc-windows-gnu) via Docker.
# The cross recipe — mingw toolchain, the winpthread linker wrap, LZMA_API_STATIC
# and the runtime-DLL staging — lives in scripts/cross-env.sh (one source of
# truth, shared with scripts/release.sh and the BR-70 check-cross CI gate). This
# is a Docker-based recipe; the release is cut on macOS/Linux with system docker.
release-windows:
    #!/usr/bin/env bash
    set -euo pipefail
    . scripts/cross-env.sh
    echo "Building Windows executable using Docker ($WIN_RUST_IMG)..."
    cross_windows "cargo build --release --bin biorouterd --bin biorouter" "" "$WIN_DLL_STAGE"
    echo "Windows executable and required DLLs created at ./target/x86_64-pc-windows-gnu/release/"

# Build Linux x64 .deb package for Ubuntu / Pop!_OS — requires Docker Desktop
# Output: ui/desktop/out/make/deb/x64/BioRouter_<version>_amd64.deb
# Note: src/bin/ will contain Linux x64 binaries after this build.
# Run 'just copy-binary' afterward to restore macOS ARM binaries.
# The Rust cross-compile uses scripts/cross-env.sh (pinned rust:1.92-bullseye,
# glibc 2.31 floor) — the SAME recipe the release + check-cross gate use. This
# recipe previously pinned `rust:latest`, silently raising the glibc floor.
make-ui-linux:
    #!/usr/bin/env bash
    set -euo pipefail
    . scripts/cross-env.sh
    echo "Step 1/2: Cross-compiling Rust binaries for Linux x64 via Docker ($LINUX_RUST_IMG)..."
    cross_linux "cargo build --release"
    echo "Step 2/2: Packaging .deb via Docker (linux/amd64)..."
    docker volume create biorouter-linux-npm-cache || true
    docker run --rm \
        --platform linux/amd64 \
        -v "$(pwd)":/ws \
        -v biorouter-linux-npm-cache:/root/.npm \
        node:20-bookworm \
        bash /ws/ui/desktop/scripts/build-linux-deb.sh
    echo ""
    echo "✓ .deb package: ui/desktop/out/make/deb/x64/"
    echo "  Run 'just copy-binary' to restore macOS ARM binaries in src/bin/."

# ── Cross-platform compile gate (BR-70) ──────────────────────────────────────
# Type-check the ENTIRE workspace — every crate, every target, INCLUDING
# #[cfg(test)] code — for Windows and Linux, using the SAME docker images and
# linker hacks as the release (both source scripts/cross-env.sh, so the gate can
# never drift from what actually ships). `cargo check` (not build): it catches
# cfg / target-dep mistakes and type errors in platform-gated code without
# paying to link, and still runs build.rs (lzma-sys / aws-lc-sys / protoc). Link
# errors are covered by the nightly `build-cross`. Needs Docker; deliberately
# NOT part of `check-everything` (it takes minutes). Expect the Windows lane to
# surface a backlog on first run — that backlog is the bug this gate exposes.
check-cross: check-cross-linux check-cross-windows

check-cross-linux:
    #!/usr/bin/env bash
    set -euo pipefail
    . scripts/cross-env.sh
    echo "→ cargo check x86_64-unknown-linux-gnu ($LINUX_RUST_IMG, glibc floor $GLIBC_FLOOR)"
    cross_linux "cargo check --workspace --all-targets --locked" \
                "/usr/src/myapp/target/cross-check/linux"

check-cross-windows:
    #!/usr/bin/env bash
    set -euo pipefail
    . scripts/cross-env.sh
    echo "→ cargo check x86_64-pc-windows-gnu ($WIN_RUST_IMG)"
    cross_windows "cargo check --workspace --all-targets --locked" \
                  "/usr/src/myapp/target/cross-check/windows"

# Full cross BUILD of the shipped binaries — catches LINK errors (the aws-lc /
# winpthread class) that `check` cannot, then asserts the glibc floor is intact.
# Nightly in CI; run locally before a release.
build-cross:
    #!/usr/bin/env bash
    set -euo pipefail
    . scripts/cross-env.sh
    cross_linux   "cargo build --release --bin biorouterd --bin biorouter"
    cross_windows "cargo build --release --bin biorouterd --bin biorouter" "" "$WIN_DLL_STAGE"
    ./scripts/check-glibc-floor.sh

# Build for Intel Mac
release-intel:
    @echo "Building release version for Intel Mac..."
    cargo build --release --target x86_64-apple-darwin
    @just copy-binary-intel

# Sign locally built binaries with the Developer ID (when the identity is in
# the keychain) so the macOS Keychain "Always Allow" grant survives rebuilds.
# Unsigned dev binaries carry a per-build cdhash requirement, so the Keychain
# treats every rebuild as a new app and prompts again; a Developer ID
# signature pins a stable designated requirement (team + identifier) instead.
# No-op on non-macOS or when no identity is installed.
sign-dev-binaries BUILD_MODE="release":
    #!/usr/bin/env sh
    [ "$(uname)" = "Darwin" ] || exit 0
    IDENTITY=$(security find-identity -v -p codesigning 2>/dev/null | grep "Developer ID Application" | head -1 | sed 's/.*"\(.*\)"/\1/')
    if [ -z "$IDENTITY" ]; then
        echo "No Developer ID identity in keychain; dev binaries stay ad-hoc signed (Keychain will re-prompt after rebuilds)"
        exit 0
    fi
    for bin in biorouterd biorouter; do
        if [ -f "./target/{{BUILD_MODE}}/$bin" ]; then
            codesign --force --sign "$IDENTITY" "./target/{{BUILD_MODE}}/$bin" && \
                echo "Signed target/{{BUILD_MODE}}/$bin (stable Keychain identity)"
        fi
    done

copy-binary BUILD_MODE="release":
    @just sign-dev-binaries {{BUILD_MODE}}
    @mkdir -p ./ui/desktop/src/bin
    @if [ -f ./target/{{BUILD_MODE}}/biorouterd ]; then \
        echo "Copying biorouterd binary from target/{{BUILD_MODE}}..."; \
        cp -p ./target/{{BUILD_MODE}}/biorouterd ./ui/desktop/src/bin/; \
    else \
        echo "Binary not found in target/{{BUILD_MODE}}"; \
        exit 1; \
    fi
    @if [ -f ./target/{{BUILD_MODE}}/biorouter ]; then \
        echo "Copying biorouter CLI binary from target/{{BUILD_MODE}}..."; \
        cp -p ./target/{{BUILD_MODE}}/biorouter ./ui/desktop/src/bin/; \
    else \
        echo "biorouter CLI binary not found in target/{{BUILD_MODE}}"; \
        exit 1; \
    fi

# Copy binary command for Intel build
copy-binary-intel:
    @if [ -f ./target/x86_64-apple-darwin/release/biorouterd ]; then \
        echo "Copying Intel biorouterd binary to ui/desktop/src/bin with permissions preserved..."; \
        cp -p ./target/x86_64-apple-darwin/release/biorouterd ./ui/desktop/src/bin/; \
    else \
        echo "Intel release binary not found."; \
        exit 1; \
    fi
    @if [ -f ./target/x86_64-apple-darwin/release/biorouter ]; then \
        echo "Copying Intel biorouter CLI binary to ui/desktop/src/bin..."; \
        cp -p ./target/x86_64-apple-darwin/release/biorouter ./ui/desktop/src/bin/; \
    else \
        echo "Intel biorouter CLI binary not found."; \
        exit 1; \
    fi

# Copy Windows binary command
copy-binary-windows:
    @powershell.exe -Command "if (Test-Path ./target/x86_64-pc-windows-gnu/release/biorouterd.exe) { \
        Write-Host 'Copying Windows binary and DLLs to ui/desktop/src/bin...'; \
        Copy-Item -Path './target/x86_64-pc-windows-gnu/release/biorouterd.exe' -Destination './ui/desktop/src/bin/' -Force; \
        Copy-Item -Path './target/x86_64-pc-windows-gnu/release/*.dll' -Destination './ui/desktop/src/bin/' -Force; \
    } else { \
        Write-Host 'Windows binary not found.' -ForegroundColor Red; \
        exit 1; \
    }"

# Run UI with latest
run-ui:
    @just release-binary
    @echo "Running UI..."
    cd ui/desktop && npm ci && npm run start-gui

run-ui-playwright:
    #!/usr/bin/env sh
    just release-binary
    echo "Running UI with Playwright debugging..."
    RUN_DIR="$HOME/biorouter-runs/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$RUN_DIR"
    echo "Using isolated directory: $RUN_DIR"
    cd ui/desktop && ENABLE_PLAYWRIGHT=true BIOROUTER_PATH_ROOT="$RUN_DIR" npm run start-gui

# Launch the dev GUI for agent-browser (vercel-labs) CDP debugging.
# Exposes Chrome DevTools Protocol on a DEDICATED port (default 9333, NOT the
# Playwright default 9222) so it never collides with a regular Google Chrome
# the user may already have listening on 9222. Config is sandboxed under an
# isolated BIOROUTER_PATH_ROOT so the dev app can't clobber ~/.config/biorouter.
#
# In another terminal, drive it with:
#   agent-browser connect 9333      # once per session
#   agent-browser snapshot -i       # list interactive elements (@e1, @e2, ...)
#   agent-browser click @e5
#   agent-browser screenshot ui.png
#   agent-browser console --json    # renderer console + errors
# See docs/desktop-ui/agent-browser-debugging.md. Override the port with PORT=...
agent-browser-ui PORT="9333":
    #!/usr/bin/env sh
    echo "Building debug binary..."
    cargo build
    just copy-binary debug
    echo "Running dev GUI with agent-browser CDP on port {{PORT}}..."
    RUN_DIR="$HOME/biorouter-runs/$(date +%Y%m%d-%H%M%S)"
    mkdir -p "$RUN_DIR"
    echo "Using isolated config dir: $RUN_DIR"
    echo "Connect with:  agent-browser connect {{PORT}}"
    cd ui/desktop && ENABLE_PLAYWRIGHT=true PLAYWRIGHT_CDP_PORT={{PORT}} BIOROUTER_PATH_ROOT="$RUN_DIR" npm run start-gui

# Run debug build + Electron with CDP on port 9222 (for Playwright MCP debugging via .mcp.json)
dev-ui-playwright:
    @echo "Building debug binary..."
    cargo build
    @just copy-binary debug
    @echo "Starting Electron with CDP on port 9222 — connect via .mcp.json playwright-electron server"
    cd ui/desktop && ENABLE_PLAYWRIGHT=true npm run start-gui

run-ui-only:
    @echo "Running UI..."
    cd ui/desktop && npm ci && npm run start-gui

debug-ui *alpha:
    @echo "🚀 Starting biorouter frontend in external backend mode{{ if alpha == "alpha" { " with alpha features enabled" } else { "" } }}"
    cd ui/desktop && \
    export BIOROUTER_EXTERNAL_BACKEND=true && \
    export BIOROUTER_EXTERNAL_PORT=3000 && \
    {{ if alpha == "alpha" { "export ALPHA=true &&" } else { "" } }} \
    npm ci && \
    npm run {{ if alpha == "alpha" { "start-alpha-gui" } else { "start-gui" } }}

# Run UI with main process debugging enabled
# To debug main process:
# 1. Run: just debug-ui-main-process
# 2. Open Chrome → chrome://inspect
# 3. Click "Open dedicated DevTools for Node"
# 4. If not auto-detected, click "Configure" and add: localhost:9229

debug-ui-main-process:
	@echo "🔍 Starting biorouter UI with main process debugging enabled"
	@just release-binary
	cd ui/desktop && \
	npm ci && \
	npm run start-gui-debug

# Run UI with alpha changes
run-ui-alpha:
    @just release-binary
    @echo "Running UI with alpha features..."
    cd ui/desktop && npm ci && ALPHA=true npm run start-alpha-gui

# Run UI with latest (Windows version)
run-ui-windows:
    @just release-windows
    @powershell.exe -Command "Write-Host 'Copying Windows binary...'"
    @just copy-binary-windows
    @powershell.exe -Command "Write-Host 'Running UI...'; Set-Location ui/desktop; npm ci; npm run start-gui"

# Run Docusaurus server for documentation
run-docs:
    @echo "Running docs server..."
    cd documentation && yarn && yarn start

# Run server
run-server:
    @echo "Running server..."
    BIOROUTER_DISABLE_KEYRING=true cargo run -p biorouter-server --bin biorouterd agent

# Run server with secret=test and the published dev user-action key, so it pairs
# with `just debug-ui` (which sends X-Secret-Key: test and X-User-Action: <same>).
# The key is DELIBERATELY public: this daemon's user-proof is whatever the person
# who started it chose, and on this path that person is the developer. It weakens
# nothing in the shipped app, whose key is 32 random bytes per launch and never
# leaves the Electron main process. Issue #56 DR-16 / open question 23.
#
# BIOROUTER_RENDERER_ORIGIN names `debug-ui`'s renderer, vite's page on its own
# port, so the daemon's WebSocket gates admit it (QA-D F7). When vite takes the
# next port because 5173 is busy, pass that origin instead.
debug-server:
    @echo "Running server in debug mode (secret=test, published dev user-action key)..."
    printf '%s\n' "$(printf 'biorouter-dev-user-action' | shasum -a 256 | cut -d' ' -f1)" | BIOROUTER_DISABLE_KEYRING=true BIOROUTER_SERVER__SECRET_KEY=test BIOROUTER_RENDERER_ORIGIN="${BIOROUTER_RENDERER_ORIGIN:-http://localhost:5173}" cargo run -p biorouter-server --bin biorouterd agent

# Check if OpenAPI schema is up-to-date
check-openapi-schema: generate-openapi
    ./scripts/check-openapi-schema.sh

# Exercise the shipping image/PDF/Office preview components in a real Electron
# renderer. This is deliberately not folded into `check-everything`: it needs a
# native Electron runtime and display. The dedicated macOS Frontend CI job runs
# it for every PR and uploads the screenshots. Live websites are a separate
# packaged/dev-app release gate because this harness cannot create the app main
# process's native WebContentsView.
preview-panel-e2e:
    cd ui/desktop && npm run test:preview-panel

# Generate OpenAPI specification without starting the UI
generate-openapi:
    @echo "Generating OpenAPI schema..."
    cargo run -p biorouter-server --bin generate_schema
    @echo "Generating frontend API..."
    cd ui/desktop && npm run generate-api

# make GUI with latest binary
lint-ui:
    cd ui/desktop && npm run lint:check

# Build the browser interface bundle biorouterd serves (BIOROUTER_SERVE_UI).
# Writes a ROOT-BASE build of the renderer to ui/desktop/src/web/ (~8.8 MB),
# which every packaging path ships beside the binaries — see extraResource in
# ui/desktop/forge.config.ts, and /usr/share/biorouter/web in the CLI-only
# packages. Output is gitignored.
#
# You do NOT need to run this before packaging: prepare-platform-binaries.js
# (macOS + Windows), ui/desktop/scripts/build-linux-deb.sh (Linux GUI) and
# scripts/build-cli-linux-packages.sh (CLI deb/rpm) each build it themselves,
# so a package can never ship a stale or absent bundle. This target is for
# pointing a locally built biorouterd at a fresh bundle by hand:
#   just build-web && BIOROUTER_SERVE_UI=ui/desktop/src/web cargo run -p biorouter-server
#
# Deliberately NOT part of `check-everything`: it is a ~15 s bundler run that
# checks nothing. `npm run lint:check` (which check-everything already runs)
# type-checks and lints the same sources, so a build here could only fail where
# tsc and eslint both passed — and every path that needs the artifact builds it.
build-web:
    cd ui/desktop && npm run build:web

# make GUI with latest binary
make-ui:
    @just release-binary
    cd ui/desktop && npm run bundle:default

# make GUI with latest binary and alpha features enabled
make-ui-alpha:
    @just release-binary
    cd ui/desktop && npm run bundle:alpha

# make GUI with latest Windows binary
make-ui-windows:
    @just release-windows
    #!/usr/bin/env sh
    set -e
    if [ -f "./target/x86_64-pc-windows-gnu/release/biorouterd.exe" ]; then \
        echo "Cleaning destination directory..." && \
        rm -rf ./ui/desktop/src/bin && \
        mkdir -p ./ui/desktop/src/bin && \
        echo "Copying Windows binaries and DLLs..." && \
        cp -f ./target/x86_64-pc-windows-gnu/release/biorouterd.exe ./ui/desktop/src/bin/ && \
        cp -f ./target/x86_64-pc-windows-gnu/release/biorouter.exe ./ui/desktop/src/bin/ && \
        cp -f ./target/x86_64-pc-windows-gnu/release/*.dll ./ui/desktop/src/bin/ && \
        echo "Starting Windows package build..." && \
        (cd ui/desktop && npm run bundle:windows) && \
        echo "Windows package build complete!"; \
    else \
        echo "Windows binary not found."; \
        exit 1; \
    fi

# make GUI with latest binary
make-ui-intel:
    @just release-intel
    cd ui/desktop && npm run bundle:intel



# Run UI with debug build
run-dev:
    @echo "Building development version..."
    cargo build
    @just copy-binary debug
    @echo "Running UI..."
    cd ui/desktop && npm run start-gui

# Install all dependencies (run once after fresh clone)
install-deps:
    cd ui/desktop && npm ci

ensure-release-branch:
    #!/usr/bin/env bash
    branch=$(git rev-parse --abbrev-ref HEAD); \
    if [[ ! "$branch" == release/* ]]; then \
        echo "Error: You are not on a release branch (current: $branch)"; \
        exit 1; \
    fi

    # check that main is up to date with upstream main
    git fetch
    # @{u} refers to upstream branch of current branch
    if [ "$(git rev-parse HEAD)" != "$(git rev-parse @{u})" ]; then \
        echo "Error: Your branch is not up to date with the upstream branch"; \
        echo "  ensure your branch is up to date (git pull)"; \
        exit 1; \
    fi

# validate the version is semver, and not the current version
validate version:
    #!/usr/bin/env bash
    if [[ ! "{{ version }}" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-.*)?$ ]]; then
      echo "[error]: invalid version '{{ version }}'."
      echo "  expected: semver format major.minor.patch or major.minor.patch-<suffix>"
      exit 1
    fi

    current_version=$(just get-tag-version)
    if [[ "{{ version }}" == "$current_version" ]]; then
      echo "[error]: current_version '$current_version' is the same as target version '{{ version }}'"
      echo "  expected: new version in semver format"
      exit 1
    fi

get-next-minor-version:
    @python -c "import sys; v=sys.argv[1].split('.'); print(f'{v[0]}.{int(v[1])+1}.0')" $(just get-tag-version)

get-next-patch-version:
    @python -c "import sys; v=sys.argv[1].split('.'); print(f'{v[0]}.{v[1]}.{int(v[2])+1}')" $(just get-tag-version)

# set cargo and app versions, must be semver
prepare-release version:
    @just validate {{ version }} || exit 1

    @git switch -c "release/{{ version }}"
    @uvx --from=toml-cli toml set --toml-path=Cargo.toml "workspace.package.version" {{ version }}

    @cd ui/desktop && npm version {{ version }} --no-git-tag-version --allow-same-version

    # see --workspace flag https://doc.rust-lang.org/cargo/commands/cargo-update.html
    # used to update Cargo.lock after we've bumped versions in Cargo.toml
    @cargo update --workspace
    @just set-openapi-version {{ version }}
    @cargo run --bin build_canonical_models
    @git add \
        Cargo.toml \
        Cargo.lock \
        ui/desktop/package.json \
        ui/desktop/package-lock.json \
        ui/desktop/openapi.json \
        crates/biorouter/src/providers/canonical/data/canonical_models.json \
        crates/biorouter/src/providers/canonical/data/canonical_mapping_report.json
    @git commit --message "chore(release): release version {{ version }}"

set-openapi-version version:
    @jq '.info.version |= "{{ version }}"' ui/desktop/openapi.json > ui/desktop/openapi.json.tmp && mv ui/desktop/openapi.json.tmp ui/desktop/openapi.json

# extract version from Cargo.toml
get-tag-version:
    @uvx --from=toml-cli toml get --toml-path=Cargo.toml "workspace.package.version"

# create the git tag from Cargo.toml, checking we're on a release branch
tag: ensure-release-branch
    git tag v$(just get-tag-version)

# create tag and push to origin (use this when release branch is merged to main)
tag-push: tag
    # this will kick of ci for release
    git push origin tag v$(just get-tag-version)

# generate release notes from git commits
release-notes old:
    #!/usr/bin/env bash
    git log --pretty=format:"- %s" {{ old }}..v$(just get-tag-version)

### s = file seperator based on OS
s := if os() == "windows" { "\\" } else { "/" }

### testing/debugging
os:
  echo "{{os()}}"
  echo "{{s}}"

# Make just work on Window
set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

### Build the core code
### profile = --release or "" for debug
### allparam = OR/AND/ANY/NONE --workspace --all-features --all-targets
win-bld profile allparam:
  cargo run {{profile}} -p biorouter-server --bin  generate_schema
  cargo build {{profile}} {{allparam}}

### Build just debug
win-bld-dbg:
  just win-bld " " " "

### Build debug and test, examples,...
win-bld-dbg-all:
  just win-bld " " "--workspace --all-targets --all-features"

### Build just release
win-bld-rls:
  just win-bld "--release" " "

### Build release and test, examples, ...
win-bld-rls-all:
  just win-bld "--release" "--workspace --all-targets --all-features"

### Install npm stuff
win-app-deps:
  cd ui{{s}}desktop ; npm ci

### Windows copy {release|debug} files to ui\desktop\src\bin
### s = os depenent file seperator
### profile = release or debug
win-copy-win profile:
  copy target{{s}}{{profile}}{{s}}*.exe ui{{s}}desktop{{s}}src{{s}}bin
  copy target{{s}}{{profile}}{{s}}*.dll ui{{s}}desktop{{s}}src{{s}}bin

### "Other" copy {release|debug} files to ui/desktop/src/bin
### s = os depenent file seperator
### profile = release or debug
win-copy-oth profile:
  find target{{s}}{{profile}}{{s}} -maxdepth 1 -type f -executable -print -exec cp {} ui{{s}}desktop{{s}}src{{s}}bin \;

### copy files depending on OS
### profile = release or debug
win-app-copy profile="release":
  just win-copy-{{ if os() == "windows" { "win" } else { "oth" } }} {{profile}}

### Only copy binaries, npm ci, start-gui
### profile = release or debug
### s = os depenent file seperator
win-app-run profile:
  just win-app-copy {{profile}}
  just win-app-deps
  cd ui{{s}}desktop ; npm run start-gui

### Only run debug desktop, no build
win-run-dbg:
  just win-app-run "debug"

### Only run release desktop, nu build
win-run-rls:
  just win-app-run "release"

### Build and run debug desktop. tot = cli and desktop
### allparam = nothing or -all passed on command line
### -all = build with --workspace --all-targets --all-features
win-total-dbg *allparam:
  just win-bld-dbg{{allparam}}
  just win-run-dbg

### Build and run release desktop
### allparam = nothing or -all passed on command line
### -all = build with --workspace --all-targets --all-features
win-total-rls *allparam:
  just win-bld-rls{{allparam}}
  just win-run-rls

build-test-tools:
  cargo build -p biorouter-test

record-mcp-tests: build-test-tools
  BIOROUTER_RECORD_MCP=1 cargo test --package biorouter --test mcp_integration_test
  git add crates/biorouter/tests/mcp_replays/
