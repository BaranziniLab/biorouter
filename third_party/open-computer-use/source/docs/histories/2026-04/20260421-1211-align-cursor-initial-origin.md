## [2026-04-21 12:11] | Task: 对齐 cursor 首次起点到官方 `(0,0)`

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local CLI`

### 📥 User Query
> 基于官方 `.app` 逆向发现 fresh cursor 从屏幕左下角 `(0,0)` 出现后，希望本仓库实现也从 `(0,0)` 开始。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit`、`docs/`

**Key Actions:**
- **[Initial Origin Alignment]**: `SoftwareCursorOverlay` 在没有上一帧 cursor tip 时，不再从目标点背后生成起点，而是按官方 fresh state 用 AppKit 全局 `(0,0)` window origin 计算首次 tip。
- **[Regression Coverage]**: 更新单测，验证默认首次 tip 对应的 cursor window origin 为 `(0,0)`。
- **[Docs Sync]**: 更新架构说明，明确首次显示从 `(0,0)` window origin 起步，后续动作继续复用上一帧 visible tip。

### 🧠 Design Intent (Why)
官方 `SkyComputerUseService` 的 `ComputerUseCursor.Window` 初始化会把 `currentInterpolatedOrigin` 和 `NSWindow` 初始 content rect 都置为 `(0,0)`。主运行时应该复用这个 fresh-session 语义，避免第一段移动从目标附近切入而偏离官方观感。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260421-1211-align-cursor-initial-origin.md`

### 🔁 Follow-up (2026-04-21 12:23)
- **[Runtime Heading Fix]**: 保留 `CursorMotion` / glyph resource 的 `-3π/4` neutral heading；主 runtime overlay 仍按 AppKit 全局坐标放置 window，路径选路用实际可见的 AppKit forward heading，但进入 visual dynamics / render state 前会把 velocity 的 y 轴翻回 CursorMotion 的 y-down screen state，避免上行时 cursor 侧边朝前。
- **[Regression Coverage]**: 补单测锁住 AppKit 上行速度到 CursorMotion screen-state velocity、render rotation、最终 AppKit forward heading 的转换关系。
- **[Docs Sync]**: 更新架构说明，明确 artwork calibration 与 runtime motion coordinate basis 分层。
