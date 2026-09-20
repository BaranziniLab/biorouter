# Visual Cursor Overlay

## 用户诉求

用户希望优化 `click` 的观感，对齐官方 `computer-use` 那种“不是抢真实鼠标，而是自己显化一个 cursor overlay 给用户看”的体验，并特别指出：

- 空闲时 cursor 会轻微晃动；
- 移动时会沿弧线前进并带一点方向变化；
- 点击时不应因为这层可视化而重新激活目标 app 或抢走当前焦点。

用户还提供了本地录屏 `00:59 ~ 01:03` 作为观察样本，并让这轮顺手结合官方 `Codex Computer Use.app` 与仓库内现有逆向资料一起分析。

## 这次改了什么

- **[MCP runtime]**：给 `mcp` 模式补了一个最小 AppKit runtime。启用 visual cursor 时，主线程保持 event loop 以承载 overlay UI，stdio server 仍然在后台线程按原样跑。
- **[软件 cursor overlay]**：新增 `SoftwareCursorOverlay`，用透明 `NSPanel` 画一个独立的软件 cursor，并支持曲线路径移动、点击 pulse、点击后 idle sway，以及 `OPEN_COMPUTER_USE_VISUAL_CURSOR=0` 显式关闭。
- **[点击路径接线]**：`ComputerUseService.click` 现在会在执行真实动作前先移动 visual cursor，再继续走原有 AX 优先 / HID fallback 策略；动作完成后再做 pulse 和停驻，不因为加了可视化而改变底层点击决策。
- **[测试与脚本]**：为 visual cursor 增加了环境变量和几条纯逻辑单测；smoke 脚本里显式关闭 overlay，避免 UI 动画影响回归稳定性。
- **[文档同步]**：更新了 `README.md`、`docs/ARCHITECTURE.md` 和执行计划，明确开源版现在已经有一层 click visual cursor，但仍未复刻官方完整 choreography。

## 设计动机

官方 bundle 和既有逆向文档已经足够说明，那个给用户看的 pointer 大概率是独立 `Software Cursor` window，而不是系统真实鼠标。对开源版来说，最合理的收敛方式不是继续把所有东西都塞进 HID 事件路径里，而是明确把“真实动作执行”与“给用户看的可视层”拆开：

- 真实动作继续优先走 AX，尽量不抢焦点。
- visual cursor 只负责解释“现在在点哪里、准备点什么”。
- 即使底层仍有 HID fallback，这层可视化也能先把产品观感和可观测性往官方方向推一大步。

## 关键文件

- `apps/OpenComputerUse/Sources/OpenComputerUse/MCPAppRuntime.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `README.md`
- `docs/ARCHITECTURE.md`

## [2026-04-17 19:52] | Follow-up: 缩小 cursor 并按目标 window 排序

### 额外改动

- **[官方 asset fallback]**：继续沿用同一份 `SoftwareCursorOverlay`，但 visual style 现在会优先在运行时从本机官方 `Codex Computer Use.app` 的 bundle 里读取 `SoftwareCursor` 资产，再做一层本地处理后用于绘制；这样既能复用官方素材，又不用把闭源资产直接 vendor 进仓库。
- **[尺寸和运动调参]**：把 cursor 的默认显示尺寸明显收小，并把首次入场偏移和移动时长拉长，避免“已经有 cursor 但看起来几乎没动”的观感。
- **[相对目标窗口排序]**：给 `AppSnapshot` 补了目标 `windowID` / `layer`，overlay 不再固定 `.floating` 置顶；现在会尽量排在目标 window 之上，但保留在更高层前台窗口之下，更接近官方“操作 A 时 cursor 不会穿透到 B 前面”的行为。
- **[动作后自动隐藏]**：overlay 不再无限期停在屏幕上。点击完成或异常 settle 后，只保留一小段停驻和轻微 sway，随后自动淡出并 `orderOut`，下次动作再重新出现。

### 为什么这么做

用户指出两个很具体的问题：cursor 太大，以及当用户正停在 B 应用上时，操作 A 的 cursor 不该压到 B 前面。前者说明第一版 overlay 还停留在“证明这条链路能跑通”的阶段，尺寸和素材选择都太粗；后者说明 window ordering 不能只看“要不要可见”，还要看“应该压在哪层”。这个 follow-up 的重点，就是把 visual cursor 从“有”推进到“手感更合理”。
