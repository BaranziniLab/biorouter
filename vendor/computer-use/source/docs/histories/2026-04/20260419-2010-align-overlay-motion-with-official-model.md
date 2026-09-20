## [2026-04-19 20:10] | Task: 主线 overlay 对齐官方 cursor motion

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 基于已有的逆向分析，调整当前主实现，让 visual cursor 的路径和速度表现尽量接近官方效果。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit/`、`docs/exec-plans/`、`docs/ARCHITECTURE.md`、`docs/histories/`

**Key Actions:**
- **[新增主线 motion 内核]**: 在 `OpenComputerUseKit` 中新增独立 `CursorMotionModel.swift`，把已确认的 `CursorMotionPath`、`CursorMotionPathMeasurement`、`20` 条官方候选、score 选择和 `VelocityVerlet` spring progress 正式接入主线 package。
- **[替换 overlay move 实现]**: 把 `SoftwareCursorOverlay` 的移动逻辑从旧的单段 cubic + `easeInOut` 切到官方候选池 + spring progress；同时保留现有 target-window 命中策略，但仅作为官方候选集合上的 tie-break。
- **[重构 visual dynamics 层]**: 删除补丁式 `terminal settle`，把主线 overlay 改成“路径层给目标点、visual dynamics 持续推进 visible tip / angle / fog”的双层模型，并让 move / pulse / idle 共用同一套状态。
- **[补齐验证与文档]**: 新增主线单元测试覆盖候选总数、参考样例 best candidate、`closeEnoughTime`，以及 visual dynamics 的“目标停止后 visible tip 继续过冲、角度短暂保留惯性后回稳”行为，并同步更新架构文档与 execution plan 状态。
- **[修正 heading/offset 分层]**: 后续又根据二进制里 `SoftwareCursorStyle.angle` 和 `CursorView._animatedAngleOffsetDegrees` 的分层证据，修正了主线和 standalone lab 的姿态模型，不再把主 heading 跟随错误压成只剩小幅 wiggle。

### 🧠 Design Intent (Why)
这次不是继续做实验 demo，而是把已经 binary-confirmed 的几何与 spring 形状真正落到主 runtime 里，缩小和官方视觉行为的偏差。后续又发现主线差异已经不再主要来自路径候选，而是来自缺少独立的姿态/渲染状态层，所以实现从“末端特判补丁”进一步升级成“路径目标 + visual dynamics”双层模型。另一方面，官方 transaction-level 的真实时长映射还没完全恢复，所以最终 wall-clock duration 继续保留本地校准，避免把动画直接拉慢到失真。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/CursorMotionModel.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/exec-plans/completed/20260419-overlay-official-cursor-motion-alignment.md`
- `docs/exec-plans/completed/20260419-visual-cursor-pose-dynamics-refactor.md`
- `docs/histories/2026-04/20260419-2010-align-overlay-motion-with-official-model.md`
