# Computer Controller hardening

> Computer-use references in historical investigations below are superseded by the [native Computer Use contract](../../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


This folder documents a single hardening pass over the **Computer Controller** extension (`crates/biorouter-mcp/src/computercontroller/`) and the `screen_capture` / `list_windows` tools it relies on in the `developer` extension. The pass happened: it was planned, executed and shipped on **2026-06-20**, and every fix described across these three documents landed in that commit — honest `osascript` / PowerShell / xdotool error reporting instead of unconditional "Script completed successfully", a primary-display default with full topology reporting for `screen_capture`, case-insensitive substring window matching, OS-invariant operating principles in the shared instructions, a `computer_control` watchdog timeout, a chat-app playbook, and `with_vision()` on the Xiaomi MiMo models. These are **historical records**, kept for the record and for provenance, not as current guidance or as checklists awaiting execution.

Come here when you are tracing why the Computer Controller behaves the way it does — why a failed script now returns an error, why a capture defaults to the primary display, or why the instructions tell the agent to stop after two failed attempts. If you instead want to know what the extension does today, leave for [the Computer Controller extension reference](../../extensions/built-in/computer-controller.md), which is the current user-facing truth. Exactly one recommendation from this pass was deliberately **not** implemented — replacing brittle Slack UI automation with a Slack Web API or webhook path — and that open question is carried by [the Slack posting investigation](../../extensions/slack-posting-investigation.md), not by anything here.

> **Identifier schemes used throughout.** Fix categories are shared by all three files: **P** = prompt / context-engineering fix, **T** = tool-hardening fix in Rust, **E** = environment or out-of-scope (OS permissions, hardware, model capability). Test cases read `<section letter><number>` (`A1`, `B6`, `H4`), where the letter groups cases by area; those are defined in the test plan's matrix. Separately, `K1`–`K5` in the plan's root-cause table identify the five **reported user problems**, not cases.

> **Note.** All live runs in this folder used **Xiaomi MiMo** against one specific macOS developer machine and a sandboxed config. Host names, display models, installed applications and endpoint behaviour are observations from those sessions, not properties of the system.

## Documents in this folder

| Document | What it covers |
|---|---|
| [Test plan and root causes](test-plan-and-root-causes.md) | The matrixed test plan: the five reported user problems with the root cause found for each, and roughly 60 numbered cases across screenshots, UI control, apps, settings, files, web, multi-app workflows, permissions, multi-monitor behaviour and cross-OS invariance. Per-case pass/fail is not recorded here — this is the plan of record for a finished pass. |
| [Executed test results](executed-test-results.md) | The executed outcomes of the pass: the code changes made, the automated tests added, the live Xiaomi MiMo run results, and the finding that MiMo vision is endpoint-specific — `mimo-v2-omni` sees screenshots, while `mimo-v2.5-pro` returned `404: No endpoints found that support image input` on the endpoint under test. |
| [Multi-app orchestration run](multi-app-orchestration-run.md) | A 20-task multi-app stress scenario — Slack messaging, web literature gathering, genome-wide association study (GWAS) resource-site tours and Word report writing — with the live findings from running it and the prompt and tool fixes that came out of them. |

The plan and the executed results are a two-document pair: read the plan for the case matrix and the `K1`–`K5` root causes, and the results for what was actually run against it. The orchestration run is a separate, broader stress of the same code on realistic multi-step workflows.

## Related documentation

- [Computer Controller extension reference](../../extensions/built-in/computer-controller.md) — the current, user-facing documentation for the tools hardened here; the place to go if you wanted present behaviour rather than history.
- [Slack posting investigation](../../extensions/slack-posting-investigation.md) — the still-open follow-up on the one recommendation this pass left unimplemented, replacing Slack UI automation with an API or webhook path.
- [Xiaomi MiMo provider](../../providers/xiaomi-mimo.md) — provider setup, host selection and the model list for the provider used in every live run here.
- [Historical records index](../README.md) — the archive this folder belongs to, and the other completed passes alongside it.
