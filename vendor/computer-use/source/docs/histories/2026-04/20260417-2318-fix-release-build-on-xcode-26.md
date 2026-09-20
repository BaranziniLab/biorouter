## [2026-04-17 23:18] | Task: 修复 0.1.7 / 0.1.8 release workflow 构建失败

### 🤖 Execution Context
* **Agent ID**: `codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> 看看为什么 `0.1.7` 和 `0.1.8` 都发布失败了，action 看看。

### 🛠 Changes Overview
**Scope:** `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`

**Key Actions:**
- **定位 release 失败根因**：通过 GitHub Actions 日志确认 `v0.1.7` 和 `v0.1.8` 都在 `Build npm release artifacts` 阶段失败，未进入 artifact upload 或 npm publish。
- **修复 Xcode 26 编译报错**：把权限引导窗口里的 `AXUIElement` 属性读取从条件下转改成显式 `CFTypeID` 校验后再强转，避开 Xcode 26.2 对 CoreFoundation 类型“条件下转必然成功”的编译错误。
- **保留原有行为边界**：只有在 AX 属性读取成功且实际是 `AXUIElement` 时才返回元素，避免为了过编译改动现有窗口查找逻辑。

### 🧠 Design Intent (Why)
这次 release 失败不是发布权限、trusted publishing 或 tag 触发条件的问题，而是 CI 新编译器对 CoreFoundation 桥接类型的静态检查更严格。显式比较 `CFTypeID` 能把“类型是否正确”这件事写清楚，同时保持运行时语义稳定，适合这类和系统 Accessibility API 打交道的代码路径。

### 📁 Files Modified
- `apps/OpenComputerUse/Sources/OpenComputerUse/PermissionOnboardingApp.swift`
- `docs/histories/2026-04/20260417-2318-fix-release-build-on-xcode-26.md`
