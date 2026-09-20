## [2026-04-17 20:43] | Task: 升级 open-computer-use 到 0.1.2 并刷新 Codex 插件安装

### 🤖 Execution Context
* **Agent ID**: `primary`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI + SwiftPM`

### 📥 User Query
> 改到 `0.1.2` 然后安装到 Codex plugin。

### 🛠 Changes Overview
**Scope:** `plugins/open-computer-use`, `packages/OpenComputerUseKit`, `apps/OpenComputerUseSmokeSuite`, `scripts`, `docs`

**Key Actions:**
- **[Version bump]**: 将插件 manifest、MCP server 自报版本、smoke client 版本、CLI 版本与 app bundle 版本统一提升到 `0.1.2`。
- **[Bundle metadata sync]**: 将打包脚本中的 `CFBundleShortVersionString` 提升到 `0.1.2`，并把 `CFBundleVersion` 递增到 `3`，避免本机缓存里的 app 元信息落后于源码版本。
- **[Docs sync]**: 同步更新 `computer-use-cli` 相关文档里的本地插件缓存路径示例，避免继续引用旧的 `0.1.1` 目录。
- **[Codex install refresh]**: 执行 `./scripts/install-codex-plugin.sh --rebuild`，把本地插件缓存刷新到 `~/.codex/plugins/cache/open-computer-use-local/open-computer-use/0.1.2`，并确认 `~/.codex/config.toml` 仍启用 `open-computer-use@open-computer-use-local`。
- **[Verification]**: 运行 `swift test` 通过，并校验缓存中的 `.codex-plugin/plugin.json` 已显示 `version = 0.1.2`。

### 🧠 Design Intent (Why)
这次改动的目标仍然是保持“源码版本、打包产物版本、插件缓存版本、MCP 握手版本”四者一致，避免 Codex 实际加载的插件缓存和当前仓库源码不一致。把安装动作放在同一轮里完成，能确保后续通过 Codex 调用到的就是这次刚升级后的 `0.1.2`。

### 📁 Files Modified
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `scripts/build-open-computer-use-app.sh`
- `scripts/computer-use-cli/main.go`
- `apps/OpenComputerUseSmokeSuite/Sources/OpenComputerUseSmokeSuite/main.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/references/codex-computer-use-cli.md`
- `scripts/computer-use-cli/README.md`
- `docs/histories/2026-04/20260417-2043-bump-open-computer-use-to-0.1.2.md`
