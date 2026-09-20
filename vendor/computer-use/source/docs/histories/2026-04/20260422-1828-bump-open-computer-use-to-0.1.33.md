## [2026-04-22 18:28] | Task: 发布 0.1.33

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local macOS shell`

### User Query
> bump a version and submit a pr

### Changes Overview
**Scope:** release version sources, feature release notes, task history

**Key Actions:**
- **[Version Bump]**: 将 Open Computer Use 的主版本源和相关测试/文档示例统一从 `0.1.32` bump 到 `0.1.33`。
- **[Release Notes]**: 在 `docs/releases/feature-release-notes.md` 记录本次 patch release 聚焦 Gemini CLI 和 opencode 的 MCP 安装支持。
- **[PR Prep]**: 基于本地新增的 Gemini/opencode 安装器提交，收口新的 release 版本线，供后续分支推送和 PR 使用。

### Design Intent
这轮版本 bump 的目标不是发布一个抽象的“文档修正”，而是把刚加入的 Gemini / opencode host 集成正式纳入对外版本线。既然当前 `HEAD` 相比远端 `v0.1.32` 已经多了用户可感知的新安装命令，就应该顺延到 `0.1.33`，保持功能提交、版本源和用户可见 release notes 一致。

### Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260422-1828-bump-open-computer-use-to-0.1.33.md`
