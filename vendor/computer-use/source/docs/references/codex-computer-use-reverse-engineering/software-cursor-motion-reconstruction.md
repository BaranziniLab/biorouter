# Software Cursor Motion Reconstruction

这份文档比 `software-cursor-motion-model.md` 更窄，专门记录这次继续往函数级推进后，已经能直接从 `SkyComputerUseService` 控制流里确认下来的实现细节，以及哪些部分仍然只是 reconstruction。

## 结论概览

这次最有价值的 4 个结论是：

- `CursorMotionPath.sample(progress)` 已经基本能按函数级行为重建出来。
- `CursorMotionPathMeasurement` 的 5 个输出字段已经能对应到采样循环和角度累计逻辑。
- `CursorMotionPath` / `Segment` 的真实字段布局已经能和写字段指令一一对上。
- 候选路径筛选不再只是“可能有一组权重”的推测，现在已经能确认 score 公式、in-bounds 优先策略，以及 `20` 条候选的几何生成形状。

这轮继续往 timing 侧推进之后，又补上了 4 个可以直接落到二进制实体上的结论：

- cursor path animation 不是抽象的 easing 黑盒，而是 `Animation.SpringAnimation` 驱动的一条进度动画链。
- `closeEnough` 相关类型和字段关系已经能从 Swift metadata 直接对上。
- spring 路径内部确实会落到 `Animation.VelocityVerletSimulation.Configuration`，并且 `stiffness / drag`、`dt = 1/240`、`VelocityVerlet` 单步更新顺序现在都能直接写出来。
- `ComputerUseCursor.Window` 里承载 cursor animation 状态的几个槽位，已经能和主控制流中的写字段顺序对上。

## 已确认的函数级行为

### 1. `CursorMotionPath.sample(progress)` 是“按 segment 选段，再做 cubic 求值”

函数地址：`0x10005c1dc`

从控制流可以确认：

1. 输入 `progress` 会先被 clamp 到 `0...1`。
2. 根据 `segments.count` 把全局 `progress` 映射到具体 `segmentIndex` 和 segment 内局部 `t`。
3. 如果 `progress >= 1`，直接落到最后一段、局部 `t = 1`。
4. 每段都按标准 cubic Bezier 公式求点：
   - 起点
   - `control1`
   - `control2`
   - 终点
5. 函数末尾还会调用一个辅助函数，返回额外两维量，结合调用点看更像是 tangent / orientation 相关数据。

这说明官方 path sampling 这层不是黑盒 easing，而是标准分段 cubic。

### 2. `CursorMotionPathMeasurement` 是固定步数采样，不是解析闭式公式

函数地址：`0x100060ac0`

能直接确认的行为：

- 它会遍历 `segments`。
- 每段固定按 `24` 个采样点推进。
- 只在相邻点距离大于 `0.01` 时才把这一小段当成有效步长。
- 对每个有效步长：
  - 累加 `length`
  - 用 `atan2(dy, dx)` 算 heading
  - 对相邻 heading 做 `[-pi, pi]` unwrap
  - 累加角度变化平方
  - 记录最大绝对转角
  - 累加总绝对转角

从寄存器和最终写回顺序可以把 5 个字段对应出来：

- `length`
- `angleChangeEnergy`
- `maxAngleChange`
- `totalTurn`
- `staysInBounds`

### 3. `staysInBounds` 是 measurement 阶段顺手维护的布尔值

这部分不是独立后处理。

`0x100060ac0` 在采样循环里会持续维护一个布尔标记，只有当所有关键采样点都还满足边界约束时，这个字段才保持 `true`。

当前最保守的解释是：

- 如果调用方给了 bounds，measurement 会把采样点逐个做包含性校验。
- 实现里能看到固定的 `20.0` 边距参与判断，因此不是严格贴边的裸矩形判断。

## 新确认的候选选择逻辑

### 0. `CursorMotionPath` 和 `Segment` 的内存布局已经钉住

结合 Swift field metadata 和 `0x10005fd98` / `0x10005c1dc` 的写字段、读字段顺序，可以把这两个关键结构的布局对上：

- `CursorMotionPath`
  - `start`: `0x00`
  - `end`: `0x10`
  - `startControl`: `0x20`
  - `arc`: `0x30`
  - `arcIn`: `0x48`
  - `arcOut`: `0x60`
  - `endControl`: `0x78`
  - `segments`: `0x88`
