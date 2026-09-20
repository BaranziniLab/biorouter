## [2026-04-21 20:58] | Task: Add native call command

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### User Query
> 统一用 `open-computer-use call` 调用 9 个 tool，并支持 JSON 数组编排连续动作。

### Changes Overview
**Scope:** Swift CLI, MCP tool dispatch, docs, npm package help

**Key Actions:**
- **Shared dispatcher**: Added a reusable `ComputerUseToolDispatcher` so MCP `tools/call` and CLI `call` share the same 9-tool argument mapping.
- **Native CLI call**: Added `open-computer-use call <tool> --args ...` plus `open-computer-use call --calls ...` / `--calls-file` for same-process sequences that preserve app snapshot state.
- **Docs and package help**: Documented the new command in README, architecture notes, and npm launcher help.

### Design Intent (Why)
The JSON-array form keeps a single `ComputerUseService` instance alive across the sequence, so actions after `get_app_state` can reuse the captured `element_index` map instead of spawning fresh processes with empty state.

### Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseToolDispatcher.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseCLI.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `README.md`
- `README.zh-CN.md`
- `scripts/npm/build-packages.mjs`
