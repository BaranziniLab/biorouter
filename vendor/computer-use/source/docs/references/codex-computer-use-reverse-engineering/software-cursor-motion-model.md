# Software Cursor Motion Model

这个文档聚焦软件 cursor 的“运动模型”本身，不再只讨论 overlay window 是否存在，而是回答另一个更具体的问题：官方演示里那条自然、可调、带一点弹性的鼠标曲线，当前已经能从哪些证据反推出结构。

结论先写在前面：结合视频、`SkyComputerUseService` 字符串，以及这次直接从 `__swift5_types` / `__swift5_fieldmd` 恢复出的类型和字段，可以比较有把握地判断官方不是只做了一条固定 cubic Bezier，而是做了一层独立的 motion engine，至少包含 3 层：

- 一层路径几何模型：`CursorMotionPath` + `Segment`。
- 一层逐帧动画/物理推进：`BezierAnimation` + `SpringAnimation` + `VelocityVerletSimulation`。
- 一层“下一次交互何时允许开始”的 timing gate：`CloseEnoughConfiguration` + `CursorNextInteractionTiming`。

## 已观察事实

### 1. 视频里有显式的调参 UI

用户提供的 X 视频里，左上角能稳定看到 5 个 slider：

- `START HANDLE`
- `END HANDLE`
- `ARC SIZE`
- `ARC FLOW`
- `SPRING`

右上角能看到 3 个 toggle：

- `DEBUG`
- `MAIL`
- `CLICK`

这说明至少在官方内部调试构建里，cursor motion 不是黑盒常量，而是一组可实时调的参数。

需要补一条边界：当前本机 shipping bundle
`~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/Codex Computer Use.app`
做整包 phrase scan 后，并没有命中 `START HANDLE`、`END HANDLE`、`ARC SIZE`、`ARC FLOW` 这些完整 label。也就是说，视频里的 slider UI 仍然是有效证据，但更像内部调试构建或未发布调试面板，而不是当前 release app 里可直接字符串恢复出来的现成界面。

### 2. `SkyComputerUseService` 里不仅有字符串，还有可恢复的 Swift motion 类型

这次除了 `strings`，还额外做了两步：

- 用 `otool -l` 确认 `__swift5_typeref`、`__swift5_reflstr`、`__swift5_fieldmd`、`__swift5_types` 这些 section 的位置。
- 解析 `__swift5_types` 中的 type descriptor，再反查 `__swift5_fieldmd`，把 field descriptor 归回具体类型名。

这样拿到的不再只是零散关键词，而是“哪个类型拥有哪些字段”的证据。

#### Cursor 路径与状态相关类型

从 `SkyComputerUseService` 静态恢复出的核心类型和字段如下：

```text
ComputerUseCursor
  delegate
  targetWindowID
  isMoving
  shouldFadeOut
  window
  correspondingApplicationPID
  style
  activityState

Window
  style
  appMonitor
  wantsToBeVisible
  cursorMotionProgressAnimation
  cursorMotionNextInteractionTimingHandler
  cursorMotionCompletionHandler
  cursorMotionDidSatisfyNextInteractionTiming
  currentInterpolatedOrigin
  useOverlayWindowLevel
  correspondingWindowID

Style
  velocityX
  velocityY
  isPressed
  activityState
  isAttached
  angle

CloseEnoughConfiguration
  progressThreshold
  distanceThreshold

CursorNextInteractionTiming
  closeEnough
  finished

ActivityState
  idle
  loading
  paused

CursorMotionPathMeasurement
  length
  angleChangeEnergy
  maxAngleChange
  totalTurn
  staysInBounds

Segment
  end
  control1
  control2

CursorMotionPath
  start
  end
  startControl
  arc
  arcIn
  arcOut
  endControl
  segments
```

这批字段把几件事基本坐实了：

- 官方 cursor path 不是一个写死的单段 cubic，而是一个 `CursorMotionPath`，里面显式保存 `segments`。
- 每个 `Segment` 自己再带 `control1` / `control2` / `end`，说明底层仍是 cubic Bezier，但高层路径可以由多段组成。
- path 旁边还有一份 `CursorMotionPathMeasurement`，字段不是只有长度，而是 `angleChangeEnergy`、`maxAngleChange`、`totalTurn` 和 `staysInBounds`。这强烈说明官方会对候选路径做几何质量评分，而不是简单取一条。
- `CloseEnoughConfiguration(progressThreshold, distanceThreshold)` 加上 `CursorNextInteractionTiming(closeEnough, finished)`，说明官方明确建模了“动画还没完全结束，但已经够接近，可以允许下一次交互”的状态。

#### 动画与速度推进相关类型

`SkyComputerUseService` 同时还带着一套独立的动画模块：

