# Software Cursor Overlay

这个文档聚焦一个很具体的问题：Codex Computer Use 在真实操作时显化出来的光标 overlay，是否是独立的软件光标，而不是直接抢占用户当前硬件鼠标。

结论先写在前面：截至 2026 年 4 月 19 日对 bundled `computer-use` `1.0.750` 的最新复查，最近截图里那支“灰色主体、白边、带阴影”的箭头仍然不能直接在 bundle 里找到一张一一对应的静态 asset；但它现在已经能从运行时 `Software Cursor` window 里直接截出来，而且和截图软件能框到的“大边框小箭头”现象完全一致。也就是说，这支箭头可以复原，但来源不是 `HintArrow`、原始 `SoftwareCursor` PNG 或 `LensSequence` 里的现成图，而更像 `CursorView` / `SoftwareCursorStyle` / `FogCursorViewModel` / `CAShapeLayer` 组合出来的运行时视觉。

## 已观察事实

### 1. 主宿主是一个无 Dock 图标的常驻 agent app

`Codex Computer Use.app/Contents/Info.plist` 可见：

```json
{
  "CFBundleExecutable": "SkyComputerUseService",
  "CFBundleIdentifier": "com.openai.sky.CUAService",
  "CFBundleName": "Codex Computer Use",
  "LSUIElement": 1
}
```

这里的 `LSUIElement = 1` 说明它是典型的 agent-style macOS app，更适合承载状态栏、浮层窗口和后台交互，而不是普通前台应用。

### 2. 主 app 资源里直接存在 cursor 相关资产

对主 app 的 `Assets.car` 做 `assetutil --info`，可以看到这些名字：

```text
Name: "CUAAppIcon_Assets/cursor"
Name: "CUAAppIcon_Assets/cursor dark"
Name: "menubar-cursor"
RenditionName: "menubar-cursor.svg"
```

这说明官方包里不仅有菜单栏图标资源，也有单独命名为 `cursor` / `cursor dark` 的图形资产。

进一步地，这些 asset 现在已经能直接导出为 PNG：

- [appicon-cursor.png](assets/extracted-2026-04-17/appicon-cursor.png)
- [appicon-cursor-dark.png](assets/extracted-2026-04-17/appicon-cursor-dark.png)
- [menubar-cursor.png](assets/extracted-2026-04-17/menubar-cursor.png)

另外，`Package_SlimCore.bundle` 和 `Package_ComputerUse.bundle` 中都能直接导出运行时用的小尺寸 `SoftwareCursor` 资源：

- [software-cursor-slimcore.png](assets/extracted-2026-04-17/software-cursor-slimcore.png)
- [software-cursor-computeruse.png](assets/extracted-2026-04-17/software-cursor-computeruse.png)

两份 `SoftwareCursor` 导出后的 PNG 二进制哈希一致，说明至少当前版本里它们引用的是同一张图。

但 2026 年 4 月 19 日针对 bundled `computer-use` `1.0.750` 的复查也补了一条很重要的纠偏：

- `HintArrow` 确认只是权限引导里的蓝色箭头，不是最近 overlay 截图里的灰白 pointer。
- 直接导出的 `SoftwareCursor` 原图尺寸是 `200x230`，里面混有额外的右侧亮部/附加图层，不能和最近截图里的最终 pointer body 一一对应。
- `LensSequence/` 下那组 `48x48` 序列帧是蓝色 lens 动效，也不是灰白箭头本体。

所以“灰白箭头能直接从 `Assets.car` 挖到一张等价 PNG”这件事，目前并没有被证据支持。

### 3. `SkyComputerUseService` 字符串里出现了完整的光标窗口和动画线索

对 `SkyComputerUseService` 做 `strings`，能直接看到这些符号和日志文案：

