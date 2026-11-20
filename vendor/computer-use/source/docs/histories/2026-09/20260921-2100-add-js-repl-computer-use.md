## [2026-09-21 21:00] | Task: Add JS REPL Computer Use

### 🤖 Execution Context
* **Agent ID**: `TRAE CLI`
* **Base Model**: `GPT-5.4`
* **Runtime**: `local macOS repository workspace`

### 📥 User Query
> Review the community PRs and the bundled official Computer Use plugin, determine how its JavaScript REPL works, and add JS REPL support to Open Computer Use.

### 🛠 Changes Overview
**Scope:** Codex plugin transport, Node.js adapter, native MCP bridge, tests, and repository documentation.

**Key Actions:**
- **Official runtime investigation**: Verified that the inspected official bundle uses a host-provided `node_repl` and instructs the model to import its asynchronous `@oai/sky` package.
- **Persistent REPL**: Added a Node.js REPL with top-level await, persistent bindings, explicit text/image output, reset, and a worker timeout boundary.
- **Native bridge**: Adapted the existing native 9-tool MCP server into an asynchronous `cua.getApp(...)` API without duplicating the platform automation implementations.
- **Compatibility**: Switched the Codex plugin to `js` / `js_reset` while preserving `open-computer-use mcp` as the existing native compatibility surface.
- **Packaging and docs**: Included the REPL files in local/plugin npm installation and documented architecture, security, reliability, and usage.
- **Lifecycle hardening**: Isolated model code in a Worker, capped execution at five minutes, reset the kernel after timeouts, made shutdown idempotent, and verified copied/symlinked adapter entrypoints.
- **Installer correction**: Flattened the plugin source into the versioned Codex cache root (matching Codex's cache layout) before adding the native payload and REPL scripts.
- **Protocol normalization**: Converted the native human-readable macOS/Linux/Windows app catalog into a stable structured array for `cua.listApps()`.
- **Concurrency and failure isolation**: Serialized direct Worker callers, converted invalid source input into structured errors, and let the outer JS timeout remain the single deadline for native calls so timeout recovery cannot leave a second timer behind.

### 🧠 Design Intent (Why)
The main value of a code tool is not another spelling of the same actions. It is the ability to keep deterministic sequencing, intermediate state, branching, retry, and final observation inside one tool round trip. The implementation therefore follows the official asynchronous Node orchestration direction while providing a small app-bound API, rather than merging the synchronous JavaScriptCore prototype from PR #75.

### 📁 Files Modified
- `scripts/node-repl/open-computer-use-repl.mjs`
- `scripts/node-repl/open-computer-use-kernel.mjs`
- `scripts/node-repl/open-computer-use-repl.test.mjs`
- `plugins/open-computer-use/.mcp.json`
- `plugins/open-computer-use/scripts/launch-open-computer-use-repl.sh`
- `scripts/install-codex-plugin.sh`
- `scripts/npm/build-packages.mjs`
- `docs/references/js-repl.md`
- `docs/ARCHITECTURE.md`
- `docs/SECURITY.md`
- `docs/RELIABILITY.md`

### ✅ Validation
- `node --test scripts/node-repl/open-computer-use-repl.test.mjs` — 12/12 passed locally and on Linux devbox with Node 22.
- `swift test` — 167 tests, 0 failures, 1 opt-in live test skipped.
- Linux and Windows `go test ./...`; Linux Python runtime tests — passed.
- `./scripts/run-tool-smoke-tests.sh` — native 9-tool and cursor-idle smoke passed.
- npm staging, `npm pack --dry-run --json`, isolated Codex plugin install, and staged/installed `tools/list` probes — passed and exposed only `js` / `js_reset`.
- Real macOS native bridge probe verified structured `cua.listApps()`, binding persistence, CPU timeout recovery, and `js_reset`. Linux devbox verified the same REPL lifecycle and native bridge with Node 22; after installing its missing AT-SPI introspection package, the headless SSH session correctly returned a structured empty app array.
- `make check-docs`, shell/Node syntax checks, action pinning, and `git diff --check` — passed. Full `scripts/ci.sh` remains blocked only by pre-existing repository-hygiene files absent from this checkout, unrelated to this change.