```text
BezierAnimation
  parameters

Parameters
  curve
  duration

InterpolatableAnimation
  id
  _value
  _targetValue
  eventHandler
  state
  _initialValue
  startTime

SpringAnimation
  simulation

BezierFunction
  x1
  x2
  y1
  y2
  cx
  cy
  bx
  by
  ax
  ay

BezierParameters
  curve
  duration

SpringParameters
  response
  dampingFraction

VelocityVerletSimulation
  configuration
  time
  velocity
  force

Configuration
  response
  stiffness
  drag
  dt
  idleVelocityThreshold

AnimationDescriptor
  bezier
  spring
```

这里最关键的不是名字本身，而是字段组合：

- `BezierParameters(curve, duration)` 说明 time progression 确实有一层显式的 Bezier timing，而不是只靠系统默认 easing。
- `BezierFunction(x1, x2, y1, y2, cx, cy, bx, by, ax, ay)` 说明这层 Bezier 不是抽象关键词，而是实际把 cubic 多项式系数预展开了。
- `SpringParameters(response, dampingFraction)` 是用户可调或上层可配置的 spring 输入。
- `VelocityVerletSimulation(configuration, time, velocity, force)` 和 `Configuration(response, stiffness, drag, dt, idleVelocityThreshold)` 则几乎可以确认底层不是一次性解析函数，而是 display-link 驱动的逐帧物理模拟。

### 3. `Fog` / `wiggle` 也不是口头说法，而是独立 view model

这次还能直接看到一组和“思考时轻微摇摆”“fog 光晕”对应的类型：

```text
FogCursorViewModel
  _velocityX
  _velocityY
  _isPressed
  _activityState
  _isAttached
  _angle

CursorView
  viewModel
  cursorRadius
  _animatedAngleOffsetDegrees
  _loadingAnimationToken
  fogRadius
  cursorScaleAnchorPoint
  fogScaleAnchorPoint
```

这说明：

- 光标朝向确实是速度驱动的，不是纯位置插值。
- “thinking wiggle” 至少在渲染层有专门的 `_animatedAngleOffsetDegrees` 和 `_loadingAnimationToken`。
- fog 也不是简单阴影，而是单独建模了 `fogRadius` 和对应的 scale anchor。

### 4. 当前开源仓库已有一版较简化的近似实现

`packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift` 当前已经具备：

- 基于起终点生成多组 cubic Bezier 候选。
- 按目标 `windowID` 做采样命中，筛掉明显飘出目标窗口的路径。
- 路径切线驱动的 cursor 旋转。
- click pulse 与 idle sway。

但它还没有显式建模这些官方证据里已出现的参数：

- `start handle`
- `end handle`
- path 分段与 turn-energy 评分
- `arc flow`
- `spring`

也没有“带速度状态的 spring settle”和“下一交互 timing gate”。

## 现在可以比较确定的结构判断

下面这部分仍然带推断成分，但已经不是“纯猜”，而是基于上面的静态类型证据。

### 1. 曲线怎么计算

目前最合理的结构是：

1. 先构造一个高层 `CursorMotionPath`。
2. 这个 path 至少包含：
   - `start`
   - `end`
   - `startControl`
   - `endControl`
   - `arc`
   - `arcIn`
   - `arcOut`
   - `segments`
3. 再把 path 展开成一组具体 `Segment`。
4. 每个 `Segment` 再用 `control1` / `control2` / `end` 生成实际 cubic Bezier。
5. 然后对整条 path 计算 `CursorMotionPathMeasurement`：
   - `length`
   - `angleChangeEnergy`
   - `maxAngleChange`
   - `totalTurn`
   - `staysInBounds`
6. 只有满足质量和 window 命中约束的路径，才会被接受并开播。

这比“从起点到终点直接拉一条单段 cubic”要复杂得多，也更接近团队成员说的 `calculates natural and aesthetic motion paths`。

### 2. 速度怎么推进

从类型和字段看，速度推进大概率不是“沿 Bezier 参数 `t` 匀速跑”，而是：

1. 用 `BezierAnimation` / `BezierParameters(curve, duration)` 控制一段基础进度。
2. 在需要弹性和停驻手感的阶段，再叠 `SpringAnimation`。
3. spring 本体通过 `VelocityVerletSimulation` 按帧迭代：
   - 维护 `time`
   - 维护当前 `velocity`
   - 维护当前 `force`
   - 按 `Configuration(response, stiffness, drag, dt, idleVelocityThreshold)` 推进
4. `DisplayLinkAnimationDriver(displayLink)` 负责逐帧驱动。

`VelocityVerletSimulation` 这个名字尤其关键，因为它已经把“用什么数值方法跑 spring”暴露出来了。就现有证据看，官方更像是在做逐帧积分，而不是单纯调用一个现成的 `CASpringAnimation` 然后交给系统黑盒求值。

