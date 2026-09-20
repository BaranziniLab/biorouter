## [2026-04-22 19:52] | Task: 增加 Gemini CLI MCP demo 视频到 README

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5 family (Codex)`
* **Runtime**: `Codex CLI on macOS`

### 📥 User Query
> Copy the provided `output.mp4` into the repo for demo use, rename it if needed, and add it to the README with a description showing Gemini CLI using our MCP.

### 🛠 Changes Overview
**Scope:** README 文档、history

**Key Actions:**
- **[Attachment Link]**: 更新 `README.md` 和 `README.zh-CN.md`，直接使用用户提供的 GitHub `user-attachments` 视频链接，让 README 引用外部托管的视频资源，而不是把视频文件收进仓库。
- **[Storage Cleanup]**: 清理这次任务里引入的本地视频 / GIF 资源提交，避免把大体积二进制文件继续留在分支历史里占用仓库存储。
- **[History]**: 继续维护同一份 history，把最终的 README 呈现方式和存储取舍记录清楚。

### 🧠 Design Intent (Why)
这次任务的重点是让 README 直接引用 GitHub 已托管的视频，同时不把 `.mp4` / GIF 二进制继续塞进仓库历史里。这样既保留了 README 顶部的演示入口，也避免为了展示视频而给仓库长期增加不必要的存储负担。

### 📁 Files Modified
- `README.md`
- `README.zh-CN.md`
- `docs/histories/2026-04/20260422-1952-add-gemini-cli-demo-video.md`
