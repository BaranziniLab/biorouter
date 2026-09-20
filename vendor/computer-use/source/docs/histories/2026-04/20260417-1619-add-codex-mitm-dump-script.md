## [2026-04-17 16:19] | Task: 新增 Codex mitm 抓包脚本

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 帮我写一个 `codex_dump.py`，用于通过 mitmproxy/mitmweb 抓 Codex 的上游 API 与 WebSocket 流量。

### 🛠 Changes Overview
**Scope:** `scripts`、`README.md`、`.gitignore`、`docs/references`、`docs/histories`

**Key Actions:**
- **[抓包脚本]**: 新增 `scripts/codex_dump.py`，支持持久化 Codex 相关 HTTP 与 WebSocket 流量。
- **[后台启动脚本]**: 新增 `scripts/start-codex-mitm-dump.sh`，自动创建 session 目录、后台拉起 mitmdump，并输出可直接 `source` 的代理环境。
- **[默认脱敏]**: 对 `Authorization`、Cookie 和常见 token 字段做脱敏，避免把登录凭证原样写盘。
- **[最小文档]**: 在 `README.md` 补充运行 mitmdump/mitmweb 抓 Codex 主链路的基本用法。
- **[样本忽略]**: 把 `artifacts/codex-dumps/` 加入 `.gitignore`，便于把分析样本留在仓库目录里长期查看。
- **[复用 runbook]**: 新增 `docs/references/codex-network-capture.md`，明确前台抓包、后台启动、session 目录约定和后续 eval 分析流程。

### 🧠 Design Intent (Why)
Codex 当前主模型调用走 `chatgpt.com/backend-api/codex/responses` WebSocket，而不是传统 REST body。仓库里需要一份可直接复用、默认脱敏、且不把真实抓包结果落进仓库的脚本，避免每次都靠聊天上下文临时拼 addon。

### 📁 Files Modified
- `.gitignore`
- `scripts/codex_dump.py`
- `scripts/start-codex-mitm-dump.sh`
- `README.md`
- `docs/references/README.md`
- `docs/references/codex-network-capture.md`
- `docs/histories/2026-04/20260417-1619-add-codex-mitm-dump-script.md`

## [2026-04-17 17:50] | Task: 改进 Codex dump script，补充本地 session 摘要

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 改进 `codex_dump.py`，让抓包目录里更容易直接看到正常请求、MCP tool 调用和 tool result。

### 🛠 Changes Overview
**Scope:** `scripts`、`README.md`、`docs/references`、`docs/histories`

**Key Actions:**
- **[本地摘要导出]**: 扩展 `scripts/codex_dump.py`，在保留 HTTP / WebSocket dump 的同时，把当前 websocket `session_id` 对应的 `~/.codex/sessions/rollout-*.jsonl` 导出成 `local-sessions/*.json`。
- **[结构化结果]**: `local-sessions/*.json` 只保留高信号字段，包括 `user_prompts`、`tool_calls`、`function_call_output` 解析结果和 `final_answer`，避免直接复制原始 session JSONL。
- **[精确匹配]**: 优先用 websocket 握手头里的 `session_id` 精确关联本地 session，避免把同时间窗口内的旧 session 一起扫进当前 dump 目录。
- **[启动健壮性]**: `scripts/start-codex-mitm-dump.sh` 现在会确认端口真的开始监听后才返回，并把 mitmdump 的 stdin 重定向到 `/dev/null`。
- **[文档顺序更新]**: 更新 runbook，明确新的默认排查顺序是 `websocket/` -> `local-sessions/` -> `logs_2.sqlite`。

### 🧠 Design Intent (Why)
用户真正要分析的通常不是“模型决定调用了哪个 tool”这一半，而是“宿主最终把什么 `function_call` 下发给本地 MCP、又收到了什么 `function_call_output`”。把这层摘要直接落进同一个 dump 目录，能显著降低来回切换 `mitm` 样本和 `~/.codex/sessions` 的成本，也更适合做长期 eval 留档。

### 📁 Files Modified
- `scripts/codex_dump.py`
- `scripts/start-codex-mitm-dump.sh`
- `README.md`
- `docs/references/README.md`
- `docs/references/codex-network-capture.md`
- `docs/references/codex-local-runtime-logs.md`
- `docs/histories/2026-04/20260417-1619-add-codex-mitm-dump-script.md`
