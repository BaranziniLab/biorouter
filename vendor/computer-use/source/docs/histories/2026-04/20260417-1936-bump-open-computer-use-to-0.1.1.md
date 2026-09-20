## [2026-04-17 19:36] | Task: 升级 open-computer-use 到 0.1.1 并刷新 Codex 插件安装

### 🤖 Execution Context
* **Agent ID**: `primary`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI + SwiftPM`

### 📥 User Query
> 升级版本到 `0.1.1`，然后把插件更新到 Codex。

### 🛠 Changes Overview
**Scope:** `plugins/open-computer-use`, `packages/OpenComputerUseKit`, `apps/OpenComputerUseSmokeSuite`, `scripts`, `docs`

**Key Actions:**
- **[Version bump]**: 将插件 manifest、MCP server 自报版本、smoke client 版本、CLI 版本与 app bundle 版本统一提升到 `0.1.1`。
- **[Docs sync]**: 同步修正文档中的示例插件缓存路径，避免继续引用旧的 `0.1.0` 目录。
- **[Codex install refresh]**: 执行 `./scripts/install-codex-plugin.sh --rebuild`，把本地插件缓存刷新到 `~/.codex/plugins/cache/open-computer-use-local/open-computer-use/0.1.1`，并更新 `~/.codex/config.toml`。
- **[Verification]**: 运行 `swift test` 通过，并校验缓存中的 `plugin.json` 已显示 `version = 0.1.1`。

### 🧠 Design Intent (Why)
这次改动的重点是把仓库内对外暴露的版本标识统一到同一个语义版本上，避免插件 manifest、Codex 缓存目录、MCP 握手版本和 CLI 文档之间出现不一致。版本号统一后，再通过安装脚本刷新本机 Codex 插件，才能确保后续实际调用和本地源码处于同一版本面。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `scripts/build-open-computer-use-app.sh`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/README.md`
- `docs/references/codex-computer-use-cli.md`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/histories/2026-04/20260417-1936-bump-open-computer-use-to-0.1.1.md`
