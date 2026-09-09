# Chat summary and academic figure acceptance

## Contract

The chat summary is a compact contextual popover, not a dashboard: use the
application's small typography, neutral surfaces, spacing and standard buttons.
Show persisted To Do state as an ordered stepper with explicit status text and a
completion count. Connections indicate list order, not inferred dependencies.
Hide the section when there are no tasks. Task updates, reopening, renaming,
replacement and chat switching must not show another chat's or an obsolete list.
The task list is a scroll region with a tab stop so a keyboard user can scroll
it; it takes no focus fill (D-15's third amendment, `.biorouter-focus-region`),
and the next Tab lands on the panel's own buttons, which do.
Amended 2026-09-08 (D-15's fourth): the focused list draws a 1px inset
`--border-accent` edge — the exemption had removed the fill *and* the UA ring,
leaving a keyboard user with no indicator at all (WCAG 2.4.7).

Generated figures prioritize the data: meaningful titles, units, source or
assumption notes when relevant, restrained theme-aware colors, and large legible
labels fitted to the available space. No decorative gradient banners or generic
subtitles. Explicit user styling remains supported. Accessible data and controls
must remain useful without relying on color alone.

## 2026-08-30 development evidence

These checks used the isolated development profile, not the installed release.
Sol owned design, implementation and review; Luna drove the desktop through
Computer Use. The daemon was rebuilt before the following fresh-artifact run.

| Check | Result |
| --- | --- |
| Existing chat with two completed tasks | Compact metrics and 2/2 completed rows visible |
| Empty new chat | Metrics visible; no To Do section |
| Three-step fictional comparison | 1/3 complete, one in progress, one pending |
| Finish verification and present a chart | 3/3 complete; fresh chart rendered |
| Reopen comparison and rename verification | 2/3 complete; correct name/status; no duplicates |
| Tool activity names | Semantic task operations, including starting, completing, reopening and renaming |
| Figure aesthetics | Neutral background, informative heading, labeled axes, series controls, data disclosure |
| Comparison accuracy | With an explicit 30-day month, 15/20 hours saved, $8/$9 per hour, $12 per incremental hour |
| Summary keyboard | Escape from inside the popover closes it and returns focus to its trigger |
| Desktop artifact resizing | Approximately 300/440-pixel previews remained rendered; hover tooltip overlap observed, not formal collision coverage |
| To Do disabled in Settings → Chat → Capabilities | Fresh chat reported unavailable and made no checklist call (~12 seconds) |
| To Do restored to its original enabled setting | New chat created/completed two items through real tools (~22 seconds); installed extension count stayed at two |

The chart maps monthly cost to x and daily time saved to y. Its plotted points
and labels agree. The test prompt did not prescribe the reverse mapping, so an
initial tester report of reversed axes was not a product defect. Native HTML
data disclosure is not a modal: Escape dismissal is not its acceptance contract.
Test Escape/focus return on the summary popover itself.

The three-step run's final summary showed 10 tool calls, 283k billed tokens and
one artifact. This is observed UI accounting, not evidence that all tokens were
uncached or that the artifact caused that usage. Investigate the provider path
before attributing or optimizing it.

The stored accounting separates 99,500 input, 1,804 output and 181,248 cached
input tokens (282,552 total); the current context was 23,928 tokens. Do not
describe this as 283k uncached input or a 283k live context. Figure generation
now also returns a small structured receipt while retaining the complete HTML
resource for the desktop. The receipt says `created`, not `rendered`: successful
HTML generation is not visual verification. Provider conversion and live usage
still need to be checked separately from that serialization regression.

### Soul evidence and retest boundary

In the isolated profile, a fresh chat selected Soul without a user selection.
The live ingestion used the shipped `update-soul` skill and the conversation
ingestion tool, creating two curated pages with raw-source provenance, index
and log updates. The synthetic QA convention was explicitly scoped as test
material, not a real user preference. Read-only lint reported zero errors and
four warnings: all four pointed at existing raw evidence files. A regression
test reproduced those false warnings before the bounded lint correction;
missing files, symlinks and bundle escapes must continue to warn.

A subsequent manual Meditation run staged raw input but the test driver quit
the development app while it was still running. It is **not a passed schedule
test**. Startup recovery, successful completion and cursor advancement remain
required; no manual schedule-file or knowledge-data repair is test evidence.

Automated evidence at this checkpoint:

- Summary parsing, state and rendering: 27 focused tests passed, including two
  review regressions observed failing before their fixes.
- Full desktop suite: 3,824 passed before those two final review regressions;
  focused tests and typechecking passed after their fixes.
- Metadata-only session read: red-first test then passed, preserving private
  session reach checks while omitting conversation history.
- Auto Visualiser Rust module: 102 passed, two ignored.
- Browser fixture harness: 26 checks passed, including 320/480/1200-pixel
  layouts, light/dark examples, explicit styles and keyboard/mouse legends.

### First academic-template batch, final automated checkpoint

- Core library: **3,297 passed, zero failed, one ignored**.
- MCP library: **1,545 passed, zero failed, seven ignored**.
- Specialized browser harness: **90 checks passed** (22 static/VM, 36 layouts,
  32 interactions including keyboard scrolling).
- Generic chart harness: **27 checks passed** (five static/VM, 12 layouts,
  ten legend/tooltip interactions).

Both Rust libraries ran in one two-worker batch. The resource-sweep and
long-title partial-dashboard regressions were observed failing before their
fixes. Dashboard receipts retain independent recovery guidance, exact created/
failed counts and up to eight bounded failure entries with explicit omissions
and truncation flags. The complete artifact retains all error cards.

The browser matrix found and fixed oversized tooltips, transition-time tooltip
positioning, rapid map-resize animation, compressed table values, and unexplained
blank all-zero donuts. Luna independently rechecked narrow/wide donut captures:
readable scrolling tables, explicit zero state and no decorative banner.

Evidence: `/tmp/biorouter-academic-resource-core-green.log`,
`/tmp/biorouter-resource-and-dashboard-red.log`, and
`/tmp/biorouter-specialized-figures.Xs09P6/` (`keyboard-final.log`,
`generic-tooltip-green.log`, plus 48 screenshots). These are local development
checks, not final repository-gate or newly rebuilt desktop evidence.

### Rebuilt desktop checkpoint

The daemon built from `05e79cfb` was launched through a PTY-backed Forge session.
The initial pipe-backed launch exited before a daemon started and is not test
evidence. The running binary's SHA-256 was
`7619ed5cbe1bc747397159f62da14de0e381686a869e2174cdd7968b51baf83a`.

Luna drove a fresh fictional clinic comparison in session `20260831_1`:
three meaningful To Do steps finished 3/3; ten tool calls produced one fresh
artifact, with no repeated render call. A costs $120/month for 30 minutes/day;
B costs $180/month for 40 minutes/day. With the stated 30-day month, average
costs were $0.133 and $0.150 per minute, and incremental cost was $0.20 per
additional minute. The parent independently inspected the final screenshot:
compact summary, quiet figure styling, correct cost/time axes and data.

The stored figure response retains the complete HTML plus a bounded structured
receipt (`status: created`, `uri: ui://scatter/chart`). This verifies persistence,
not token savings in an uninstrumented external provider. No resource-discovery
`Method not found` errors appeared in this daemon's log during the run.

Fresh Soul lint in session `20260831_2` loaded `knowledge-lint` and returned zero
errors, warnings, informational findings or truncated diagnostics. No repair
was requested.

The subsequent manual Meditation actually completed from 17:31:00 to 17:34:04
PDT in `20260831_3`. It made one recent-mode recall call with the exact prior
successful-run cursor, then ingested three explicit non-scheduled session IDs:
`20260831_2`, `20260831_1`, and `20260830_24`. Raw staging was `3fc95ec` and curated
updates were `7b0b098`; index/log and provenance links were retained. The parent
verified `currently_running: false`, no error, and `last_run` advancing to
`2026-08-31T00:34:04.581369Z` without editing schedule storage. The initial
apparently-stalled observation was an intermediate snapshot, not a failed run.
Only this isolated test corpus was used; this does not validate all possible
real-user preference inferences.

Two follow-ups emerged: startup wrote cache-only "Soul recovery" commits,
reproduced by an executed Rust regression before its fix; and `BIOROUTER_PATH_ROOT` isolates the
backend but not Electron user data. Future dev launches must also pass the
existing Electron `--user-data-dir` option and verify its resolved log/config
path. Do not copy or clean the installed application's data for a test.

The shipped schema prompt audit also found obsolete `kb_ingest_source`/
`kb_query` names and nonexistent `kb_lint` repair parameters. All three template
files failed a direct contract check before correction, then passed. The added
Rust regression also passed; existing customized schemas are not overwritten.

The manual-run HTTP request waits for completion, while its UI initially only
disabled the run button. A bounded UI fix adds accessible pending feedback,
without claiming completion or changing that API contract. Its two failing
tests passed after the fix; all nine schedule UI tests passed together.
Live pending-message verification and richer in-place run progress remain
separate checks. This patch does not add polling or new cancellation behavior.

## Remaining release evidence

- Finish the scientific-renderer groups below; generic charts and the first six
  specialized templates do not establish all-renderer coverage.
- Finish capability-toggle, subagent and Soul/Meditation live scenarios.
- Run integrated server regressions and all repository gates on the final diff.
- Resolve active-request external catalog refresh and approval-actor questions
  recorded in the coding-agent bridge documentation.
- Remote synchronization and merge remain subject to the outstanding approval
  and review gates. This checkpoint is not a release approval.

### Scientific-renderer follow-up matrix

| Group | Renderers | Required preservation and edge cases |
| --- | --- | --- |
| Quantitative color | Heatmap, calendar, choropleth, shared sequential scale | Monotonic luminance, honest legends, observed zero versus missing, constant ranges, long labels, leap day/timezone boundaries, explicit map view |
| Outcome axes | Kaplan–Meier, forest | Step-after survival geometry, probabilities/censoring, explicit colors; log ratios around 1 versus linear differences around 0, confidence intervals and weights |
| Hierarchies | Network, dendrogram | Directed/weighted edges, isolated nodes, dragging, category cues, long internal/leaf labels, hierarchy and leaf counts; do not imply quantitative branch lengths |

The pre-fix source audit confirmed that the shared sequential scale had relative
luminance 0.127 → 0.657 → 0.266 at normalized values 0/0.5/1, and is used by
heatmap, calendar and choropleth. It is not a monotonic sequential encoding.
Source inspection also found label-to-HTML interpolation in network/heatmap
tooltips and choropleth text. The new templates use a monotonic scale and literal
text; regression fixtures exercise malicious-looking labels without execution.

The follow-up browser matrix is seven renderers × three widths × two themes.
Preserve data semantics, add bounded readable tables and keyboard scrolling, and
measure labels/tooltips rather than applying an indiscriminate font increase.
The new scientific harness baseline has ten passes and 14 failures. Executing
the actual calendar template in a VM confirmed that a March 7–9 fixture drops
March 9 in Los Angeles and shifts labels to March 6–8 in Tokyo; UTC passes.
The final scientific matrix passed 119 checks: 33 static/VM checks, 42 layouts,
and 44 interaction/invariant/edge checks. Evidence is in
`/tmp/biorouter-scientific-figures.L7JK7J/matrix-final.log`. Browser tests caught
buried network arrowheads, fixed by accounting for target-node radius. VM tests
caught zero forest weights being replaced by default weights, and duplicate
calendar dates influencing the scale despite their earlier values being
discarded. Zero remains zero in the table with a documented minimum marker;
calendar duplicates retain the existing last-entry-wins behavior consistently.
Forest invalid weights and calendar expanded/negative years now have Rust
validation regressions pending the final serial run.

Luna reviewed the final corrected forest close-value and network screenshots.
Ticks remain distinct, arrows are visible, and narrow figures explicitly scroll
instead of shrinking labels or overflowing the page. Earlier four browser
harness failures (selectors, resize timing and document reuse) were corrected
in the harness, not misreported as application defects.

The final inventory also includes area, boxplot, bubble, gauge, histogram,
Manhattan, Mermaid, sunburst, volcano, word cloud and dashboard templates.
Those eleven need a follow-on consistency/semantics audit and targeted
fixtures; completing the seven-renderer matrix is not all-renderer coverage.
Do not change statistical or spatial encodings just to unify their appearance.

Luna's lightweight VM probes executed the current gauge and histogram template
scripts with stubbed canvas/Chart hosts. Accepted gauge input
`{value:150,min:0,max:100}` produced arc fractions `[1,0]` and center text `100`,
silently replacing the actual measurement. Histogram input
`{values:[1.00001,1.00002],bins:2}` produced counts `[1,1]` but two identical
`1.00–1.00` labels. These are confirmed template regressions, not browser tests.
A mixed empty/nonempty boxplot was accepted by the backend; source inspection
and an isolated quantile probe found undefined/NaN statistics for the empty
group. The follow-up hierarchy matrix now covers this case.

### Follow-up verification checkpoints

- Local commit `c926829a` contains Soul cache-only history suppression, corrected
  shipped schema tool guidance, and accessible pending feedback for manual
  schedules. The serial Rust run passed 3,298 core and 1,550 MCP tests in
  `/tmp/biorouter-scientific-distributions-core-rerun.log`; the schedule UI set
  passed nine tests. The new pending message still needs a rebuilt live-app check.
- Gauge/histogram: 43 checks passed (11 static/VM, 12 layouts, 20 interaction and
  numeric edge checks) in `/tmp/biorouter-distributions-browser-final.log`.
  Three numeric probes first failed: extreme finite gauge spans, overflowing
  histogram spans, and zero-width bins between adjacent floats. Histogram axis
  titles also clipped at 320/480px before measured wrapping. Counts and exact
  measurements remain unchanged; reduced precision-limited bins are disclosed.
  Luna reviewed five final captures without finding overlap or misleading data.
- Boxplot/sunburst/wordcloud: 61 checks passed (17 static/VM, 18 layouts, 26
  interaction/edge checks) in
  `/tmp/biorouter-hierarchies-figures.1BqgzX/matrix-final.log`. Actual browser tests
  caught boxplot hover interception and wordcloud font-box collisions. Empty
  groups, zero/missing values, duplicate labels, negative hierarchy values,
  omitted words and safe literal labels have explicit handling. Original numeric
  boxplot statistics are retained in the table. Luna reviewed the final captures.
- Area/bubble/volcano/Manhattan: the initial executed VM matrix failed 12 checks;
  the first browser matrix passed 48 and failed eight narrow tooltip checks.
  Responsive layouts fixed the tooltips, but Luna correctly found visible
  ResizeObserver error cards that the first harness had missed. The harness now
  explicitly checks rendered alerts. Both chart layout and shared overflow-hint
  reporting defer/coalesce mutations outside observer delivery; each path has an
  executed failing-then-passing VM regression. The final matrix passed 58 checks
  (14 static/VM, 24 layouts, 20 interactions) in
  `/tmp/biorouter-cartesian-browser-final.log`. Luna rechecked six overwritten
  area/bubble captures and confirmed the banners were gone. The earlier 56-pass
  run is not acceptance evidence. Data fixes preserve exact bubble radii, CSS
  color channels and prototype-like chromosome labels as literal data. A new
  numeric-validation Rust test first failed (one pass/one fail); the backend fix
  is written but its rerun is pending.
- The shared resize repair was then rerun serially through every previously
  completed figure harness: generic 27, specialized 90, scientific 119,
  distribution 43 and hierarchy 61 checks all passed. Including Cartesian 58,
  that checkpoint is 398 checks across 144 layouts. Logs are
  `/tmp/biorouter-*-shared-resize-rerun.log` and the Cartesian final log above.
- Mermaid/dashboard template checks covered all 60 planned layouts. The actual
  typed-tool regressions then failed in three places before repair: colliding
  IDs, lost definitions/references/labels, and unknown/ambiguous Gantt
  dependencies. `/tmp/biorouter-typed-mermaid-red.log` includes a concrete
  three-node flow collapsing into one node with self-loops. The optional export
  test did not export in this red run; actual generated-HTML browser acceptance
  still follows the compiler repair.
- Latest hierarchy and Cartesian Rust tests, subsequent template edits, final UI
  typechecking/gates, and Mermaid/dashboard checks are not covered by the earlier
  3,298/1,550 Rust checkpoint. No release or remote synchronization is implied.

### Final integrated regression run

The full desktop suite passed **3,829 tests in 373 files** using two workers
(`/tmp/biorouter-final-ui-suite-run.log`). An initial invocation rejected the
unsupported `--minWorkers` option before running tests; only the corrected run
is acceptance evidence.

The subsequent combined Rust run passed **3,298 core tests and 1,563 MCP tests**,
with zero failures, one core ignore and seven MCP ignores
(`/tmp/biorouter-final-core-mcp.log`). This includes the latest scientific,
distribution, hierarchy, Cartesian and typed Mermaid regressions. The actual
tool outputs for nine diagram kinds were exported to the explicit temporary
directory `/tmp/biorouter-mermaid-typed.VgAfVh/` for browser verification.

The typed compiler now uses collision-free internal IDs while preserving visible
labels, implicit referenced nodes and the state start/end sentinel. Gantt
dependencies resolve exact IDs before unambiguous ID lists and reject unknown,
duplicate or ambiguous inputs. A separate Sol review found no concrete blocker
in the Cartesian validation or gauge/histogram changes. These checks do not
replace final browser, repository-gate or rebuilt desktop evidence.

The consolidated Mermaid/dashboard browser matrix then passed **94 checks**:
12 static/VM checks, 54 layouts of actual tool outputs, 12 parsed/rendered
identity and dependency checks, two security probes, eight dashboard interaction
checks and six dashboard layouts. Evidence is
`/tmp/biorouter-mermaid-final.8UxajD/matrix-final.log`. Two earlier failures came
from inspecting Mermaid state before its extraction/render phase; the final
checks inspect rendered identities, visible labels and compiled transitions,
and explicitly reject a collapsed-identity negative fixture. A dark-text issue
was confined to a hand-authored tall-panel test fixture that omitted the shared
theme runtime; no production theme change was needed. Luna rechecked the final
dark dashboard and wide class diagram and found no current visual defect.

A final copy review found sunburst guidance promising every node in a table that
is deliberately bounded to 500 rows. The wording now describes that bound and
points to the row-limit caption. Its executed VM regression failed before the
correction (17 passes, one failure), then passed (18 passes, zero failures).
This last sentence change follows the full Rust checkpoint above; the follow-up
hierarchy browser and repository gates provide subsequent validation.

The hierarchy rerun passed **62 checks** with the final guidance
(`/tmp/biorouter-hierarchies-final-caption.log`). Combined with the earlier
matrices and the final Mermaid/dashboard run, the accepted figure checkpoint is
**493 checks across 204 layouts**. This is renderer/browser coverage, not a
claim that every live provider or packaged application has been retested.

The original Destiny archive digest was rechecked as
`a95600b77f0b6a5b5744cacd6cedfa3d19bd16d8ac59662d3be2dc4b727eb529`.
Its retained audit identified repository-skill installation delegated to a child
without shell/editor tools, followed by an invalid Developer-inheritance retry.
The final core run passed
`destiny_style_repository_install_is_routed_to_the_audited_skills_manager`
and the bridge roster, live-grant and dispatch-revocation regressions. The
earlier successful Skills-importer dry-run does not establish approval-dependent
installation or permanent-removal/residue coverage. Those remain distinct from
the separate live skill hot-load/use/hot-unload evidence.

A later metadata-only recheck of the same archive confirmed the exact target
repository as `https://github.com/Broccolito/destiny-skill` (also referenced with
the `.git` suffix). Its three tool requests were a subagent call without an
explicit extension list, a workspace watch, and a second subagent call requesting
`extensions: ["developer"]`; the second spawn returned `isError: true`.
The retry must exercise that exact repository through the audited Skills manager,
not substitute another installed skill or repeat the unsupported child grant.
The archive configuration and credential contents were not displayed or changed.

A no-window probe of this worktree's Electron runtime confirmed that the launch
flag resolves `app.getPath('userData')` to
`/private/tmp/biorouter-goal-gui.gsVSPC/state/desktop`. The first comparison used
the equivalent `/tmp` alias and reported a mismatch; the canonical-path check
passed. BioRouter was not launched by this probe. The real Forge launch must
still confirm the forwarded flag and resolved log path before live testing.

The first full repository gate stopped at three prohibited UTF-8 string slices
in the Gantt resolver. Sol replaced them with character iteration and splits at
match-proven boundaries, without suppressing lints, and added multibyte
whitespace/Unicode-ID regressions. The entire combined Rust suite then passed
again: **3,298 core and 1,563 MCP tests**, with the same ignores and zero failures
(`/tmp/biorouter-final-core-mcp-unicode.log`). All nine newly exported diagram
HTML files matched the SHA-256 manifest captured before that correction, so the
actual-output browser evidence applies unchanged.

The separate UI lint command passed typechecking, ESLint, generated-theme
consistency, all **332 contrast assertions**, and semantic-token mirror checks
(`/tmp/biorouter-final-ui-lint.log`). Luna owns the serial full-gate rerun; any
source fixes remain Sol's responsibility.

The all-target Clippy rerun also found a slice in the new test assertion itself;
Sol replaced it with `strip_prefix` without changing the contract. The subsequent
**complete `just check-everything` passed**, including strict all-target Clippy,
UI checks, OpenAPI regeneration/comparison, version, branding, cross-compile,
registry generation and privacy-registry checks
(`/tmp/biorouter-final-repository-gates-final.log`). Generated schema/SDK files
had no diff. The exact edited identity assertion then passed as one MCP test
(`/tmp/biorouter-final-mermaid-assertion-tests.log`); the filtered core target
ran zero tests and is not additional core evidence.

### Remaining ingestion runtime guarantees

A read-only follow-up at `879ca4c1` confirmed that the successful Meditation run
does not establish a whole-operation timeout or streamed chat-ingest progress.
`knowledge/macros/ingest.rs` acquires locks, refreshes and stages input before
the bounded model loop, then settles/verifies afterward. Refresh still awaits
blocking work without an overall deadline. Both chat ingest helpers pass
`event_sink: None`; the bridge waits for the complete response. Its advisory
600-second task budget is not an enforced dispatch deadline, and transport can
wait 31 minutes.

These are larger follow-ups, alongside safe active-request catalog refresh.
Implementing a timeout by dropping the outer future could leave blocking work
or staged changes unsettled; the design needs cancellation, draining and cleanup
coverage. User triage was requested before expanding that runtime work. No
bounded-total-latency or streaming-progress guarantee is claimed here.

### Full server validation

The first server run passed 547 library tests and failed one stale source-order
assertion looking for the former `get_session(&session_id, true)` call. The route
now supports metadata-only reads, and source inspection confirmed the privacy
gate still precedes both modes. The runtime private metadata/history regression
also passed. Sol updated the assertion to locate the first session read
independently of its history argument, retaining the gate-before-read check,
and corrected the history-only explanatory comment.

The full rerun passed **1,219 tests with zero failures** across library, binary
and integration targets (`/tmp/biorouter-final-server-tests-rerun.log`). Ten
opt-in real Claude/Codex tests remained ignored because they require signed-in
CLIs and spend provider quota; zero-test targets and ignored cases are not
additional coverage. The subsequent complete repository gate rerun also passed
(`/tmp/biorouter-post-server-repository-gates.log`, exit zero).

### Final production-default desktop checkpoint

The production-default daemon and CLI built successfully from `502930f5` in
4m54s. The daemon SHA-256 was
`b55d66a4298ebbe091f72e7facce95e285b8e9192c9821840d9267369f5636f3`.
Actual Electron arguments, renderer working-directory metadata, profile files,
and daemon open-file paths confirmed isolation under the test profile and
`/tmp/biorouter-final-live.cqHHQg`. No installed-user profile was used.

Luna's natural SQLite/comparator request in parent `20260831_4` successfully
spawned child `20260831_5`, but the child correctly reported that its actual
tools could neither create the project nor execute tests. Its grants were the
audited Knowledge, Skills and Extension Manager subset. Developer, Code
Execution, Computer Controller and native host tools are intentionally absent
from this subscription bridge; configured enablement is not callable access.
No SQLite fixture, comparator or executed tests were produced. The attempted
live steer targeted the same child but arrived after it stopped, returning
`target child has no turn in flight; live steering cannot be delivered`.
Spawn is proven; useful executable delegation is blocked on this provider, and
this terminal-child attempt does not prove live steering. Four blocked checklist
items correctly remained pending, and the failed activity was named
`Workspace Send Prompt`. The parent reported the blocker without fabricating
work. No daemon warning, error or resource-method-not-found line appeared.

The unnecessary spawn despite an unavailable execution surface exposed a
planning-guidance weakness. A new focused prompt-contract regression failed
before the wording correction (`/tmp/biorouter-child-preflight-red.log`). The
full green rerun passed 3,299 core and 1,563 MCP tests with zero failures
(`/tmp/biorouter-child-preflight-full-green.log`); all repository gates passed
again (`/tmp/biorouter-child-preflight-repository-gates.log`). The nine exported
Mermaid fixtures still match the previously browser-verified hashes. A supported
follow-on live scenario remains a separate acceptance check.
Do not weaken the credential boundary to make the SQLite scenario pass.

Manual Meditation in `20260831_6` completed from 19:13:56 to 19:16:23 PDT.
Luna observed the accessible pending message and disabled run button, followed
by completion and a re-enabled button. Parent screenshot inspection confirmed
both states; read-only schedule inspection confirmed `currently_running: false`,
no error and `last_run: 2026-08-31T02:16:23.603801Z`. No manual schedule or
knowledge-store repair was made. The development app was then quit normally
after completion, not interrupted mid-run.

### Versa-first live validation and first-load regressions

The user configured Versa in the isolated profile and requested Azure/OpenAI
first, plus Versa Claude coverage; subscription CLIs are fallbacks only if Versa
does not work. A safe endpoint check confirmed HTTPS UCSF gateway hosts without
printing or copying credentials. The installed user's profile remains untouched.

Versa Azure `gpt-5.5-2026-04-24`, session `20260831_11`, executed the real Apps SDK
workflow using the production CLI at `34bf62bd`. Developer shell/file/analysis,
Code Execution orchestration, app creation/build/lint, and real-browser smoke
all executed. The model's final report said 15/15 after repairing the synthetic
app, but that is not a blanket product acceptance result. Inspection found:

- The dashboard starter declares an empty metrics object while binding six
  visible values. Initial labels are now explicit unavailable-data labels, not
  fabricated numeric measurements.
- The resolved manifest projection dropped `surface.state_initial`, even when
  present on disk. This made authoring/read-back misleading and could discard
  initial state on an editable-manifest round trip. The projection now fills
  omitted defaults without replacing serialized surface fields.
- The smoke harness injected app identity and endpoint but omitted the manifest
  initial state. It therefore continued reporting blank values after the model
  correctly supplied initial state. The model worked around that with duplicate
  app-side initialization. The harness now follows the served app's initial
  state contract; a rebuilt repeat must succeed without that workaround.

Actual fail-first MCP run: 250 passed, three intended assertion failures, two
ignored (`/tmp/biorouter-dashboard-state-red.log`). After the changes: 253 passed,
zero failed, two ignored (`/tmp/biorouter-dashboard-state-green.log`). The separate
executing browser regression first failed to paint declared zero/false/string
values while correctly rejecting absent initial values. All five final smoke
tests pass, including script-like strings and malformed manifests
(`/tmp/biorouter-smoke-initial-state-green.log`). Missing browsers and skipped
execution are not accepted as passes.

The original live workflow also demonstrated several avoidable single-call
Code Execution wrappers and repeated repair/report turns. These are recorded
as efficiency observations, not provider outages. Its reports are under
`/tmp/biorouter-final-workflow.1CjILG/workspace`; its model-written success claim
must be read alongside the independent findings above. This was CLI/browser
smoke evidence, not a Luna visual acceptance of the embedded preview.

The newly requested file-link harness has an executed fail-first baseline:
33 selected cases, 25 failing and eight baseline safety passes. Coverage includes
later-turn shorthand, same-session provenance, ambiguous names, session loading,
shell working directories, encoded filename identity, and source-line preview
handoff (`/tmp/biorouter-file-links-red.log`). Implementation and green validation
are in progress; earlier UI-suite counts do not cover these new changes.

The approved managed-app preview fix also has an executed fail-first assertion:
the managed URL incorrectly reused the public remote-browser session
(`/tmp/biorouter-managed-preview-red.log`). Independent security review and real
Electron transport/lifecycle tests remain required before claiming isolation.

### Artifact and stream-boundary follow-up (August 30, 2026)

The user subsequently accepted Chromium's remaining DNS behavior for public-facing
artifacts and declined a separate isolation process. This changes the artifact
acceptance criterion, not daemon credentials, private-file authorization, or the
external-page network policy. Do not describe the preview as having no external
network activity: TURN hostname resolution and DNS prefetch can bypass the pinned
app proxy. The revised probe characterizes that limitation with independent
positive controls while still requiring TCP destination and app-route denial.

The final focused managed-preview run passed 198 tests across eight files
(`/tmp/biorouter-managed-preview-focused-final.log`). The actual Electron probe
passed eight checks (`/tmp/biorouter-managed-preview-electron-final.log`): app and
bundle loading without credentials; daemon/cross-app/other-listener/service-worker
denial; IndexedDB persistence across reload; TCP/TURNS denial; DNS characterization;
and backend revocation. This is real isolated Electron execution, not yet the
rebuilt application's Luna GUI acceptance.

File-link reliability now passes all 81 selected cases and all 306 tests in the
six-file focused run (`/tmp/biorouter-file-links-full-focused-final.log`). The
broader run initially caught an old typography fixture lacking a session working
directory; the fixture now supplies one without weakening the resolver contract.
Qualified relative paths use the actual session directory; only unambiguous
earlier same-session provenance resolves bare filenames. Ambiguity stays inert.
Same-path artifact/app live-refresh tests independently reproduced two failures
(`/tmp/biorouter-artifact-realtime-red.log`); implementation is in progress.

The provider matrix was also expanded beyond fallback use: Versa GPT and Claude,
plus Codex and Claude Code, must be exercised with meaningful build/database
tasks. The observed Versa GPT deployment is `gpt-5.5-2026-04-24`; a requested
GPT-5.6 deployment must be verified rather than inferred from the public model
catalog. Strong-model agents implement/review; Luna executes tests and drives UI.

Versa Claude Opus 4.8 session `20260831_12` built and tested a synthetic SQLite
comparator, but had three unclosed streamed tool blocks: one checklist call and
two file-editor calls. Existing metadata-only logs cannot establish whether the
JSON itself was malformed. The warning previously conflated missing block closure
with truncated arguments. The model recovered to produce real files and eight
unit tests, but attempted a nonexistent checklist namespace and later misstated
which option saved the most time. Neither successful file creation nor its final
report proves checklist or answer-quality acceptance.

Executed GPT decoder baseline: eight cases, seven failures and one positive
control (`/tmp/biorouter-openai-stream-red.log`). Confirmed failures include lost
pending calls at `[DONE]`, dispatch at unconfirmed EOF/length boundaries,
non-object JSON panic, valid no-space SSE fields ignored, and usage-only events
prematurely closing argument assembly. Two Bedrock classification regressions
also failed as expected (`/tmp/biorouter-bedrock-stream-red.log`). At the real
agent-loop boundary, all three unsigned retry-feedback cases failed while the
signed-abort control passed (`/tmp/biorouter-malformed-recovery-red.log`). Surgical
corrections and independent review are underway; these red runs are not green
validation. The diagnostic wire probe uses synthetic input and metadata-only
reports, never executes generated tools, and must pass offline framing controls
before its observations are used to attribute a transport/SDK/provider cause.

The decoder/recovery fixes are now checkpointed locally in `eab580ca`. Final
focused results: ten OpenAI boundary tests, two Bedrock classification tests,
four agent-loop recovery tests, 40 existing OpenAI unit tests, and 54 existing
Bedrock unit tests all passed. Independent review caught a post-`[DONE]` polling
edge in the first patch; a terminal latch and poison-tail/pending-tail regressions
close it. An initial compile failure from a missing macro import and tracing's
`Value` name shadowing was corrected before these green results. These focused
results do not replace the full repository gates or rebuilt desktop acceptance.

Nine offline wire-probe controls passed before live capture. Two identical
medium SQLite-generation requests to Versa Claude Opus 4.8 returned HTTP 200/H2
and clean EOF at complete, CRC-valid event-frame boundaries, but with missing
tool-block stop, message-stop, and usage events. The first stopped after 20 tool
argument bytes at 10.940 seconds; the second after 71 bytes at 43.525 seconds.
Both argument strings were incomplete JSON. Replaying each exact response body
through the SDK produced identical events, indices, and argument lengths.
A short checklist request completed: 478 argument bytes, `tool_use` stop reason,
524 input and 286 output tokens, 32.628 seconds. All requests used the same
64,000-token configured inference ceiling. No returned diagnostic tool executed.

This evidence places the missing completion in the received HTTPS body, not the
local SDK's index handling or BioRouter's JSON assembly. It does not distinguish
the UCSF gateway from upstream Bedrock/model delivery. The differing cutoff
times and successful longer/larger checklist response do not support a simple
uniform timeout or response-byte ceiling. The HTTP/1.1 differential and Versa GPT
checks remain separate follow-up evidence; do not call a successfully captured
incomplete stream a successful generated tool call.

### Current-source preview and provider validation checkpoint

The realtime preview implementation is checkpointed locally in `2ebedfb0`.
The final focused run passed 191 tests across nine files and TypeScript checking
passed (`/tmp/biorouter-artifact-realtime-focused-final2.log` and
`/tmp/biorouter-artifact-realtime-typecheck-final2.log`). The expanded real Electron
probe passed 11 checks (`/tmp/biorouter-artifact-realtime-electron.log`), including
same-view source updates, dirty values retained after blur, explicit refresh with
IndexedDB persistence, and pending/connecting SDK work deferring refresh. Updates
occur at successful tool boundaries; arbitrary external filesystem watching is
not implemented. External websites never auto-refresh because a local tool ran.

The full UI run then passed 4,054 of 4,056 tests across 382 files
(`/tmp/biorouter-post-preview-full-ui.log`). Both failures were in the old thinking
disclosure fixture, which omitted required message metadata through an unsafe
type cast. The fixture now supplies real visibility metadata and no longer uses
the cast. No production contract was relaxed; the full rerun is still required.
The materially lighter, single-file follow-up passed both corrected tests in
1.65 seconds (`/tmp/biorouter-thinking-fixture-only.log`). This does not replace
the denied full-suite rerun.

The updated `biorouter-self-test.yaml` passed template validation with the
already-built CLI (`/tmp/biorouter-selftest-template-validate.log`) in a fresh
isolated profile. This only validates YAML and the workflow template; no model
or self-test phase executed, and it is not rebuilt-binary acceptance.

The wire probe now passes all 15 offline controls
(`/tmp/biorouter-versa-wire-controls-final.log`). A shorter 10–15-line SQLite
editor request completed on the same Claude Opus 4.8 model, editor schema, path,
and inference ceiling: HTTP 200/H2, 20,041 CRC-valid event bytes, 519 argument
bytes forming an object, closed tool block, `tool_use` message stop, and usage
678 input / 415 output tokens in 13.420 seconds
(`/tmp/biorouter-versa-wire-python-short.json`). This rules out a universal editor
schema rejection, but does not identify the gateway/upstream cause of the two
larger-request failures. The HTTP/1.1 version of the larger request returned
HTTP 500 after 35.638 seconds; it is not a justified production workaround.

The real Versa Azure provider/decoder probe reported `gpt-5.6` unavailable
(`/tmp/biorouter-versa-gpt56-todo.json`); an exact alternative deployment name
remains a user input, not a model-catalog assumption. Verified deployment
`gpt-5.5-2026-04-24` completed both cases: a 314-byte checklist call in 9.164
seconds and a 15,068-byte SQLite editor call in 41.965 seconds, each with object
arguments, `tool_calls` finish reason, and terminal usage
(`/tmp/biorouter-versa-gpt55-todo.json`,
`/tmp/biorouter-versa-gpt55-python.json`). These probes never execute their tools;
they establish stream completion, not correctness of generated applications.

Independent review found two pre-existing risks in the GPT probe's normal
provider path: the shared HTTP client can follow cross-origin redirects with a
custom API-key header, and upstream error text reaches ordinary request/tracing
logs before the probe's safe summary projection. There is no evidence here of
an actual redirect or credential disclosure, but the probe cannot promise a
metadata-only side effect on error. Further live GPT probes are paused pending
the scoped hardening decision. These API concerns are separate from the accepted
artifact DNS limitation; neither is silently treated as resolved.

The production CLI/daemon still require rebuilding from these commits. Final
repository gates, rebuilt-app Luna interaction, useful Codex/Claude Code tasks,
and accepted running-child steering remain distinct acceptance items. No remote
push, merge, or release is claimed by this checkpoint.

At 20:51 PDT the next full UI rerun was rejected by execution auto-review before
process creation because only 127 GiB remained free, below the repository's
200 GiB resource threshold. No rerun or workaround was attempted. The read-only
resource snapshot showed 62.65% CPU idle, load 11.22, 40% memory free, 6.49 GiB
swap in use, and no thermal warning. Further heavy validation needs recovered
resources or explicit informed user approval; no unrelated files were removed.

### Remaining acceptance requirements at `2ebedfb0`

- External capability toggles during a provider request still revoke dispatch
  without refreshing its prompt/tool snapshot. Manager-driven refresh is covered;
  the external-event path needs the separately triaged safe refresh boundary.
- The actual skill removal succeeded, but its approval actor is not established.
  A metadata-only read of isolated session `20260830_25` confirms the successful
  `skills__removeSkillPackage` receipt at 22:57:45 UTC and failed later reload,
  not who approved it. Permanent Spoke removal and attributable approve/cancel
  checks remain required. Do not delete credentials or historical records merely
  to manufacture zero residue.
- The exact Destiny repository/bundle path still needs approved real
  install, load/use, removal and failed-reload checks; its importer dry-run and
  the separate skill lifecycle are not substitutes.
- Meaningful Codex and Claude Code tasks, accepted steering of a still-running
  child, subsequent child work, and parent collection remain required. Use the
  child's actual audited Knowledge/Skills/Extension Manager tools; parent
  Agent Drafter support does not grant a child native shell/editor access.
- Current-source dashboard initialization, decoder recovery, file navigation and
  live preview behavior still need rebuilt-app Luna verification. Recorded
  successful Soul ingestion and Meditation (including pending/completion UI)
  remain valid evidence; they are not reopened simply because another check is
  pending. Explicit useful live receipts for Memory and Computer Controller are
  still missing from this QA record.
- Full repository gates and a green full UI rerun remain required. Remote source
  synchronization is separately awaiting clearance of the denied push. No merge
  or release is authorized by partial validation.

### Approved API hardening and resumed validation

The user explicitly approved API redirect/private-log hardening and serial
validation below the 200 GiB disk threshold. The resumed snapshot showed 122 GiB
free disk, 64.22% CPU idle, 39% memory free, 11.37 GiB swap, and no thermal
warning. Keep one validation job active; this approval does not clear the denied
GitHub push or unrelated larger-scope changes.

The complete desktop suite passed **4,056 tests across 382 files** in 212.18
seconds (`/tmp/biorouter-post-preview-full-ui-approved.log`). This supersedes
the prior fixture-failure checkpoint without claiming rebuilt-app acceptance.

Executed fail-first tests confirmed five private-log failures and five of ten
API-boundary failures (`/tmp/biorouter-private-error-logging-red.log`,
`/tmp/biorouter-api-redirect-red.log`). All four cross-port 307/308 cases sent a
request to the second loopback listener, and the transport DEBUG log exposed
the synthetic prompt. Same-origin preservation and redirect-limit controls
passed. Private errors were formatted/logged; HTTP and retry trace paths also
exposed the synthetic sentinel. No real credential was used in these tests.

The correction pins every API redirect to the original scheme, hostname and
effective port, preserves the ten-hop limit, and survives client rebuilding.
Private request logs branch before formatting upstream errors and retain only a
redaction marker plus an optional trusted enum classification. Shared HTTP,
Google-compatible and retry traces retain status/count/class metadata, not raw
payloads. The redundant transport prompt log is removed. Returned provider
errors and retry budgets are unchanged; deliberate public/full request logging
remains available. Existing historical log files are not rewritten or deleted.

Luna's green runs passed **32 logging utility tests, two origin-policy tests,
and 13 real HTTP integration tests** (`/tmp/biorouter-private-error-logging-green.log`,
`/tmp/biorouter-api-origin-green.log`, `/tmp/biorouter-api-redirect-green.log`).
Coverage includes 301/302/303 as well as 307/308, zero requests reaching denied
destinations, scheme downgrade/hostname/effective-port cases, constructor and
rebuild paths, private/public logging contrast, a formatter that must never run,
and exact original errors and retry counts. Independent Sol review found no
actionable issue. The normal Versa GPT probe inherits these transport/logging
fixes; live useful-task and full-repository validation still follow separately.

The first resumed full core/MCP command stopped at the core library with 3,303
passes, four failures and one ignored test
(`/tmp/biorouter-post-hardening-core-mcp.log`). Three snapshots differed only in
the intended persistent absolute-file-link guidance. The fourth test still
asserted the replaced wording. All three diffs were reviewed before updating
expectations; the behavior test now requires verified absolute paths on every
turn, distinct short labels versus full targets, and an absolute source-line
example. The production prompt was not relaxed. MCP and integration binaries
had not yet run when Cargo stopped; the focused and full reruns follow separately.

The focused prompt-manager rerun passed all 24 tests
(`/tmp/biorouter-prompt-manager-green.log`). Independent Sol review confirmed
that only the expected snapshot paragraph and assertions changed. The full
core/MCP rerun remains a separate gate.

That rerun passed the 3,307 core library tests (one ignored), then stopped in
the legacy `providers` integration target: 13 passed, one failed, five ignored
(`/tmp/biorouter-post-hardening-core-mcp-rerun.log`). The failure was a real
Anthropic HTTP 400 insufficient-credit response, not evidence of a local
provider regression. The target loads `.env` and previously ran thirteen
network/provider tests whenever credentials were present; only five Bedrock
tests were explicitly ignored. Cargo's `--offline` prevents dependency network
access, not test-body API calls. No further live-provider retry was attempted.
This exposed a test-isolation defect: ordinary regression runs must not infer
authorization for billed calls from configured credentials. The narrow follow-up
marks live tests opt-in while retaining registry and deterministic checks, with
a harness-metadata regression that never executes a provider call. Useful Versa,
Codex and Claude Code acceptance remains required independently of this gate.

The metadata guard failed before the fix on the ordinary OpenAI test
(`/tmp/biorouter-live-provider-optin-red.log`), without executing an API call.
After all eighteen live cases were marked opt-in, execution review refused the
unfiltered target before process creation because of its earlier live behavior.
That command was not retried. Exact safer invocations passed the metadata guard
and registry check independently (one test each, nineteen filtered):
`/tmp/biorouter-live-provider-optin-green-exact.log` and
`/tmp/biorouter-provider-registry-green-exact.log`. Independent Sol review found
no actionable issue. Further aggregate validation explicitly excludes all
eighteen live test names as well as retaining their ignored attributes; none of
those excluded cases counts as a passing API test.

The explicitly filtered aggregate was accepted and completed **3,695 core unit
and integration tests across 67 targets**, with seventeen ignored and the
eighteen live provider cases filtered out. It then passed **1,565 MCP library
tests** (seven ignored), but stopped at the generated Mermaid CDN fixture
contract (`/tmp/biorouter-post-hardening-core-mcp-offline-final.log`). Core
documentation tests and remaining MCP integrations had not run at that point.

The fixture diff was reviewed before replacement: it lacked the already
implemented academic palette, readability helpers and current Mermaid layout.
The corrected fixture's embedded runtime exactly matches `_common.js`; its HTML
matches `mermaid_template.html` after expected substitutions. Independent Sol
review confirmed unchanged CDN URL/tag and strict Mermaid security. Only the
generated fixture changed, not production rendering code. The contract and
real-browser replay must pass before the remaining full MCP/server/gate runs.

The corrected CDN contract passed its single test
(`/tmp/biorouter-autovis-cdn-contract-green.log`). Both real Chromium replay
checks also passed in 7.07 seconds (`/tmp/biorouter-autovis-cdn-browser-green.log`):
the current figure renders after vendored-library inlining, while the unreachable
ESM-import negative control produces an error instead of a diagram. The browser
run did not need a CDN request or a user-app session.

The full MCP rerun passed **1,635 tests**, with eleven ignored and no failures
(`/tmp/biorouter-post-hardening-mcp-final.log`). Its documentation-test phase had
zero cases, not additional executed coverage. The separate core documentation
run passed two cases, with two ignored
(`/tmp/biorouter-post-hardening-core-doc.log`). Server tests and repository gates
remain the next serial stages.

The full server suite subsequently passed **1,219 tests**, with ten ignored and
no failures (`/tmp/biorouter-post-hardening-server.log`). Its documentation phase
contained zero cases. The required `just check-everything` run follows separately;
the test totals alone do not establish lint, schema or repository-gate success.

The first repository-gate attempt stopped on Clippy's double-ended-iterator
lint in the new OpenAI stream regression. Replacing `.last()` with `.next_back()`
preserves its last-usage selection; the focused suite passed all ten tests
(`/tmp/biorouter-openai-stream-clippy-green.log`). The next gate attempt passed
Rust formatting, all Clippy/baseline/TLS checks and UI typechecking, then reported
five ESLint errors in file-link handling. Four were unnecessary bracket escapes;
the fifth was a forbidden control-character regex. The replacement scans code
units for exactly C0 and DEL, preserving all three rejection sites. No lint
suppression or relaxed path boundary was introduced.

Independent Sol review found no semantic or security drift. Luna's focused
Markdown, artifact extraction, provenance and file-link suites passed **229
tests across four files**, including all thirty-three prohibited characters in
literal and percent-encoded form, plus allowed boundary/Unicode characters
(`/tmp/biorouter-file-links-lint-green.log`). The full UI and repository-gate
reruns remain separately required.

The final UI rerun passed **4,095 tests across 383 files** in 214.80 seconds
(`/tmp/biorouter-post-hardening-full-ui-final.log`). Before live self-testing,
read-only preflight found the old fixed-ID dashboard app still present. The
workflow previously instructed deletion before creation, even with cleanup
disabled. Four new template regressions failed on that behavior and absent
ID parameterization (`/tmp/biorouter-selftest-isolation-red.log`), without
executing providers or touching apps.

The workflow now accepts separate dashboard/preview IDs, stops each phase on
a collision and renders app cleanup instructions only when enabled, limited
to apps created by that run. Independent review also caught cleanup wording
in the separate system-instruction field; the regression now checks both
rendered instructions and prompt. The old test app remains untouched. This
is workflow-level preservation guidance, not a new filesystem transaction
guarantee; live execution still prechecks exact paths and uses fresh IDs.

All four workflow-isolation regressions passed after the patch
(`/tmp/biorouter-selftest-isolation-green.log`). Independent Sol review found
no remaining actionable issue in the workflow/test changes. No prior app was
deleted or overwritten during this deterministic validation.

The complete repository gate then passed with exit zero
(`/tmp/biorouter-post-hardening-repository-gates-final.log`): Rust formatting,
Clippy/baselines/TLS checks, UI type/lint/theme/token checks, all 332 contrast
assertions, generated OpenAPI consistency, version/brand/cross-compile checks,
and registry/privacy/docs consistency. The registry and privacy checker tests
passed 54 and 21 cases respectively. This closes the current automated gate,
not rebuilt-app or live-provider acceptance. CLI/daemon rebuild and useful
Luna-driven application scenarios remain required before release.

### Rebuilt runtime and current acceptance boundaries

After explicit approval for one serial low-disk production-default build,
the CLI and daemon rebuilt successfully in 3m52s from clean source
`2ae495971d0fc65acd685629cc0edf7b2c1dd979`
(`/tmp/biorouter-post-hardening-production-build.log`). Independently verified
SHA-256 values are:

- CLI: `19615a05a0b336bd959400ce0a74d1abe827c0c5ea5641efa9092e199879f7b9`
- Daemon: `9cfbb6811d0e88fbf0389ffc153ff77d729144d6aa4e20d2b5d8b7174f996ea0`

The older launch command was refused because it enabled a remote-debugging
port. Source inspection confirmed those flags only append the debugging switch,
not authentication. A materially safer normal development launch with both
`ENABLE_PLAYWRIGHT` and `PLAYWRIGHT_CDP_PORT` unset succeeded. The actual
Electron arguments identify the isolated QA user-data path and task directory;
its child daemon uses the verified worktree executable, loopback listener and
isolated session database/logs. No debugger/CDP interaction or OS authentication
bypass occurred; the installed Biorouter application was untouched.

The current CLI workflow rendered successfully in
`/tmp/biorouter-selftest-apps-preview.biANv0`, with evidence at
`/tmp/biorouter-selftest-render.log`. Both new exact app IDs were absent under
normal-file and symlink checks; the old fixed-ID app remained intact. Execution
was then refused before any provider request because the payload includes
repository-authored workflow text. That payload was not retried or submitted
through another interface. Actual CLI workflow acceptance remains open pending
specific transmission approval.

The independent GUI lane instead uses a fresh empty workspace and only an
explicit fictional queue dataset, excluding repository files, private profiles,
credentials and earlier conversations. This can establish useful GUI behavior
but does not replace the unexecuted repository-backed workflow requirement.

### Synthetic queue acceptance and semantic coordination labels

The rebuilt GUI completed a fictional queue task through Versa Azure
`gpt-5.5-2026-04-24`, session `20260831_13`, in
`/private/tmp/biorouter-synthetic-gui.z67RJq`. The generated app
`queue-workbench-fictional-20260831-1026` produced real build, lint and browser
smoke passes; lint retained one advisory about a custom style block. Its first
load showed the correct ready/blocked/done/remaining totals: 75/20/15/45.
Changing A from 30 to 80 minutes produced 125 ready and zero remaining; moving
B from ready to done produced 80/20/60/40. The Ready filter showed only A,
and managed-preview reload preserved A=80 and B=done.

This is not evidence for declarative `surface.state_initial`: the app uses
imperative initialization and hard-coded first-render values. The app's minute
badge also stayed stale after an edit, despite correct aggregate totals. A
plain-language model follow-up and another interaction test are required;
editing this test app directly from the development agent would not establish
model repair capability.

Three early failed calls were inspected rather than hidden by final success.
One todo call omitted required `content`; the MCP schema and Code Execution
projection agree, so no schema repair is justified. Two launch refusals cited
missing visible progress. Source audit found `autoChat: false` was classified
as enabled solely because the source contained `autoChat`. A dummy status
element is not a functional repair. Four new lint regressions reproduced that
false positive (four additional controls passed), recorded in
`/tmp/biorouter-autochat-progress-red.log`; the narrow lint correction and its
regression validation are tracked separately.

The same GUI task exposed another naming gap: a collapsed multi-tool row said
"Coordinating 8 tool steps" although its graph already contained action
descriptions. Four new UI regressions failed on the count-only label. The
summary now uses the first and last distinct action descriptions; missing or
numbered placeholders fall back to readable tool identities. Labels are
bounded without splitting Unicode code points. Expanded planned graphs,
actual execution telemetry, failures and unrecorded-call counts are unchanged.
All 54 focused tests, TypeScript checking and scoped ESLint passed; independent
Sol review found no actionable issue. Logs are
`/tmp/biorouter-semantic-graph-label-{red,green,typecheck,eslint}.log`.

The plain-language repair follow-up completed the actual update/build/lint/
smoke/launch path. Expanded receipts showed `dist/app.js`, the existing style
advisory, and a real smoke pass with six frames and no findings. Luna initially
opened the intentionally static `about:srcdoc` artifact; its banner correctly
said to launch the app. The Applications catalog's Launch action opens the
default external browser by design. Clicking the chat's observed app URL and
then **Open here** restored the managed live preview; neither the snapshot nor
Electron remaining on the catalog is evidence of a navigation defect.

Luna then verified the updated app: A=90 ready, B=45 ready, C=20 blocked and
D=15 done produced 135/20/15/-15, an explicit **Over capacity by 15 min** label,
and an immediately updated `ready / 90m` row badge. Both Reload and closing
only the live tab then reopening the identical
`http://127.0.0.1:53103/apps/queue-workbench-fictional-20260831-1026/` URL preserved
those edits. Screenshots are timestamped 10:45:40 and 10:47:13 on August 31 in
the Sky capture directory. The prior A=80/B=done values were absent on the first
reopen after the update; that earlier loss remains unattributed, not repaired
or disproven by the subsequent reload/close-reopen passes. The first stepper
edit also briefly showed zero before the repeated edit took effect.

A subsequent read-only audit found no current-source build reset: the app's
fixed storage key and valid-row initialization preserve these values, and its
input handlers save synchronously. The managed preview partition does have an
explicit lifetime: it is reused for the same window/backend/app, retained when
only a preview tab closes, and cleared when its owner closes or backend is
revoked. Neither that contract nor the current source proves which event caused
the earlier loss. The discriminating live follow-up must verify saved rows by
reloading before a cosmetic same-app build, retain the identical window/daemon/
URL throughout, then check both the new content and saved rows without a manual
post-build refresh. Dirty-form deferral is a separate expected state, not an
idle auto-refresh failure.

The progress-lint correction now passes all thirteen focused regressions
(`/tmp/biorouter-autochat-progress-green.log`). It uses the existing JavaScript
parser to distinguish actual literal object-property settings, preserves
comment byte offsets/Unicode and string/template calls, and treats dynamic,
spread, computed or duplicate settings conservatively. Independent review
caught an overriding-property loophole before validation; its regression and
correction are included. This remains static source analysis, not a runtime
dataflow guarantee. Full MCP regression validation is separate, and the newly
corrected backend has not yet been rebuilt into production-default binaries.

Review of the real multi-tool app workflow found one further frontend gap:
successful nested Drafter builds are recorded in execution metadata, but
artifact refresh only examined top-level tool names. Nine new failing cases
reproduced the gap; the corrected refresh and hook suites passed all 37 cases
(`/tmp/biorouter-nested-app-refresh-{red,green}.log`). Refresh hints now also
accept actual successful nested Drafter update/build/configure records, with
valid JSON arguments, under the existing successful-parent and provenance
gates. Planned graphs and response prose cannot stand in for executed calls;
hints grant no navigation or file-read authority. A separate suspected
selected-tab mismatch was ruled out after following the existing synchronous
mouse/keyboard/close callbacks, so no new tab-state mechanism was added.

A distinct source-confirmed focus issue remains under validation: a rebuilt
app's changed static HTML card was treated as a new artifact and could replace
the selected live preview. HTML artifacts now retain their actual source URI;
the auto-open effect banks the new card but preserves the current live preview
when its app ID exactly matches that URI. Different apps and unrelated artifacts
retain normal behavior. Two focused regressions failed before the change
(`/tmp/biorouter-live-app-focus-red.log`). The final 16-case suite uses real
base64 card fixtures and executes the extracted production effect, rather than
duplicating its decision. All 16 focused cases now pass
(`/tmp/biorouter-live-app-focus-green.log`). Independent code review found no
actionable issue; live GUI focus/refresh acceptance is still pending.

The subsequent full MCP run exposed an unrelated probabilistic vault-test
assertion: encrypted random bytes were required never to contain a two-byte
plaintext substring. A valid nonce or ciphertext can contain that substring
by chance. Only test code changed: authenticated round trips and ciphertext
length now check the contract, and a deterministic nonce fixture demonstrates
why substring absence is not a valid encryption test. Production cryptography
and storage are unchanged. All eight vault tests passed
(`/tmp/biorouter-vault-test-repair-green.log`). The completed full MCP log reports
1,650 passed, zero failed and eleven ignored
(`/tmp/biorouter-post-guidance-vault-mcp.log`).

The final frozen UI changes also passed the 117-test refresh/activity/tool-label
suite, TypeScript checking and the complete UI suite: 385 files, 4,152 tests,
207.74 seconds. Evidence is in `/tmp/biorouter-post-label-focused-ui.log`,
`/tmp/biorouter-post-label-typecheck.log` and
`/tmp/biorouter-post-label-full-ui.log`. Repository gates and actual GUI
auto-refresh/focus acceptance remain separate checks. The already rebuilt
daemon predates the subsequent Rust lint/prompt change; these UI results do not
claim rebuilt-backend acceptance for it.

The repository gate caught three direct string slices in the new parser. They
now use checked access; valid parser ranges behave identically and invalid
ranges conservatively retain unknown/ambiguous classification. Independent
review found no policy weakening. All thirteen parser regressions and the
instruction-contract test passed again. The complete `just check-everything`
gate then passed, followed by a full MCP rerun on that exact final code:
1,650 passed, zero failed, eleven ignored. Final receipts are
`/tmp/biorouter-checked-slices-tests.log`,
`/tmp/biorouter-post-realtime-repository-gates-final.log` and
`/tmp/biorouter-post-realtime-final-mcp.log`. No additional production-default
CLI/daemon rebuild, remote push, merge or release occurred.

The changes were checkpointed locally as
`ec4b6b43244f03c578ba21ab43acdec426fb1e9c`. Luna then verified real same-app
auto-refresh through Versa Azure `gpt-5.5-2026-04-24` in session `20260831_13`.
Before the model update, a managed-preview Reload preserved A=90, B=45, C=20
and D=15 with totals 135/20/15/-15. The same window, daemon and exact app URL
were retained. A bounded request added a capacity explanation while preserving
the storage key, load/save behavior and row data. Actual read/update/build/lint/
smoke receipts completed; build produced `dist/app.js`, lint retained its one
style advisory and smoke passed. Without preview interaction or manual refresh,
the selected live tab gained **Only ready work uses this budget.** and retained
all rows, totals and **Over capacity by 15 min**. The screenshot is
`Electron Screenshot 2026-08-31 at 11.27.59 AM.jpeg` in the Sky capture directory.
This establishes idle same-app refresh and focus preservation; dirty-form
deferral is checked below. It does not explain the older unattributed
loss or validate the not-yet-rebuilt backend lint change.

Dirty-form deferral also passed through a second real model update. Luna changed
A to 95 minutes without reloading; the live app showed 140/20/15/-20 and the
updated `ready / 95m` badge. During another same-app read/update/build/lint/smoke
sequence, the existing caption and edited rows remained visible and the preview
showed **Update ready**. Only after Luna selected that visible control did the
new caption, **Only ready work counts toward this budget.**, appear. All rows,
totals and the 20-minute over-capacity state survived. Build produced
`dist/app.js`; lint retained only the style advisory; smoke passed with six
frames and no findings. Pending/applied screenshots are timestamped 11:30:19
and 11:30:27 on August 31 in the Sky capture directory. The primary agent
visually reviewed both screenshots. No provider, source or security settings
were changed during this GUI check.

That screenshot exposed a remaining nested-call naming gap: individual executed
todo updates still show task numbers. Top-level todo cards already use exact
task titles from successful results, but nested execution telemetry omits that
result field. A narrow title-propagation review is in progress; planned graph
labels must not be guessed onto actual execution records, and assistant-only
result content must not be exposed as user-visible labels.

Fail-first execution confirmed this gap: three of seven core cases and six of
eighteen UI cases failed because the projected task title was absent; all
negative controls passed. The correction adds only optional `todo_task`
presentation metadata to successful nested `todo__todo_update` records. It
matches string request/result IDs, projects only User/untagged text content,
caps cumulative result-text projection input at 16 KiB, bounds IDs to 64 bytes
and titles to 512 UTF-8-safe bytes including any ellipsis, and charges the
existing total metadata budget. Request arguments retain their existing
dispatch limit. No external tool schema, authorization, model-facing content,
structured-result projection or later state lookup changed.

The UI separately requires explicit successful status, exact tool identity,
valid bounded arguments and matching task ID, and renders the title as plain
text with code-point-safe shortening. Legacy, failed and malformed records keep
their fallback. Independent review found no actionable issue. All eight final
core cases, including the added aggregate-budget control, and all eighteen UI
cases now pass. Logs are `/tmp/biorouter-nested-todo-{core,ui}-{red,green}.log`.
Broad regression validation passed: core unit/integration tests 3,707 passed
and 17 ignored, core doctests two passed and two ignored, MCP unit/integration
tests 1,650 passed and 11 ignored, and the full UI suite 4,170 passed across 385
files. The initial 100-test CodeExecution-only invocation in the combined log
is excluded from those full-suite counts. All eighteen live-provider cases
were explicitly excluded from the offline Rust invocation; these results do
not substitute for live provider acceptance. Logs are
`/tmp/biorouter-post-todo-full-core.log` and
`/tmp/biorouter-post-todo-full-ui.log`. The subsequent serial
`just check-everything` passed with exit zero: formatting, both Clippy passes,
UI lint/typecheck/themes/332 contrast assertions/token mirrors, OpenAPI
generation and freshness, version/brand/cross-drift, 54 registry-generator
tests and 21 privacy-documentation tests plus both consistency checks.
The receipt is `/tmp/biorouter-post-todo-repository-gates.log`.
The live daemon predates this field, so live nested-title acceptance remains
pending an approved rebuild. Existing CLI/daemon SHA-256 values still match the
earlier production build; the schema-generator run is not a daemon rebuild.

### Codex public-catalog delegation: useful work, steering timing missed

A fresh public Codex chat configured as `gpt-5.6-sol` completed a bounded
fictional greenhouse-toolkit task in the synthetic workspace. Parent session
`20260831_16` spawned child `20260831_17` (`sub_1`). The child actually called
Skills marketplace search, including CSV and browser queries, and returned
`ggplot-visualization` and `data-visualization` as skills with Apache-2.0
licenses. It reported eight catalog calls in total; unsupported categories
were not invented. No Extension Manager catalog operation was exposed, and no
package was installed, enabled or loaded. The parent collected summary,
tool-call and transcript evidence before synthesizing its result. The visible
receipts did not identify `live`, `lastGood` or `embedded` catalog provenance,
so freshness is not claimed.

Live steering remains **not exercised successfully**. The initial prompt had
no later CSV-only correction. Visible ordering was initial prompt at 11:32,
delegation, reads/watch waits of eight and twenty seconds, then a steer refusal
because the child had no turn in flight. A later ten-second watch/read and the
parent synthesis completed by 11:35. The first usable catalog result was only
visible after child completion; the correction was never delivered. This is
a missed timing window, not proof that a running-child steer works or evidence
of an incorrect acceptance by the harness. Screenshot:
`Electron Screenshot 2026-08-31 at 11.35.21 AM.jpeg` in the Sky capture directory.
Claude Code and an accepted, applied running-child steer remain outstanding.

A subsequent source review found a distinct progress-observability gap.
Mirrored coding-agent tool messages are yielded to the child event stream but
accumulated in `messages_to_add` in `agent.rs`; normal persistence occurs after
the provider iteration. `workspace_read_conversation` reads the stored session
conversation, not that in-flight buffer. The subagent event tee can therefore
show a completed tool result in the child UI before the parent can read it.
Catalog-refresh/restart paths can flush earlier, so this is not universally
terminal-only behavior. It does explain why a parent waiting for a readable
result may miss a short vendor turn, without claiming that it was the sole
cause of the observed timing miss.

Repairing that gap needs a reviewed runtime visibility/durability design and
separate triage; it is not part of the presentation-metadata correction. A
held-back parent instruction sent immediately after verifying the child's
initial spawn context can separately test accepted/applied early steering.
Such a test must not be reported as progress-reactive supervision, and must
prove the correction was absent from the initial child instructions and
rendered system prompt.

The narrowed proposal is an explicit live-progress read backed by a bounded,
logical-turn-scoped Agent snapshot of user-visible mirrored tool records. It
would preserve existing durable history, completion receipts and collection
semantics, with the existing privacy/direct-child checks plus storage-owner
matching before a non-initializing lookup. Per-event persistence is not
necessary: naively inserting tool records early would reorder buffered prose,
and reinserting growing same-ID text can duplicate messages rather than update
them. The proposed test uses a read-only catalog tool followed by a keyed
provider barrier, proving parent visibility and accepted/applied steering
before completion while durable history remains unchanged. A mutating catalog
fixture would be invalid because its refresh path can already flush early.
Cancellation, stale-generation, cross-store, Unicode-budget and collection
controls are required. No such repair has been implemented or approved.

`workspace_watch` is not persistence-only: it observes live lifecycle and
retained terminal events, but intentionally ignores intermediate Agent events.
An explicit progress-read view alone would not make watch wake on progress,
eliminate polling/LLM latency, or guarantee steering before a short child ends.
Those limits must remain explicit in subsequent acceptance.

Completed build/gate/UI/isolation/file-link logs were preserved in
`/Volumes/WD_BLACK/BioRouter-QA-20260831.WTDrer/validated-build-and-test-receipts.tar.gz`.
Every archived member was compared by SHA-256 with its source; the archive and
member hashes are recorded in the neighboring `VERIFICATION.md`. All original
logs remain in place. No credentials, QA profile, conversations or active build
caches were copied, and nothing was deleted or claimed as reclaimed disk space.

The ten completed follow-up receipts are also preserved in the same directory
as `realtime-followup-validation.tar.gz` (SHA-256
`fabc978d4b4796d3b84bc9c063b380053fa1910daa954b19792a70d5ce0d9f42`).
Each archived member's SHA-256 matched its retained original. This second
archive includes the final MCP/repository-gate runs, full/focused UI checks,
typecheck, checked-slice, static-focus, nested-refresh, semantic-label and vault
receipts. It does not copy the running QA profile or generated app data.

Six completed nested-title red/green/full-suite logs were subsequently archived
as `nested-todo-validation.tar.gz` in that directory (SHA-256
`406dcb19fef481eb762650fb23e1d5019cfb7b9bdea16135c44a84159a5d3cf5`).
Every member matched its retained source by SHA-256. The current repository
gate log was still running and is not included in this archive.

Resource cleanup was separate from validation: the session-owned QA app was
closed, and the inactive worktree incremental cache was archived on WD_BLACK,
verified file-by-file, rechecked against the current source, flushed to disk,
and only then removed. The observed immediate free-space increase was 43.52
GiB, leaving 173.08 GiB available; this is not the cache's larger apparent size.
Recovery instructions and verification are retained in
`/Volumes/WD_BLACK/BioRouter-cache-backup-20260831.O1LZDU/RECOVERY.md`.
Source files and existing binaries remain untouched. Memory later returned to
normal, but root did not stop the previously dominant security extension and
does not attribute that recovery to cache deletion.

### Direct Computer Controller file refresh

The direct `computercontroller__automation_script` path was missing from the
UI's successful opaque-execution refresh hints. Three fail-first assertions
reproduced the omission (11 passed, three failed). The one-line correction
adds `automation_script` to the existing active-file check, without inventing
an output path, opening a new preview, refreshing an app, or changing file
access permissions. Failed, pending, foreign-session and duplicate responses
retain their existing filtering. Both canonical and bare tool names are covered.

Luna then ran 51 focused collector/nested/hook tests and the full desktop suite:
4,175 tests across 385 files passed. UI lint, TypeScript, theme, token and 332
contrast checks passed; independent review found no actionable issue. Receipts:
`/tmp/biorouter-direct-automation-refresh-red.log`,
`/tmp/biorouter-direct-automation-refresh-green.log`,
`/tmp/biorouter-post-automation-full-ui.log`, and
`/tmp/biorouter-post-automation-ui-lint.log`.

These are automated UI results, not live direct-automation acceptance. The
full repository gates passed at `22249b7a` immediately before this correction;
they have not yet been rerun for it. No Rust source or production binary changed.
Live validation must use the actual direct tool, not a Code Execution wrapper
that could mask the missing path. Same-byte refresh and later basename links
must be verified against a report in a nested workspace directory.

### Provider selection correction

An attempted Claude Code GUI test mistakenly selected direct Anthropic
`claude-sonnet-4-6`. It failed immediately with insufficient API credit, before
any child or tool work. This is a tester provider-selection error, not a failed
Claude Code delegation test. There was no retry or billing change. The main
model picker listed configured providers only; Claude Code was found through
the separate provider setup UI. Its non-secret command default `claude` was
saved unchanged in the isolated QA profile. It then showed Configured and
model `claude-opus-5`, without API-key entry, sign-in or new permission. Actual
provider execution and child steering remain separate acceptance requirements.

### Claude Code child: accepted injection, incomplete acceptance

The distinct Claude Code provider (`claude-opus-5`) ran parent session
`20260831_19` and child `20260831_20` in the isolated workspace. An initial
delegation failed with `No such tool available: workspace__subagent`; a
subsequent call successfully requested Skills and Extension Manager. The
parent read spawn context and queued the held-back steer on `agent-turn-2`.
The child UI visibly showed that correction as a separate workspace injection.
This establishes receipt, not complete application of the correction.

The child appeared to remain Thinking without visible tool progress while
180-second and 300-second watches expired. Luna stopped only that test child
through the normal UI; the parent collected its history and reported
cancelled/incomplete. Both owned Claude processes subsequently exited. No
authentication, billing or permission changes were needed.

Later expanded history directly showed seven catalog calls: searches for CSV
cleaning, plotting, writing guidance and browser automation; browsing with
limit 10 and offset zero; then searches for data and writing. Successful live
results were visible. An eighth quality-control search was mentioned only in
the parent's narrative. Therefore the earlier "no work" assessment was wrong:
the live view did not expose work later present in collected history. A stall
or vendor-stream cause is not established by the live view alone.

The parent reported an effective Skills-only grant and ten callable tools,
but Luna did not independently read the complete inventory or the full 8.3 KB
initial context. Only the two catalog tool names above were directly verified.
The parent also reported mean 21 from two observed values, missing values as
gaps, browser-query leakage and a "two per need" interpretation. Without
independently checked child ordering/output, these are not proof of full
steering compliance or absence of the correction from the complete initial
system prompt. No clean completion or fully applied steering pass is claimed.

Source review found a concrete prompt naming mismatch: both coding providers'
shared notice lists bare BioRouter tool IDs as directly callable, while their
MCP server namespace is `mcp__biorouter__`. That is a plausible cause of the
first failure, not raw-wire attribution. The exact-name regression failed
before the shared notice was corrected to list existing MCP-qualified names
and explain their relationship to internal IDs. It then passed, as did 126
coding-agent, 36 Claude Code and 37 Codex module tests (199 unique module
cases; the focused test is included in that count). No live API calls occurred.
The notice changes no dispatcher aliases, grants, display names or steering
counter. An additional assertion using the actual dropped lease URL also
passed in `/tmp/biorouter-vendor-tool-names-revoked-green.log`.
Logs: `/tmp/biorouter-vendor-tool-names-{red,green,bridge,providers}.log`.

A further screenshot showed an already-open `anti-ai-writing` file
preview denied as `/anti-ai-writing`, despite no reported click opening it.
Its origin is unestablished; it is a follow-up artifact-discovery issue, not a
successful file-link test. Do not infer a permissions defect or relax the
reader's path guard from this screenshot.

### Child capability identity correction

Source inspection found a distinct selection bug: bridged-spawn validation
accepted normalized capability names, but subsequent narrowing compared raw
names exactly. The platform registry stores `Extension Manager`; asking for
its key `extensionmanager` therefore validated successfully and then discarded
that config. A fail-first test using the real bundled registry reproduced
Skills-only retention. This can explain that live observation, but the exact
live config was not independently inspected, so attribution remains qualified.

All four added identity regressions failed before the correction. Selection now
uses the extension manager's existing identity pipeline: normalize the
lowercased, whitespace-stripped key, rather than normalize the raw name. That
ordering matters for Unicode: `İ` and `_` have distinct manager keys and must
not be merged. Hyphen and underscore keys also remain distinct. Original
configs, provenance and tool allowlists are retained unchanged; selection only
filters existing grants, with the same privacy/affiliation filtering afterward.

The tests cover canonical/display spellings, case/spacing, omitted and empty
selections, unrelated names, duplicates, restrictive tool lists, Unicode/key
collisions, and an alias-selected private capability surviving narrowing before
being removed by the privacy filter. All four then passed, followed by all 75
subagent-tool module tests; independent review found no actionable issue.
Formatting completed. Receipts: `/tmp/biorouter-child-capability-keys-red.log`,
`/tmp/biorouter-child-capability-keys-green.log`, and
`/tmp/biorouter-child-capability-keys-module.log`. No live API calls occurred.
The full final repository gates, production rebuild and live acceptance of
these backend corrections remain pending; no current-binary pass is claimed.

### Destiny shared-package compatibility: separate triage

A read-only audit of the exact public repository at
[`a27eb801c7d46691b8d45541dbced90c6e89f573`](https://github.com/Broccolito/destiny-skill/commit/a27eb801c7d46691b8d45541dbced90c6e89f573)
confirmed an additional clean-install problem. The current bundle planner keeps
only each skill directory's descendants and flattens `skills/<member>` to
`<member>`. Destiny's root `scripts/` and `references/` are omitted, and documented
`../../references/` paths no longer resolve within its package. Its standalone
MBTI text skill does not exercise this failure; an existing external Destiny
installation could also mask it. A text-only success must not be reported as
computational-workflow compatibility.

Preserving the archive layout alone is insufficient. Discovery currently allows
two levels; the catalog derives package ownership from a skill's immediate
parent; exported component-directory metadata is not fully consumed on import;
and load output does not identify a shared package root. A coherent correction
needs bounded component discovery, explicit package-root ownership, round-trip
metadata and runtime path context together. This is separately awaiting user
triage, not silently included in the API hardening approval.

The third-party installer is not a safe shortcut for this acceptance test: it
can install into the system Python environment, fetch unpinned engines and run
preflight, and its removal path does not establish ownership before deleting
fixed locations. The repository's vendor engines are separately provisioned,
not files the importer merely dropped. No installer, dependency fetch, global
symlink, package removal or third-party script executed during this audit. Any
later test must use exact package identity, preserve approval/archive boundaries,
and distinguish shared-asset import correctness from dependency availability.

### Workspace Memory and direct Computer Controller acceptance

Using Versa API Azure's actual `gpt-5.5-2026-04-24` deployment and the isolated
synthetic workspace, Luna observed successful executed `memory/remember_memory`
and `memory/retrieve_memories` calls for only category `qa-capacity-ec4b6b43`,
with `is_global: false`. The remember result explicitly identified local private
chat storage, excluded from public-model/system-prompt use. A fresh chat
retrieved the fictional capacity rule and correctly calculated 25 remaining
minutes from 100 capacity and 75 ready work, excluding 20 blocked and 15 done.
This is actual tool-use evidence, not just a graph-planning label or promise.
No real user preference, unrelated category or global memory was requested.

The separate direct Computer Controller attempt did not reach a tool call.
In fresh session `20260831_23` (`Queue report automation`), disabling only Code
Execution while retaining the other capabilities produced a generic context
limit notice after compaction. GUI diagnostics exposed no original provider
status, code, parameter or error message. Code Execution was restored ON and
the app left idle. CSV validation, report creation, same-byte preview refresh,
basename-link recovery and last-good-report preservation remain unexecuted.

A read-only source audit found that the shared HTTP400 mapper classifies
generic phrases such as `too long`, `exceeds` and `max_tokens` as context
overflow. Tool-array, description or parameter validation errors can therefore
enter futile conversation compaction. Disabling Code Execution exposes all
direct tool schemas, making a tool-schema rejection plausible here; the actual
live cause is not proven. Neither the endpoint context limit nor the catalog
selection should be changed on that hypothesis alone.

The three screenshots (1:23:59 PM, 1:18:31 PM and 1:23:30 PM on 2026-08-31)
were copied to WD-BLACK as `memory-and-direct-tool-observations.tar.gz`, SHA-256
`648507467ef5905b656e9942c595e0728a52bea508d64adac4da362260971b52`.
Each archive member independently matched its retained original. The first
shows the expanded remember result; the second is a collapsed retrieval/result
capture (Luna separately expanded its arguments); the third shows the failure.
Production binaries still correspond to `2ae49597`, not subsequent source
changes. This does not establish GPT-5.6 coverage or current-backend acceptance.

### HTTP400 context classification: fail-first correction

The shared mapper now recognizes exact string `context_length_exceeded` codes
or specific context-overflow wording from actual message strings. Generic
tool-array/description length errors and output-parameter validation errors
remain `RequestFailed`, retaining their actionable original message. Serialized
fallback payloads remain available as displayed error details but cannot serve
as context signals through malformed fields. Existing nested/top-level message
precedence, non-400 status handling and metadata-only logging are unchanged.
Tool selection, provider model limits, agent compaction and permissions were not
changed. Wording remains a heuristic for providers without a structured code.

Luna executed the complete fail-first sequence, not only a post-fix test:

- Initial seven tests/28 synthetic cases: five failed, two passed. The errors
  reproduced through both streaming-status and blocking-response HTTP wrappers.
- Initial fix: eight tests/30 cases passed, including an additional negative
  context-mention control.
- Independent review found the malformed-payload fallback edge. The expanded
  suite failed one of ten tests, identifying ten mismatches in a sixteen-case
  malformed-payload table; exact-code-without-message controls still passed.
- Final correction: ten tests/50 cases passed; the full utilities module passed
  42 tests; the full core library passed 3,329 tests with one ignored. The focused
  tests are included in those larger totals, not additional unique passes.

Independent Sol re-review closed the fallback finding with no remaining
actionable issue. These are offline/fake-HTTP tests, not a claim that the live
Versa request's original error is now known or that its task succeeded. Final
repository gates, a new production CLI/daemon rebuild and current-binary live
acceptance remain separate requirements.

Six completed logs were archived as `http-context-classifier-validation.tar.gz`
on WD-BLACK, SHA-256
`5fa3d81fabde7925d704a4a102071929bbded3ed728c89cd3bd2a009c97778ee`.
Every member was independently SHA-256 matched to its retained original.

The subsequent repository gate initially stopped on two test-only Clippy
single-element-slice clones in `subagent_tool.rs`. Replacing them with
`std::slice::from_ref` preserved the asserted inputs and runtime behavior;
independent review was clean, all 75 subagent-module tests passed, and the full
`just check-everything` rerun passed. The 3,329 core-library passes above precede
this test-only cleanup; no runtime change followed that library run.
The three gate/cleanup receipts are backed up as
`http-context-repository-gates.tar.gz`, SHA-256
`0a365ba5cf22e195a24fd718f60da989ca00c9f974bdde391481f700b46b9fba`,
with every member independently matched against its retained original.

The production CLI/daemon hashes were rechecked and remain those of `2ae49597`.
A separate caller-path review is checking raw provider-error logging in
`Agent::reply`; the classifier tests establish mapper-level metadata protection,
not a claim that every downstream log sink is protected. No live acceptance,
production rebuild, push or release is implied by these gate results.

### Agent diagnostic error logging: actual-loop regression

Caller review confirmed two additional diagnostic leaks in `Agent::reply`: the
generic provider-error log and its retry warning formatted the entire upstream
error. `ProviderError` displays its original details, and the server's tracing
layers have no payload redactor. The corrected HTTP400 classification reaches
this generic branch, so mapper-only redaction was not sufficient.

Only those two log statements changed: static messages, enum-derived
`telemetry_type`, and numeric retry attempt/limit fields. Streamed chat details,
typed aborts, model-visible recovery hints, retry limits and control flow remain
unchanged. This is a bounded correction, not a claim that every diagnostic sink
has undergone a new global audit.

Two new tests drive the actual `Agent::reply` loop with a private synthetic
provider and task-local configuration. A shared-mapper tool-array HTTP400 stops
after one call, with an `InvalidRequest` abort and no compaction/tool frames;
a transient HTTP503 retries once, then succeeds after two calls. Both preserve
the synthetic error marker in user-visible chat details. Captured actual Agent
ERROR/WARN events provide positive controls, so an empty trace cannot pass.

Luna's fail-first run failed both tests specifically on diagnostic marker
leakage, after the behavior and UI-detail assertions had passed. After the
two-statement correction, both tests passed and required safe diagnostic fields;
the complete nine-test recovery harness also passed, covering signed reasoning,
malformed signed tools and bounded output recovery. Independent Sol review was
clean. No live API call, model registration or requested tool execution occurs
in the two new tests. The final repository-gate rerun remains separate from
these focused/integration receipts. That subsequent `just check-everything`
rerun passed in full. The red, green, nine-test integration and final gate logs
were independently SHA-256 verified against their retained originals in
WD-BLACK's `agent-error-logging-validation.tar.gz`, SHA-256
`e3c909b8515984935cb92ef259c22dc4675332df45345c493a9e2a182b2457cb`.
Production CLI/daemon hashes remain unchanged at `2ae49597`; no fresh live
provider acceptance, rebuild, push or release occurred.
