## [2026-04-17 21:30] | Task: 收敛 visual cursor 的窗口感知轨迹

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续深挖官方 `Codex Computer Use.app` 里的 cursor / overlay 实现，因为当前开源版效果还不够好；希望把能挖出来的行为迁回我们自己的实现里。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit`, `docs/references`, `docs/histories`

**Key Actions:**
- **[Window-aware path selection]**: 给 `SoftwareCursorOverlay` 增加多候选 Bezier 路径选择；当 snapshot 带有目标 `windowID` 时，会对候选路径的控制点和关键采样点做 window hit-test，优先选择仍然落在目标 window 上的路径。
- **[Conservative fallback]**: 增加严格直线的保守 fallback，避免所有曲线路径都偏离目标窗口时继续硬播夸张轨迹。
- **[Ordering resilience]**: overlay 现在会在排序前校验目标 window 是否仍存在，并在移动动画与 idle sway 期间持续检查目标 window 是否失效，失效后回退到普通前置排序。
- **[Docs sync]**: 更新逆向分析文档，补充官方实现里“绑定具体 target window id”和“轨迹窗口命中检查”的推断，并把这轮实现沉淀到 history。

### 🧠 Design Intent (Why)
这轮不是继续把 overlay 做得更花，而是补上官方实现里更关键的一层约束：cursor 轨迹和目标窗口之间的关系。仅仅“排到目标 window 上面”还不够，如果控制点和中间采样点明显飘出目标窗口，观感就会和官方差很多。把路径选择改成 window-aware 之后，能在不动输入注入链路的前提下，先把最明显的视觉偏差收回来。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/histories/2026-04/20260417-2130-window-aware-cursor-path-selection.md`
