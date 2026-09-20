# 非抢焦点 computer-use 交互改造

## 目标

让 `open-codex-computer-use` 在常见交互中尽量避免抢占用户当前焦点和真实鼠标位置，同时把与官方 `computer-use` 的对比样本沉淀到仓库里，便于后续做数据分析和 eval。

## 范围

- 包含：
- 收集一组 `computer-use` 与 `open-codex-computer-use` 对同一目标 app 的对比调用样本，并保存 tool call 与结果。
- 调整 `get_app_state` 与动作型 tools 的实现，减少不必要的 `activate` 和全局 HID 事件。
- 为键盘与点击路径补充更合适的定向投递或 AX 优先策略。
- 同步测试、架构文档、质量说明和 history。
- 不包含：
- 复刻官方闭源实现的私有 overlay、宿主集成和完整后台事件路由。
- 在本轮里解决所有第三方 app 的兼容性差异。

## 背景

- 相关文档：
- `docs/ARCHITECTURE.md`
- `docs/REPO_COLLAB_GUIDE.md`
- `docs/QUALITY_SCORE.md`
- 相关代码路径：
- `packages/OpenCodexComputerUseKit/Sources/OpenCodexComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenCodexComputerUseKit/Sources/OpenCodexComputerUseKit/ComputerUseService.swift`
- `packages/OpenCodexComputerUseKit/Sources/OpenCodexComputerUseKit/InputSimulation.swift`
- 已知约束：
- 当前实现大量依赖 `NSRunningApplication.activate` 与 `CGEvent.post(tap: .cghidEventTap)`。
- 鼠标类事件若继续走全局 HID，理论上仍会移动真实鼠标指针。
- SDK 可用能力里可确认 `CGEventPostToPid` 和 `AXUIElementPostKeyboardEvent`，但需要验证行为边界。

## 风险

- 风险：去掉 `activate` 后，某些依赖前台窗口的 snapshot 或动作可能拿不到期望元素。
- 缓解方式：snapshot 改成优先读取 app 自身窗口/焦点信息，不再强依赖前台态；必要时保留显式降级路径。
- 风险：`CGEventPostToPid` 或 `AXUIElementPostKeyboardEvent` 在部分 app 上行为和全局 HID 不一致。
- 缓解方式：先在 fixture 上做样本验证；代码中保留有边界说明的 fallback。
- 风险：为了减少副作用而过度牺牲兼容性。
- 缓解方式：优先改“能不抢焦点就不抢”，不是无条件禁止所有全局输入。

## 里程碑

1. 收集双工具对比样本并固定归档结构。
2. 实现非抢焦点优先的 snapshot / 输入策略。
3. 完成验证、文档同步和 history 留痕。

## 验证方式

- 命令：
- `swift test`
- `./scripts/run-tool-smoke-tests.sh`
- 手工检查：
- 对 fixture app 分别执行 `get_app_state`、`click`、`type_text`，记录调用前后前台 app 和鼠标坐标。
- 观测检查：
- 查看 `artifacts/tool-comparisons/20260417-focus-behavior/` 下的双目录样本是否完整。

## 进度记录

- [x] 里程碑 1
- [x] 里程碑 2
- [x] 里程碑 3

## 决策记录

- 2026-04-17：本轮优先解决“抢焦点/抢鼠标”这一类强副作用问题，并把对比数据直接落仓库，后续 eval 先基于真实样本迭代，而不是只靠口头描述。
- 2026-04-17：`get_app_state` 改为不再显式激活目标 app；键盘事件改走 `CGEvent.postToPid`，点击优先走 AX action / AX hit-test，只有 drag 或无法命中 AX 元素时才退回全局 HID。
