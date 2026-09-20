## [2026-04-21 16:24] | Task: 记录官方 computer-use sender auth 变化

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 调查 `computer-use-cli` 通过 `app-server` 调用官方 bundled `computer-use` 返回 `Sender process is not authenticated` 的原因，并判断是否仍能直接调用。

### 🛠 Changes Overview
**Scope:** `docs/`, `scripts/computer-use-cli/`

**Key Actions:**
- **[Runtime Finding]**: 记录官方 `computer-use` `1.0.755` 中 raw app-server helper 只能稳定列 tools，实际 tool call 会被 service-side sender authorization 拒绝。
- **[Docs Sync]**: 更新 `computer-use-cli` README、参考文档和架构边界，避免后续继续把 raw `mcpServer/tool/call` 当作官方 Computer Use 的通用直连入口。
- **[CLI Test Target]**: 将 `computer-use-cli` 的 bundled plugin 自动发现调整为本地兼容性测试优先解析 `1.0.750`，并新增 `COMPUTER_USE_PLUGIN_VERSION` / `--plugin-version` 用于显式切换测试版本；app-server 模式会把解析出的测试版本作为临时 `mcp_servers."computer-use"` 覆盖传给 Codex host，`--plugin-version host` 可回到 host 自身配置。
- **[Verification]**: `resolve-server` 默认解析到非 translocated 的 `~/.codex/plugins/computer-use` 旧安装根；`list-tools --transport app-server` 可列出 9 个工具且呈现旧版 schema；`call list_apps --transport app-server` 在当前工作区可返回应用列表。cache 里的 `1.0.750` 带 `com.apple.quarantine` 时会被 LaunchServices AppTranslocation，并返回 `Apple event error -1708: Unknown error`。
- **[Bug Fix]**: 修正 app-server 临时 MCP 覆盖的 `-c` key 写法。Codex CLI override 不解析 quoted dotted key，`mcp_servers."computer-use".command` 会落到错误 key；改为 `mcp_servers.computer-use.command` 后覆盖才真正生效。
- **[Bug Fix]**: 默认旧版测试目标优先选 `~/.codex/plugins/computer-use`，并用 manifest version 校验它确实是 `1.0.750`，避免 cache 目录 quarantine 导致 AppTranslocation。

### 🧠 Design Intent (Why)
官方 `SkyComputerUseClient` 的 parent launch constraint 只解释了“谁能启动 client”，不能解释已经通过 Apple Events/TCC 后仍被拒绝的工具调用。文档需要把 parent constraint、Apple Events/TCC、service-side sender authorization / active IPC client 追踪分开写，后续对比官方和开源实现时才不会混淆根因。

### 📁 Files Modified
- `docs/ARCHITECTURE.md`
- `docs/exec-plans/active/20260417-official-tool-alignment.md`
- `docs/references/codex-computer-use-cli.md`
- `docs/references/codex-computer-use-reverse-engineering/runtime-and-host-dependencies.md`
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/app_server.go`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/main_test.go`