```text
cursorWindow
imageView
imageLayer
cursorRadius
cursorScaleAnchorPoint
cursorMotionProgressAnimation
Moving mouse to %s
Start Bezier cursor animation (%{public}s).
Move cursor to (%f, %f) %s animation (%{public}s).
Signal cursor movement completion (%{public}s).
Enable the virtual cursor in Computer Use.
Detach the computer use cursor from the command palette.
```

这组证据比“有个 cursor 资源”更强，因为它已经落到了窗口对象、图层对象和运动动画对象这一层。

继续往下看当前 `1.0.750` 的字符串，还能看到另一组比“单张图片 asset”更关键的渲染侧证据：

```text
SoftwareCursorStyle
FogCursorStyle
FogCursorViewModel
AgentCursor
CursorView
cursorRadius
fogRadius
cursorScaleAnchorPoint
fogScaleAnchorPoint
CAShapeLayer
SkyLensView
isTinted
imageLoadingTasks
currentFrameIndex
animationStartTime
```

这说明当前实现至少不是“把一张 PNG 直接贴上去”这么简单，而更像：

- 有独立的 cursor / fog style 和 view model；
- 至少一部分画面通过 `CAShapeLayer` 或相近的 shape/layer 路径生成；
- 另外一部分视觉特效再叠加 `imageLayer`、`SkyLensView` 和序列帧动画。

### 4. 当前 `1.0.750` 的运行时窗口里仍然有 `Software Cursor`

2026 年 4 月 19 日把官方 service 挂住后，直接按 owner pid 全量枚举 `CGWindowListCopyWindowInfo`，能在当前 bundled `1.0.750` 里看到 `Codex Computer Use` 进程名下至少两个关键窗口：

```text
OwnerName: Codex Computer Use
WindowName: Software Cursor
Layer: 0
Bounds: X=795 Y=353 Width=126 Height=126
IsOnscreen: 1
```

```text
OwnerName: Codex Computer Use
WindowName: Item-0
Layer: 25
Bounds: X=-14336 Y=0 Width=38 Height=37
```

其中：

- `Item-0` 明显像菜单栏状态项窗口。
- `Software Cursor` 是最关键的直接证据，名字已经把职责写出来了。
- 这次不是只停留在窗口元数据层面，而是已经能直接对这个 window 做截图：
  - [official-software-cursor-window.png](assets/extracted-2026-04-19/official-software-cursor-window.png)
  - [official-software-cursor-window-252.png](assets/extracted-2026-04-19/official-software-cursor-window-252.png)
  - [official-software-cursor-window-252-center-crop.png](assets/extracted-2026-04-19/official-software-cursor-window-252-center-crop.png)
  - [official-software-cursor-pointer-raw-crop.png](assets/extracted-2026-04-19/official-software-cursor-pointer-raw-crop.png)

其中两条抓图链路给出的尺寸差异很关键：

- `screencapture -l <windowid>` 和 `CGWindowListCreateImage(..., .bestResolution)` 得到的是 `170x170`，更像“已经裁过 framing/shadow 的可见窗口图”。
- 直接调用 `CGWindowListCreateImage(..., .boundsIgnoreFraming | .bestResolution)` 得到的是 `252x252`，正好等于 `126x126` 逻辑窗口在 `backingScaleFactor = 2.0` 屏幕上的像素尺寸。
- 这张 `252x252` 图里，真正带 alpha 的 fog/body 区域只有 `152x152`，中心亮色 pointer 只有大约 `30x29`，所以视觉上才会出现“边框范围很大，但中间箭头很小”的效果。

这说明“最终视觉不能从 bundle asset 直接导出”和“最终视觉能从运行时 overlay 抠出来”这两件事可以同时成立，而且你截图软件框到的大边框，本质上就是 `Software Cursor` window 的完整 `252x252` 像素边界。

### 5. `Software Cursor` 会随着一次次工具动作改变位置

对 Finder 连续做两次安全点击后，再次枚举窗口，`Software Cursor` 的坐标发生了变化：

第一次：

```text
WindowName: Software Cursor
Bounds: X=1949 Y=696 Width=126 Height=126
```

