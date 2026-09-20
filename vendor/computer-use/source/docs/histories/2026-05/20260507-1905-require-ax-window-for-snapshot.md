## [2026-05-07 19:05] | Task: Align unavailable window state

### 🤖 Execution Context
* **Agent ID**: `Codex`
* **Base Model**: `GPT-5`
* **Runtime**: `macOS local shell`

### 📥 User Query
> 对比最新版 open-computer-use 和官方 computer-use 的工具返回，继续修正差异。

### 🛠 Changes Overview
**Scope:** `packages/OpenComputerUseKit`

**Key Actions:**
- **[Snapshot Guard]**: `get_app_state` now requires a real AX window before rendering an accessibility tree.
- **[Role Filtering]**: Focused-window and first-window candidates are filtered to `AXWindow`, avoiding misleading app-root-only trees when an app exposes no usable key window.

### 🧠 Design Intent (Why)
Official `computer-use` returns an unavailable-window error for the observed Lark state, while open-computer-use previously rendered only the application/menu-bar root and implied the app was actionable. Requiring a real accessibility window makes the failure explicit and prevents stale or non-actionable element indexes from reaching follow-up tools.

### 📁 Files Modified
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`

### 🔁 Follow-up (2026-05-07, require matching visible CG window)

**Additional evidence:** Official `computer-use` `1.0.770` strings include `cgWindowNotFound`, `noWindowsAvailable`, `matchingWindowNotFound`, and `AXWindowMiniaturized` / `AXWindowDeminiaturized`, and current official calls return `cgWindowNotFound` for Lark, Chrome, and System Settings when the desktop exposes no usable window state.

**Additional changes:**
- Filtered window candidates to non-minimized `AXWindow` elements.
- Required a matching on-screen `CGWindow` before rendering a real-app snapshot.
- Synchronized `docs/ARCHITECTURE.md` with the stricter window precondition.

**Validation:**
- `swift test`
- `./scripts/check-docs.sh`
- `git diff --check`
- `./scripts/run-tool-smoke-tests.sh`
- Manual comparison: official `computer-use` and current source build both return unavailable-window errors for the current Lark / Chrome / System Settings no-window state.

### 🔁 Follow-up (2026-05-07, match official no-window text)

**Additional changes:**
- Introduced a shared `computerUseNoWindowFoundMessage` constant with the official observed text: `Apple event error -10005: cgWindowNotFound`.
- Reused that message for both missing AX window and missing visible CG window paths.
- Added a focused unit test for the exact no-window message.
- Synchronized `docs/ARCHITECTURE.md` to call out the official-style no-window error text.

**Validation:**
- `swift test --filter NoWindowErrorMessageMatchesOfficialShape`
- Manual comparison: source build now returns the same no-window text as official `computer-use` for current Lark and Chrome no-window states.
