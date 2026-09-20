## [2026-04-22 13:38] | Task: Fix visual cursor coordinate space

### Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `Codex CLI`

### User Query
> When running `swift run OpenComputerUse call --calls-file examples/textedit-overlay-seq.json`, the virtual cursor does not hover over the right place.

### Changes Overview
**Scope:** `OpenComputerUseKit` visual cursor target conversion, tests, architecture docs

**Key Actions:**
- **[Coordinate conversion fix]**: Converted visual cursor target points from AX / `CGWindowList` screen-space into AppKit global coordinates before moving the overlay window.
- **[Regression coverage]**: Updated visual cursor target tests and added a focused conversion test for y-down screen-space to AppKit global placement.
- **[Docs sync]**: Clarified the overlay coordinate-space boundary in `docs/ARCHITECTURE.md`.

### Design Intent (Why)
The actual click path is mostly AX-driven, but the visible overlay is an AppKit panel. TextEdit toolbar targets exposed that these two paths were using different global coordinate conventions, so the click could succeed while the cursor animation visibly landed in the wrong vertical position.

### Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`
- `docs/histories/2026-04/20260422-1338-fix-visual-cursor-coordinate-space.md`
