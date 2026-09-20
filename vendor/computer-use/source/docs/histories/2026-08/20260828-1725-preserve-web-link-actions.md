## [2026-08-28 17:25] | Task: 修复 Web 链接动作节点解析

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex desktop`

### 📥 User Query
> 执行修复：BOSS 页面“职位管理”被解析成没有独立 link 的 button，导致无法点击跳转。

### 🛠 Changes Overview
**Scope:** `OpenComputerUseKit` macOS Accessibility snapshot renderer

**Key Actions:**
- **[保留导航语义]**: 通用动作节点包含带 URL 的 `AXLink` 后代时，不再提升为 compact `button`，也不再合并其文本摘要。
- **[保持通用按钮能力]**: 没有 URL 链接后代的紧凑 `AXGroup` / `AXUnknown` 仍按原规则保留为可操作 `button`。
- **[补回归覆盖]**: 增加带链接后代的 compact action 节点测试，防止 BOSS 导航链接再次被父摘要吞掉。

### 🧠 Design Intent (Why)
Chrome/BOSS 的 Accessibility tree 可能同时暴露通用父动作和真实 URL 链接子节点。父动作适合 icon-only 控件，但对导航链接必须保留子节点的独立 `element_index`，否则 AX 点击可能只触发父 wrapper 而不发生 URL 导航。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-08/20260828-1725-preserve-web-link-actions.md`
