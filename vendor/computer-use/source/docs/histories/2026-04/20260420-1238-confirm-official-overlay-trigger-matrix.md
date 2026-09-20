## [2026-04-20 12:38] | Task: 确认官方 overlay cursor 的 tool 触发矩阵

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI + official computer-use app-server probe`

### 📥 User Query
> 通过 `scripts/computer-use-cli/` 的 `app-server` 连到官方的 `computer-use`，结合 bundled app 二进制分析，确认原来在调用什么 tool 的时候会出现 overlay cursor；并进一步确认 `type_text / press_key` 会不会触发。

### 🛠 Changes Overview
**Scope:** `docs/references`, `docs/histories`

**Key Actions:**
- **[补做官方路径实测]**: 用隔离 `HOME` 强制只启用 bundled `computer-use`，避免把本地 `open-computer-use` 和官方实现混淆。
- **[沉淀 tool 触发矩阵]**: 把 `set_value / scroll / drag / perform_secondary_action / click / type_text / press_key` 的 overlay 触发结论补进 reverse-engineering 文档。
- **[记录 click 分叉]**: 明确 element-scoped `click` 和 coordinate `click` 不一定走同一条执行链。

### 🧠 Design Intent (Why)
这类结论如果只留在临时日志和聊天上下文里，下次很容易重复踩坑，尤其当前机器默认配置里官方插件是关的，本地开源实现是开的。把“必须先切到官方路径”和“哪些 tool 真会拉起 `Software Cursor`”一起落文档，能直接减少后续逆向分析误判。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/histories/2026-04/20260420-1238-confirm-official-overlay-trigger-matrix.md`
