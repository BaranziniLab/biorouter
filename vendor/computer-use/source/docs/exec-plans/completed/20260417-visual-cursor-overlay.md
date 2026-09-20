# Visual Cursor Overlay

## 目标

让 `open-computer-use` 在 `click` 路径上具备一套可见的软件 cursor overlay：点击前能沿曲线移动到目标点、点击时有明确的视觉反馈、点击后可以短暂停留并做轻微 idle sway，随后自动隐藏，同时继续保持当前 AX 优先、尽量不抢焦点的执行策略。

## 范围

- 包含：
- 为 `mcp` 模式补一个可承载 AppKit overlay 的主线程 runtime。
- 实现独立透明窗口的软件 cursor overlay。
- 把 `click` 路径接到 overlay，但不改变现有 AX 优先 / HID fallback 的决策。
- 增加必要测试，并同步 README、架构文档和 history。
- 不包含：
- 本轮不复刻官方闭源 cursor 的私有素材、完整 choreography 或私有事件注入链路。
- 本轮不把 drag、scroll、键盘输入全部接入统一 overlay。

## 背景

- 相关文档：
- `docs/ARCHITECTURE.md`
- `docs/REPO_COLLAB_GUIDE.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/PLANS_GUIDE.md`
- 相关代码路径：
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/InputSimulation.swift`
- 已知约束：
- 当前 `mcp` 模式是同步 `readLine()` 主循环，没有长期运行的 AppKit event loop。
- 官方实现已有独立 `Software Cursor` 窗口和 Bezier motion 证据，但开源版目前没有 overlay UI。
- 点击路径已经做过 AX 优先收敛，新增 overlay 不能把这层行为边界改坏。

## 风险

- 风险：为了跑 overlay 而改动 `mcp` runtime，可能影响现有 stdio 读写稳定性。
- 缓解方式：保留最小变更面；只在 visual cursor 开启时切到 AppKit runtime，并让 stdin 读取仍保持串行。
- 风险：overlay 的 AppKit UI 可能干扰 smoke suite 或普通 app 点击命中。
- 缓解方式：overlay window 设为透明、忽略鼠标事件，并给 smoke suite 留显式关闭开关。
- 风险：视觉动画与真实动作不同步，反而让用户更困惑。
- 缓解方式：把 overlay 明确设计成“视觉提示层”，动作仍按现有 AX/HID 路径执行，并让点击脉冲只围绕最终目标点展开。

## 里程碑

1. Runtime 与 overlay 基础设施落地。
2. `click` 路径接入 visual cursor。
3. 验证、文档同步与归档。

## 验证方式

- 命令：
- `swift test`
- `./scripts/run-tool-smoke-tests.sh`
- 当前结果：
- `swift test` 通过。
- `./scripts/run-tool-smoke-tests.sh` 仍然卡在既有 `list_apps`/fixture 约束：当前 `AppDiscovery.listCatalog()` 只收敛带 bundle-id 的 user-facing app，而 smoke fixture 是直接运行的可执行文件，不会出现在这条输出里；这不是本轮 visual cursor 引入的回归。
- 后续 follow-up 已继续收敛 cursor 尺寸、官方 asset fallback 和相对目标 window 的排序逻辑；该部分继续保持 `swift test` 通过。
- 手工检查：
- 对一个真实 app 连续执行多次 `click`，确认 overlay 会在目标点附近短暂停留并做轻微 idle sway，随后自动消失。
- 在 AX action 命中场景下，确认点击后前台 app 不会被额外 `activate`。
- 观测检查：
- `CGWindowListCopyWindowInfo` 能看到 `open-computer-use` 进程下存在一个透明 overlay window。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-17：优先实现“独立 overlay window + 曲线移动 + 点击脉冲 + idle sway”，不试图在这一轮复刻官方的全部私有 choreography。
- 2026-04-17：`mcp` 模式只在 visual cursor 开启时切到 AppKit runtime，避免把一整轮 runtime 重构强行施加到所有无 UI 场景。
- 2026-04-17：smoke 继续显式关闭 visual cursor；因为这条回归链路的目标是验证 tools 行为闭环，而不是验证 UI 动画本身。
- 2026-04-17：对于官方 bundle 资产，优先采用“运行时读取 + 本地处理后绘制”的方式，而不是把闭源图片直接 vendoring 到开源仓库里。
