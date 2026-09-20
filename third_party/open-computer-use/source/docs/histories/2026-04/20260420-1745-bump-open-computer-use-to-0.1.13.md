## [2026-04-20 17:45] | Task: 发布 0.1.13

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 你可以自己bump一个version，所有版本号都+个小版本吧

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Version Bump]**: 把插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、单测里的 client version 与 CLI 文档路径统一提升到 `0.1.13`。
- **[Release Notes]**: 在 `docs/releases/feature-release-notes.md` 追加 `0.1.13`，记录 `Cursor Motion` 命名收口和 DMG GitHub Releases 流程。
- **[Release Guide Sync]**: 把 `docs/releases/RELEASE_GUIDE.md` 里的本地 DMG 构建、tag 推送和删 tag 示例统一切到 `0.1.13`，避免示例仍停在旧版本。
- **[Validation]**: 已重跑 `swift test` 与 npm staging 构建，并直接检查 `open-codex-computer-use-mcp/package.json`，确认 staging 版本已经从 `0.1.12` 变成 `0.1.13`。

### 🧠 Design Intent (Why)
这次不是额外功能开发，而是把仓库当前对外可见的版本源和示例统一推进一个 patch 版本，避免后续打 tag、看文档或跑 smoke / CLI 时继续混着 `0.1.12` 与新 release 内容。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260420-1745-bump-open-computer-use-to-0.1.13.md`