- `Segment`
  - `end`: `0x00`
  - `control1`: `0x10`
  - `control2`: `0x20`

`0x10005c1dc` 里对 `Segment` 的读取顺序也能说明：

- segment 的起点不是单独存的。
- 第一段起点来自 `CursorMotionPath.start`。
- 后续段的起点来自上一段的 `Segment.end`。
- `Segment.end / control1 / control2` 都是 cubic 采样时直接参与公式的绝对点。

### 1. 候选总量不是 18，而是 20

函数地址：`0x10005fd98`

这次把候选生成函数继续拆平后，可以确认：

- 有两张系数表：
  - `tableA = [0.55, 0.8, 1.05]`
  - `tableB = [0.65, 1.0, 1.35]`
- 双层循环枚举 `3 x 3` 组组合。
- 每组组合会再走左右镜像两条分支。
- 在进入双层循环前，函数还会先构造两条 base candidate。

因此总候选量不是之前保守写法里的“18 条左右”，而是更接近：

- `2` 条 base candidate
- `3 x 3 x 2 = 18` 条镜像候选
- 合计 `20` 条

### 2. 候选生成已经能落到字段级几何，不只是 table 级猜测

仍然是 `0x10005fd98`。

现在能直接确认：

- 有一组 guide 相关系数会先经 `swift_once` 初始化到全局：
  - `(-0.6946583704589973, 0.7193398003386512)`
- 这组数值本身是 confirmed；但后续对照官方视频后，直接把它当成固定屏幕坐标 guide vector 会在部分象限下产生明显不符的扭曲回环。当前 reconstruction 更保守的做法是：把它当成 path-local basis 里的系数对，再投到世界坐标去生成 candidate 几何。这一层“local-basis projection”目前仍属于 reconstruction-level inference，不是已经由反汇编逐指令坐实的结论。
- path builder 会先构造两条 base candidate：
  - `base-full-guide`
  - `base-scaled-guide`
- 再围绕两张系数表构造 `18` 条两段 cubic 的 arched candidate。

能直接确认的主尺度包括：

- `distance * 0.41960295031576633`
- `distance * 0.9`
- `distance * 0.15`
- `distance * 0.2765523188064277`
- `distance * 0.5783555327868779`
- `clippedTravel * 0.65`

其中有一条之前写得过于粗糙的结论需要修正：

- `arcExtent` 仍然可以安全写成：

```text
arcExtent = clamp(distance * 0.5783555327868779, 38, 440)
```

- 但 `handleExtent` 不是简单的 `clamp(..., 50, 520)`，而是这版 bundled binary 里的分段逻辑：

```text
rawHandle = distance * 0.2765523188064277

if rawHandle < 50:
  handleExtent = 50
else if rawHandle < 640:
  handleExtent = rawHandle
else:
  handleExtent = 520
```

在这组 guide 系数原始存储满足 `x < 0, y > 0` 的前提下，base candidate 的 guide travel 也能从原始分支树化简成这组 piecewise：

```text
startExtent =
  48                               if distance * 0.41960295031576633 < 48
  distance * 0.41960295031576633   if distance * 0.41960295031576633 < 640
  640                              if distance * 0.15 < 640
  640                              otherwise

endExtent =
  48       if distance * 0.41960295031576633 < 48
  distance * 0.9
           if distance * 0.41960295031576633 < 640
  48       if distance * 0.15 < 640
  640      otherwise
```

再叠加 bounds clipping 后，会得到：

- `fullStartControl = start + guide * startExtent`
- `fullEndControl = end - guide * endExtent`
- `scaledStartControl = start + guide * (startExtent * 0.65 after clipping)`
- `scaledEndControl = end - guide * (endExtent * 0.65 after clipping)`

镜像候选也已经能落成具体几何：

- 先用 midpoint、signed normal、`tableA` 和 `handleExtent` 算 arc anchor。
- 再用 `tableB`、`arcExtent` 和一条由 `(dx, arcExtent)` 归一化得到的 forward vector 算 `arcIn / arcOut`。
- 每条 arched candidate 是两段 cubic：
  - 第一段：`start -> arc`
  - 第二段：`arc -> end`

### 3. 评分公式已经可以直接写出来

函数地址：`0x100060da0`