当前已经恢复出的 shipping 默认 progress spring 为 `response=1.4`、`dampingFraction=0.9`、`dt=1/240`。按 `CloseEnoughConfiguration(progressThreshold=1.0, distanceThreshold=0.01)` 计算，endpoint-lock / close-enough 时间约为 `343 / 240 = 1.4291667s`。这意味着默认可见移动不应该再叠加一层本地距离压缩；短距离和长距离都会复用同一条 spring progress 时间线，只是路径几何和可见姿态的速度感不同。

### 3. 什么时候允许下一次交互开始

`CloseEnoughConfiguration(progressThreshold, distanceThreshold)` 和 `CursorNextInteractionTiming(closeEnough, finished)` 这组类型，说明官方把“动作已足够接近，可继续下一步”和“动作完全结束”明确区分开了。

这意味着：

- 视觉动画还没完全 settle 时，系统可能已经允许模型进入下一次 tool interaction。
- 这个 gate 至少会同时看两件事：
  - 已经跑过了多少进度 `progressThreshold`
  - 离目标还剩多少距离 `distanceThreshold`

这比“每次都傻等动画播完再继续”更贴近官方演示里那种连续、不断句的操作感。

## 参数映射的修正判断

和上一版文档相比，这里有一处重要修正。

### `START HANDLE`

现在看，最可能映射到 `CursorMotionPath.startControl`，以及最终分段 cubic 上的 `control1`。

### `END HANDLE`

现在看，最可能映射到 `CursorMotionPath.endControl`，以及最终分段 cubic 上的 `control2`。

### `ARC SIZE`

上一版把它直接映射到 `arcHeight`，这个判断现在要降级。

新的证据显示：

- cursor path 本体字段里确认到的是 `arc`、`arcIn`、`arcOut`。
- `arcHeight` 目前只在 `SystemSettingsAccessoryTransitionGeometryStyle` 上被确认，不像 cursor path 本体字段。

所以更保守的说法是：`ARC SIZE` 更可能先作用在 `CursorMotionPath.arc` 或分段控制点偏移量上，而不是已经能直接确认是某个叫 `arcHeight` 的 cursor 字段。

### `ARC FLOW`

这条判断比上一版更稳了，因为 `CursorMotionPath` 里确实存在 `arcIn` / `arcOut`：

- `arcIn` 大概率控制起点侧进入主弧线的节奏。
- `arcOut` 大概率控制终点侧回收至目标的节奏。
- `ARC FLOW` 大概率是在这两个量之间做重分配。

### `SPRING`

这条也比上一版更具体了。现在可以把它落到一整套链路上：

- 上层参数：`SpringParameters(response, dampingFraction)`
- 运行时模拟：`VelocityVerletSimulation`
- 模拟配置：`Configuration(response, stiffness, drag, dt, idleVelocityThreshold)`

## 对独立实现的设计启发

如果要在当前仓库里抽一个后续可独立开源的版本，建议不要再把所有逻辑继续塞进 `SoftwareCursorOverlay`，而是拆成 4 层：

### 1. Motion Parameters

纯值类型，不依赖 AppKit。

建议至少包含：

- `startHandle`
- `endHandle`
- `arcHeight`
- `arcFlow`
- `spring`

### 2. Motion Path Builder

输入起点、终点和参数，产出：

- `CursorMotionPath`
- `segments`
- `control1`
- `control2`
- `measurement`
- 切线

这里单独负责几何，不负责时间。

### 3. Motion Simulator

在路径几何之上叠时间推进：

- Bezier progress animation
- spring simulation
- velocity
- force
- next-interaction timing gate
- completion timing

### 4. Cursor Renderer / Demo Host

最外层才接 AppKit / SwiftUI，负责：

- overlay window
- debug slider UI
- target point 标注
- click / mail / debug toggle

## 对仓库落点的建议

为了后续独立开源更干净，这块建议先作为单独目录推进，而不是直接和 MCP runtime 耦合：

- 目录建议：`experiments/CursorMotion/`
- 第一阶段先做一个纯本地 demo app，不接真实 tool call。
- 等参数模型稳定后，再决定是否把其中的 `Motion Parameters` / `Path Builder` 下沉回 `packages/` 复用。

这样可以避免两个问题：

- 为了追官方手感，反复改动线上 `click` overlay。
- demo/调参 UI 代码污染主产品边界。

## 当前判断

当前最合理的独立实现方向不是“继续微调现有候选 Bezier 权重”，而是：

1. 先把 motion model 从 overlay 渲染里拆出来。
2. 明确把 `handle`、`arc`、`spring`、`closeEnough timing` 建模成一等参数。
3. 把 path builder 做成“多段 cubic + measurement”而不是单段模板曲线。
4. 把 timing simulator 做成“Bezier progress + spring settle + velocity state”。
5. 做一个带 slider/toggle 的本地 Swift demo。
6. 用这个 demo 逼近视频里的轨迹和停驻手感。

只有这样，后面这个目录才真的适合单独开源。
