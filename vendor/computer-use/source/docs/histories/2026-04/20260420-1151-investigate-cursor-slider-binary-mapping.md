## [2026-04-20 11:51] | Task: 深挖 cursor slider 的 binary 映射

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 现在我们有了这些参数调节，现在我们看看之前逆向分析的，再去二进制里挖掘一下，这几个参数调节的，看看二进制里是否有涉及这个，以及他们调节后分别对实际的曲线有什么影响

### 🛠 Changes Overview
**Scope:** `scripts/cursor-motion-re/`、`docs/references/`、`docs/exec-plans/`

**Key Actions:**
- **[新增 slider-study 分析入口]**: 在 `scripts/cursor-motion-re/official_cursor_motion.py` / `reconstruct_cursor_motion.py` 中新增 `slider-study` 子命令，输出 shipping bundle phrase scan、binary-confirmed motion terms，以及 5 个 slider 的参数敏感性分析。
- **[补 shipping bundle 证据边界]**: 明确记录当前 release bundle 未命中 `START HANDLE`、`END HANDLE`、`ARC SIZE`、`ARC FLOW` 这些完整 phrase；`SPRING` / `DEBUG` / `MAIL` / `CLICK` 仅作为歧义 token 命中，不再被误写成“debug UI 仍然随 release shipping”。
- **[沉淀 slider 映射文档]**: 新增 `software-cursor-slider-parameter-investigation.md`，把 `start/end handle`、`arc size/flow`、`spring` 各自对应的 binary-confirmed 几何 / timing 量，以及默认样例 / 居中样例下的实际曲线影响单独沉淀。
- **[同步旧文档口径]**: `software-cursor-motion-model.md` 不再只写“视频里有 slider UI”，而是补充说明当前 shipping bundle phrase scan 没有直接命中这组 label。
- **[新增独立执行计划]**: 为这轮参数映射调查新增 active execution plan，避免继续挤在之前的 cursor reconstruction 或 CursorMotion UI 任务里。

### 🧠 Design Intent (Why)
这轮的关键不是继续猜 5 个 slider 在 UI 上长什么样，而是把“release binary 里还能直接确认什么”和“我们如何基于这些量推断 slider 对曲线的影响”明确分层。这样后续继续挖 `ARC FLOW` 或 spring remap 时，就不会把 shipping 证据、视频证据和本地 lab 调参混成一团。

### 📁 Files Modified
- `scripts/cursor-motion-re/official_cursor_motion.py`
- `scripts/cursor-motion-re/README.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-slider-parameter-investigation.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/exec-plans/active/20260420-cursor-slider-binary-investigation.md`
