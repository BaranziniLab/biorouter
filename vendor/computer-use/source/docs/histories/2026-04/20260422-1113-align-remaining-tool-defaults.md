## [2026-04-22 11:13] | Task: 对齐剩余 Computer Use tools 的默认行为

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### 📥 User Query
> `click` 和 `set_value` 已经对齐，继续把其他 7 个 tool 一个一个过一遍，落 TODO，并按官方 `.app` 逆向结果对齐细节。

### 🛠 Changes Overview
**Scope:** `OpenComputerUseKit` tool surface / input routing, fixture smoke support, docs

**Key Actions:**
- **[Execution Plan]**: 新增 active plan，把剩余 7 个 tool 拆成 checklist，并记录官方 `1.0.755` 的静态类型线索和验证命令。
- **[scroll schema]**: 将 `scroll.pages` 从 `integer` 对齐到官方 `number` schema，支持 fractional pages，并补 `pages must be > 0` 与 invalid direction 的官方风格错误。
- **[required string]**: dispatcher 对 required string 统一拒绝空字符串，返回 `Missing required argument: <name>`，覆盖 `type_text` / `press_key` / `set_value` / `scroll` 等工具。
- **[非物理 pointer 默认路径]**: `scroll` / `drag` 默认改为 `CGEvent.postToPid` 定向事件；只有显式设置 `OPEN_COMPUTER_USE_ALLOW_GLOBAL_POINTER_FALLBACKS=1` 时才允许全局 `.cghidEventTap` 物理指针兜底。
- **[Smoke fixture]**: 为 SwiftPM 裸 executable fixture 保持强引用 delegate，并仅对内部 `OpenComputerUseFixture` 注入 synthetic list identifier，恢复 9-tool smoke suite 覆盖。

### 🧠 Design Intent (Why)

官方 binary 暴露 `MouseEventTarget`、`KeyboardEventTarget`、`EventTap`、`SystemFocusStealPreventer`、`UIElementScrollOperation` 等类型，说明默认动作路由不是简单把 fallback 全部发到系统级硬件光标。开源版先把仍会移动真实鼠标或激活 app 的默认路径收掉：能用 AX action 就走 AX，不能时走 pid-targeted event；物理指针 fallback 只保留为显式调试开关。

### 📁 Files Modified
- `docs/exec-plans/active/20260422-remaining-tool-official-alignment.md`
- `docs/ARCHITECTURE.md`
- `docs/references/codex-computer-use-reverse-engineering/baseline-architecture.md`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AppDiscovery.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseToolDispatcher.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/Errors.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/FixtureBridge.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/InputSimulation.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ToolDefinitions.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `apps/OpenComputerUseFixture/Sources/OpenComputerUseFixture/main.swift`

### 🔁 Follow-up (2026-04-22, close remaining tool checklist)

- **[具体化 milestones]**: active plan 不再使用 `里程碑 1/2/3` 占位，改成可执行的三项进度记录：官方证据基线、动作路径收敛、只读/键盘类复核和收尾。
- **[secondary action 对齐]**: `perform_secondary_action` 的 invalid action 错误改为官方 binary 暴露的字符串形态；fixture 的 `Raise` 路径不再调用 global pointer prepare。
- **[press_key key table]**: 根据官方 `1.0.755` binary 里的 key table 字符串，补齐 `BackSpace`、`Page_Up`、`Prior` / `Next`、`F1...F12` 和常见 `KP_*` xdotool alias。
- **[收口验证]**: 官方 `1.0.755` app-server `tools/list` 与本地 direct `tools/list` 的 9 个 tool surface 一致；`swift test` 与 9-tool smoke suite 均通过。
