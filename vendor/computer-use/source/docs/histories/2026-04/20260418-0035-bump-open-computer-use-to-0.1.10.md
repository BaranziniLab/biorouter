## [2026-04-18 00:35] | Task: 发布 0.1.10

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 加个小版本，提交改动，git tag推送

### 🛠 Changes Overview
**Scope:** `plugins/`, `packages/`, `apps/`, `scripts/`, `docs/`

**Key Actions:**
- **[Version Bump]**: 把插件 manifest、Swift/Go 侧版本常量、smoke suite 初始化版本和单测中的 client version 统一提升到 `0.1.10`。
- **[Release Notes]**: 在发布记录中补充 `0.1.10` 的用户价值，明确这次发版聚焦权限身份稳定性与 onboarding 生命周期收口。
- **[Release Prep]**: 为本轮权限与安装路径修复建立独立发版 history，便于后续提交、打 tag 和回溯。

### 🧠 Design Intent (Why)
这一轮用户可见变化已经跨过“本地修一修”的边界，涉及 bundle identity、权限持久化体验、npm 安装路径优先级和 onboarding 生命周期。单独发一个 patch version，可以把这些权限体验收口成一个明确的发布边界，避免后续 npm 包、tag 和历史记录继续错位。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260418-0035-bump-open-computer-use-to-0.1.10.md`