第二次：

```text
WindowName: Software Cursor
Bounds: X=1949 Y=796 Width=126 Height=126
```

这说明它不是静态装饰，而是一个会被操作事件驱动移动的独立窗口。

### 6. 当前 `1.0.750` 的 app-server 返回图里仍然看不到 cursor，但窗口本身是能抓到的

2026 年 4 月 19 日又补做了一次同线程实测：

- 直接通过 `codex app-server` 在同一个 ephemeral thread 里串联 `get_app_state` 和 `click`，避免 CLI 每次新开 thread 导致的“Finder 未激活”误差。
- `click` 返回的结果图里看不到可见 cursor，说明这支箭头不是普通 `get_app_state` / `click` 返回图像里 baked 进去的内容。
- 一开始对 `CGWindowListCopyWindowInfo` 做高频差分时，只看到了 `Item-0` 这类小窗，没有第一时间把 `Software Cursor` 本体筛出来。
- 但把 service 挂住后改成按 owner pid 全量枚举，`Software Cursor` 窗口又能在当前 `1.0.750` 里直接看到，并且可以被 `screencapture -l <windowid>` 单独截出。
- 再往下一层直接调旧版 `CGWindowListCreateImage` 符号时，还能看到一个更准确的分层：默认抓图链只给出 `170x170` 的裁剪结果，而 `boundsIgnoreFraming + bestResolution` 会返回完整的 `252x252` runtime overlay。

这次把前面“当前版本没有稳定复现出同名窗口”的说法收紧成了更准确的表述：

- 普通 `click` / `get_app_state` 返回图像里看不到这支箭头；
- 但当前版本下，`Software Cursor` 作为独立 window 仍然存在，只是如果只做“新窗口差分”而不按 pid 全量扫，很容易漏掉它；
- 因此“host 侧可能参与了一部分合成”仍然成立，但“service 侧已经没有独立 software cursor window”这个结论当前不成立。

### 7. 2026 年 4 月 20 日补做 tool 级触发矩阵后，可以确认 overlay 不是只绑定 `click`

这轮又补做了一次更细的官方路径实测，重点不是“有没有 `Software Cursor`”，而是“哪些 public tool 会把它真正拉出来”。

这里有一个前提很重要：

- 这台机器的日常 Codex 用户态配置里，官方 bundled `computer-use@openai-bundled` 默认是关的，而本地 `open-computer-use` MCP 是开的。
- 所以这次所有样本都先切到一个隔离的临时 `HOME`，只启用官方 bundled `computer-use`，再通过 `codex app-server` 走同一条 signed host 路径。
- 否则很容易把本地开源实现和官方闭源实现混在一起，得出错误结论。

在这个前提下，对 `1.0.750` 做同线程 tool call，并同时抓 `SkyComputerUseService` 日志与 `CGWindowList` 后，结论可以收成下面这张表：

| Tool | 2026-04-20 结果 | 观察到的直接证据 |
| --- | --- | --- |
| `set_value` | 会触发 | 命中 `Prepare to interact with element ...`、`Move to location ...`、`Move cursor to ...`；对 `Activity Monitor` 搜索框样本还能看到 `Start Bezier cursor animation ...` |
| `scroll` | 会触发 | 命中 `Prepare to interact with element 1`、`Move cursor to ...`、`Start Bezier cursor animation ...`、`Moving mouse to ...` |
| `drag` | 会触发 | 命中 `Move cursor to ...`、`Start Bezier cursor animation ...`、`Moving mouse to ...`、`Dragging from ... to ...` |
| `perform_secondary_action` | 会触发 | 对 `TextEdit` window 的 `Raise` 样本命中 `Prepare to interact with element 0` 和 `Move cursor to ...` |
| `click` | 分路径 | 坐标点击会命中 `Move cursor to ...`、`Start Bezier cursor animation ...`、`Moving mouse to ...`、`Clicking at ...`；但 element-scoped `click(element_index)` 不一定会 |
| `type_text` | 这轮未触发 | 前台 `TextEdit`、前台 `Activity Monitor`、以及“Finder 在前台但向后台 `TextEdit` 投递文字”三组样本里，都没看到 `Computer Use Cursor` 日志，也没枚举到 `Software Cursor` window |
| `press_key` | 这轮未触发 | 前台 `TextEdit` 的 `Return`，以及“Finder 在前台但向后台 `TextEdit` 投递 `Return`”两组样本里，都没看到 cursor motion 日志 |
| `get_app_state` | 未触发 | 没有看到 `Computer Use Cursor` 相关日志 |
| `list_apps` | 未单独重放，但静态上不支持它会触发 | 当前没有任何与 cursor motion 相关的运行证据 |

