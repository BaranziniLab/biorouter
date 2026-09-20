## [2026-04-22 20:36] | Task: Add Linux Computer Use Runtime

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local repo + Ubuntu GNOME VM over SSH`

### 📥 User Query
> Extend `open-computer-use` to Linux now that macOS and Windows exist. Confirm whether Linux has an AX-like background app automation surface, implement the same 9 tools, upload to the Ubuntu desktop VM, test every tool, and keep a plan/TODO for follow-up tracking.

### 🛠 Changes Overview
**Scope:** Linux runtime, build scripts, CI, docs

**Key Actions:**
- **[Runtime]**: Added `apps/OpenComputerUseLinux`, a Go CLI/MCP runtime exposing the same 9 Computer Use tools, `call --calls`, MCP initialize/tools list, and per-process snapshot cache.
- **[Bridge]**: Embedded a Python GI / AT-SPI2 bridge for app/window discovery, accessibility tree snapshots, semantic actions, editable text, value setting, and best-effort key/mouse fallback.
- **[Build/CI]**: Added `scripts/build-open-computer-use-linux.sh` for arm64/amd64 builds and wired Linux Go tests into `scripts/ci.sh`.
- **[Docs]**: Updated README, architecture, reliability, security, CI/CD, quality score, and added an active Linux runtime execution plan with remaining TODOs.
- **[Validation]**: Built the Linux arm64 binary, uploaded it to the Ubuntu GNOME VM, and verified `list_apps`, MCP `tools/list`, plus a Text Editor sequence covering `get_app_state`, `set_value`, `type_text`, `press_key`, `perform_secondary_action`, `click`, `scroll`, and `drag`.

### 🧠 Design Intent (Why)
Linux desktop automation should not inherit macOS `.app` / TCC assumptions or Windows UIA / Win32 assumptions. AT-SPI2 is the closest Linux equivalent to macOS AX for semantic UI automation, but Wayland does not provide a universal background keyboard/mouse/screenshot model. The first slice keeps the public tool protocol stable, makes the runtime buildable and testable as a standalone Linux binary, and documents coordinate input and screenshot paths as best-effort.

### 📁 Files Modified
- `apps/OpenComputerUseLinux/go.mod`
- `apps/OpenComputerUseLinux/main.go`
- `apps/OpenComputerUseLinux/main_test.go`
- `apps/OpenComputerUseLinux/runtime.py`
- `scripts/build-open-computer-use-linux.sh`
- `scripts/ci.sh`
- `README.md`
- `README.zh-CN.md`
- `docs/ARCHITECTURE.md`
- `docs/CICD.md`
- `docs/QUALITY_SCORE.md`
- `docs/RELIABILITY.md`
- `docs/SECURITY.md`
- `docs/exec-plans/active/20260422-linux-computer-use-runtime.md`
