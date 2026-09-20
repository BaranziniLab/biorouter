## [2026-04-21 21:03] | Task: 发布 0.1.23

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 提交相关改动，bump version 并推送。

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Version Bump]**: 将插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、测试 MCP client version 与 CLI 文档路径统一提升到 `0.1.23`。
- **[Release Notes]**: 在用户可见发布记录中增加 `0.1.23`，说明本次 patch release 聚焦原生 `open-computer-use call` 和 JSON 数组连续动作编排。
- **[Release Trigger]**: 基于 `v0.1.22` 之后 main 上的 CLI call 功能提交，准备用 `v0.1.23` tag 推送触发新的 GitHub Actions release。

### 🧠 Design Intent (Why)
`v0.1.22` 之后 main 已经包含原生 `open-computer-use call` 入口、共享 MCP/CLI dispatcher 和连续动作 JSON 编排能力。发布前需要把 npm manifest、CLI 版本、测试输入和文档中的版本源一起提升，避免 tag 与实际 npm staging 包版本不一致。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260421-2103-bump-open-computer-use-to-0.1.23.md`
