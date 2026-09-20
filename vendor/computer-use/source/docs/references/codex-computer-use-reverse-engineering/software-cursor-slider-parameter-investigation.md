# Software Cursor Slider Parameter Investigation

这份文档专门回答一个更聚焦的问题：

- 视频里看到的 5 个 slider (`START HANDLE`、`END HANDLE`、`ARC SIZE`、`ARC FLOW`、`SPRING`) 在当前 shipping 的 `Codex Computer Use.app` 里有没有直接证据？
- 如果 shipping bundle 里没有这组调试 UI 文案，那么它们更接近哪些已经 binary-confirmed 的几何 / timing 量？
- 这些量一旦变化，实际会把曲线的哪一段拉长、收紧或后移？

## 结论先写在前面

当前最稳的结论有 4 条：

1. 在 shipping bundle 中，没有扫到 `START HANDLE`、`END HANDLE`、`ARC SIZE`、`ARC FLOW` 这 4 个完整 slider phrase。
2. `SPRING` / `DEBUG` / `MAIL` / `CLICK` 这类单词在 shipping bundle 里能扫到，但它们都是高度歧义的 token，不能据此宣称“视频里的 debug UI 还保留在 release app 里”。
3. 虽然 slider 文案没有直接出现在 release bundle 中，但 binary 里明确保留了对应的 motion 结构：
   - `CursorMotionPath.startControl`
   - `CursorMotionPath.arc`
   - `CursorMotionPath.arcIn`
   - `CursorMotionPath.arcOut`
   - `CursorMotionPath.endControl`
   - `Animation.SpringParameters(response, dampingFraction)`
4. 目前最合理的说法是：
   - 视频里的 slider 更像是内部调试构建对这些底层几何 / timing 量做的一层调参 UI。
   - shipping binary 里保留的是“固定常量 + 候选表 + 分段逻辑”，不是同名 slider label。

## 证据分层

### 1. shipping bundle phrase scan

对本机 shipping bundle

`~/.codex/plugins/cache/openai-bundled/computer-use/1.0.750/Codex Computer Use.app`

做整包字节扫描后，当前结果是：

- `START HANDLE`: 未命中
- `END HANDLE`: 未命中
- `ARC SIZE`: 未命中
- `ARC FLOW`: 未命中
- `SPRING`: 命中，但属于歧义单词
- `DEBUG`: 命中，但属于歧义单词
- `MAIL`: 命中，但属于歧义单词
- `CLICK`: 命中，但属于歧义单词

这里最关键的是前四项。它们作为完整 phrase 没出现在 shipping bundle 里，所以不能把“release binary 里存在 slider label”当成既成事实。

### 2. motion struct / timing struct 证据

虽然 phrase 没命中，但 `SkyComputerUseService` 的 Swift metadata 和字符串里已经能直接恢复出这批 motion 结构：

- `CursorMotionPath`
  - `start`
  - `end`
  - `startControl`
  - `arc`
  - `arcIn`
  - `arcOut`
  - `endControl`
  - `segments`
- `CursorMotionPathMeasurement`
  - `length`
  - `angleChangeEnergy`
  - `maxAngleChange`
  - `totalTurn`
  - `staysInBounds`
- `Animation.SpringParameters`
  - `response`
  - `dampingFraction`

因此“底层曲线确实可拆成 handle / arc / spring 这些量”已经是 binary-backed 结论；当前没坐实的，是“内部调试 UI 上的 5 个 slider 与这些字段之间的一一映射关系”。

## 当前最合理的 slider 映射

下面这层仍然带 inference，但每一项都尽量只挂到已经确认的 binary 量。

### `START HANDLE`

当前最接近：

- `CursorMotionPath.startControl`
- candidate builder 中的 `startExtent`

在当前 binary-lift 里，`startExtent` 来自一条 piecewise：

```text
48
distance * 0.41960295031576633
640
```

再叠加 bounds clipping 后，写到 `startControl`。

直观影响：

- 主要改变起步阶段“先顺车头方向甩出去多远”。
- handle 更大时，曲线前段更长、更晚才往目标收。
- handle 更小时，起步更快回咬主轴。

### `END HANDLE`

当前最接近：

- `CursorMotionPath.endControl`
- candidate builder 中的 `endExtent`

它控制终点前那一段导向量拉多长，直观上更像“刹车和收尾的手柄长度”。

直观影响：

- 更大时，末段更容易拉出更长的收束钩子。
- 更小时，末段更早贴回目标。

但这项特别容易被 bounds clipping 吃掉，所以某些样例里会看起来“几乎没变”。

### `ARC SIZE`

当前最接近：

- `handleExtent`
- `arcExtent`
- `tableA`
- `tableB`

其中已确认的主尺度是：

```text
handleExtent = piecewise(distance * 0.2765523188064277)
arcExtent = clamp(distance * 0.5783555327868779, 38, 440)
tableA = [0.55, 0.8, 1.05]
tableB = [0.65, 1.0, 1.35]
```

直观影响：

- 更大时，arched family 的 apex 离 chord 更远，曲线更宽、更弯、长度更长。
- 更小时，arched family 更像收紧后的椭圆或浅弧。

