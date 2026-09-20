## [2026-04-19 22:03] | Task: 沉淀发版必读文档

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 这个发版本的落个独立的md到docs把，然后AGENTS.md加个发版本必读的指向。这样未来新发版本就都知道怎么做了

### 🛠 Changes Overview
**Scope:** `docs/`、`AGENTS.md`

**Key Actions:**
- **[Release Guide]**: 新增 `docs/releases/RELEASE_GUIDE.md`，把版本源、release checklist、tag 推送命令、GitHub Actions 排查方式，以及 tag 打错后的修复路径单独沉淀下来。
- **[Agent Routing]**: 在 `AGENTS.md` 里新增“发版本必读”导航，让版本 bump / 打 tag / 查 release 失败这类任务一开始就能命中正确文档。
- **[Docs Index Sync]**: 更新 `docs/releases/README.md`，把面向用户的 release note 入口和面向维护者的发版指南区分开。

### 🧠 Design Intent (Why)
这次 release 修复暴露出一个典型问题：版本源和 tag 约定如果只存在于聊天里，下一次发版时仍然很容易再踩一遍。把“必须先改哪几个版本文件、怎么验证 staging 包真的变成新版本、CI 失败先查什么”沉淀成独立文档，再在 `AGENTS.md` 里做最短路径导航，后续 Agent 和人都能按同一套流程执行，不需要继续靠记忆维持。

### 📁 Files Modified
- `AGENTS.md`
- `docs/releases/README.md`
- `docs/releases/RELEASE_GUIDE.md`
- `docs/histories/2026-04/20260419-2203-document-release-guide.md`
