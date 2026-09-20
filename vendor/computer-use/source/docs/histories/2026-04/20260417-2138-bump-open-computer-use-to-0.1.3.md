## [2026-04-17 21:38] | Task: 发布 0.1.3

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 可以，加个版本发

### 🛠 Changes Overview
**Scope:** `plugins/`、`packages/`、`apps/`、`scripts/`、`docs/`

**Key Actions:**
- **统一版本号**：把插件 manifest、MCP server version、smoke suite client version 和 `computer-use-cli` 版本统一 bump 到 `0.1.3`。
- **更新文档示例**：把 CLI 文档里引用插件缓存目录的版本路径从 `0.1.2` 更新到 `0.1.3`。
- **准备发布**：让 npm 分发链路基于新的插件版本号生成 `0.1.3` 包并发布。

### 🧠 Design Intent (Why)
上一轮 README 改动需要同步到 npm 页面，最直接的方式就是做一次 patch release。把版本来源保持单点一致，可以避免 npm 包、MCP server 自报版本和插件缓存路径样例之间再次漂移。

### 📁 Files Modified
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `docs/references/codex-computer-use-cli.md`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/main.go`
