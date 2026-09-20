## [2026-04-17 23:58] | Task: 简化 README

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 简化 README，只保留介绍、quick start、更多子命令和协议；介绍里提到 codex-computer-use 与 OpenAI 文章，quick start 写 npm 安装、授权、`open-computer-use doctor` 和 MCP JSON 配置。

### 🛠 Changes Overview
**Scope:** `README`、`docs/histories`

**Key Actions:**
- **重写 README 结构**: 删除冗长的源码运行、抓包和实现细节，只保留四段式入口文档。
- **保留关键上手路径**: 明确 `npm i -g open-computer-use`、`open-computer-use doctor`、权限授权和 MCP JSON 配置。
- **补充常用命令说明**: 简要列出 `install-claude-mcp`、`install-codex-mcp`、`install-codex-plugin` 等命令用途。

### 🧠 Design Intent (Why)
README 现在更像安装入口而不是项目手册。把首次使用路径压缩到最短，可以降低用户理解成本；更细节的实现和仓库协作信息继续留在 `docs/`。

### 📁 Files Modified
- `README.md`
- `docs/histories/2026-04/20260417-2358-simplify-readme.md`
