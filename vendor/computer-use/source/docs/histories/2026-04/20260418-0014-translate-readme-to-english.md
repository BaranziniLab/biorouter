## [2026-04-18 00:14] | Task: 把 README 改成英文版

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 现在的 README.md 就放英文版的。

### 🛠 Changes Overview
**Scope:** `README`、`docs/histories`

**Key Actions:**
- **英文重写 README**: 把现有中文 README 改成英文版，保留当前的产品介绍、Quick Start、更多命令和 License 结构。
- **保留权限说明**: 延续 npm 全局安装路径应作为稳定授权对象的说明，避免英文版丢掉关键使用约束。

### 🧠 Design Intent (Why)
用户要求把当前主 README 直接作为英文入口文档，因此这次不新增双语文件，而是直接替换为英文内容，同时保持安装路径和权限说明不变。

### 📁 Files Modified
- `README.md`
- `docs/histories/2026-04/20260418-0014-translate-readme-to-english.md`
