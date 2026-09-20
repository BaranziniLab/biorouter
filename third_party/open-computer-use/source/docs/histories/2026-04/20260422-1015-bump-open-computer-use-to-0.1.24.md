## [2026-04-22 10:15] | Task: 发布 0.1.24

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 提交 click 修正相关改动，bump version 并推送。

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Version Bump]**: 将插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、测试 MCP client version 与 CLI 文档路径统一提升到 `0.1.24`。
- **[Release Notes]**: 在用户可见发布记录中增加 `0.1.24`，说明本次 patch release 聚焦 `click` 的非侵入默认行为和全局物理指针 fallback opt-in。
- **[Release Trigger]**: 基于 click 全局指针 fallback 修正提交，准备用 `v0.1.24` tag 推送触发新的 GitHub Actions release。

### 🧠 Design Intent (Why)
`v0.1.23` 之后 main 已经包含 click 行为修正：AX 可处理的多次点击不会再直接落入全局鼠标路径，AX 失败后的物理指针 fallback 也默认关闭。发布前需要把 npm manifest、CLI 版本、测试输入和文档中的版本源一起提升，避免 tag 与实际 npm staging 包版本不一致。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260422-1015-bump-open-computer-use-to-0.1.24.md`
