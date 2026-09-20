## [2026-04-22 10:57] | Task: 保持 visual cursor 的 interaction 间 idle 状态

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5.4`
* **Runtime**: `Codex CLI on macOS + SwiftPM`

### User Query
> 逆向官方 `.app` 看它如何管理 overlay cursor 显示；当前本地 `click` / `set_value` 之间 cursor 会消失并反复从左下角 `(0,0)` 出发，而官方会 idle 在当前位置，下次操作继续移动。

### Changes Overview
**Scope:** `OpenComputerUseKit` visual cursor runtime、逆向文档、架构文档

**Key Actions:**
- **[官方生命周期复查]**: 对 bundled `computer-use` `1.0.755` 复查 Swift metadata，确认 `ComputerUseCursor.Window.currentInterpolatedOrigin` 和 `wantsToBeVisible` / `shouldFadeOut` 是分离状态。
- **[运行时日志对照]**: 通过 unified log 复查官方 service 的 cursor movement，确认多次 movement 之间复用同一个 cursor window，最后一次 movement 后约 5 分钟才由 service idle timeout 终止。
- **[本地 idle 生命周期修正]**: `SoftwareCursorOverlay` 不再在 `click` / `set_value` 收尾后 0.5 秒级清空状态，而是保留 idle 约 5 分钟，让后续 tool call 从当前 visible tip 继续。
- **[补测试和文档]**: 新增 idle timeout 常量回归测试，并同步 `ARCHITECTURE.md` 与 reverse-engineering 文档。

### Design Intent
官方的 `(0,0)` 起点是 fresh service / fresh cursor window 语义，不是每次 action 的收尾语义。把本地短延迟 hide 改成较长的 idle cleanup，可以保留当前进程内的 `displayedTipPosition` / visual dynamics 状态，避免连续工具调用时反复回到左下角。

### Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `scripts/npm/build-packages.mjs`
- `docs/ARCHITECTURE.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
- `docs/histories/2026-04/20260422-1057-preserve-visual-cursor-idle-state.md`

### Follow-up (2026-04-22, turn-ended cleanup)

- **[确认缺口]**: 复查后确认本地 `open-computer-use turn-ended` 只是单独 CLI 进程打印确认，不会影响正在运行的 MCP overlay；这不能满足“任务结束 cursor 消失”。
- **[MCP 内部 hook]**: `StdioMCPServer` 新增 `notifications/turn-ended`，收到后立即 reset 当前进程里的 visual cursor。
- **[Codex notify 兼容]**: CLI `turn-ended` 现在接受 Codex legacy notify 追加的 after-agent payload，并通过 macOS distributed notification 通知正在运行的 AppKit MCP 进程清理 cursor；`MCPAppRuntime` 会监听这条通知。
- **[同步测试文档]**: 新增 CLI payload 解析和 MCP notification 回归测试，并更新架构与逆向文档。

**Follow-up Files:**
- `apps/OpenComputerUse/Sources/OpenComputerUse/MCPAppRuntime.swift`
- `apps/OpenComputerUse/Sources/OpenComputerUse/OpenComputerUseMain.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/MCPServer.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/OpenComputerUseCLI.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/SoftwareCursorOverlay.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/references/codex-computer-use-reverse-engineering/software-cursor-overlay.md`
