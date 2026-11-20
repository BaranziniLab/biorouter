# PR #101: cross-platform selective observation

## Goal

Integrate current main, review and fix selective AX/screenshot collection, extend it to macOS and Windows at the user's request, verify through CLI/native MCP/current JS, and push follow-up commits to the original PR. Do not merge.

## Scope and decisions

- Both channels default to true; at least one is required. Existing calls and action read-back behavior stay compatible.
- AX-only skips capture/encoding; screenshot-only skips tree/focus/selection and invalidates old snapshot aliases, including PID replacement and macOS query indexes. Fresh AX restores element actions.
- AX-only cannot validate screenshot coordinates: require a fresh image observation before coordinate click/drag.
- Use disposable AppKit, GTK and WPF fixtures. macOS follows normal signing/session/permission admission; Linux uses private Xvfb/D-Bus on the existing container engine; Windows uses the hosted Actions desktop. No personal apps or local desktop VM.
- Automatic unchanged-image suppression remains separate. It needs exact image comparison, first/reset/forced output and session/app/window scope; unchanged AX does not establish unchanged pixels.

## Progress

- [x] Merge main and retain existing bounded-focus, hidden-node and read-back behavior.
- [x] Add macOS collection gates, schema/dispatcher/app-agent/CLI support and scoped cache/query invalidation.
- [x] Add Windows Go/PowerShell collection gates, strict options, cache behavior and protocol tests.
- [x] Expose matching snapshot flags on all platforms and reject AX-only coordinate input.
- [x] Verify normal signed macOS release and real GTK through CLI/native MCP/Worker JS; run relevant suites/builds.
- [x] Complete hosted real Windows WPF/UIA/MCP/CLI/Worker JS and screenshot-boundary regression.
- [x] Synchronize final docs/history/PR, remotes and original PR branch; leave PR open.

## Validation and findings

- Main through `7cb8bb7` integrated, preserving the original contribution.
- Older aliases after PID replacement initially retained stale indexes; tests now cover invalidation for both previous/new PIDs and preserve unrelated applications.
- macOS CLI originally printed only its first text block; it now retains omission notes too.
- Swift: 382 tests, 8 skipped, zero failures; selective-observation 8/8. Existing native tool and cursor smoke passed.
- Normally signed macOS release + real AppKit AX/native MCP/CLI/Worker JS passed valid PNG, trace-confirmed collection gates, snapshot/query invalidation, coordinate refusal, real value update and recovery. Fixture cannot become key/main.
- Linux: Go 31, Python 35 and real GTK/AT-SPI/Xvfb passed collection helper gates, PNG, Unicode selection, old name/PID index refusal, coordinate actions, AX recovery and CLI/Worker JS. Existing focus and hidden-detail checks passed.
- Node: 44 passed. Windows Go: 29 passed. Production PowerShell function tests passed under PowerShell 7 and actual Windows PowerShell 5.1 with instrumented dependencies.
- Linux/Windows ARM64/AMD64 builds and targeted vet passed; syntax/docs/whitespace/action-pinning checks passed.
- WinForms fixture did not expose a settable ValuePattern; the positive UIA-write fixture was changed to WPF instead of broadening production input fallbacks. The real WPF/UIA/CLI/native MCP/Worker JS desktop smoke passed on Windows 2025.
- Temporary Linux containers and build artifacts removed. No Wayland/other-toolkit performance claim or version bump.

## Outcome

All three platforms now provide the same explicit observation choices through CLI/native MCP/JS. Real desktop tests passed for AppKit, GTK and WPF; the Windows job additionally verified exact 1x1 Win32, minimized and DWM-cloaked screenshot suppression and image recovery. WPF DPI layout is avoided only for the exact one-pixel fixture; production screenshot thresholds and input fallbacks were unchanged.

Windows desktop evidence: [Windows observation run](https://github.com/iFurySt/open-computer-use/actions/runs/39018965035) at `ae8ec2f`. Final follow-up commits are pushed to #101; the PR remains open for the maintainer to merge. Automatic image deduplication remains in the debt tracker.
