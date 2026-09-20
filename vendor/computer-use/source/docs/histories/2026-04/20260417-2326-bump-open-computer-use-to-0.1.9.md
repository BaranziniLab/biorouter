## [2026-04-17 23:26] | Task: 发布 0.1.9

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 基于0.1.8再推一个

### 🛠 Changes Overview
**Scope:** 插件 manifest、版本常量、smoke/test、发布文档

**Key Actions:**
- **统一版本号到 `0.1.9`**：同步更新插件 manifest、Swift/Go 侧版本常量、smoke suite 初始化版本和单元测试中的 client version。
- **补 release 文档**：更新 release workflow 的 tag 示例，并在 feature release notes 中记录 `0.1.9` 这次“修复发布构建失败”的发布目的。
- **衔接前一轮修复**：基于刚修复的 Xcode 26 编译问题推进新版本，而不是复用已经失败过的 `v0.1.8` tag。

### 🧠 Design Intent (Why)
`v0.1.7` 和 `v0.1.8` 对应的 release runs 都已经失败，继续复用旧 tag 既不干净，也容易混淆真正包含修复的发布边界。直接发布 `0.1.9` 可以把“修复 CI 构建错误”作为一个明确的新版本切出去，后续排查 npm 和 GitHub release 记录也更清楚。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseVersion.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `README.md`
- `docs/releases/feature-release-notes.md`
- `docs/histories/2026-04/20260417-2326-bump-open-computer-use-to-0.1.9.md`
