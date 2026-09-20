# 主线 overlay 对齐官方 cursor motion

## 目标

把主线 `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift` 的 cursor move 行为，从当前的近似单段 Bezier + 固定 duration easing，调整为更接近官方 `Codex Computer Use.app` 的两层模型：

- 官方 lift 的 `20` 条候选路径、measurement 和 score 选择。
- 官方 lift 的 `VelocityVerlet` progress 推进与 `closeEnough` 返回时机。

## 范围

- 包含：
  - 在主线 package 中新增独立的 Swift cursor motion 内核。
  - 把 overlay 的 path selection 切到官方 lift 的 `CursorMotionPath` / `Segment` / measurement / score。
  - 把 overlay 的 move timing 切到 binary-backed `VelocityVerlet` progress。
  - 保留当前仓库已有的 target-window 命中优先策略，但把它降为官方候选集合上的 tie-break，而不是继续使用旧的 7 条固定候选。
  - 补单元测试、history 和架构文档。
- 不包含：
  - 不修改 `experiments/CursorMotion/`。
  - 不把这次变更扩展到 pulse / fog / idle sway 的完整官方 choreography。
  - 不宣称已经恢复官方 `finished` gate 的全部字段语义命名。

## 背景

- 当前主线 overlay 用的是简化版单段 cubic + 固定时长 `easeInOut`。
- 官方 binary 这两天已经确认到：
  - `20` 条候选路径。
  - `CursorMotionPathMeasurement(length, angleChangeEnergy, maxAngleChange, totalTurn, staysInBounds)`。
  - score 公式与 in-bounds 优先策略。
  - `SpringAnimation -> VelocityVerletSimulation` 的 `response=1.4`、`dampingFraction=0.9`、`dt=1/240`、`stiffness` / `drag` 公式和单步更新顺序。
  - `CloseEnoughConfiguration(progressThreshold=1.0, distanceThreshold=0.01)`。

## 风险

- 风险：一口气把当前 overlay 的候选与 timing 都换掉，可能让“窗口命中优先”的现有行为退化。
- 缓解方式：保留现有 target-window hit-test，但把它放到官方候选池上做 tie-break。

- 风险：主线点击调用在 `moveCursor` 结束后立刻执行真实点击，如果直接等到 spring 数值完全静止，体验会变慢。
- 缓解方式：主线 `moveCursor` 对齐官方 `closeEnough` 语义，到达 `progress >= 1` 且 `abs(target - progress) <= 0.01` 时返回。

- 风险：reverse-engineering 里对 `0x1005934b0` 仍有字段命名级不确定性。
- 缓解方式：主线只引入已确认的 path / measurement / `VelocityVerlet` / close-enough 逻辑，不把未证实的 finished 命名伪装成 exact。

## 验证方式

- `swift test`
- 针对 path 模型补单元测试，至少覆盖：
  - `CursorMotionPath` 起终点与 straight fallback。
  - 官方候选数量为 `20`。
  - 参考样例的 best candidate 与逆向脚本一致。
  - spring progress 会在 close-enough gate 返回，并出现端点锁定。

## 进度记录

- [x] 抽出主线可复用 motion 内核
- [x] 切换 overlay 到官方候选 + spring progress
- [x] 文档、history、测试同步完成

## 决策记录

- 2026-04-19：主线 overlay 这次只吃进已 binary-confirmed 的几何与 timing 内核，不等待 `finished` predicate 剩余字段命名完全坐实。
- 2026-04-19：target-window 命中策略保留，但从“旧 7 候选的主选择器”改成“官方候选池之上的 tie-break”。
- 2026-04-19：主线 runtime 直接复用官方 `closeEnough` spring shape，但不把 `1.429166...` 当作真实 wall-clock move duration；实际耗时继续按当前仓库已验证的本地校准公式映射。

## 结果记录

- 已在 `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/CursorMotionModel.swift` 新增主线可复用 motion 内核，包含 `CursorMotionPath`、candidate measurement/score、`VelocityVerlet` progress animator 和官方候选生成逻辑。
- 已在 `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift` 切换到官方候选池选路，并把 target-window 命中采样降级为 tie-break。
- 已在 `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift` 补充候选数量、参考样例 best candidate 和 `closeEnoughTime` 回归测试。
- 已执行 `swift test`，当前通过。