### `ARC FLOW`

当前没有恢复出一个独立的 `flow` 字段。

shipping binary 里最接近的固定量是：

```text
arcAnchorBias = guide * (startExtent * 0.65)
```

也就是 arc anchor 会被往 guide 方向推一段，而不是严格落在 chord midpoint 上。

因此当前最保守的说法是：

- `ARC FLOW` 更像在调“apex 沿路径前后偏移多少”，而不是单纯调“弧有多大”。
- 它主要改变的是“最宽的那一段出现在路径更前还是更后”。

这条判断比 `START/END HANDLE` 和 `ARC SIZE` 更弱，因为 shipping binary 当前没有恢复出一个明确叫 `flow` 的独立字段。

### `SPRING`

这项是目前 5 个里 binary 证据最直接的。

cursor move 的 timing 链已经直接确认会走：

- `Animation.SpringParameters(response=1.4, dampingFraction=0.9)`
- `Animation.VelocityVerletSimulation.Configuration(dt=1/240, idleVelocityThreshold=28800)`

所以至少可以确认：

- “spring 确实是 cursor motion 的一等 timing 输入”。
- shipping 默认档的 release 常量就是 `response=1.4`、`dampingFraction=0.9`。

当前没恢复的是：

- 内部 debug UI 那个单一 `SPRING` slider，究竟怎么映射成 `response/dampingFraction` 这对值。

动画库附近还有一个 `0x1005879a4` 的 piecewise remap helper，但当前证据仍指向“cursor move 主链直接用 1.4 / 0.9”，而不是先经过那条 helper。

## 对实际曲线的影响

下面是对两个样例做 `slider-study` 后，最值得记的几条行为结论。

### 样例 A：lab 默认点位

输入：

```text
start = (220, 440)
end   = (860, 260)
bounds = (0, 0, 1120, 760)
```

baseline 选中的仍然是 `base-scaled-guide`，不是 arched family。

因此：

- `START HANDLE`
  - `-25%` 时，chosen path 长度约 `744.1`，最大离 chord 偏移约 `44.6`
  - `+25%` 时，chosen path 长度约 `776.9`，最大离 chord 偏移约 `49.8`
  - 说明它直接影响当前可见主路径
- `END HANDLE`
  - 这组样例里几乎没变
  - 原因不是“binary 里没有 end handle”，而是 `endControl` 已经被 bounds clipping 钉住
- `ARC SIZE`
  - 当前 chosen path 仍然没变
  - 但 best arched candidate 的离 chord 偏移会从约 `90.1` 涨到约 `152.9`
  - 说明这项当前更像在改 arched family 的竞争力和宽度
- `ARC FLOW`
  - 当前 chosen path 仍然没变
  - 但 best arched candidate 的 apex progress 会从约 `0.558` 挪到约 `0.863`
  - 说明它主要在改“最宽的弧出现在更前还是更后”
- `SPRING`
  - baseline endpoint-lock 约 `1.4291667s`
  - `response -15%` 时，endpoint-lock 提前到约 `1.225s`
  - `response +15%` 时，endpoint-lock 延后到约 `1.6375s`

### 样例 B：更居中的 end-handle 样例

输入：

```text
start = (240, 420)
end   = (760, 360)
bounds = (0, 0, 1280, 900)
```

这组更能看出 `END HANDLE` 的实际作用：

- `end_extent -25%`
  - chosen path 长度约 `632.4`
  - 最大离 chord 偏移约 `55.2`
  - `endControl ≈ (899.55, 177.60)`
- `end_extent +25%`
  - chosen path 长度约 `666.4`
  - 最大离 chord 偏移约 `75.1`
  - `endControl ≈ (939.02, 126.0)`

也就是说：

- end-handle 不是“没参与 shipping binary”
- 而是它的可见影响非常依赖当前路径是否先被 bounds clip 住

## 当前最稳的边界

可以直接说的：

- shipping binary 里没有 `START HANDLE` / `END HANDLE` / `ARC SIZE` / `ARC FLOW` 这些完整 label phrase。
- shipping binary 里明确存在 `startControl / arc / arcIn / arcOut / endControl / SpringParameters(response,dampingFraction)`。
- `SPRING` 对 timing 的参与是 direct binary evidence。
- `START HANDLE` / `END HANDLE` / `ARC SIZE` / `ARC FLOW` 当前更像是对 release builder 中固定几何量的一层调试 UI 映射。

还不能直接说死的：

- 5 个 slider 与 shipping binary 内部字段之间已经一一对上。
- `ARC FLOW` 已经恢复到一个独立字段。
- 单一 `SPRING` slider 的 release 映射已经确认一定经过 `0x1005879a4`。

## 可重复命令

查看当前 binary 的 motion 结构与常量：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py inspect --pretty
```

查看 slider 敏感性分析：

```bash
python3 scripts/cursor-motion-re/reconstruct_cursor_motion.py slider-study \
  --start 220 440 \
  --end 860 260 \
  --bounds 0 0 1120 760 \
  --pretty
```