其中有三条更关键的收敛：

- `set_value` 确认不是纯“AX 直接赋值然后结束”这么简单。至少在部分控件上，它会先走一段 element preparation，再把软件光标移动到目标附近。
- `type_text` 和 `press_key` 当前更像纯键盘注入路径。尤其在“目标 app 不在前台”的样本里，`TextEdit` 文本内容确实被改动了，但整个时间窗里仍然没有 `Software Cursor` window，也没有任何 `Computer Use Cursor` motion 日志。
- `click` 需要拆成两类看：坐标点击更像“真实鼠标模拟”，而 element-scoped `click` 有时会走更偏 Accessibility action 的路径，因此不一定显出 overlay。

这一点和二进制里的 feature flag 也对得上：

- `feature/computerUseCursor`
- `feature/computerUseAlwaysSimulateClick`
- `Prefer simulating physical clicks over Accessibility actions.`

也就是说，官方实现更像是“按这次 interaction 最终走的是哪条执行链”来决定是否出 overlay，而不是简单按 public tool 名字一刀切。

### 8. 2026 年 4 月 22 日复查：cursor 位置状态不会在每次 interaction 后立刻销毁

这次针对 bundled `computer-use` `1.0.755` 又做了一次静态 + 运行时复查，重点看“多次 `click` / `set_value` 之间是否应该从 fresh `(0,0)` 重来”。

静态上，`SkyComputerUseService` 的 Swift metadata 仍然能恢复出同一组状态字段：

```text
ComputerUseCursor
  isMoving
  shouldFadeOut
  window
  style
  activityState

ComputerUseCursor.Window
  wantsToBeVisible
  cursorMotionProgressAnimation
  cursorMotionNextInteractionTimingHandler
  cursorMotionCompletionHandler
  cursorMotionDidSatisfyNextInteractionTiming
  currentInterpolatedOrigin
  useOverlayWindowLevel
  correspondingWindowID
```

这组字段的关键点是：`wantsToBeVisible` / `shouldFadeOut` 这类可见性状态，和 `currentInterpolatedOrigin` 这个位置状态是分开的。也就是说，官方结构上并不是“淡出或一次动作结束就把 cursor origin 清空”。

运行时日志也支持这个判断。同一轮官方 service 中，连续多次 `Move cursor to ...` / `Start Bezier cursor animation ...` / `Signal cursor movement completion ...` 之间，AppKit 反复把同一个 `ComputerUseCursor.Window` 对象对应的同一个 window number `orderFront`。最后一次 movement completion 后，约 5 分钟才出现 `Codex Computer Use idle timeout reached; terminating service`。

因此当前更准确的生命周期模型是：

- fresh service / fresh cursor window 初始化时，`currentInterpolatedOrigin` 和 window origin 从 `(0,0)` 起步。
- 每次 action move 会更新同一个 `ComputerUseCursor.Window` 的插值 origin 和 style 状态。
- movement completion 后 cursor 进入 idle / resting 姿态，并继续保留当前位置，短间隔的下一次 interaction 从当前 visible cursor 继续移动。
- task / turn 结束会通过 `turn-ended` lifecycle hook 触发清理，cursor 应该消失。
- `(0,0)` 应该是 fresh session / service reset 语义，不应该在每个 `click` 或 `set_value` 收尾后重放。

