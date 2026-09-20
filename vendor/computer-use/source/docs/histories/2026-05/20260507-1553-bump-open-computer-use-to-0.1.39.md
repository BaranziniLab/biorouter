## [2026-05-07 15:53] | Task: 发布 0.1.39

### 🤖 Execution Context
* **Agent ID**: `primary`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI + SwiftPM`

### 📥 User Query
> 提交推送发版。

### 🛠 Changes Overview
**Scope:** `release`, `plugins/open-computer-use`, `apps`, `packages`, `scripts`, `docs`

**Key Actions:**
- **[Version bump]**: 将 Open Computer Use 版本源从 `0.1.38` bump 到 `0.1.39`。
- **[Release notes]**: 补充 `0.1.39` 用户可见发布记录，说明 macOS app denylist 收缩到密码管理器。
- **[Release prep]**: 准备 `v0.1.39` tag 对应的 npm / GitHub Release 验证材料。

### 🧠 Design Intent (Why)
本次发版把 app 安全阻止策略从宽泛的硬编码高风险列表收缩到密码管理器，减少 Chrome、终端和系统组件这类常规自动化目标被误拦的情况。版本源、发布记录和 history 需要与 tag 一起保持一致，避免 npm 产物和 GitHub Release 版本漂移。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `apps/OpenComputerUseLinux/main.go`
- `apps/OpenComputerUseWindows/main.go`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/releases/feature-release-notes.md`
