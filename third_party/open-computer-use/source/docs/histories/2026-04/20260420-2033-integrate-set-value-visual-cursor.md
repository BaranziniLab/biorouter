## [2026-04-20 20:33] | Task: 集成 `set_value` visual cursor 移动链路

### 🤖 Execution Context
* **Agent ID**: `primary`
* **Base Model**: `gpt-5.4`
* **Runtime**: `Codex CLI on macOS + SwiftPM + Go`

### 📥 User Query
> 现在我们 Cursor Motion 已经具备了计算曲线的能力，要集成到 `open-computer-use` 里；范围先收窄到 `click` 和 `set_value`，并希望补一条可直接通过 MCP 触发的 `TextEdit` 对比序列，方便和官方 bundled `computer-use` 平行观察效果。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit/`、`packages/OpenComputerUseKit/Tests/`、`scripts/computer-use-cli/`、`docs/`

**Key Actions:**
- **[`set_value` 接入 visual cursor]**: 在 `ComputerUseService` 里新增 `VisualCursorTarget` 与目标点解析 helper，让真实 app 模式下的 `set_value` 在执行 `AXUIElementSetAttributeValue` 前先跑 `SoftwareCursorOverlay.moveCursor(...)`，成功和失败都统一走 `settle` 收尾。
- **[统一 click/set_value 内部目标解析]**: `click` 复用同一套 visual cursor target 表达，避免继续把 overlay point / window 组装逻辑散落在分支里，同时保持 `type_text`、`press_key`、`scroll`、`drag` 的现有行为不变。
- **[补测试与可复现样例]**: 新增 `makeVisualCursorTarget(...)` 单测；补 `scripts/computer-use-cli/examples/textedit-set-value-click-raise-seq.json`，并在 README 里写明如何把同一份 calls file 同时指向官方 app-server 和本地 `OpenComputerUse` direct MCP。
- **[同步架构文档]**: `docs/ARCHITECTURE.md` 更新为“`click` / `set_value` 都会驱动 visual cursor move”，并明确两者的收尾差异是 `click pulse` vs `settle only`。

### 🧠 Design Intent (Why)
这次不追求把所有动作型 tools 都挂上 overlay，而是先收敛到用户已经确认最需要对比的两条路径：`click` 和 `set_value`。`click` 已有官方风格的 motion / pulse 链路，`set_value` 缺的是“先把视觉 cursor 移到目标区域，再做值写入”的最小补齐。把目标点与 target window 封成可复用 helper 后，既能让 `set_value` 复用当前主线已经验证过的 cursor motion 参数，也能避免未来继续复制同类 glue code。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/computer-use-cli/examples/textedit-set-value-click-raise-seq.json`
- `scripts/computer-use-cli/README.md`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-20, fix non-intrusive `click 0` for window/root elements)

- **[官方行为复查]**: 重新对官方 bundled `computer-use` 跑 `TextEdit` 的 `click element_index=0` 样本，并配合 `SkyComputerUseService` unified log 过滤，确认官方日志链路是 `Prepare to interact with element 0` -> `Finished preparing interaction with element 0` -> `Dispatch click to element 0`；没有出现本仓库先前那种必须退回全局 pointer 才能完成的迹象。
- **[修正本地 click 决策]**: `ComputerUseService.performPreferredClick` 不再只会试 `AXPress` / `kAXFocusedAttribute` / `AXConfirm`；现在会把窗口/根元素常见的 `AXRaise`、`kAXMainAttribute`、`kAXFocusedAttribute` 都纳入 element-targeted 左键路径，只有这些都失败才允许 `clickGlobally(...)`。
- **[移除过严 settable 门槛]**: 针对 `TextEdit` window 这类 `AXUIElementIsAttributeSettable(kAXFocusedAttribute) == false`、但直接 `AXUIElementSetAttributeValue(..., true)` 仍成功的场景，`click` 相关的布尔属性写入不再把 `isSettable` 当作硬前置条件。
- **[补 fallback tracing]**: 新增默认关闭的 `OPEN_COMPUTER_USE_DEBUG_INPUT_FALLBACKS` 环境变量；打开后，只有真的命中全局 pointer fallback 才会往 stderr 打一行调试信息，便于继续做官方/本地 A/B。
- **[验证结果]**: 打开 `OPEN_COMPUTER_USE_DEBUG_INPUT_FALLBACKS=1` 后，当前 `examples/textedit-overlay-seq.json` 这条本地 direct MCP 序列已不再打印 `global pointer fallback`，同时 `swift test` 全绿。

