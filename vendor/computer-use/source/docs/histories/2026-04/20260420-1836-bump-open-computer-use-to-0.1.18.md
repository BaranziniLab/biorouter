## [2026-04-20 18:36] | Task: 发布 0.1.18

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 可以，bump 小版本 tag 推，然后看看结果

### 🛠 Changes Overview
**Scope:** `apps/`、`docs/`、`packages/`、`plugins/`、`scripts/`

**Key Actions:**
- **[Version Bump]**: 将插件 manifest、Swift/Go 版本常量、smoke suite 初始化版本、测试 MCP client version 与 CLI 文档路径统一提升到 `0.1.18`。
- **[Release Notes Correction]**: 为 `0.1.17` 增加准确说明，记录其 `package-npm` 已成功但 `Cursor Motion` notarization 因缺少 hardened runtime 失败；新增 `0.1.18` 作为真正补齐 hardened runtime 后的 patch release。
- **[Release Trigger]**: 基于 hardened runtime 修复后的 `HEAD` 收口新版本，准备用 `v0.1.18` tag 推送触发新的 GitHub Actions release。

### 🧠 Design Intent (Why)
`0.1.17` 已经把 npm 包成功发布出去，因此不再适合继续复用同版本重试所有 release 步骤。最稳妥的方式是把 notarization 真正需要的 hardened runtime 修复打进新的 patch release，让 `0.1.18` 成为首个同时具备 Developer ID 签名与可 notarize `Cursor Motion` 资产的版本。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260420-1836-bump-open-computer-use-to-0.1.18.md`
