## [2026-04-20 18:15] | Task: 发布 0.1.15

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 提交相关改动，然后加个版本号 git tag 推送触发一波看看。

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Version Bump]**: 把插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、单测中的 MCP client version 与 CLI 文档路径统一提升到 `0.1.15`。
- **[Release Notes]**: 在用户可见发布记录里追加 `0.1.15`，说明这次 release 的核心是统一 `Open Computer Use.app` 的跨渠道权限身份与签名链。
- **[Release Trigger]**: 基于上一条功能 commit 收口 release 输入，准备用 `v0.1.15` tag 推送触发 GitHub Actions 的 npm 包与 DMG 发布链路。

### 🧠 Design Intent (Why)
这次用户要验证的不是单纯本地修复，而是“签名身份统一”这件事能否真正进入发布链。把版本 bump、tag 和 CI trigger 单独收成一个 patch release，可以把 npm/GitHub Releases 的外部分发行为和本地验证结果对齐，避免功能修复已经在本地 commit 里，release 输入却还停在旧版本。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260420-1815-bump-open-computer-use-to-0.1.15.md`