这次最重要的推进，是把 candidate measurement 之后的 score 组合方式拆出来了。

对每条候选路径，函数会先算：

- `directDistance = max(distance(start, end), 1)`
- `excessLengthRatio = max(length / directDistance - 1, 0)`

然后 score 为：

```text
score =
  320 * excessLengthRatio
  + 140 * angleChangeEnergy
  + 180 * maxAngleChange
  + 18 * totalTurn
  + (staysInBounds ? 0 : 45)
```

这说明官方筛选逻辑明确偏好：

- 不要比直线长太多
- 不要有太大的角度抖动
- 不要有单次过猛的转向
- 不要有累计转向过多
- 如果候选跑出 bounds，直接加固定 penalty

### 4. 选择策略是“先保 in-bounds，再比 score”

`0x100060da0` 在算完所有候选的 measurement 和 score 之后，不是直接对整批取最小值。

它会先做一遍过滤：

- 如果存在 `staysInBounds == true` 的候选，只在这批 in-bounds 候选里取最小 score。
- 只有当没有任何 in-bounds 候选时，才会回退到全体候选里取最小 score。

这点对外观很重要，因为它解释了为什么官方路径看上去既“俏皮”又不容易穿窗体 / 越界。

## 新确认的 timing / animation 链

### 1. `CloseEnoughConfiguration` / `CursorNextInteractionTiming` 已经能落到真实嵌套类型

Swift metadata 里已经能直接恢复出这组父子关系：

- `ComputerUseCursor.CloseEnoughConfiguration`
  - `progressThreshold`
  - `distanceThreshold`
- `ComputerUseCursor.CursorNextInteractionTiming`
  - `closeEnough`
  - `finished`

这说明之前看到的 `1.0` 和 `0.01` 不是散落常量，而是已经能映射到真实字段：

- `progressThreshold = 1.0`
- `distanceThreshold = 0.01`

从 `0x10005be24..0x10005be3c` 的写栈顺序看，这两项正是 cursor path animation 在组 `next interaction timing` 时使用的 close-enough 配置。

### 2. `AnimationDescriptor` / `SpringParameters` / `Transaction` 的真实字段也已经恢复

同样通过 `__swift5_types` + `__swift5_fieldmd`，已经能确认：

- `Animation.AnimationDescriptor`
  - `bezier`
  - `spring`
- `Animation.SpringParameters`
  - `response`
  - `dampingFraction`
- `Animation.Transaction`
  - `priority`
  - `delay`
  - `completion`
  - `id`
  - `driverSource`
  - `descriptor`

这里最关键的是：

- cursor path 主链里用到的 spring 常量已经能直接确认是
  - `response = 1.4`
  - `dampingFraction = 0.9`
- 二进制同时确实导入了
  - `SwiftUI.Animation.spring(response:dampingFraction:blendDuration:)`

因此“官方 cursor path animation 用的是 spring 而不是自定义 bezier duration”这点已经是 binary-backed 结论，不再只是字符串级猜测。

### 3. `SpringAnimation` 会继续落到 `VelocityVerletSimulation.Configuration`

新的关键链路是：

- `Animation.SpringAnimation` metadata accessor：`0x1005768c4`
- allocating wrapper：`0x100576790`
- designated init 主体：`0x10057652c`
- `Animation.VelocityVerletSimulation.Configuration` metadata accessor：`0x100591fd4`
- `Configuration` 初始化主链：`0x100592f20`
- config completion：`0x100593cfc`

这条链路说明：

- cursor path animation 的“速度”底层不是简单的定长采样。
- 它通过 spring 参数继续构造成 `VelocityVerletSimulation`。

而 `Animation.VelocityVerletSimulation.Configuration` 的字段已经能直接恢复为：

- `response`
- `stiffness`
- `drag`
- `dt`
- `idleVelocityThreshold`

目前能直接确认的数值和公式有：

- `dt = 1/240 = 0.004166666666666667`
- `idleVelocityThreshold = 28800.0`
- `0x100593cfc`
  - `stiffness = min(response > 0 ? (2π / response)^2 : +inf, 28800.0)`
- `0x100593f18`
  - `drag = 2 * dampingFraction * sqrt(stiffness)`
- `0x100593404`
  - 如果 `targetTime - time > 1.0`，先把 `time` 钳到 `targetTime - 1/60`
  - 然后按 `dt` 循环推进，直到 `time >= targetTime`
