# PR #97: Windows scrolling, DPI and explicit global input

## Goal

Integrate latest main into the original PR, fix issues found in review, add repeatable real Windows desktop tests, and push reviewable follow-up commits. Do not merge the PR.

## Decisions

- Preserve the original contributor commit and current selective-observation behavior.
- Fix missing scroll bounds and align Win32/UIA/capture to physical pixel coordinates.
- Keep global input opt-in. Default and explicitly selected app_post/accessibility delivery must not switch to real input silently.
- Remove synthetic Alt focus tricks; verify target foreground, geometry and pointer hit target before physical input. Validate keys before pressing modifiers, release pressed keys/buttons in finally, and report native input failures.
- Exercise only owned fixtures on a hosted Windows desktop; reuse the existing read-only-permissions Windows workflow, without a local desktop VM.
- Record actual DPI and toolkit coverage; do not infer high-DPI/WinUI validation from a normal-DPI WPF test.

## Progress

- [x] Fetch latest main and original PR, resolve conflicts and adapt tests to the existing service bridge seam.
- [x] Review/fix global input, coordinate freshness and cleanup; add focused regression tests.
- [x] Add actual Windows scrolling, physical click/drag/key, gate/default/refusal and DPI/capture checks alongside existing observation smoke.
- [x] Run Go tests/vet/builds, PowerShell tests, relevant JS checks, documentation checks and hosted desktop CI.
- [x] Update docs/history/PR, synchronize remotes, push and report merge readiness.

## Initial findings

Main is `31a7d7d` (#101). The original PR adds a second process-global bridge test seam, now superseded by service.runBridge. Global drag/scroll ignore focus failure; global click does not confirm its target; key validation occurs after modifier downs and has no finally cleanup. The global-key branch changes every auto press_key when the pointer gate is enabled. Keep auto on window messages and expose real keys through explicit Windows key_method=global. These require fixes before merge.

## Desktop findings

First hosted run passed Go, PowerShell and the existing observation smoke. The real default scroll test exposed a second coordinate bug: WM_MOUSEWHEEL/HWHEEL lParam was incorrectly converted to client coordinates. Preserve screen coordinates and report PostMessage failure, then rerun the actual scroll-offset check.

The corrected real desktop run passed at `6a3c281` (Actions 39088560728), including physical input and Worker JS. Further review adds strict runtime-ID/current-frame checks for indexed physical fallback, with live moved/removed-element refusal and re-observation recovery checks.

The strict-element run passed production input through all four physical wheel directions, then hit a test-state sharing race before the final element cases. Fixture readers now permit atomic rename/delete and retry transient reads; both desktop harnesses share the helper. This is harness I/O, not a product acceptance bypass.

## Outcome

Original contributor commit retained, current main integrated, all reviewed changes pushed to #97. The PR remains open for the maintainer to merge.

- Windows Go 32 tests, local race/vet and amd64/arm64 cross-builds passed. Node 44 passed, including global keyMethod forwarding.
- Production-function fault tests passed on PowerShell 7 locally and Windows PowerShell 5.1 in CI: gate/focus/hit/geometry refusal, invalid chords, interrupted drag/chord, continued cleanup after one release fails, and foreground loss before the main key.
- [Full hosted desktop run](https://github.com/iFurySt/open-computer-use/actions/runs/39089480400) passed at `ab214b3`: default PostMessage scrolling, actual physical click/drag/four-way wheel/combination keys, explicit app_post, moved/occluded windows, moved/removed indexed controls, AX recovery, right-edge capture and Worker JS input. Existing selective-observation/Unicode/1x1/minimized/cloaked smoke also passed.
- Actual fixture DPI was 96, physical window 640x460. Independent 125%/150%, WinUI and multiple-monitor acceptance were not run; the original contributor's Windows 11/WinUI 3/150% results remain separate evidence.
- Docs, whitespace and action pinning checks passed. No version bump, local desktop VM or PR merge. Automatic observation/image-dedup behavior was not changed.
