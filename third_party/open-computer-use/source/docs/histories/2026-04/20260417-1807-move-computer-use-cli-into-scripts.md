## [2026-04-17 18:07] | Task: 移动 computer-use-cli 并补仓库文档

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 把 `computer-use-cli/` 移动到 `open-codex-computer-use` 仓库里的合适位置，并根据该仓库 `AGENTS.md` 的约定补充文档，告诉后续 AI 这个工具怎么使用。

### 🛠 Changes Overview
**Scope:** `scripts/computer-use-cli`, `README`, `docs/ARCHITECTURE.md`, `docs/references`, `docs/histories`

**Key Actions:**
- **[Relocate helper CLI]**: 把独立 Go 调试工具移动到 `scripts/computer-use-cli/`，让它和仓库级自动化脚本放在同一层。
- **[Document repo-level usage]**: 在仓库根 `README.md`、架构文档和 references 索引里补上入口，避免后续 Agent 只能靠聊天上下文知道这个工具存在。
- **[Add durable reference]**: 新增 `docs/references/codex-computer-use-cli.md`，说明官方 bundled `computer-use` 为什么不能依赖普通 stdio client 直连，以及应该如何使用 app-server 模式探测它。

### 🧠 Design Intent (Why)
这个 CLI 的职责是调试和探测，不是仓库主产物，放在 `scripts/` 下比塞进 `apps/` 或 `packages/` 更符合边界。与此同时，`AGENTS.md` 明确要求把仓库级知识沉淀到 `docs/`，所以用一份独立 reference 文档承接“为什么存在”和“AI 应该怎么用”，再从 `README` 和 `ARCHITECTURE` 做导航，比把细节堆进 `AGENTS.md` 更符合这个仓库的文档纪律。

### 📁 Files Modified
- `scripts/computer-use-cli/README.md`
- `scripts/computer-use-cli/app_server.go`
- `scripts/computer-use-cli/go.mod`
- `scripts/computer-use-cli/go.sum`
- `scripts/computer-use-cli/main.go`
- `scripts/computer-use-cli/main_test.go`
- `README.md`
- `docs/ARCHITECTURE.md`
- `docs/references/README.md`
- `docs/references/codex-computer-use-cli.md`
- `docs/histories/2026-04/20260417-1807-move-computer-use-cli-into-scripts.md`
