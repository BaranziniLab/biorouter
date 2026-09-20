## [2026-04-17 21:32] | Task: 补 README 里的 MCP JSON 配置示例

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> README 要加一个标准 MCP JSON 配置示例，明确告诉用户是不是 `npm install -g open-computer-use` 再加一段配置就能用了。

### 🛠 Changes Overview
**Scope:** `README.md`、`scripts/npm/build-packages.mjs`

**Key Actions:**
- **补仓库 README**：新增“全局安装 + `mcpServers` JSON + 首次授权”的标准使用路径。
- **补 npm 包 README 模板**：让后续通过 npm 发布的新版本也自动带上相同的 MCP 配置示例。
- **补可选环境变量示例**：给出关闭 visual cursor overlay 的 JSON 配置写法。
- **补命令注释**：在 `doctor`、`mcp`、`install-codex-plugin` 命令上方加一行说明，降低首次上手理解成本。

### 🧠 Design Intent (Why)
对于 MCP server 来说，用户最需要的是一段可以直接粘贴进 client 配置的 JSON，而不是先理解仓库结构或插件安装逻辑。把这条路径放到 README 前面，可以直接回答“装完以后怎么配”这个问题，也更符合实际使用习惯。

### 📁 Files Modified
- `README.md`
- `scripts/npm/build-packages.mjs`