- `0x100594110`
  - `velocityHalf = velocity + force * (dt / 2)`
  - `current = current + velocityHalf * dt`
  - `force = stiffness * (target - current) + (-drag) * velocityHalf`
  - `velocity = velocityHalf + force * (dt / 2)`

因此现在已经可以明确写成：

- 这条链路是 binary-backed 的 `VelocityVerlet` 弹簧仿真，而不是模糊的“某种 spring-like easing”。
- `stiffness / drag` 这两个核心量已经不再停留在“存在但公式未抄平”的状态。

### 4. `SpringAnimation` 的 frame update / finished predicate 已经基本拆开

这里最关键的两个函数是：

- `0x1005761bc`
- `0x1005934b0`

`0x1005761bc` 现在已经能确认成这条控制链：

- 通过 `0x1005730bc` 从 hidden self 的 `0x68` 槽位拷一个 current-value-like buffer。
- 通过 `0x100573390` 从 hidden self 的 `0x70` 槽位拷一个 target-value-like buffer。
- 调 `0x100593404` 推进 spring simulation。
- 调 `0x1005934b0` 计算 finished predicate。
- finished 时走 optional `nil` 返回；未 finished 时走 optional `some(updatedValue)` 返回。

`0x1005934b0` 当前已经能确认两段 gate：

- 第一段 gate：
  - 从 hidden self 再取两个标量槽位。
  - 先做 `Swift.max(slotA, slotB)`。
  - 再把另一个阈值字段平方后比较。
  - 只有 `max(slotA, slotB) <= threshold^2` 才会继续往下跑。
- 第二段 gate：
  - 明确构造了一个 `0.01` float literal。
  - 用 `SIMDStorage` 把它广播成和 animatable value 同标量数的向量。
  - 后面会跑一串逐分量乘法、减法和类型转换。
  - 末尾不是用近似比较，而是把一个差值派生标量 `A` 做成：
    - `A > 0`
    - `0 > A`
  - 只有两边都不成立时，才算 finished。

需要明确区分 confirmed 与 inference：

- 已确认：
  - `0x1005761bc` 的 optional `nil / some(updatedValue)` 分支结构
  - `0x1005934b0` 的 threshold-square gate
  - `0.01` float literal 广播
  - 双向比较实现 exact-zero gate
- 仍然是 inference：
  - `0x68 / 0x70` 很可能就是 `InterpolatableAnimation._value / _targetValue`
  - `0x1005934b0` 里 metadata `+0x30` 取到的阈值字段，很可能就是 `idleVelocityThreshold`

### 5. 可见几何会先锁到终点，这点现在已经能从几段二进制证据拼起来

这里要把“confirmed”与“组合推断”分开写：

- 已确认：
  - `CursorMotionPath.sample(progress)` 会把 `progress` clamp 到 `0...1`
  - `SpringAnimation` 的 progress 由上面的 `VelocityVerlet` 链驱动
  - `0x1005761bc` 的 finished 返回是单独由 `0x1005934b0` 控的
- 因此可以谨慎推出：
  - 一旦 raw spring progress 首次 `>= 1.0`，可见几何位置就会被 clamp 到 path endpoint
  - 这件事可能早于 raw spring state 在数值上完全静止

这也是为什么新的独立 demo 里要同时输出：

- `raw_progress_first_ge_target_time`
- `first_endpoint_lock_time`
- `close_enough_first_time`

在当前样本里，通常能直接看到：

- raw progress 已经越过 `1.0`
- 几何点已经停在终点
- 但 raw spring velocity / force 仍然非零

这条“端点先锁住、finished 还未必立刻返回”的结论，目前仍然按 inference 标注，因为它依赖把 `sample(progress)` 的 clamp、`VelocityVerlet` 的原始状态、以及 `0x1005934b0` 的 finished gate 三块证据拼在一起。

### 6. `ComputerUseCursor.Window` 的 animation 状态槽位已经能对上主控制流写入

`ComputerUseCursor.Window` 的真实字段顺序现在也能恢复：

- `style`
- `appMonitor`
- `wantsToBeVisible`
- `cursorMotionProgressAnimation`
- `cursorMotionNextInteractionTimingHandler`
- `cursorMotionCompletionHandler`
- `cursorMotionDidSatisfyNextInteractionTiming`
- `currentInterpolatedOrigin`
- `useOverlayWindowLevel`
- `correspondingWindowID`

