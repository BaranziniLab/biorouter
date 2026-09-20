## [2026-07-30 14:31] | Task: 保留通用节点中的动作边界

### 🤖 Execution Context
* **Agent ID**: `/root`
* **Base Model**: GPT-5
* **Runtime**: Codex desktop

### 📥 User Query
> 修复 Chrome 中 BOSS 直聘转发弹窗的 snapshot 将“站内同事”“转发至其他”“邮件转发”合并成一个 container、导致目标没有独立 `element_index` 的问题。

### 🛠 Changes Overview
**Scope:** `OpenComputerUseKit` macOS Accessibility snapshot renderer

**Key Actions:**
- **[保留动作边界]**: 通用文本容器摘要遇到带 `AXPress`、`AXConfirm` 或 `AXOpen` 的 `AXGroup` / `AXUnknown` 子节点时停止合并，避免父摘要吞掉可点击后代。
- **[输出可操作按钮]**: 将具备有效紧凑 frame 的通用主动作节点渲染为 `button`，允许短文本后代直接成为按钮摘要。
- **[控制树规模]**: 普通纯文本容器仍使用原有摘要压缩；零尺寸和大面积通用动作容器仍不会误标为紧凑按钮。
- **[验证]**: 补充动作摘要边界和紧凑动作节点测试；完整 `swift test` 通过 3 项 StandaloneCursor 与 151 项 OpenComputerUseKit 测试（1 项显式 live test 跳过、0 失败），并用当前本地构建在真实 Chrome 弹窗中确认三个选项分别获得独立 button 和 Frame。

### 🧠 Design Intent (Why)
Chrome Web Accessibility 树会把文字选项暴露为带主点击动作的通用节点，再在其下放置静态文本。旧摘要逻辑只检查后代角色，不检查通用子节点的动作，因此会跨越真实交互边界，把多个选项合成一个不可精确操作的 container。本次修复只阻止摘要跨越通用主动作节点，不关闭普通文本压缩，以兼顾交互语义和节点预算。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-07/20260730-1431-preserve-generic-action-boundaries.md`
