# 官方 cursor motion 重建脚本

## 目标

继续逆向官方 `Codex Computer Use.app` 的 cursor motion 实现，在不触碰现有 `CursorMotion` 的前提下，于 `scripts/` 下落一套独立的二进制分析与 demo 脚本，能够基于指定起终点输出候选路径、采样点和几何测量结果。

## 范围

- 包含：
  - 深挖 `SkyComputerUseService` 中与 `CursorMotionPath`、`CursorMotionPathMeasurement`、`BezierAnimation`、`SpringAnimation` 相关的函数级行为。
  - 在 `scripts/` 下实现独立的 Python 脚本，读取官方 bundled app，提取 motion 类型、常数和候选系数表。
  - 实现一版 binary-guided 的 path sampling / measurement / candidate generation demo，输出 JSON 坐标样本与测量数据。
  - 把新的函数级分析和脚本使用方式沉淀到 `docs/` 与 history。
- 不包含：
  - 不修改 `experiments/CursorMotion/`。
  - 不把这次脚本直接下沉到主 MCP runtime。
  - 不宣称已经 100% 还原闭源算法中所有 scoring / timing 细节。

## 背景

- 相关文档：
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
  - `docs/PLANS_GUIDE.md`
- 相关代码路径：
  - `scripts/`
  - `~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/Codex Computer Use.app`
- 已知约束：
  - 当前仓库已有另一条 session 在持续修改 `experiments/CursorMotion`，本任务应避免与之冲突。
  - 官方 bundle 为闭源二进制，当前主要依赖 `otool`、`llvm-objdump`、Swift metadata 和常量表恢复。
  - 这次 demo 优先做成纯脚本 / JSON 输出，不依赖 GUI。

## 风险

- 风险：把“直接从函数控制流确认的行为”和“根据系数表做的重建”混为一谈。
- 缓解方式：脚本、文档和输出里显式标注 `confirmed_from_binary` 与 `reconstructed`.

- 风险：路径生成虽然已经 lift 到字段级，但仍可能把调用前那层 runtime bounds 发现和真正的 timing 绑定误写成“已完全恢复”。
- 缓解方式：把 `0x10005fd98` 的候选几何、`0x10005fa84` 的 bounds 预处理、以及 duration / animation descriptor 三部分分开标注。

- 风险：动到现有实验目录或 Package target，和另一条 session 发生冲突。
- 缓解方式：所有新增代码仅放到新的 `scripts/` 子目录和对应文档。

## 里程碑

1. 函数级逆向收敛。
2. 独立脚本实现。
3. 验证、文档与收尾。

## 验证方式

- 命令：
  - `python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py inspect`
  - `python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py demo --start 100 120 --end 720 380 --bounds 0 0 1280 800 --pretty`
- 手工检查：
  - 对照脚本输出的类型字段、系数表和常量，确认与当前逆向结论一致。
  - 检查路径采样和 measurement 输出是否符合 `CursorMotionPath` / `CursorMotionPathMeasurement` 的函数级分析。
- 观测检查：
  - 确认脚本能在本机默认 bundled app 路径上直接工作。
  - 确认文档中把 exact lift 和 reconstruction 分开表述。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-19：这条线不继续改 `CursorMotion`，而是单独在 `scripts/` 下做纯脚本 demo，避免与另一条 session 冲突。
- 2026-04-19：优先实现已从二进制函数级确认的 path sampling 和 measurement，再在此基础上做 candidate generation reconstruction，而不是反过来直接猜整套参数模型。
- 2026-04-19：`0x100060da0` 已确认 score 公式为 `320 * excessLengthRatio + 140 * angleEnergy + 180 * maxAngle + 18 * totalTurn + 45 * outOfBounds`，并确认“优先选 in-bounds 再取最小 score”的策略；duration 仍保留为未完全恢复。
- 2026-04-19：`0x10005fd98` 已进一步 lift 到字段级，当前脚本能按 bundled binary 生成完整 `20` 条候选，包含两条 base candidate、两段 cubic 的 arched candidate、真实 `CursorMotionPath/Segment` 布局，以及 guide vector `(-0.6946583704589973, 0.7193398003386512)`；runtime bounds 发现与 timing 仍单独保留为未完全恢复。
- 2026-04-19：timing 侧已经确认到真实类型和初始化链：`ComputerUseCursor.CloseEnoughConfiguration(progressThreshold=1.0, distanceThreshold=0.01)`、`ComputerUseCursor.CursorNextInteractionTiming.closeEnough(...)`、`Animation.SpringParameters(response=1.4, dampingFraction=0.9)`、`Animation.AnimationDescriptor.spring(...)`、`Animation.SpringAnimation`、`Animation.VelocityVerletSimulation.Configuration(dt=1/240, idleVelocityThreshold=28800)`，并把 `ComputerUseCursor.Window` 的 animation 状态槽位对回 `cursorMotionProgressAnimation / cursorMotionNextInteractionTimingHandler / cursorMotionCompletionHandler / cursorMotionDidSatisfyNextInteractionTiming`。
- 2026-04-19：`0x100593cfc` / `0x100593f18` / `0x100593404` / `0x100594110` 已经把 `VelocityVerlet` 的 `stiffness`、`drag`、stale-time clamp 和单步更新顺序全部直译出来；当前剩余不确定点不再是数学公式本身，而是 wall-clock duration 的上层调度。
- 2026-04-19：`0x1005761bc` / `0x1005934b0` 已经把 `SpringAnimation` 的 frame update / finished predicate 拆开；能确认 hidden self `0x68 / 0x70` 两个 buffer、threshold-square gate、`0.01` float literal 广播和 exact-zero gate，但 `0x68 / 0x70` 对应 `_value / _targetValue` 仍按 inference 标注。
