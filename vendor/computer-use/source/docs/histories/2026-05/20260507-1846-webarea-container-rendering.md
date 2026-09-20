## [2026-05-07 18:46] | Task: Align WebArea container rendering

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5.5`
* **Runtime**: `Codex CLI, macOS`

### User Query
> Continue optimizing `open-computer-use` toward official `computer-use`, using Lark / Electron comparisons and reverse-engineering where useful.

### Changes Overview
**Scope:** `OpenComputerUseKit` accessibility snapshot rendering.

**Key Actions:**
- Compared current Feishu / Lark output from official bundled `computer-use` `1.0.770` with the local `open-computer-use` source build.
- Used strings/symbol inspection on the official bundled app to confirm WebArea/Electron-specific accessibility paths are present in the current official client/service.
- Adjusted generic wrapper elision so WebArea containers keep shallow layout wrappers and deeper branching containers, while still eliding deep single-child wrapper chains.
- Added unit coverage for the WebArea-specific elision boundary.

### Verification
- `OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY=1 swift run OpenComputerUse call get_app_state --args '{"app":"com.electron.lark"}'`
  - Confirmed the sampled tree remains free of `Scroll To Visible` and `selectable` noise.
  - Confirmed `container -> text + image`, `text entry area`, `SideEdgeView`, `menu bar`, and `zoom the window` remain present.
  - Confirmed the Lark node count moved close to the official sample in the same session: local source last index `389`, official `computer-use` sample last index `395`.

### Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