结合 `0x10005be94..0x10005bf18` 的一串连续写字段，可以把 cursor move 动画主链里这几个写入直接对应到：

- `cursorMotionProgressAnimation`
- `cursorMotionNextInteractionTimingHandler`
- `cursorMotionCompletionHandler`
- `cursorMotionDidSatisfyNextInteractionTiming`

这意味着顶层控制流已经不只是“某个匿名对象在写几个槽位”，而是可以对回真实 `ComputerUseCursor.Window` 内部状态。

### 7. `SpringParameters` 区域还有一个已确认的 piecewise remap helper

`0x1005879a4` 位于 `Animation.SpringParameters` 一侧，它会把第二个输入量做如下 piecewise 映射：

```text
if x <= -1:
  mapped = +inf
else if x < 0:
  mapped = 1 / (1 + x)
else if x == 0:
  mapped = 0
else:
  mapped = 1 - min(x, 1)
```

这说明 bundled binary 里确实存在一层 spring 参数规整逻辑，能够把一个 `[-1, 1]` 附近的量映射到阻尼相关参数。

需要强调的是：

- cursor path move 这条链路里当前已经直接确认使用的是 `response = 1.4`、`dampingFraction = 0.9`。
- 上面这个 remap helper 是同一套 animation 库里的另一条可复用 spring 参数路径，不代表 cursor move 主链一定先经过这个 remap。

## 仍在重建中的部分

### 1. 自动 bounds 发现还没有直接 lift 到脚本输入层

主 app 在调用 `0x10005fd98` 前，还会先走一次 `0x10005fa84`：

- 从运行时屏幕 / 区域列表里挑出同时覆盖起点和终点的 bounds。
- 如果找不到这样的单个 rect，再对候选 rect 做 union。

当前脚本为了保持独立、可重复，要求调用方直接传 `--bounds`，没有把这段 runtime screen discovery 接进来。

### 2. duration / 真正的时间速度还没有完全恢复

目前已经能稳定输出：

- path 采样点
- tangent
- `speed_units_per_progress`

但这里的“speed”仍然是几何速度，即：

- 相邻等 progress 采样点之间的距离

真正的时间速度还缺少一层：

- progress 随时间如何推进
- `BezierParameters.duration` / `AnimationDescriptor` / `CloseEnoughConfiguration` 在 cursor motion 主链路里的最终接法

也就是说，现在已经能比较可靠地回答“路线怎么走”，但对“这条路线多快走完”还没有到可以宣称 exact 的程度。

比上一版更进一步的是：

- spring family
- close-enough 阈值
- `response / dampingFraction`
- `VelocityVerletSimulation.Configuration` 的字段
- `stiffness / drag`
- `dt = 1/240`
- `0x1005761bc` / `0x1005934b0` 的 update + finished predicate 控制流

这些已经都能直接从二进制确认。

仍然没有完全 lift 完的，是：

- `Animation.Transaction` 在 cursor move 这条主链里的完整组装顺序
- `0x1005934b0` 第二段里几个泛型临时 buffer 的精确语义命名
- `0x68 / 0x70 -> _value / _targetValue` 的最终符号级证明
- 真实 wall-clock duration 如何从 spring 仿真和 transaction 调度一起体现到最终时间轴上

## 脚本化落点

这次为了避开 `CursorMotion`，在 `scripts/cursor-motion-re/` 下落了一套独立脚本：

- `reconstruct_cursor_motion.py inspect`
  - 读取官方 binary，输出已恢复的 motion 类型、字段、常量和候选系数表。
- `reconstruct_cursor_motion.py demo`
  - 输入起终点和可选 bounds，输出候选路径、measurement 和采样点。

脚本里明确区分两类实现：

- `confirmed_from_binary`
  - 分段 cubic path sampling
  - `CursorMotionPathMeasurement`
  - 候选系数表提取
  - 候选 score 公式
  - in-bounds 优先选择策略
- `reconstructed`
  - duration / timing model
  - 调用前的 runtime bounds 发现

这样后续继续深挖时，可以逐项把 reconstruction 替换成更准确的函数级实现，而不是整套推倒重来。
