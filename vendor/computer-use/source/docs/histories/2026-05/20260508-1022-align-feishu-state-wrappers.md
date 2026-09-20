# Align Feishu state wrappers

## User request

用户重启 Codex 后要求继续对比 `open-computer-use` 与官方 `computer-use`，确认新版 Dev app 在 Feishu / Electron state rendering 上的差异。

## Changes

- 对齐官方 `computer-use` 的 Electron state tree：单子节点、无自有语义、仅带 `settable/string` trait 的 generic wrapper 现在会被压平。
- WebArea 下不再仅因为处在浅层就保留单子节点 generic container，减少 Feishu 页面顶部和正文区的冗余层级。
- 文本合并规则新增独立时间范围保护，避免把日程标题、倒计时和 `HH:mm - HH:mm` 时间段合成一整句。
- 文本摘要允许 AXLink 参与合并，让内容列表项能更接近官方的单行摘要形状。
- AXLink 的独立渲染也统一保留 markdown 链接形态，并避免长 URL 同时输出重复 `Description`。
- 补充单元测试覆盖 wrapper elision 和时间范围合并边界。

## Validation

- `swift test`
- `./scripts/build-open-computer-use-app.sh debug`
- 直连新构建 Dev app 抽样 `get_app_state`，确认 Feishu 顶层 `ClientView` 前的空 wrapper 已压平，日程时间范围不再被合并。
- 直连新构建 Dev app 抽样链接渲染，确认 Feishu 长链接以 markdown link 输出且不重复 `Description`。
- `./scripts/run-tool-smoke-tests.sh`
- `./scripts/check-docs.sh`
- `git diff --check`

## Affected files

- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
