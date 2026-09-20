# AGENTS Instructions

BioRouter is an AI agent framework in Rust with CLI and Electron desktop interfaces.

## Setup
```bash
source bin/activate-hermit
cargo build
```

## Commands

### Build
```bash
cargo build                   # debug
cargo build --release         # release  
just release-binary           # release + openapi
```

### Test
```bash
cargo test                   # all tests
cargo test -p biorouter      # specific crate
cargo test --package biorouter --test mcp_integration_test
just record-mcp-tests        # record MCP
```

### Lint/Format
```bash
cargo fmt
./scripts/clippy-lint.sh
cargo clippy --fix
```

### UI
```bash
just generate-openapi        # after server changes
just run-ui                  # start desktop
cd ui/desktop && npm run test:run   # test UI (single pass; bare `npm test` is watch mode and never exits)
```

## Structure
```
crates/
├── biorouter          # core logic
├── biorouter-acp      # Agent Communication Protocol
├── biorouter-authprompt # macOS auth prompt helper (binary)
├── biorouter-bench    # benchmarking
├── biorouter-cli      # CLI entry; browser access is `biorouter serve`, not a separate crate
├── biorouter-sandbox  # capability-scoped sandboxed execution
├── biorouter-server   # backend (binary: biorouterd)
├── biorouter-mcp      # MCP extensions
└── biorouter-test     # test utilities

ui/desktop/           # Electron app
```

## Development Loop
```bash
# 1. source bin/activate-hermit
# 2. Make changes
# 3. cargo fmt
# 4. cargo build
# 5. cargo test -p <crate>
# 6. ./scripts/clippy-lint.sh
# 7. [if server] just generate-openapi
# 8. just check-everything      <- run before pushing; CI runs these too, though the
#                                  merge-blocking checks are the three `test (<os>)`
#                                  jobs, `Unit tests (vitest)` and `no-ai-coauthor`
```

`just check-everything` runs all ten checks, six of which nothing else in
this file mentions:

```bash
cargo fmt --all
./scripts/clippy-lint.sh
cd ui/desktop && npm run lint:check
./scripts/check-openapi-schema.sh
./scripts/check-version-consistency.sh    # CLI/daemon/GUI/README versions agree
./scripts/check-brand-consistency.sh      # productName "Biorouter" + brand assets
./scripts/check-computer-use-naming.sh    # the capability is "Computer Use" wherever a person reads it
./scripts/check-no-cross-drift.sh         # cross-compile recipes / glibc floor pin
just check-registry                       # the BAAM registry generator still refuses what it must
just check-privacy-registry               # the BAAM private set agrees in all three committed copies
```

## Rules

Test: Prefer tests/ folder, e.g. crates/biorouter/tests/
Test: When adding features, update biorouter-self-test.yaml, rebuild, then run `biorouter run --workflow biorouter-self-test.yaml` to validate
Error: Use anyhow::Result
Provider: Implement Provider trait see providers/base.rs
MCP: Extensions in crates/biorouter-mcp/
Server: Changes need just generate-openapi

## Code Quality

Comments: Write self-documenting code - prefer clear names over comments
Comments: Never add comments that restate what code does
Comments: Only comment for complex algorithms, non-obvious business logic, or "why" not "what"
Simplicity: Don't make things optional that don't need to be - the compiler will enforce
Simplicity: Booleans should default to false, not be optional
Errors: Don't add error context that doesn't add useful information (e.g., `.context("Failed to X")` when error already says it failed)
Simplicity: Avoid overly defensive code - trust Rust's type system
Logging: Clean up existing logs, don't add more unless for errors or security events

## Never

Never: Edit ui/desktop/openapi.json manually
Never: Edit Cargo.toml use cargo add
Never: Hand-edit a version file — use `scripts/release.sh bump <ver|major|minor|patch>`. Six files must move together (Cargo.toml, ui/desktop/package.json, package-lock.json x2, openapi.json, README badge) and `just check-versions` fails if one drifts
Never: Add a Co-Authored-By trailer naming an AI tool (Anthropic, Claude, OpenAI, ChatGPT, Gemini, Copilot). The required `no-ai-coauthor` check rejects the whole branch until the message is rewritten
Never: Skip cargo fmt
Never: Merge without ./scripts/clippy-lint.sh
Never: Comment self-evident operations (`// Initialize`, `// Return result`), getters/setters, constructors, or standard Rust idioms

## Entry Points
- CLI: crates/biorouter-cli/src/main.rs
- Server: crates/biorouter-server/src/main.rs
- UI: ui/desktop/src/main.ts
- Agent: crates/biorouter/src/agents/agent.rs
- Workspace ext: crates/biorouter/src/agents/workspace_extension.rs
- Subagents: crates/biorouter/src/agents/subagent_tool.rs
