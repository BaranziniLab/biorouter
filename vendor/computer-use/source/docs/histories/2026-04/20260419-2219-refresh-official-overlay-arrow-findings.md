## [2026-04-19 22:19] | Task: 刷新官方 overlay 箭头结论

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `gpt-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 继续从官方 `Codex Computer Use.app` 里往下挖，确认最近截图里那支灰色白边带阴影的箭头到底是不是 bundle 里的某个静态 asset；用户明确指出它不是此前导出的 `SoftwareCursor` 或 `HintArrow`，要求改查 overlay 渲染链。

### 🛠 Changes Overview
**Scope:** `docs/references/codex-computer-use-reverse-engineering/`、`docs/histories/`

**Key Actions:**
- **[补做当前版本纠偏]**: 重新核对 bundled `computer-use` `1.0.750` 里的 `HintArrow`、`SoftwareCursor`、`LensSequence` 和当前截图中的灰白箭头，确认三者都不能直接等同于最终 pointer。
- **[补强渲染侧证据]**: 基于 `SkyComputerUseService` 的字符串，补记 `SoftwareCursorStyle`、`FogCursorViewModel`、`CursorView`、`CAShapeLayer`、`SkyLensView`、`currentFrameIndex` 等命名，把结论从“像某张图片 asset”收敛到“更像代码/图层组合渲染”。
- **[补做同线程 app-server 实测]**: 通过同一个 `codex app-server` thread 串联 `get_app_state` 和 `click`，确认 `click` 返回截图中看不到该箭头；随后把官方 service 挂住并按 owner pid 全量枚举，重新在当前 `1.0.750` 中抓到 `Software Cursor` 命名窗口。
- **[落运行时样本]**: 直接对 `Software Cursor` window 做 `screencapture -l <windowid>`，把完整窗口和原始箭头 crop 落到 `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-19/`。
- **[补抓完整 2x overlay 边界]**: 继续直接调用 `CGWindowListCreateImage(..., .boundsIgnoreFraming | .bestResolution)`，确认 `Software Cursor` 的完整运行时像素边界是 `252x252`，而不是默认抓图链里看到的 `170x170` 裁剪结果；这和用户截图软件框到的大边框一致。
- **[补独立测试脚本]**: 新增 `scripts/render-synthesized-software-cursor.swift`，用单文件 AppKit 脚本把 `126x126` overlay 独立渲染到屏幕。默认模式直接读取仓库里保存的 `official-software-cursor-window-252.png` 作为官方基线图，确保尺寸和轮廓先对上；`--procedural` 继续保留纯代码 fallback，单独迭代 fog 和 pointer 近似实现。
- **[补默认档晃动效果]**: 继续在独立脚本里给默认 reference-baseline 模式接入轻微的 center-fixed angle wobble，并补 `--snapshot-delay`，方便直接导出不同时间相位的独立样本。
- **[按二进制证据收紧 wobble]**: 随后又重新对齐仓库里的逆向文档和 `SkyComputerUseService` 的 runtime 证据，确认 `CursorView` 侧能直接看到的是 `_animatedAngleOffsetDegrees` / `_loadingAnimationToken`，`FogCursorViewModel` 侧是 velocity / pressed / activity / angle；因此把默认档从“整图平移 + 呼吸缩放 + pulse”收紧回“中心固定、顺时针/逆时针轻微摆角”的小幅 angle wobble。
- **[调整中心摆角振幅]**: 根据后续对官方视觉的回看，又把默认档的中心摆角调到接近“时钟 `55` 分到 `00` 分”的总摆幅，让独立脚本更接近用户观察到的钟摆式旋转范围。
- **[更新逆向文档]**: 修改 `software-cursor-overlay.md`，把结论收敛成“最终灰白箭头不能从 bundle 静态 asset 直接导出，但能从运行时 `Software Cursor` window 直接截出”。

### 🧠 Design Intent (Why)
这次不是继续扩大猜测范围，而是为了把仓库里的说法修到和证据一致。此前文档已经证明官方存在独立 software cursor / overlay 渲染链，但如果继续把“最近截图里的灰白箭头”直接等同于导出的 `SoftwareCursor` asset，或者继续把 `screencapture -l` 抓到的 `170x170` 小图当成完整边界，都会误导后续逆向方向。把结论更新成“静态 asset 不对，运行时 `Software Cursor` window 还在，而且完整像素边界其实是 `252x252`，只是在默认抓图链里被裁掉了 framing/padding”之后，后续工作就能更明确地围绕 `CursorView`、`SoftwareCursorStyle`、`FogCursorViewModel` 和 host/service 合成边界继续挖。再把一条独立 Swift 脚本单独落仓库，并把默认渲染收敛到“直接展示官方 `252x252` runtime baseline、`--procedural` 再单独试代码近似”，就能先把独立测试入口稳定下来，再继续拆 procedural 复刻。

### 📁 Files Modified
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `scripts/render-synthesized-software-cursor.swift`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-19/official-software-cursor-window.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-19/official-software-cursor-window-252.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-19/official-software-cursor-window-252-center-crop.png`
- `docs/references/codex-computer-use-reverse-engineering/assets/extracted-2026-04-19/official-software-cursor-pointer-raw-crop.png`
- `docs/histories/2026-04/20260419-2219-refresh-official-overlay-arrow-findings.md`