## 当前推断

### 1. 黄色小鼠标是软件光标，不是系统硬件光标

当前最合理的解释是：

- 真正的系统事件仍然由 automation / event tap 路径发给目标 app。
- 用户看到的黄色小鼠标，是 `SkyComputerUseService` 自己渲染的一层“软件光标”。
- 这层软件光标单独运动、单独做动画，因此视觉上能模拟“鼠标在动”，但不需要真的抢走用户平时看到的系统箭头。

### 2. 它现在仍然是独立 overlay window，但最终视觉不是静态 asset

当前结论成立的原因有三点：

- 字符串里直接出现 `cursorWindow`。
- 当前 `1.0.750` 的运行时 `CGWindowList` 仍然能枚举到命名明确的 `Software Cursor` 窗口。
- 这个窗口 owner 是 `Codex Computer Use`，而不是被控 app。

所以更稳妥的说法应该是：

- 官方现在仍然有独立 software cursor / overlay window；
- 这层 window 由 service 维护，而不是目标 app 内部插入的绘制层；
- 但 window 里的最终灰白箭头，不等于 bundle 里某张可直接导出的 PNG。

### 3. 当前灰白箭头能从运行时窗口抠出，但仍更像代码/图层组合，而不是可直接导出的 asset

这一点目前仍带推断，但方向已经比“直接来自 `SoftwareCursor` 图片”更明确：

- `HintArrow`、`SoftwareCursor`、`LensSequence` 都和最近截图里的灰白箭头对不上。
- 二进制里不仅有 `imageView` / `imageLayer`，还有 `SoftwareCursorStyle`、`FogCursorViewModel`、`CursorView`、`CAShapeLayer`、`cursorRadius`、`fogRadius` 这一整套更偏“运行时渲染”的命名。
- 官方 `click` 返回截图里看不到这支箭头，但单独截 `Software Cursor` window 又能把它抓出来，说明它确实存在于独立 overlay 合成链里，而不在普通 screenshot 返回链路里。
- 这支箭头所在的完整 runtime overlay 边界是 `252x252`，其中大量区域只是透明 padding 和半透明 fog，不是单独放在 bundle 里的“现成 cursor 贴图”。

所以当前更像“独立 window + 部分素材 + 代码绘制 + host/service 合成”的运行时视觉，而不是一张可以从 `Assets.car` 直接导出的最终 pointer。

### 4. 仓库里已经补了一条独立的合成验证脚本

为了避免后续每次都得重新挂住官方 service 才能肉眼校准，这轮也额外补了一条单文件 Swift 验证入口：

```bash
swift scripts/render-synthesized-software-cursor.swift --seconds 12
```

这条脚本现在分成两档：

- 默认档：直接读取仓库里已经落下来的 `official-software-cursor-window-252.png`，把这张 `252x252` runtime overlay 基线图独立渲染到一个 `126x126` 透明 window 上，并只额外叠一层“中心固定、像钟摆一样左右摆角”的 angle wobble。当前独立脚本把这段 wobble 收到接近“时钟 `55` 分到 `00` 分”的总摆幅，用来先把“独立测试”和官方视觉姿态做准。
- `--procedural` 档：继续保留一版纯代码 fallback，用 radial fog + pointer contour 去近似官方视觉，方便后面单独迭代 pointer path、fog falloff 和按压状态。

也就是说，这条脚本当前优先服务“可独立验证的官方基线”，而不是宣称 procedural 版本已经 1:1 复刻完成。

这条 idle wobble 后来又按二进制证据收紧过一轮：

- `CursorView` 一侧目前能直接确认的是 `cursorRadius`、`_animatedAngleOffsetDegrees`、`_loadingAnimationToken`、`fogRadius` 和两套 scale anchor；
- `FogCursorViewModel` 一侧当前能确认的是 `_velocityX`、`_velocityY`、`_isPressed`、`_activityState`、`_isAttached`、`_angle`；
- 所以独立脚本默认档现在不再对整张 runtime baseline 图做“呼吸式整体缩放”或额外平移，而是只保留一个以图案中心为轴的小幅 angle offset。

