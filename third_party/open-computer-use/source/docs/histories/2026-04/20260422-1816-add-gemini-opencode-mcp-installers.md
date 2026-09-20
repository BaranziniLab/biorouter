## [2026-04-22 18:16] | Task: 增加 Gemini 和 opencode 的 MCP 安装支持

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5 family (Codex)`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> add quick support for gemini and opencode, you can test it by run gemini and opencode from command line

### 🛠 Changes Overview
**Scope:** 安装脚本、npm launcher、README 文档

**Key Actions:**
- **[Installer Support]**: 新增 `scripts/install-gemini-mcp.sh` 和 `scripts/install-opencode-mcp.sh`，分别对接 Gemini CLI 和 opencode 的 MCP 配置格式。
- **[Shared Config Helper]**: 扩展 `scripts/install-config-helper.mjs`，支持 Gemini JSON 配置写入，以及 opencode `mcp.<name> = { type: "local", command: [...] }` 的幂等安装与旧别名清理。
- **[Packaging and Docs]**: 更新 `README.md`、`README.zh-CN.md` 和 npm launcher/build 脚本，让 npm 安装后的 `open-computer-use` 也能直接转发 `install-gemini-mcp` / `install-opencode-mcp`。
- **[Repo Hygiene]**: 将 `.gemini/` 加入 `.gitignore`，避免 Gemini 默认 project-scope 安装把本地配置噪音带进工作区。

### 🧠 Design Intent (Why)
这次改动延续仓库现有的“内置 install 子命令”模式，而不是要求用户手动查不同 host CLI 的配置格式。Gemini 默认是项目级 `.gemini/settings.json`，opencode 则会合并多个 JSON 配置文件，所以 helper 需要显式处理目标文件选择、幂等写入和旧别名清理，避免用户安装一次后留下重复配置或脏工作区。

### 📁 Files Modified
- `.gitignore`
- `README.md`
- `README.zh-CN.md`
- `scripts/install-config-helper.mjs`
- `scripts/install-gemini-mcp.sh`
- `scripts/install-opencode-mcp.sh`
- `scripts/npm/build-packages.mjs`
