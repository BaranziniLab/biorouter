## [2026-04-21 21:20] | Task: 对齐 click 的非侵入默认行为

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `local macOS shell`

### 📥 User Query
> 继续逆向确认官方 Computer Use 的 click/HID 逻辑，并在得出结论后修正本仓库实现，避免 click 抢用户鼠标。

### 🛠 Changes Overview
**Scope:** `OpenComputerUseKit` click 行为与架构文档

**Key Actions:**
- **[官方行为确认]**: 静态确认官方包有 AX action、EventTap/CGEvent 鼠标事件生成、`clickEventTap`、`MouseEventTarget` 和 `feature/computerUseAlwaysSimulateClick`，但没有直接导入公开 `CGEventPost` / `IOHID*` 符号；`AlwaysSimulateClick` 默认值为关闭。
- **[默认关闭全局 click 兜底]**: `click` 在 AX 路径失败后不再默认调用全局 `.cghidEventTap` 鼠标事件，必须设置 `OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS=1` 才允许物理指针兜底。
- **[修正 click_count 分支]**: `AXPress` / `AXConfirm` / `AXShowMenu` 支持按 `click_count` 重复执行，避免多次点击因为 `clickCount != 1` 直接落入全局鼠标路径。
- **[测试补充]**: 新增全局指针兜底环境变量默认关闭的单元测试。

### 🧠 Design Intent (Why)
官方实现有物理点击模拟能力，但它不是简单把所有 fallback 都发到系统级硬件光标；二进制里还能看到 focus-steal suppression、target 类型和 feature flag。当前本仓库直接用 `.cghidEventTap` 加 `.mouseMoved` 会移动用户真实光标，和工具说明里的后台交互预期冲突。先把 click 的高风险兜底改成显式 opt-in，同时保留可调试逃生口。

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260421-2120-disable-click-global-pointer-default.md`
