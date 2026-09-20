# 渲染 AX placeholder 值

## 用户诉求

继续对齐开源版 `open-computer-use` 和官方 `computer-use` 的 app state 输出，尤其是 Electron / Browser 这类复杂 AX tree 的返回形状。

## 主要改动

- 在 AX renderer 中读取 `AXPlaceholderValue` / `AXPlaceholder`。
- 当 placeholder 不等于 title、description 或 value 时，在对应元素行追加 `Placeholder: ...`。
- 增加 placeholder segment 的单元测试，避免重复渲染 description 或已有 value。

## 设计动机

官方 Chrome 输出会在地址栏元素上保留 placeholder，例如 `Ask Google or type a URL`。开源版之前只输出 description 和 value，缺少这个语义字段，导致 browser / Electron app state 信息少于官方。

## 验证

- Chrome 本地回归确认地址栏行包含 `Placeholder: Ask Google or type a URL`。
- `swift test --filter AccessibilityRendererFormatsPlaceholderSegment`
- `./scripts/build-open-computer-use-app.sh debug`

## 受影响文件

- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
