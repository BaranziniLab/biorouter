## [2026-04-17 15:32] | Task: 安装 open-computer-use Codex 插件

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> 把我们的 open-codex-computer-use 安装到 codex app 的 plugin 里，plugin 名叫 open-computer-use。

### 🛠 Changes Overview
**Scope:** `plugins/open-computer-use`, `scripts/`, `README.md`, `.agents/plugins/`

**Key Actions:**
- **[Plugin Packaging]**: 在仓库内新增 repo-local Codex marketplace 和 `open-computer-use` 插件 manifest、MCP wrapper 与展示资源。
- **[Local Install Flow]**: 新增 `scripts/install-codex-plugin.sh`，用于构建 app、注册本仓库为本机 Codex marketplace、把插件缓存包安装到 `~/.codex/plugins/cache/...`，并启用插件。
- **[Docs Sync]**: 在 README 补充插件安装入口和行为说明，避免插件接入方式只存在聊天上下文里。

### 🧠 Design Intent (Why)
把插件定义版本化落在仓库里，比只手工改 `~/.codex/config.toml` 更可追溯，也更符合这个仓库“Agent-first、知识落盘”的约束。安装脚本顺手清理旧的直连 MCP 配置，是为了避免同一套 computer-use tools 被重复注册。

### 🔁 Follow-up Fix (2026-04-17 15:38)
- 补齐真实安装缺口：Codex Desktop 实际从 `~/.codex/plugins/cache/<marketplace>/<plugin>/<version>/` 加载插件，单纯写 `config.toml` 不会让插件出现在 UI 里。
- 更新 launcher，使其同时支持：
  - 从源码仓库里的 `dist/OpenCodexComputerUse.app` 直接运行
  - 从 Codex 插件缓存目录里的 `OpenCodexComputerUse.app` 运行
- README 同步改成当前真实行为，避免“重启后就会出现”的说明继续误导后续安装。

### 📁 Files Modified
- `.agents/plugins/marketplace.json`
- `plugins/open-computer-use/.codex-plugin/plugin.json`
- `plugins/open-computer-use/.mcp.json`
- `plugins/open-computer-use/assets/open-computer-use.svg`
- `plugins/open-computer-use/assets/open-computer-use-small.svg`
- `plugins/open-computer-use/scripts/launch-open-computer-use.sh`
- `scripts/install-codex-plugin.sh`
- `README.md`
- `docs/histories/2026-04/20260417-1532-install-open-computer-use-plugin.md`
