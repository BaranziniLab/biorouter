# Open Source Swift Computer Use

## 目标

在当前仓库内落一版可本地运行、可测试、可继续演进的 Swift 开源实现：通过 stdio MCP 暴露 9 个 `computer-use` tools，基于 macOS Accessibility / 截图 / 输入事件完成最小可用能力，并配套一套可重复执行的端到端验证路径。

## 范围

- 包含：
  - 用 Swift Package Manager 建立仓库级可构建工程。
  - 实现 `list_apps`、`get_app_state`、`click`、`perform_secondary_action`、`scroll`、`drag`、`type_text`、`press_key`、`set_value` 这 9 个 tools。
  - 提供一个本地可启动的 fixture app，作为安全、稳定、可回归的 UI 测试目标。
  - 增加 MCP smoke / integration test，覆盖 9 个 tools 的真实调用。
  - 更新架构、质量、README、history 等仓库文档。
- 不包含：
  - 当前阶段不复刻官方闭源 app 的签名边界、私有 IPC、overlay UI 和 plugin 自安装逻辑。
  - 当前阶段不承诺和官方输出文本逐字符兼容。
  - 当前阶段不做面向发布渠道的 `.app` 包装和 notarization。

## 背景

- 相关文档：
  - `docs/references/codex-computer-use-reverse-engineering/README.md`
  - `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
  - `docs/references/codex-computer-use-reverse-engineering/internal-ipc-surface.md`
  - `docs/references/codex-computer-use-reverse-engineering/tool-call-samples-2026-04-17.md`
- 相关代码路径：
  - `apps/`
  - `packages/`
  - `scripts/`
- 已知约束：
  - macOS Accessibility 和 Screen Recording 权限会直接影响真实行为。
  - 这次实现运行在开源、无官方签名宿主的前提下，不能依赖官方私有 caller constraint。
  - 工具验证必须选安全动作，避免误触系统敏感开关。

## 风险

- 风险：AX 树和窗口截图在不同 app 上差异很大，导致测试脆弱。
  - 缓解方式：增加自有 fixture app，把 9 个 tool 的回归路径固定在受控 UI 上。
- 风险：输入事件、屏幕坐标和多显示器坐标系容易出现偏移。
  - 缓解方式：对 session 内元素 frame、window frame 和 screenshot 坐标统一建模，并在测试里使用固定窗口尺寸。
- 风险：无权限时工具行为不清晰。
  - 缓解方式：实现显式 permission check / error message，并把诊断命令写进 README 和 plan。

## 里程碑

1. 方案与脚手架收敛。
2. Swift MCP server + macOS automation 能力落地。
3. 9 个 tools 验证、文档同步与交付收尾。

## TODO

- [x] 建立 Swift 包结构和构建入口。
- [x] 实现 MCP stdio transport、tool registry 和 JSON-RPC request handling。
- [x] 实现 app discovery、session、AX tree、screenshot 和 element indexing。
- [x] 实现 7 个动作型 tools 的输入与 AX action 执行。
- [x] 建立本地 fixture app，提供可点击、可输入、可滚动、可拖拽的稳定界面。
- [x] 编写并跑通 9 个 tools 的端到端测试。
- [x] 更新 `docs/ARCHITECTURE.md`、`README.md`、`docs/QUALITY_SCORE.md` 和 history。

## 验证方式

- 命令：
  - `swift build`
  - `swift test`
  - `swift run OpenCodexComputerUseFixture`
  - `swift run OpenCodexComputerUse mcp`
  - `scripts/run-tool-smoke-tests.sh`
- 手工检查：
  - 确认 fixture app 可启动、可聚焦、可见稳定窗口。
  - 确认 `get_app_state` 能返回 screenshot 路径、窗口信息和 AX tree。
  - 确认 9 个 tools 在 fixture app 上都能观察到状态变化。
- 观测检查：
  - 记录 smoke test 输出和失败上下文。
  - 记录权限缺失时的显式诊断信息。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-17：先做一个“开源可运行实现 + 自有 fixture app”的组合，而不是直接尝试复刻官方闭源 service / client / IPC 分层。这样能在无官方签名边界的前提下，先把真实能力和测试闭环跑通。
- 2026-04-17：优先使用 Swift 标准库和系统框架，尽量不引入第三方依赖，减少供应链面和构建不确定性。
- 2026-04-17：对普通 app 仍然保留真实 AX / screenshot / CGEvent 路径；对仓库内 fixture app 增加合成状态与 command bridge，用来保证 9 个 tools 有稳定、低风险、可回归的 smoke path。
