## [2026-04-19 22:05] | Task: 发布 0.1.12

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 0.1.12的tag我在github上删了，你本地删一下，修复好后再重打

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Tag Cleanup]**: 删除本地 `v0.1.12` tag，避免在修正版本源之前继续沿用一份已经指向错误 npm 产物版本的 tag。
- **[Version Bump]**: 把插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、单测里的 client version 和文档示例统一提升到 `0.1.12`。
- **[Release Notes]**: 更新功能发布记录，把权限浮窗动效/回位修复和这次 release workflow 的版本收口一起记到 `0.1.12`。
- **[Publish Validation]**: 本地重跑 `swift test` 和 npm staging 构建，确认生成包的版本已经从 `0.1.11` 变成 `0.1.12`，不再触发 npm “不能覆盖已发布版本”的 403。

### 🧠 Design Intent (Why)
这次不是功能性新开发，而是修 release 工具链的版本一致性。tag 已经走到 `v0.1.12`，但 npm staging 产物仍然从插件 manifest 里读取 `0.1.11`，导致 CI 尝试重发旧版本直接失败。把“发布源版本”和所有对外暴露的版本字符串重新收口后，tag、运行时、smoke/test 和 npm 产物才会重新一致。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260419-2205-bump-open-computer-use-to-0.1.12.md`
