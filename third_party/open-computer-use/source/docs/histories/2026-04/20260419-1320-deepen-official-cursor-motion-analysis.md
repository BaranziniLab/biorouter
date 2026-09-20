## [2026-04-19 13:20] | Task: 深挖官方 cursor motion 静态模型

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续分析官方 `Codex Computer Use.app`，重点看 `calculates natural and aesthetic motion paths` 相关算法，尽量从 app 里找到曲线和速度怎么计算。

### 🛠 Changes Overview
**Scope:** `docs/references/codex-computer-use-reverse-engineering/`、`docs/histories/`

**Key Actions:**
- **[补强静态分析证据]**: 对 `SkyComputerUseService` 的 `__swift5_types` / `__swift5_fieldmd` 做静态恢复，不再只依赖 `strings` 关键词。
- **[确认 cursor path 类型]**: 文档新增 `ComputerUseCursor`、`Window`、`Style`、`CloseEnoughConfiguration`、`CursorNextInteractionTiming`、`CursorMotionPathMeasurement`、`Segment`、`CursorMotionPath` 的字段级证据。
- **[确认 timing / spring 类型]**: 文档新增 `BezierAnimation`、`SpringAnimation`、`BezierFunction`、`BezierParameters`、`SpringParameters`、`VelocityVerletSimulation`、`Configuration`、`AnimationDescriptor` 的字段级证据。
- **[修正旧推断]**: 把此前把 `ARC SIZE` 直接映射到 `arcHeight` 的说法降级，改为更保守地映射到 cursor path 的 `arc` / 控制点偏移。
- **[明确 next-interaction gate]**: 基于 `CloseEnoughConfiguration(progressThreshold, distanceThreshold)` 和 `CursorNextInteractionTiming(closeEnough, finished)`，补充“动画未完全结束即可进入下一交互”的 timing 机制判断。

### 🧠 Design Intent (Why)
之前 motion model 文档主要停留在“看到哪些字符串，所以推测有哪些层”的阶段。这次把 Swift 元数据也解析出来，是为了把曲线与速度模型从概念推断推进到字段级证据，减少后续开源实现时把官方结构猜错的概率。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
- `docs/histories/2026-04/20260419-1320-deepen-official-cursor-motion-analysis.md`
