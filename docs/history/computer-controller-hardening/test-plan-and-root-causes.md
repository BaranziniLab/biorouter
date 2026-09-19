# Computer Controller — test plan and root causes

> Computer-use references in historical investigations below are superseded by the [native Computer Use contract](../../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


> **What this is.** The matrixed test plan for the Computer Controller extension: the five reported user problems with the root cause found for each, and roughly 60 numbered cases across screenshots, UI control, apps, settings, files, web, multi-app workflows, permissions, multi-monitor behaviour and cross-OS invariance.
> **Status:** Historical record — written for the 2026-06-20 multi-monitor / vision / honest-errors change. Its fixes shipped in that commit, and the executed outcomes live in [the executed test results](./executed-test-results.md). This is the plan of record for a finished pass, not a live checklist.
> **Audience:** maintainers working on the Computer Controller extension.
>
> **Identifier scheme.** Cases are `<section letter><number>` — `A1`, `B6`, `H4` — where the section letter groups cases by area (A screenshots and vision, B basic UI interaction, C application control, D system settings, E files and documents, F web, G multi-app workflows, H errors and permissions, I multi-monitor, J cache, K cross-OS invariance, whose IDs read `K-OS1`…`K-OS5`). Separately, `K1`–`K5` in the root-cause table below identify the five **reported problems**, not cases. Fix categories are **P** prompt / context-engineering fix, **T** tool-hardening fix in Rust, and **E** environment or out-of-scope (OS permissions, hardware, model capability).

Scope: the **Computer Controller** extension (`crates/biorouter-mcp/src/computercontroller/`,
tools `computer_control`, `automation_script`, `web_scrape`, `cache`, `pdf_tool`,
`docx_tool`, `xlsx_tool`) plus the **`screen_capture` / `list_windows`** tools it relies
on (in the `developer` extension, `crates/biorouter-mcp/src/developer/rmcp_developer.rs`).

Per-case pass/fail is **not** recorded here. This plan carried an empty status column
for every case in sections A through J; execution outcomes live in the sibling results
document, which reports on cases `A1`, `A2`, `B6`, `B8`, `C4` and `H4` by name and on
several further scenarios by description. Section K is the exception below because it is
a static code review rather than a live run, so its findings were recorded inline.

The plan states that all live testing uses **Xiaomi MiMo `mimo-v2.5-pro`** as a project
requirement, and that MiMo is declared vision-capable (`with_vision()`) so screenshots
reach the model the same way they do for Anthropic and OpenAI.

> **Warning.** No source is cited for that requirement, and the executed results
> contradict it in practice: `mimo-v2.5-pro` on the endpoint under test returned
> `404: No endpoints found that support image input`, so the vision cases were run on
> `mimo-v2-omni` instead. See
> [the MiMo vision finding](./executed-test-results.md#mimo-vision-is-model--and-endpoint-specific).

## Known problems this plan targets

Reported by the user; `K1`–`K5` are problem identifiers, not case IDs.

| # | Reported problem | Root cause found | Fix |
|---|---|---|---|
| K1 | Shuffles screens / circles without progress | `computer_control` always returned "Script completed successfully" even when the AppleScript/PowerShell failed (stderr + exit status were discarded) → fake success → blind retries | **T** (honest error reporting) + **P** (progressive operating principles) |
| K2 | Burns tokens going back and forth | Same fake-success loop + no "stop after 2 failed attempts" guidance | **T** + **P** |
| K3 | Multi-monitor: "all things failed to be found" | `screen_capture` captured `monitors.get(0)` (not primary), never reported how many displays exist | **T** (primary default + topology report) + **P** |
| K4 | App nav (Slack etc.): doesn't know what's clickable | exact-string window matching; no guidance to prefer app-native automation over blind clicks | **T** (substring window match + list on miss) + **P** (clickability order) |
| K5 | Mac-only specifics break on Win/Linux | per-OS instruction blocks existed but core guidance was thin | **P** (OS-invariant operating principles in shared instructions) |

## A — Screenshots and vision (`screen_capture` / `list_windows`)

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| A1 | "Take a screenshot of my main screen and describe what you see." | screen_capture | Captures **primary** display; model describes UI from the image | K3, vision | T |
| A2 | "How many monitors do I have and what's on each?" | screen_capture | Result lists full display topology (index, name, res, primary); model reports count | K3 | T |
| A3 | "Screenshot my second monitor." | screen_capture(display:1) | Captures display 1; if it doesn't exist, error lists valid indices | K3 | T |
| A4 | "Screenshot the Slack window." | list_windows → screen_capture(window_title) | Substring match finds "Slack \| …"; captures it | K4 | T |
| A5 | "Screenshot a window that isn't open (e.g. 'Photoshop')." | screen_capture | Error lists the open window titles to choose from (no dead-end) | K4 | T |
| A6 | "List all open windows." | list_windows | Returns titles; empty titles filtered | — | — |
| A7 | "Capture display index 99." | screen_capture(display:99) | Helpful error: "Display 99 does not exist. N display(s) … valid indices 0..=N-1" + topology | K3 | T |
| A8 | "Read the text in this screenshot." | screen_capture | Vision: model OCRs/reads on-screen text (requires vision — now enabled for MiMo) | vision | T(prereq) |
| A9 | "What app is currently focused?" | screen_capture + describe | Model identifies foreground app from the image | — | P |
| A10 | "Compare the layout of my two monitors." | screen_capture x2 (display 0 and 1) | Two captures; model uses topology to pick indices, doesn't loop on display 0 | K3 | T |

## B — Basic UI interaction (`computer_control`)

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| B1 | "Open the Calculator app." | computer_control | App launches; success reported only if it actually launched | K1 | T |
| B2 | "Type 'hello world' into the focused text field." | computer_control | Types; verifies via screenshot before claiming done | K2 | P |
| B3 | "Press Cmd+S / Ctrl+S." | computer_control | Sends keystroke; reports honestly | — | P |
| B4 | "Click the button labeled 'Save'." | computer_control | Prefers named-element click; if not found, lists elements rather than guessing coords | K4 | P |
| B5 | "Scroll down in the active window." | computer_control | Single action then verify | K2 | P |
| B6 | "Run an AppleScript/PowerShell that has a syntax error." | computer_control | Returns an **error** (not fake success); model stops and reports | K1 | T |
| B7 | "Do a UI action while accessibility permission is denied." | computer_control | Permission error surfaced; model tells user to grant Accessibility, does NOT retry | K1 | T+P |
| B8 | "Activate an app that doesn't exist." | computer_control | Error surfaced honestly | K1 | T |
| B9 | "Move the mouse to coordinates (100,100) and click." | computer_control | Works but guidance steers toward named elements first | K4 | P |
| B10 | "Open System Settings to the Displays pane." | computer_control | Opens specific pane via app-native automation | — | P |

## C — Application control

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| C1 | "Quit the Notes app." | computer_control | Quits via app automation | — | P |
| C2 | "Create a new note titled 'Test' with body 'Hello'." | computer_control | Uses app's scripting interface | K4 | P |
| C3 | "Open Safari/Chrome and go to example.com." | computer_control | Opens URL via browser automation | — | P |
| C4 | "List the names of all running applications." | computer_control / automation_script | Returns process/app list | — | P |
| C5 | "Bring the Terminal window to the front." | computer_control | Activates/raises window | — | P |

## D — System settings (must be OS-invariant in guidance)

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| D1 | "Set the system volume to 30%." | computer_control | Adjusts volume via OS API; honest result | K5 | P |
| D2 | "Mute the volume." | computer_control | Mutes | K5 | P |
| D3 | "Toggle dark mode." | computer_control | Toggles appearance | K5 | P |
| D4 | "Set screen brightness to 50%." | computer_control | Adjusts brightness (may be E on some hardware) | K5/E | P |
| D5 | "Turn Wi-Fi off and back on." | computer_control | Toggles Wi-Fi; honest result | K5 | P |
| D6 | "What OS am I on and what's the version?" | automation_script | Returns OS/version (cross-platform command choice) | K5 | P |

## E — File operations and document tools

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| E1 | "Extract the text from this PDF." | pdf_tool(extract_text) | Returns text | — | — |
| E2 | "Extract images from this PDF." | pdf_tool(extract_images) | Saves PNGs to cache | — | — |
| E3 | "Read this .docx and summarize it." | docx_tool(extract_text) | Returns structured text | — | — |
| E4 | "Create a .docx with a heading and a paragraph." | docx_tool(update_doc) | Writes doc | — | — |
| E5 | "Add an image to a .docx with a caption." | docx_tool(add_image) | Embeds image | — | — |
| E6 | "Open this .xlsx and show columns / a cell / a range." | xlsx_tool | Reads cells/ranges correctly (row/col not transposed) | — | — |
| E7 | "Find a value in the spreadsheet." | xlsx_tool(find) | Returns matches | — | — |
| E8 | "Copy fileA to fileB, then read fileB." | automation_script | Shell copy + read; honest exit codes | K1 | T |
| E9 | "Sort & dedupe lines of a text file." | automation_script | Correct output; failures reported | K1 | T |
| E10 | "Run a shell command that fails (exit 1)." | automation_script | Reports failure with stderr + exit code (already correct) | K1 | — |

## F — Web

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| F1 | "Scrape example.com as text." | web_scrape(text) | Saves to cache; returns path | — | — |
| F2 | "Fetch a JSON API and validate it." | web_scrape(json) | Rejects invalid JSON with clear error | — | — |
| F3 | "Download a binary file." | web_scrape(binary) | Saves bytes | — | — |
| F4 | "Scrape a URL that 404s." | web_scrape | HTTP error surfaced | — | — |
| F5 | "Fill a form on a web app." | computer_control (browser automation) | App-native browser automation preferred over scraping | K4 | P |

## G — Complex multi-app workflows (the hard cases)

| ID | Task | Tool(s) | Expected | Targets | Fix |
|----|------|---------|----------|---------|-----|
| G1 | "Send 'Standup in 5 min' to the #general channel in Slack." | screen_capture + computer_control | Finds Slack window (substring), navigates progressively, verifies the message field before typing/sending; stops + asks if blocked | K1,K2,K4 | T+P |
| G2 | "Read two text files, synthesize them, and paste the summary into a new Note." | automation_script + computer_control | Reads files via shell, composes, opens Notes, inserts text; verifies each step | K2 | P |
| G3 | "Take a screenshot, describe the error dialog, and click 'OK'." | screen_capture + computer_control | Vision reads dialog; clicks named button | K4, vision | T+P |
| G4 | "Find the latest CSV in Downloads, open it, and chart column 2." | automation_script + xlsx/auto-visualiser | Locates file, reads data, renders chart | — | P |
| G5 | "Copy text from app A and paste into app B." | computer_control (clipboard) | Uses clipboard get/set; verifies | K2 | P |
| G6 | "Set a reminder for 3pm." | computer_control | App-native Reminders/Calendar automation | — | P |
| G7 | "Search for a file by name and open it." | automation_script + computer_control | Shell find + open | — | P |
| G8 | "Summarize the content of the currently open document and email it." | computer_control | Multi-step; progressive verification; honest failure if mail not configured | K1,K2 | T+P |

## H — Errors, permissions and robustness

| ID | Task | Expected | Targets | Fix |
|----|------|----------|---------|-----|
| H1 | UI script blocked by Accessibility permission | Error names the permission; model instructs user, does NOT loop | K1 | T+P |
| H2 | Screen Recording permission missing | `monitor.capture_image()` error surfaced clearly | K3 | T |
| H3 | Same failing action attempted twice | Model stops repeating (per operating principle #2) | K2 | P |
| H4 | Tool returns no output on success | Model told it's normal; verifies instead of repeating | K2 | T |
| H5 | Headless / no display | screen_capture: "No displays were detected." | K3 | T |

## I — Multi-monitor specifics

| ID | Task | Expected | Targets | Fix |
|----|------|----------|---------|-----|
| I1 | 2 monitors, target on secondary | Model reads topology, captures display 1, doesn't loop on 0 | K3 | T |
| I2 | Primary is not index 0 | Default capture still picks the primary (is_primary) | K3 | T |
| I3 | Monitor unplugged mid-session | Re-enumeration reflects new count | K3 | T |

## J — Cache management

| ID | Task | Expected |
|----|------|----------|
| J1 | List cached files | `cache(list)` returns entries |
| J2 | View a cached file | `cache(view, path)` returns content |
| J3 | Delete a cached file | `cache(delete, path)` removes it |
| J4 | Clear cache | `cache(clear)` empties it |

## K — Cross-OS invariance review (static)

This section is a static code review rather than a live run, which is why it is the only
section carrying results inline.

| ID | Check | Result |
|----|-------|--------|
| K-OS1 | Honest error reporting present on macOS, Windows, Linux backends | ✅ all three return Err with stderr + exit code |
| K-OS2 | Operating principles are OS-neutral (no hard-coded mac-only steps in shared instructions) | ✅ principles in shared `instructions`; OS specifics stay in per-OS blocks |
| K-OS3 | screen_capture/list_windows use cross-platform `xcap` (no per-OS code) | ✅ |
| K-OS4 | Permission guidance phrased generically ("Accessibility / Screen Recording / Automation") | ✅ |
| K-OS5 | `automation_script` picks shell per OS via `get_shell_command()` | ✅ |

## Results summary

See [the executed test results](./executed-test-results.md) for the executed-run notes
(live MiMo TUI/GUI runs + automated tests). Cases that require specific third-party apps
(Slack, Notes) or OS permissions that were not granted in that environment are marked
**E** and were verified by code review and the honest-error path rather than a live click.

## Related documentation

- [Computer Controller executed test results](./executed-test-results.md) — what was actually run on 2026-06-20, the code changes made, and the MiMo vision finding.
- [Multi-app orchestration run](./multi-app-orchestration-run.md) — the 20-task multi-app stress scenario that exercised case G1's Slack path in anger, plus the watchdog and chat-app fixes it produced.
- [Computer Controller extension reference](../../extensions/built-in/computer-controller.md) — current documentation for the tools this plan tests.
- [Xiaomi MiMo provider](../../providers/xiaomi-mimo.md) — provider setup, host selection and the model list for the provider used for live testing.