**Follow-up Files:**
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-20, align runtime cursor glyph with `CursorMotion`)

- **[移除 bundle 小图裁剪链]**: `SoftwareCursorOverlay` 不再扫描官方 `Codex Computer Use.app` bundle 里的 `SoftwareCursor` 资源再本地裁剪；这条路径和当前仓库对官方 overlay 的逆向结论不一致，也会让主 runtime 和 `CursorMotion` 各自维持不同的 glyph 形状。
- **[抽共享程序化 glyph renderer]**: 在 `OpenComputerUseKit` 里新增共享 `SoftwareCursorGlyphRenderer`，把 `CursorMotion` 当前那套灰色 pointer + 白边 + fog 的程序化绘制逻辑、`126x126` 画布尺寸和 tip-anchor 标定收敛成同一份实现。
- **[主 runtime 切到同款 cursor]**: `SoftwareCursorView` 现在直接调用共享 renderer，`click` / `set_value` 的 visual cursor 都会使用和 `CursorMotion` 同款的程序化 glyph，而不是旧的渐变三角指针或 bundle 裁出来的小图。
- **[补共享校准测试]**: 新增一条单测，固定 `126x126` 画布和 `60.35 x 70.3` 的 tip-anchor 标定，避免后续再次把主 runtime 的命中点校准漂走。

**Follow-up Files:**
- `Package.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorGlyphRenderer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `experiments/CursorMotion/Sources/CursorMotion/SynthesizedCursorGlyphView.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-20, remove PNG-first glyph split and re-isolate `CursorMotion`)

- **[撤回 `CursorMotion` 的 PNG-first glyph]**: 继续对照当前实现后，确认 `CursorMotion` 仍然优先加载 `official-software-cursor-window-252.png`，这和用户要求的“代码绘制 cursor”不一致；现在 lab 已改成固定使用程序化 pointer/fog glyph，不再依赖这张 PNG。
- **[解除实验线对主 runtime 的直接依赖]**: 上一轮为了复用 glyph renderer 临时让 `CursorMotion` 直接依赖了 `OpenComputerUseKit`，这和仓库里一直强调的“实验线独立于主 MCP runtime”边界相冲突；现在已把 renderer 抽到中立 `SoftwareCursorGlyphKit` target，由 runtime 和 lab 共同依赖。
- **[清理打包脚本的过时资源链]**: `scripts/build-cursor-motion-dmg.sh` 不再要求把 `official-software-cursor-window-252.png` 复制进 `.app` bundle，因为 packaged `CursorMotion` 已经不再从 bundle 读取这张图。
- **[同步当前文档]**: README、架构说明和 active execution plan 都更新成“程序化 glyph + 中立 target 共享”的最终状态，避免继续留下 PNG-first 和 `CursorMotion -> OpenComputerUseKit` 两套互相冲突的说法。

**Follow-up Files:**
- `Package.swift`
- `packages/SoftwareCursorGlyphKit/Sources/SoftwareCursorGlyphKit/SoftwareCursorGlyphRenderer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `experiments/CursorMotion/Sources/CursorMotion/SynthesizedCursorGlyphView.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/build-cursor-motion-dmg.sh`
- `experiments/CursorMotion/README.md`
- `docs/ARCHITECTURE.md`
- `docs/exec-plans/active/20260418-standalone-cursor-lab.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-20, restore `CursorMotion` and scope glyph reuse to runtime)

