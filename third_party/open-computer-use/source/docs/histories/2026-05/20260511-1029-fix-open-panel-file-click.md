# Fix open panel file click

## User Request

> Investigate why `open-computer-use` could click directories in a macOS upload/open file panel but could not click files in the right-hand file list.

## Changes

- **[Open panel tree rendering]**: Added `AXContents` / `AXVisibleChildren` traversal so native open panel column views expose visible file items instead of stopping at empty scroll areas.
- **[Modal window capture]**: Updated window capture selection to prefer a frontmost overlapping modal/sheet window over a stale exact title match with the underlying main window.
- **[List item click]**: Made left click select native `AXList` items through `AXSelectedChildren`, and made coordinate clicks try the smallest containing snapshot element before generic AX hit-test results.
- **[Regression coverage]**: Added unit tests for modal window selection and visible-child list traversal.
- **[Docs]**: Updated `docs/ARCHITECTURE.md` with the open panel traversal and list selection behavior.

## Rationale

The live Nomi open panel exposed the sidebar through `AXRows`, but the file list was under `AXScrollArea -> AXContents -> AXList -> AXVisibleChildren`. The previous renderer did not traverse those attributes, so file rows were absent from the tool state. Coordinate clicks then hit generic sheet/list elements and could report success through focus-style fallbacks without selecting a file.

The fix keeps the preferred path semantic: surface the visible file items, select native list children through AX, and only use targeted mouse fallback after the more precise candidates fail.

## Files

- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/AccessibilitySnapshot.swift`
- `packages/OpenComputerUseKit/Sources/OpenComputerUseKit/ComputerUseService.swift`
- `packages/OpenComputerUseKit/Tests/OpenComputerUseKitTests/OpenComputerUseKitTests.swift`
- `docs/ARCHITECTURE.md`

## Validation

- `swift test`
- Live local CLI check with `OPEN_COMPUTER_USE_DISABLE_APP_AGENT_PROXY=1 .build/debug/OpenComputerUse call get_app_state --args '{"app":"Nomi"}'` showed `IMG_7605.JPG`, `IMG_7624.HEIC`, and `IMG_7753.JPG` as file-list elements.
- Live local CLI coordinate click changed the open panel preview from `IMG_7605.JPG` to `IMG_7753.JPG`, with the `Open` button enabled.
