## [2026-04-19 13:33] | Task: 新增独立 cursor motion 逆向 demo

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续深挖官方 cursor motion，并在独立目录里做一个不和 `CursorMotion` 冲突的 demo，最好能指定起终点输出路线和速度相关采样。

### 🛠 Changes Overview
**Scope:** `scripts/cursor-motion-re/`、`docs/exec-plans/active/`、`docs/references/`、`docs/histories/`

**Key Actions:**
- **[新增独立执行计划]**: 在 `docs/exec-plans/active/` 下为这条逆向 + 脚本化 demo 线路单独建 plan，并显式声明不触碰 `CursorMotion`。
- **[实现脚本化逆向工具]**: 新增 `scripts/cursor-motion-re/`，用纯 Python 实现最小 Mach-O section 解析、Swift field metadata 恢复和官方 binary 中的常量 / 候选系数表提取。
- **[实现 binary-guided demo]**: 新增 CLI，支持 `inspect` 和 `demo` 两个子命令；`demo` 可对指定起终点生成候选路径、measurement 和采样点 JSON，并默认输出精简版 `candidate_summaries + chosen_candidate`。
- **[确认候选打分与选择]**: 把 `0x100060da0` 中的 score 公式和 “prefer in-bounds, then choose minimum score” 策略回填进脚本输出，不再沿用之前的猜测权重。
- **[继续 lift 候选几何]**: 把 `0x10005fd98` 里 `CursorMotionPath/Segment` 的真实字段布局、guide vector、两条 base candidate、`3 x 3 x 2` arched candidate 以及 `20` 条候选总数直接落到脚本里，不再停留在“缺第二条 base candidate”的状态。
- **[补齐 timing 实证]**: 从 Swift metadata、type descriptor 和函数链继续下钻，确认 `ComputerUseCursor.CloseEnoughConfiguration`、`CursorNextInteractionTiming`、`Animation.SpringParameters`、`AnimationDescriptor`、`Transaction`、`VelocityVerletSimulation.Configuration` 的真实字段关系，并把 cursor path animation 的 `response=1.4`、`dampingFraction=0.9`、`dt=1/240`、`idleVelocityThreshold=28800` 和 `ComputerUseCursor.Window` 的 animation 状态槽位写回脚本与文档。
- **[补平 VelocityVerlet 数学公式]**: 继续深挖 `0x100593cfc`、`0x100593f18`、`0x100593404`、`0x100594110`，把 `stiffness = min((2π / response)^2, 28800)`、`drag = 2 * dampingFraction * sqrt(stiffness)`、stale-time clamp，以及单步 `VelocityVerlet` 更新顺序全部落进独立脚本。
- **[恢复 SpringAnimation finished predicate]**: 继续拆 `0x1005761bc` / `0x1005934b0`，确认 frame update 会在推进 simulation 后跑 finished predicate，并记录 threshold-square gate、`0.01` float literal 广播、exact-zero 双向比较，以及“端点先锁住、finished 仍可能稍后才成立”的谨慎 inference。
- **[固化函数级分析]**: 新增 `software-cursor-motion-reconstruction.md`，把已确认的 `sample(progress)`、`CursorMotionPathMeasurement`、候选 score 公式和候选总量形状单独沉淀。
- **[同步目录导航]**: 更新 reverse-engineering README，把新的 reconstruction 文档纳入索引。

### 🧠 Design Intent (Why)
当前已有另一条 session 在持续推进 `CursorMotion`，直接继续改实验 target 风险太高。这次改成 `scripts/` 下的独立脚本，一方面能继续深挖闭源实现，另一方面也能把“已确认”和“仍在 reconstruction”的部分明确分层，便于后续逐步替换。

### 📁 Files Modified
- `docs/exec-plans/active/20260419-official-cursor-motion-reconstruction.md`
- `docs/references/codex-computer-use-reverse-engineering/README.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-reconstruction.md`
- `docs/histories/2026-04/20260419-1333-add-binary-guided-cursor-motion-re-demo.md`
- `scripts/cursor-motion-re/README.md`
- `scripts/cursor-motion-re/official_cursor_motion.py`
- `scripts/cursor-motion-re/reconstruct_cursor_motion.py`
