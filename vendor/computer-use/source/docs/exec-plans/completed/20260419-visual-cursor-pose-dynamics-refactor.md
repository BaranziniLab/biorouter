# 主线 visual cursor 姿态动力学重构

## 目标

把主线 `SoftwareCursorOverlay` 从“path sample 直接驱动 tip + tangent 直接驱动角度”的单层模型，重构成更接近官方结构的双层模型：路径层只负责给出 `currentInterpolatedOrigin` 风格的运动目标，真正显示出来的 tip/velocity/angle/fog 由独立的 visual dynamics 状态持续推进。

## 范围

- 包含：
  - 在 `OpenComputerUseKit` 中引入可复用的 visual cursor dynamics 内核。
  - 删除当前临时补丁式的 `terminal settle`，改成贯穿 move / pulse / idle 的统一 2D 状态。
  - 调整 `SoftwareCursorView` 的渲染输入，增加 velocity-driven 的姿态与 fog/offset 表现。
  - 补单元测试、架构文档和 history。
- 不包含：
  - 不修改 `experiments/CursorMotion/`。
  - 不宣称已经精确恢复官方 `FogCursorViewModel` 的全部字段公式。
  - 不把这次改动扩展到完整 host choreography 或加载态 token。

## 背景

- 相关文档：
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-model.md`
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-motion-reconstruction.md`
  - `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- 相关代码路径：
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/CursorMotionModel.swift`
  - `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- 已知约束：
  - 另一条 session 仍在推进 `CursorMotion`，这次不能去碰实验 target。
  - `CursorMotionPath.sample(progress)` 的 endpoint clamp 已经 binary-confirmed。
  - `Style.velocityX / velocityY / angle`、`currentInterpolatedOrigin`、`FogCursorViewModel._velocityX / _velocityY / _angle`、`CursorView._animatedAngleOffsetDegrees` 这些状态存在，但还没有完整公式级 lift。

## 风险

- 风险：这次改动会碰 overlay 的核心更新循环，容易把 click/pulse/idle 串联关系打断。
- 缓解方式：把 visual dynamics 做成纯 Swift 内核，优先用单元测试覆盖“跟随、过冲、角度滞后、静止回稳”。

- 风险：如果 visual tip 的独立动力学参数过强，真实点击点和视觉 cursor 会偏太远。
- 缓解方式：约束 tip lag/fog offset 上限，把 click/pulse 期间的目标位置固定在真实点击点周围。

- 风险：当前 reverse-engineering 还没恢复官方所有 render anchor 公式。
- 缓解方式：这次只引入有明确证据支撑的状态分层，不伪装成 exact 复刻；未知部分保留为仓库内可调但有测试保护的近似实现。

## 里程碑

1. 确定主线要拆出来的 visual dynamics 状态与渲染输入。
2. 在 `OpenComputerUseKit` 中完成重构并替换 `terminal settle`。
3. 补测试、文档、history，并完成验证。

## 验证方式

- 命令：
  - `swift test`
- 手工检查：
  - move 结束后不再出现明显 endpoint-pivot 翻转。
  - 横向或斜向进入终点时，visible tip 能出现自然的小幅前冲/回弧。
  - pulse / idle 能继承同一套姿态状态，而不是重新归零。
- 观测检查：
  - 主线代码里不再依赖临时 `terminal settle` 补丁驱动收尾。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-19：这次不继续强化 endpoint 附近的特判补丁，而是直接切到“路径目标 + 独立 visual dynamics”双层模型，原因是当前问题已经来自状态分层缺失，不再是单条路径候选选择错误。
- 2026-04-19：主线 visual cursor 这次不伪装成 exact 复刻 `FogCursorViewModel` 公式，而是先把已经有二进制证据支撑的状态分层落到主 runtime：`currentInterpolatedOrigin` 风格目标点、独立 `velocity/angle`、以及 velocity-driven fog/body lag。

## 结果记录

- 已在 `OpenComputerUseKit` 中新增 `CursorVisualDynamicsConfiguration`、`CursorVisualDynamicsState`、`CursorVisualRenderState` 和 `CursorVisualDynamicsAnimator`，作为主线 overlay 的可复用 visual dynamics 内核。
- 已删除主线 overlay 的临时 `terminal settle` 路径，`move`、`pulse`、`idle` 现在统一改为持续推进同一套 2D visual dynamics 状态。
- 已把 `SoftwareCursorView` 的输入从单一 `rotation` 扩展到 `rotation + cursorBodyOffset + fogOffset + fogScale`，让速度滞后和 fog 能体现在主 runtime 画面上。
- 已补充 visual dynamics 的回归测试，并执行 `swift test` 通过。
