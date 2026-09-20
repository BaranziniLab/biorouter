## [2026-05-07 18:25] | Task: Polish accessibility snapshot formatting

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5.5`
* **Runtime**: `Codex CLI, macOS`

### User Query
> Continue optimizing `open-computer-use` toward official `computer-use`, comparing Lark / Electron returns and using reverse-engineering where useful.

### Changes Overview
**Scope:** `OpenComputerUseKit` accessibility snapshot rendering.

**Key Actions:**
- Matched the official window button help/action format by rendering `Help:` without a leading comma and adding the comma before `Secondary Actions` when prior metadata exists.
- Special-cased `AXZoomWindow` to render as `zoom the window`, matching the official full-screen button output.
- Restored short text-summary wrapper nodes to `container` instead of `text`, which better matches official Electron rows where compact text usually lives under a container parent.
- When a compact text-summary container also has image descendants, rendered the summary as a child `text` row and preserved up to four image descendants. This moves Lark message rows closer to the official `container -> text + image` shape without expanding long menu/body content.

### Verification
- `OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY=1 swift run OpenComputerUse call get_app_state --args '{"app":"com.electron.lark"}'`
  - Confirmed `full screen button Help: this button also has an action to zoom the window, Secondary Actions: zoom the window`.
  - Confirmed short Lark message summaries render as `container` parent rows.
  - Confirmed message rows with avatars/images render as `container -> text + image`.
  - Confirmed `text entry area`, `SideEdgeView`, and focused `HTML 内容 messenger-chat, URL: ...` are still present.
  - Confirmed no `Scroll To Visible` or `selectable` noise in the sampled tree.
- Added `testAccessibilityRendererRendersSummariesWithImagesAsChildren` to cover the summary/image split decision.

### Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
