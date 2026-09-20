## [2026-04-17 21:54] | Task: 发布 0.1.4

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 0.1.4 重新发

### 🛠 Changes Overview
**Scope:** `plugins/`、`packages/`、`apps/`、`scripts/`、`docs/`

**Key Actions:**
- **统一版本号**：把插件 manifest、MCP server version、smoke suite client version 和 `computer-use-cli` 版本统一 bump 到 `0.1.4`。
- **更新文档示例**：把 CLI 文档里引用插件缓存目录的版本路径从 `0.1.3` 更新到 `0.1.4`。
- **准备重新发布**：基于已经修好的 npm symlink launcher 模板重新生成并发布 `0.1.4` 包。

### 🧠 Design Intent (Why)
`0.1.3` 虽然已经把新 bundle 名称和图标带出去了，但 npm 全局 symlink 启动路径仍有 bug。发一个新的 patch release，能把 launcher 修复正式同步到 npm，而不是继续依赖本机热修。

### 📁 Files Modified
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `docs/references/codex-computer-use-cli.md`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/main.go`
