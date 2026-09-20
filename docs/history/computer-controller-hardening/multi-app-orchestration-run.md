# Computer Controller — multi-app orchestration run

> Computer-use references in historical investigations below are superseded by the [native Biorouter Copilot contract](../../design/computer-use-integration-plan.md). Old script/control and Developer capture routes are removed; web/document tools now belong to `webdocuments`. Historical source paths and test receipts are not current executable guidance.


> **What this is.** A 20-task multi-app stress scenario for the Computer Controller — Slack messaging, web literature gathering, genome-wide association study (GWAS) resource-site tours and Word report writing — together with the live findings from running it and the prompt and tool fixes that came out of those findings.
> **Status:** Historical record — a completed 2026-06-20 hardening pass. The fixes described here (the `computer_control` watchdog timeout, the chat-app playbook, recovery principle #6) landed in that commit. The one recommendation left open, a Slack Web-API path, is carried by [the Slack posting investigation](../../extensions/slack-posting-investigation.md).
> **Audience:** maintainers working on the Computer Controller extension.

The goal was to stress the Computer Controller on realistic, multi-step, multi-app
workflows rather than single tool calls: Slack messaging, web literature gathering on
multiple sclerosis, touring GWAS summary-statistics resource sites, and writing MS Word
reports. The model was **mimo-v2-omni** (vision). Roughly half of this document is the
task list; the rest is the live findings, root-cause analysis and fixes.

Each problem found is tagged with a fix category: **P** = prompt / context-engineering
fix, **T** = tool-hardening fix in Rust, **E** = environment or out-of-scope (OS
permissions, hardware, model capability). These are the same categories used in
[the Computer Controller test plan and root causes](./test-plan-and-root-causes.md).

The live findings below cover a subset of the 20 designed tasks — tasks 1 and 2 (Slack),
5 (PubMed literature), 9 (GWAS Catalog) and 13 (Word report). The remaining tasks are the
designed scenario, recorded here so the same stress run can be repeated.

> **Note.** The environment is one specific developer machine and does not generalize: a
> macOS host with Slack, Microsoft Word (AppleScript + `docx_tool`), Safari and Chrome
> installed, signed in to the **Broccolito Lab** Slack workspace, whose channels include
> `#spoke-tech` and — importantly for task 1 — no `#general`. Runs used a sandboxed
> config. Reproducing the Slack tasks requires an equivalent workspace.

Tool inventory the agent can orchestrate:

- `web_scrape` (HTTP fetch → cache), `computer_control` (AppleScript: browser/Slack/Word UI),
  `docx_tool` (write `.docx` directly), `pdf_tool`, `xlsx_tool`, `cache`,
  `developer` shell + `screen_capture`/`list_windows`.

## Designed tasks

### Slack

Slack is not meaningfully AppleScript-scriptable, so these are UI-automation only.

1. Open Slack, surface the **Broccolito Lab** workspace, and report the visible channels.
2. **Send** a clearly-labeled test message to a Broccolito Lab channel.
3. Read the latest message in a Slack channel and summarize it.
4. Search Slack for messages mentioning "GWAS" and report what you find.

### Web literature on multiple sclerosis

5. Fetch a PubMed search for "multiple sclerosis GWAS" and list the top article titles.
6. Open a review article on MS genetics in the browser and extract the abstract.
7. Gather 5 recent MS literature references (title + link) from a search.
8. Open Google Scholar for "multiple sclerosis susceptibility loci" and capture the top results.

### GWAS summary-statistics resource tours

9. Tour the **GWAS Catalog** (ebi.ac.uk/gwas), search "multiple sclerosis", report summary-stat downloads.
10. Visit **IEU OpenGWAS** and find MS datasets / summary stats.
11. Visit **FinnGen** and locate the MS endpoint summary statistics.
12. Find the **IMSGC** MS GWAS resource and report its download links.

### MS Word reports

13. Write a Word report summarizing MS-genetics literature gathered.
14. Create a Word doc with a title, headings, and a bibliography of MS references.
15. Append a "GWAS resources" section (resource + URL table) to an existing report.
16. Insert a figure with a caption into a Word report.

### Full multi-app orchestration (hardest)

17. Gather MS literature → synthesize a summary → write it into a Word report → post a Slack message announcing it.
18. Search GWAS Catalog for MS summary stats → compile the download links into a Word table → save the report.
19. Read two local text files → synthesize → write a Word report → notify Slack.
20. Find MS GWAS summary stats across 3 sites → write a comparison report in Word → post a summary to Slack.

## Live results and findings

### Tasks that worked well

- **Task 5 (PubMed literature)** ✅ — `web_scrape` fetched the PubMed results page; the
  model parsed the HTML with shell (`sed`/`grep`) and extracted 5 real MS-GWAS article
  titles, and correctly suggested the E-utilities API for structured data.
- **Task 9 (GWAS Catalog tour)** ✅ — the model used the GWAS Catalog **JSON API**
  (`/gwas/api/search?q=multiple sclerosis`) via `web_scrape`, extracted real study
  accessions (GCST003566, GCST005531, …) and the `fullPvalueSet` summary-stats flag,
  and documented the working endpoint. Genuinely useful output.
- **Task 13 (MS Word report)** ✅ (with a note) — a valid 36 KB `.docx` was produced.
  Note: the model **bypassed `docx_tool`** and used `developer` shell + `python-docx`
  instead. Both work; this is a model-preference observation, not a failure. `docx_tool`
  (update_doc creates-if-missing) remains available and is more portable (no python-docx
  dependency).

Conclusion: **web/data/report tasks are reliable.** The hard part — as the user
reported — is **GUI app automation**, specifically Slack.

### Tasks 1 and 2 (Slack) — the observed failure and the fixes

**Observed:** the agent got **stuck in Slack's search/quick-switcher overlay**. It typed
"general" expecting a `#general` channel (this workspace has none — only a Slackbot DM),
Slack said *"couldn't find anything by that name"*, and the agent didn't know to dismiss
the popup and use the composer. The transcript also showed it issuing a **blind
coordinate click** (`click at {230,170}`) and repeatedly hitting
`System Events ... AppleEvent timed out (-1712)` — **each hung call blocked ~120 s**,
burning time/tokens (the user's "circling / consuming tokens" symptom).

**Root causes and fixes:**

| Cause | Category | Fix |
|---|---|---|
| `computer_control` could hang ~120 s on a blocked UI (AppleEvent timeout) | **T** | Added a **watchdog**: the script runs under a tokio timeout (default 45 s, `BIOROUTER_COMPUTER_CONTROL_TIMEOUT_SECS`). On timeout it returns fast with actionable guidance ("UI is blocked by a modal/overlay; screenshot, press Escape, change approach; do NOT re-run") instead of stalling. |
| No app-specific playbook for chat apps → wrong flow (search vs. composer vs. quick-switcher) | **P** | Added a **"Driving messaging & chat apps"** section to the instructions: composer is the box at the bottom (click → type → Return → verify); switch channels with the quick switcher (Cmd/Ctrl+K) **not** Search (Cmd/Ctrl+G); don't assume `#general` exists. |
| Got stuck in an overlay and kept typing into it | **P** | Added operating principle **#6 (recover from popups/hangs)**: if a popup/dialog/search is open or an action hangs, press Escape, screenshot, reassess; never keep typing into an unresponsive popup or re-send a timed-out script. |
| Blind coordinate clicks | **P** | Reinforced the existing method-preference order (app automation → keyboard → named element → coordinates last). |
| Slack itself is not meaningfully AppleScript-scriptable | **E** | Inherent; UI automation is the only path. For reliable Slack posting, a Slack API/webhook integration would be the real solution (out of scope for this pass; noted as a recommendation). |

**Verification (live, mimo-v2-omni):** re-ran the exact recovery step that was stuck.
The agent activated Slack, **pressed Escape to clear the leftover search popup**, took a
screenshot, and correctly reported **(a) no search popup open** and **(b) the composer is
at the bottom** ("Message #spoke-tech input field"). This is precisely the step it could
not do before (trapped in the search overlay, never locating the composer). With the
overlay cleared and the composer identified, the composer→type→Return send flow proceeds
normally.

### Keychain grant lost on every dev rebuild

An operational problem discovered during testing: the dev `biorouter` binary was
*adhoc*-signed, so every `cargo build` changed its signature and **voided the macOS
Keychain grant** for the provider API key (runs then failed with
"XIAOMI_MIMO_API_KEY not found"). Re-signing the dev binary with the UCSF Developer ID
(stable identity, as `just copy-binary` does) restores the grant and keeps it across
rebuilds. Worth doing automatically after a dev rebuild.

### Recommendation, not implemented in this pass

For *reliable* Slack posting, add a Slack Web-API / incoming-webhook path instead of UI
automation — UI automation will always be brittle for Slack. The fixes above make the UI
path fail gracefully and recover, but an API integration is the real solution (separate,
larger piece of work). That option is worked through in
[the Slack posting investigation](../../extensions/slack-posting-investigation.md),
which reaches the same conclusion.

## Related documentation

- [Computer Controller executed test results](./executed-test-results.md) — the sibling record of the same 2026-06-20 pass, covering the single-tool cases, automated tests and the MiMo vision finding.
- [Computer Controller test plan and root causes](./test-plan-and-root-causes.md) — the case matrix and the K1–K5 root causes that this run was designed to stress.
- [Slack posting investigation](../../extensions/slack-posting-investigation.md) — the follow-up on replacing Slack UI automation with an API or webhook path.
- [Computer Controller extension reference](../../extensions/built-in/computer-controller.md) — current documentation for `computer_control`, `web_scrape`, `docx_tool` and the rest of the tool inventory used above.
