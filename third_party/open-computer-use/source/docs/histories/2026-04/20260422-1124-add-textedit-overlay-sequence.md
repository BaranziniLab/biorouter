## [2026-04-22 11:24] | Task: 新增 TextEdit overlay cursor 测试序列

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5.4`
* **Runtime**: `Codex CLI on macOS + SwiftPM`

### User Query
> `examples/textedit-overlay-seq.json` 这个帮我编排一下多个 tool 调用，用来测试最新的 cursor 效果。

### Changes Overview
**Scope:** 手工测试样例

**Key Actions:**
- **[Root examples]**: 新增根目录 `examples/textedit-overlay-seq.json`，补齐文档里引用的样例路径。
- **[Cursor-focused sequence]**: 编排 `get_app_state -> set_value -> click -> set_value -> click -> set_value`，中间穿插状态刷新，便于观察连续 `click` / `set_value` 之间 cursor 是否保留 idle 位置。
- **[Visible target spread]**: 将 click 目标改成 TextEdit rich text toolbar 的对齐按钮，避免窗口中心 click 和正文中心几乎重合导致后续位移不可见。
- **[Observation text]**: 在写入 TextEdit 的文本里明确标记 A/B/C 三段，便于手工观察每次 overlay movement 的起点和终点。

### Files Modified
- `examples/textedit-overlay-seq.json`
- `docs/histories/2026-04/20260422-1124-add-textedit-overlay-sequence.md`