- **[恢复 CursorMotion 边界]**: 用户明确反馈 `CursorMotion` 本身已经正常，不应被主 runtime 集成改动牵连；当前已撤回 `CursorMotion -> SoftwareCursorGlyphKit` 的 package 依赖，恢复它原来的独立 target 形态。
- **[恢复 PNG-first lab glyph]**: `SynthesizedCursorGlyphView` 重新回到“优先读取官方 `official-software-cursor-window-252.png`，缺失时再使用本地 procedural fallback”的实现，DMG 打包脚本也继续把这张参考图复制进 bundle。
- **[runtime 内部复刻即可]**: `click` / `set_value` 需要的程序化 glyph renderer 只保留在 `OpenComputerUseKit` 内部，作为主 MCP runtime overlay 的实现细节；不再新增中立共享 target，也不要求 `CursorMotion` 复用 runtime 代码。
- **[同步文档口径]**: README、架构说明和 active execution plan 都改回“CursorMotion 独立、PNG-first；OpenComputerUseKit 自己参考 CursorMotion fallback 绘制”的边界，避免继续误导后续改动。

**Follow-up Files:**
- `Package.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorGlyphRenderer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `experiments/CursorMotion/Sources/CursorMotion/SynthesizedCursorGlyphView.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/build-cursor-motion-dmg.sh`
- `experiments/CursorMotion/README.md`
- `docs/ARCHITECTURE.md`
- `docs/exec-plans/active/20260418-standalone-cursor-lab.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-21, fix runtime cursor drawing direction)

- **[修正 runtime heading 基线]**: `SoftwareCursorOverlay` 的 resting heading 改回与 `CursorMotion` 一致的 y-down 坐标基线 `-3π/4`，避免 `TextEdit` 这类直接使用截图 / CGWindow 坐标的工具序列在纵向运动时出现可见箭头方向镜像。
- **[补 AppKit 绘制转换]**: `SoftwareCursorGlyphRenderer` 继续接收 screen-space motion state，但在实际 AppKit 绘制前把 `rotation` 和 `dy` 转成 y-up drawing state，复刻 `CursorMotion` 里 `drawingAngle` / `drawingVector` 的边界。
- **[保持 CursorMotion 不动]**: 本次只改 `OpenComputerUseKit` runtime，未修改 `experiments/CursorMotion` 的 demo 实现。
- **[补单测与文档]**: 新增 drawing-state 转换单测，并在架构文档里写明 runtime motion 坐标与 AppKit drawing 坐标的转换点。

**Follow-up Files:**
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorGlyphRenderer.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`

### 🔁 Follow-up (2026-04-21, align procedural pointer neutral axis)

- **[修正侧边朝前]**: 用户继续反馈 runtime cursor 仍然不是 tip 朝前，而是侧边在前；复查后确认 motion rotation 语义已经对齐 `CursorMotion`，偏差来自 runtime 程序化 pointer contour 自身的 neutral 轴不等于官方 baseline 的 `-3π/4`。
- **[加 artwork neutral correction]**: `SoftwareCursorGlyphRenderer` 在 pointer 自身中心上额外应用固定 artwork correction，把 procedural contour 的天然轴校准到 `CursorMotion` / 官方 baseline 的 forward 方向；motion 层的 heading / path 逻辑保持不变。
- **[保持实验线不动]**: 本次仍只改 `OpenComputerUseKit` runtime，没有修改 `experiments/CursorMotion`。
- **[补单测与文档]**: 新增 corrected neutral heading 单测，并同步架构文档，明确这是 runtime 内部的程序化轮廓校准。

**Follow-up Files:**
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorGlyphRenderer.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260420-2033-integrate-set-value-visual-cursor.md`
