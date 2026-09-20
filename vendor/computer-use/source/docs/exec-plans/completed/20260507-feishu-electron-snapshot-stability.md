# Feishu Electron Snapshot Stability

## 目标

对比官方 `computer-use` 与本仓库 `open-computer-use` 操作 Feishu 的真实表现，修复本仓库在 Electron/WebView 深层 UI 上 `get_app_state` 树渲染不完整的问题，并用可重复命令验证修复有效。

## 范围

- 包含：macOS `get_app_state` 的 AX tree traversal、Feishu/Electron 深层 UI 的手工对比验证、必要的单元测试与文档/history。
- 不包含：重写输入模拟策略、引入新 MCP tool、处理 Windows/Linux runtime 的 Electron 兼容性。

## 背景

- 相关文档：`docs/ARCHITECTURE.md`、`docs/RELIABILITY.md`、`docs/QUALITY_SCORE.md`
- 相关代码路径：`packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- 已知约束：普通第三方 app 的 AX tree 和截图依赖 macOS Accessibility 与 ScreenCaptureKit；Electron app 的 WebView 层级比原生 AppKit 控件更深。

## 风险

- 风险：放宽遍历深度可能让某些复杂 app 输出过长或遍历耗时增加。
- 缓解方式：保留节点总数上限，并增加测试覆盖深层树可达性和节点预算。

## 里程碑

1. 对比官方 `computer-use` 与 `open-computer-use` 在 Feishu 上的 `list_apps` / `get_app_state` 行为。
2. 收敛代码层原因并实现最小修复。
3. 运行单元测试、smoke 和 Feishu 实机回归，记录结果。

## 验证方式

- 命令：`swift test`
- 命令：`./scripts/run-tool-smoke-tests.sh`
- 手工检查：分别用 `computer-use` 和 `open-computer-use` 对 `com.electron.lark` 执行 `get_app_state`，确认本仓库输出能覆盖聊天消息与输入框，并返回截图。

## 进度记录

- [x] 确认两个 MCP 都能发现运行中的 `Feishu — com.electron.lark`。
- [x] 复现差异：官方 `get_app_state` 能展开到消息与 entry area，本仓库截图正常但 AX 树停在浅层 WebView 容器。
- [x] 完成最小修复与测试。
- [x] 完成修复后验证并记录 history。

## 决策记录

- 2026-05-07：优先修 AX tree traversal 深度，不改变动作型 tool 的输入策略；本轮复现的失败点是树过浅，截图链路在现场样本中已经返回有效 PNG。
- 2026-05-07：仅加深 traversal 仍会被 Electron 空 wrapper 消耗 500 节点预算，因此同时过滤空字符串、`AXScrollToVisible` 噪音和无语义 generic wrapper；修复后 Feishu state 能在 500 节点内返回消息、entry area 和 PNG 截图。