如果需要抓不同 wobble 相位的静态样本，可以直接加：

```bash
swift scripts/render-synthesized-software-cursor.swift \
  --seconds 1.4 \
  --snapshot-delay 0.9 \
  --save-png /tmp/software-cursor-wobble.png
```

## 为什么这套体验不会抢用户鼠标

如果这个判断成立，那么体验上“看起来像有一个额外的小黄鼠标在代操作，但用户自己的真实鼠标仍然自由”就很好解释了：

- automation 层负责发真实点击和移动事件；
- overlay 层只负责给人看；
- 两者在视觉上对齐，但逻辑上分离。

这比直接操纵系统可见光标更稳，也更容易做出平滑路径、点击高亮和 delay 控制。

## 更细一层的实现推断

继续看 `ComputerUseCursor.Window` 的静态分析后，可以再补两条对开源实现很关键的推断。

### 1. 官方不仅调 window level，还会绑定具体 target window id

从 `ComputerUseCursor.Window` 的 ivar 和辅助函数可以看到：

- 它同时保存了 `useOverlayWindowLevel` 和 `correspondingWindowID`。
- helper `0x10005d650` 会在 `useOverlayWindowLevel` 开启时检查 `correspondingWindowID` 是否还存在于当前窗口列表里。
- helper `0x10005d7bc` 在目标 window 仍有效时，会继续把 cursor window 排到对应 window 之上；目标失效时会清掉当前动画状态并回退到普通前置排序。

这说明官方做的不是“把 overlay 永远设到某个固定 level”，而是“尽量相对某个具体 window 维持排序；一旦目标 window 消失或失效，再切回兜底行为”。

### 2. 官方在生成/接受轨迹前，会做窗口命中检查

`ComputerUseCursor.Window` 的大方法里出现了一组很有辨识度的名字：

- `distanceThreshold`
- `closeEnough`
- `control1`
- `control2`
- `staysInBounds`

对应 helper `0x10005c388` 的行为也很说明问题：

- 它会读取 `correspondingWindowID`。
- 用当前候选点坐标去做 window hit-test。
- 把命中的 window number 和 `correspondingWindowID` 比较。
- 只有在关键采样点仍然落在目标 window 上时，才接受这组路径/控制点。

换句话说，官方那条 Bezier 不是“先固定算出来再硬播”，而是带有一层 target-window-aware 的约束判断，避免鼠标视觉轨迹明显飘出被控窗口。

## 还没完全确认的点

- 还没有抓到一次动画进行中的连续窗口内容变化，目前手里还是静态帧和离散位置点。
- 当前 `get_app_state` / `click` 返回的截图里通常看不到这个光标，推测截图采样路径和 overlay 合成路径并不完全相同。
- 最近截图里那支灰白 pointer 的最终 body / shadow / fog 到底是哪一层在 host 侧合成，仍然需要继续往 `CursorView` 或 `Codex.app` host path 深挖。

## 对开源实现的启发

如果后续要做 `open-computer-use` 的可用体验，这条发现很有价值：

- 可以把“真实输入注入”和“软件光标可视化”明确拆开。
- 软件光标可以做成独立 overlay window，而不是强依赖真实鼠标位置。
- 如果手上有 `windowID`，Bezier 候选最好先做一轮窗口命中采样，再决定用哪组 `control1` / `control2`。
- overlay 最好维护“相对目标 window 的排序”和“目标 window 是否还活着”的状态，而不是只在动画开始时排一次层级。
- 事件执行即使失败，overlay 也可以保持可观测，有利于调试和用户信任。
- 菜单栏状态项和软件光标都放在同一个 `LSUIElement` agent app 里，是一条已经被官方实现验证过的产品形态。
