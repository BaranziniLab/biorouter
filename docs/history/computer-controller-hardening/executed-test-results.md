# Computer Controller — executed test results

> Computer-use references in historical investigations below are superseded by the [native Biorouter Copilot contract](../../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


> **What this is.** The executed outcomes of the 2026-06-20 Computer Controller hardening pass: the code changes made, the automated tests added, the live Xiaomi MiMo run results, and the finding that MiMo vision is endpoint-specific.
> **Status:** Historical record — the pass completed on 2026-06-20, and every change listed here (honest `osascript` errors, primary-display default, substring window match, MiMo `with_vision()`) shipped in that commit.
> **Audience:** maintainers working on the Computer Controller extension.

This is the results half of a two-document pair. The plan half — the reported user
problems, their root causes, and the ~60-case test matrix — is
[the Computer Controller test plan and root causes](./test-plan-and-root-causes.md).
Case identifiers used below (`A1`, `A2`, `B6`, `B8`, `C4`, `H4`) refer to that plan's
matrix, where section letters group cases by area (A = screenshots and vision,
B = basic UI interaction, C = application control, H = error and permission handling).
One identifier below, `FIX2`, does not appear in the plan's matrix and has no
definition in either document.

Fix categories are carried over from the plan: **P** = prompt / context-engineering fix,
**T** = tool-hardening fix in Rust, **E** = environment or out-of-scope (OS permissions,
hardware, model capability).

All live runs used **Xiaomi MiMo** — `mimo-v2.5-pro` for non-vision cases, and
`mimo-v2-omni` where the model must *see* a screenshot. Runs used a sandboxed config at
`XDG_CONFIG_HOME=/tmp/ceexec-sandbox` so the real `~/.config/biorouter` was untouched.

> **Note.** The environment details in this record are specific to the machine that ran
> the pass — a macOS host with the MiMo Token-Plan SGP endpoint configured — and are not
> expected to generalize. Treat host names, display models, and endpoint behaviour as
> observations from one session, not as properties of the system.

## MiMo vision is model- and endpoint-specific

> **Warning.** This is the most reusable fact in this document. Vision worked with
> `mimo-v2-omni` and returned HTTP 404 with `mimo-v2.5-pro` on the endpoint under test.

- The harness vision path is **correct and verified**: screenshots are sent in the
  standard OpenAI vision format and a vision-capable MiMo model receives and
  interprets them end-to-end.
- **`mimo-v2.5-pro` on the configured Token-Plan SGP host returns `404: No endpoints
  found that support image input`** — that deployment does not serve image input.
- **`mimo-v2-omni` works** for vision (it described the live screen correctly).
- Recommendation: for Computer Controller or any screenshot-driven task, use
  `mimo-v2-omni` (or a MiMo vision endpoint that serves v2.5-pro images). The
  `with_vision()` declaration is kept for the MiMo family (capability is intrinsic;
  the SGP route's 404 is a deployment gap), but the working vision route today is omni.

## Code changes made

| Area | File | Change | Category |
|------|------|--------|----------|
| Honest errors (macOS) | `computercontroller/platform/macos.rs` | osascript now returns `Err` with stderr+exit on failure; permission errors get an actionable hint | T |
| Honest errors (Windows) | `computercontroller/platform/windows.rs` | PowerShell now returns `Err` with stderr+exit on failure | T |
| Honest errors (Linux) | `computercontroller/platform/linux.rs` | `run_checked` surfaces non-zero exit/stderr for xdotool/wmctrl input commands | T |
| Honest errors (shared) | `computercontroller/mod.rs` `computer_control_impl` | no longer claims "completed successfully" unconditionally; distinguishes no-output success with a "verify, don't repeat" message | T |
| Multi-monitor | `developer/rmcp_developer.rs` `screen_capture` | defaults to the **primary** display; reports full display topology on every capture; out-of-range error lists valid indices | T |
| App/window matching | `developer/rmcp_developer.rs` `screen_capture` | case-insensitive **substring** window match; on miss, lists open window titles | T |
| Progressive/clickability | `computercontroller/mod.rs` instructions | OS-invariant operating principles: progressive verification, stop-after-2-failures, permission handling, method-preference order, multi-monitor awareness | P |
| Rename | `computercontroller/mod.rs` `get_info` | Model Context Protocol (MCP) server `title` = "Computer Controller" | P |
| MiMo vision | `providers/xiaomi_mimo.rs` | declare models `with_vision()` so the harness/UI treat MiMo as image-capable (GUI image-upload was gated off before) | T |

## Automated tests added

All of the following pass.

- `macos.rs`: `successful_script_returns_stdout`, `empty_output_script_still_succeeds`, `failing_script_returns_err_with_reason`
- `mod.rs` (`computer_control_tests`): `computer_control_reports_failure_instead_of_fake_success`, `computer_control_no_output_success_guides_to_verify`
- `rmcp_developer.rs`: `describe_monitors_empty_is_safe_and_explains_indexing`, `describe_monitors_lists_each_connected_display_with_index`
- `base.rs`: MiMo added to `known_vision_models_have_supports_vision_true`

Full suites green: `biorouter-mcp` lib (68 computercontroller/developer tests), `biorouter` lib (xiaomi_mimo + vision + code_execution regression: 31).

## Live runs across both MiMo models

| Case | Model | Result |
|------|-------|--------|
| A1/A2 screenshot + topology + describe | mimo-v2-omni | ✅ Captured primary display; MiMo accurately described the real screen ("macOS desktop… Claude.ai chat window… terminal output") and read back the display list verbatim |
| FIX2 topology report | mimo-v2-omni | ✅ `0: "DELL S2722QC" 1920x1080 at (0,0) scale 2.0 [primary]` reported and read back |
| B6/B8 failing control script | mimo-v2.5-pro | ✅ `Computer control script failed: osascript exited with status 1: … Can't get application "NoSuchApp…". (-1728)` — honest error, **no retry loop** |
| H4 no-output success | mimo-v2.5-pro | ✅ Returned the "ran with no errors and produced no output … verify … do not blindly repeat" guidance |
| C4 frontmost-app automation | mimo-v2.5-pro | ✅ `Script completed successfully. Output: Claude` (System Events automation permitted) |
| GUI backend: screen_capture | (GUI biorouterd) | ✅ via `/agent/call_tool`: topology text + image block returned (HTTP 200) |
| GUI backend: failing control | (GUI biorouterd) | ✅ HTTP 500 (tool `Err` surfaced; in the `/reply` agent loop this becomes a model-visible error, as the terminal-UI (TUI) run confirms) |
| GUI backend: no-output success | (GUI biorouterd) | ✅ HTTP 200 + verify-guidance |

## Live runs on mimo-v2-omni, full Computer Controller flow

| Case | Task | Result |
|------|------|--------|
| Progressive open+verify | Open Calculator, then screenshot to confirm | ✅ Activated once, took ONE screenshot, described it accurately — no circling |
| **UI automation + clickability (headline)** | Compute 7+8 by clicking buttons, verify | ✅ `click button "C"` failed with honest error `(-1728)`; instead of looping, the model **queried the UI hierarchy to discover elements** and **switched to keystrokes** (method-preference principle), verified via screenshot, reported **15** correctly |
| Window capture by title | Screenshot the Calculator window via `window_title` substring | ✅ Substring "Calculator" matched the window; described "7+8 … 15" |
| System setting (OS-invariant) | Get system output volume | ✅ Model's own AppleScript syntax errors surfaced honestly `(-2740)`; it self-corrected progressively and reported volume 50 — no silent-failure loop |
| Window not found | Screenshot 'AdobePhotoshop2099' | ✅ Tool returned "No open window matches … Pick one of the titles above"; model listed offered windows (Claude Code, Calculator, Activity Monitor, …) and stopped — no blind retry |
| Screenshot + describe + topology | Capture primary, describe, report displays | ✅ Accurate description of the live screen; topology read back |

These runs jointly demonstrate the three target behaviors are fixed: **progressive
verification** (act → screenshot → confirm), **no circling on failure** (honest errors
drive adaptation, not blind retries), and **clickability awareness** (discover elements /
prefer keystrokes over brittle named-button clicks).

## Failure-mode classification

- **Prompt/context-fixable (P):** the circling/non-progressive behavior was largely
  driven by (a) the fake-success bug and (b) missing operating principles —
  both now addressed. Clickability/app-navigation is improved by guidance to prefer
  app-native automation and named elements over coordinate clicks.
- **Tool-hardening (T):** the root cause of "circling and burning tokens" was the
  silent-success bug (macOS/Windows discarded exit status + stderr). Fixed. The
  multi-monitor "not found" problem was a tool bug (index-0 default, no topology).
  Fixed. Window matching brittleness fixed.
- **Environment (E):** OS Accessibility/Screen-Recording/Automation permissions and
  the MiMo vision endpoint availability are outside the code; the tools now surface
  these clearly so the agent (and user) can act instead of looping.

## Related documentation

- [Computer Controller test plan and root causes](./test-plan-and-root-causes.md) — the case matrix and the K1–K5 root-cause table these results were run against.
- [Multi-app orchestration run](./multi-app-orchestration-run.md) — the sibling run record covering Slack, web literature and Word workflows, and the watchdog/playbook fixes that came out of it.
- [Computer Controller extension reference](../../extensions/built-in/computer-controller.md) — the current user-facing documentation for the tools exercised here.
- [Xiaomi MiMo provider](../../providers/xiaomi-mimo.md) — provider setup, host selection and the model list for the provider used in these runs.
