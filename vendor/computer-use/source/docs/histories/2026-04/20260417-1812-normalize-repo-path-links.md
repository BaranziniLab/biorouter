## [2026-04-17 18:12] | Task: 清理仓库内绝对路径链接

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 把仓库里写成仓库根绝对路径的目录引用清理掉，改成相对路径，并在 `AGENTS.md` 对应的文档里强调后续只能写相对路径。

### 🛠 Changes Overview
**Scope:** `README.md`、`docs/REPO_COLLAB_GUIDE.md`、`docs/histories`

**Key Actions:**
- **[清理链接]**: 把 `README.md` 中指向仓库内 reference 文档的两处绝对路径链接改成相对路径。
- **[补充约束]**: 在 `docs/REPO_COLLAB_GUIDE.md` 的文档纪律里新增规则，明确仓库内路径引用不得使用机器相关的绝对路径。
- **[留存变更]**: 新增本次 history，记录路径规范清理的目的和范围。

### 🧠 Design Intent (Why)
仓库内文档如果写死本机绝对路径，会让链接不可移植，也会把环境偶然性带进仓库知识。把 repo-local 引用统一成相对路径，才能保证换机器、换用户目录、换协作者后仍然稳定可用。

### 📁 Files Modified
- `README.md`
- `docs/REPO_COLLAB_GUIDE.md`
- `docs/histories/2026-04/20260417-1812-normalize-repo-path-links.md`
