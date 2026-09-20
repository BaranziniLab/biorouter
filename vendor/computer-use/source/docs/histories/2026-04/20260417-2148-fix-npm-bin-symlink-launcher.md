## [2026-04-17 21:48] | Task: 修复 npm 全局 symlink 启动路径

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> `open-computer-use` 在 Codex 里作为 MCP 启动失败，握手阶段连接提前关闭，想知道为什么。

### 🛠 Changes Overview
**Scope:** `scripts/npm/build-packages.mjs`

**Key Actions:**
- **定位根因**：确认全局安装后的 `/opt/homebrew/bin/open-computer-use` 是 symlink，而 launcher 用 `BASH_SOURCE[0]` 直接算包根目录，导致把包根误判成 `/opt/homebrew`。
- **修复 launcher 模板**：让 npm 包生成的 `bin/open-computer-use` 先解析 symlink，再推导 `package_root`，兼容 Homebrew 风格的全局 npm bin 路径。
- **本机临时修复**：同步修补当前已安装的全局 launcher，并把 `~/.codex/config.toml` 改成真实脚本路径，立即恢复当前环境可用性。

### 🧠 Design Intent (Why)
全局 npm 安装最常见的入口就是系统 bin 目录里的 symlink。如果 launcher 不能正确解析 symlink，发布到 npm 的包在默认安装路径上就会直接失效。这个问题比单纯文档问题更严重，必须在生成模板上修掉，而不是只靠配置绕过去。

### 📁 Files Modified
- `scripts/npm/build-packages.mjs`
