# Decision models in the desktop app

> **What this is.** How the desktop app shows a chat's decision-model activity: the Decisions
> section of the Chat summary, the decision history view (a tab of the artifact side panel), the
> Monitors dialog and its install panel, the gate chip on a phase row and the phase gate dialog it
> opens, the Supervision panel of a workflow, the `decide` card and the per-turn marker in the
> transcript, a decision chat's composer and card, the Decision models line of the Usage panel, and
> the per-chat count in History, and how each surface says when a decision model makes a chat
> public (DR-33).
> Covers what each one reads, the owner rule that keeps thousands of calls out of the chat, and the
> tests that hold each part.
> **Status:** Current. Built on the decision-model branch (`claude/decision-models`, November 2026),
> and simplified after the live QA of 2026-11-10 (simplicity first: the essentials up front, the
> rest one click away, see [design.md](../../design.md) P9).
> **Audience:** developers working on the Chat summary, the artifact side panel, History, the
> composer, the Usage panel or the decision routes.

A decision model answers a typed question (yes or no, one of several options, a score level)
with a probability for every answer, in tens of milliseconds. A chat uses one in four ways: a
monitor that watches the chat and acts when a criterion is met, the agent's `decide` tool over
many chunks of input, a gate on an orchestration phase, or a decision model bound as the chat's
own model. The agent loop, the routes and the decision chat's card are described in
[the agent-loop guide](../agent-loop/decision-models.md). This page covers the side panel, the
`decide` card, a decision chat's composer and the Usage panel.
The design builds on Albert Lee's prototype in PR #469, which first wired Baseten's decision
models into the loop as exit criteria.

## The owner rule: aggregate, never flood

Decision calls can number in the thousands in one chat (a monitor every iteration, a `decide`
batch over 600 notes). The rule that follows, binding on every surface:

| Surface | What it draws | Never |
|---|---|---|
| Chat summary | One row per source: each monitor that exists now, every `decide` call together, the decision chat | A row per call, a merged "A + B" row, a row for a removed monitor, a row for a gate |
| Phase row | One gate chip: Gate, Passed, Not passed, Needs you, Blocked or Skipped | Rows for the gate's evaluations |
| History (the chat list) | One count on the chat's own row | Child rows, or a count on a subagent run |
| Decision history view | Every call, one row each, windowed and paged | |

The decision history view is the only place individual calls are listed.

## What the panel reads

Everything comes from the daemon's decision routes, through `utils/sessionDecisions.ts`, except the phase gate dialog's writes, which go through `utils/phaseGates.ts`:

| Route | Read by | Shape (`crates/biorouter/src/decision/`) |
|---|---|---|
| `GET /sessions/{id}/decisions/summary` | the summary section, the history view's filters | `SessionDecisionSummary` (`log.rs`) |
| `GET /sessions/{id}/decisions` | the history view's list, a page at a time | `DecisionPage` of `DecisionRecord` |
| `GET /sessions/{id}/decisions/{decision_id}` | the history view's detail pane | `DecisionDetail`, with the context |
| `GET` and `PUT /sessions/{id}/decision-settings` | the summary (monitors), the Monitors dialog | `DecisionSettings` (`settings.rs`) |
| `GET /decisions/models` | the Monitors dialog's model list | `DecisionModelsResponse` of `DecisionModelInfo` (`crates/biorouter-server/src/routes/decisions.rs`, not this folder; it carries `unavailableReason`, the daemon's reason this chat may not use a model, and `sessionUnavailableReason`, its reason this chat may use none) |
| `GET` and `POST /decisions/models/install` | the install panel | `DecisionInstallStatus` (`crates/biorouter-server/src/routes/decisions.rs`): the offered models, whether each is installed, where Ollama is, and the current or last install |
| `PUT`, `DELETE /sessions/{id}/phase-gates/{phase_id}` and `POST …/decision` | the phase gate dialog, through `utils/phaseGates.ts` | `PhaseGateResponse`: the daemon's sentence, whether it applied at once, and the plan |

Every request carries `userActionHeaders()`. The routes ask `session_reach`, so without the
person's proof a private chat's decisions would be refused. The module calls each route through
its generated function (`listDecisionModels`, `getSessionDecisionSummary` and the others in
`api/sdk.gen.ts`).

The parsers mirror the Rust shapes one to one and are defensive in one direction only, as the
subagent roster's are: a value this build does not know degrades to a neutral one (an unknown
outcome reads as an error, never a pass; an unknown probability source reads as uncalibrated),
and a row with no id is dropped. `utils/sessionDecisions.test.ts` reads `DecisionOrigin`,
`DecisionOutcome`, `MonitorTiming` and `MonitorAction` from the Rust files, so a variant added
on one side fails there. Rust's externally tagged enums (`{"binary":{}}`, `{"yes":0.8}`) become
tagged unions in the renderer, and `questionToWire`, `thresholdToWire` and
`decisionSettingsToWire` turn them back.

**How a model is named.** The records keep Baseten's slug
(`decision-models/baseten-joey-2b-latest`), and the panel names a model with one helper,
`decisionModelLabel` in `utils/sessionDecisions.ts`: a Baseten decision model by its alias
(`joey-1`, `elephant-1`, `mammoth-1`, through `decisionModelName` in `utils/usageStats.ts`, the
rule the model picker, the Usage panel, Home and the decision chat's reply follow), an unknown
Baseten slug without its `decision-models/` prefix, and an Ollama tag as it is (`tev1:0.8b`). The
history rows and detail pane, the decision card, the Monitors list and the `decide` card's head
all use it, so one batch on `joey-1` reads `joey-1` on every surface. A decision card names a
Baseten model by its alias even when Baseten reported the id that served it; the detail pane
keeps that id under "Served by".

Where a person reads which model ran, the alias carries where it runs, through
`decisionModelDisplay` in `components/decisions/decisionWords.ts`: `joey-1 · Baseten`,
`tev1:0.8b · this computer`, and `Ollama` for an Ollama decision model on another computer. In a
`biorouter serve` browser "this computer" reads "the computer Biorouter runs on". No surface shows
a provider key (`ollama_decision`, `baseten_decision`) or a `decision-models/...` slug.

**One set of words.** `decisionWords.ts` holds the words every decision surface shares, so the
history, the cards, the marker, the summary and the dialogs cannot drift apart:

- **A monitor check fired or stayed quiet.** It is never "passed" or "failed" on a surface a
  person reads (three testers read "17 failed" as 17 errors), and a quiet check is never drawn
  with a cross. The records, the wire and `--json` keep `passed` and `failed`.
- **A gate passed or did not pass** ("Passed", "Not passed").
- **A `decide` call, a decision chat or an ad hoc call has no verdict.** Its answer is what it
  shows.
- **A probability names its outcome.** Beside an answer it is that answer's own probability
  (`No · P(no) 0.88`, never P(yes) beside "no"); beside a threshold it says which outcome the
  threshold watches (`P(yes) 0.29 · threshold 0.50`).
- **A rule a person sets is said in words** (`ruleInWords`): "when P(yes) is at or above 0.85",
  "when P(yes) is below 0.30". The compact form (`P(yes) ≥ 0.85`) is kept for records: the history,
  a card's Details and a tooltip.
- **No internal ids up front.** A question's name (`mentions_fever`), a monitor id, a turn or
  batch id never reaches the front of a surface, and nothing in this chrome is set in mono. The
  raw JSON in the history's Details keeps every id.

## The Decisions section of the Chat summary

`components/summary/SummaryDecisions.tsx`, drawn by `ChatSummary` between the subagents and the
statistics, in the docked rail and in the header popover alike.

- **One line by default.** The heading is a toggle (`summary/SummarySectionHeading.tsx`, drawn
  like the roster's group toggle) with a short count: "1 fired", "1 fired · 1 error", "1 error",
  "None fired" when the chat has monitors and none fired, or "10 checks" when it has none. The
  section opens on its own only when a monitor fired or a check errored; a blocked gate does not
  open it, because its chip already says "Needs you" on the phase row. Once the person opens or
  closes it, that choice holds for as long as the summary stays mounted for the chat.
- **Which rows exist** (`summary/summaryDecisionRows.ts`, which then reads the aggregates through
  the unchanged `decisionSourceRows`):
  - one row per monitor that exists now, in the order the settings list them, so a monitor
    nothing has checked yet still has its row ("Not checked yet");
  - a call that asked several monitors at once counts on each monitor's row. The daemon joins
    such a call's monitor ids with `+` (`PlannedRequest::origin_id` in `decision/monitor.rs`); the
    copy on each row carries the call's count and errors, never its answer or probability, which
    one combined record cannot assign. No merged "A + B" row is ever drawn;
  - a removed monitor leaves no row, and its checks stay in the decision history;
  - no gate rows: a gate's one place in the summary is its chip on its phase row;
  - then one row for every `decide` call together ("Agent checks") and one for the decision
    chat's answers ("This chat's answers").

  A chat with 5,000 calls draws as many rows as it has sources.
- **A row** is one 28px line: a glyph, the name, and one trailing word. No probability, no bar
  and no call count (the count is in the row's accessible name and in the history).

  | Source | Glyph | Trailing |
  |---|---|---|
  | Monitor whose last check fired | flag, accent ink | Fired |
  | Monitor that was checked and did not fire | eye, muted (never a cross) | Quiet |
  | Monitor never checked | open circle | Not checked yet |
  | Monitor switched off | struck eye | Off |
  | Monitor whose last check errored | warning | Error |
  | Agent checks, This chat's answers | the decision mark | the count, or Error after a failed call |

  A monitor row's accessible name is its word, its name and "checked N times", plus the errors
  when there are any.
- **Pressing a row** opens the decision history filtered to that source.
- **Monitors** is a 24px icon button (`SlidersHorizontal`) on the heading line, named and
  tooltipped "Monitors", shown whether the section is open or closed. It opens the Monitors
  dialog. A decision chat draws none.
- **Hidden** while the chat has made no decision calls and has no monitors.

The stylesheet, `summary/summaryDecisions.css`, keeps the summary's rules: two type sizes, rows
of 28px, no hairline. `SummaryDecisions.test.tsx` reads it to hold them, and holds the rows to the
rule above: monitors A, B and C plus an "A+B" aggregate give rows A, B and C, a deleted monitor
gives no row, and the section never says "Passed", "Failed" or "P ".

### The hook

`hooks/useSessionDecisions.ts` reads the summary and the settings on the same gate as the
subagent roster: while the summary is shown or the chat's turn runs. It re-reads when a
`decide` or `workspace_orchestrate` tool call answers, or a decision notice or a decision chat's
reply lands (`decisionRevision`). While a turn runs it polls every 3 seconds. The settings are
read on every poll, in every chat, because other writers change them and leave nothing in this
chat's transcript to re-read on: a parent agent's `workspace_monitor`, a parent's save copying
its monitors into running subagents, a workflow's seed, `biorouter session monitors` and another
window. Without that, a subagent's tab opened with no monitors kept showing none for the rest of
the turn after its parent put one on it. The summary is polled only in a chat with a monitor
switched on or with decision calls on record, so a monitor that arrives mid-turn starts it from
the next poll. A daemon without the routes answers 404, which reads as nothing to draw, not as
an error, and is not polled.

`BaseChat.tsx` mounts the hook below the subagent roster and above the early return, passes the
plan's phase gates and the monitors the newest turn marker says fired, and mounts the Monitors
dialog itself: the popover form of the summary unmounts when it closes, and a dialog inside it
would go with it.

## The decision history view

`components/decisions/DecisionHistory.tsx`, opened as a tab of the artifact side panel with the
kind `decisions` (`artifacts/artifactTypes.ts`). `decisionHistoryArtifact(sessionId, filter)`
builds the source. There is one tab per chat: opening it again with another filter re-filters
that tab, and `openedAt` makes the same filter applied twice still apply. The tab draws the
decision mark, has no "open externally" action, and puts its call count in the preview bar.

The decision mark is `ENTITY_ICONS.decision` (`components/icons/entity-icons.ts`, a gauge), one
glyph for one concept: the summary's Decisions rows, the per-turn marker, the decision-chat
card's model line, the Decision models rows of the model picker, this tab, the count in History
and every `decisions__*` tool row (`utils/toolGlyph.ts`, whose test reads the tool names from
`decisions_extension.rs`) draw it, and nothing else does.

- **One call, selected.** A decision-chat card's "Decision history" link opens the view on that
  one call (`DecisionHistoryFilter.decisionId`, sent as `decisionId`), selected and with its
  detail asked for at once, and with its source and turn as well, which still narrow the list on
  a daemon from before `decisionId`. Setting any filter in the view lets the single call go.

- **Filters.** One line: a **Source** chip, an **Outcome** chip and a **Filters** chip, then,
  only when a filter is set, the removable chips and "Clear filters". Each chip is a `FacetChip`
  (`components/ui/facet-chip.tsx`, the Knowledge graph's facet chip) that opens a menu; engaged,
  it reads what it holds ("Source: Monitors", "Outcome: Fired").
  - **Source** lists "Any source", then only the sources this chat has (Monitors, Decide calls,
    Phase gates, Decision chat, Ad hoc calls), plus the current one if a link chose another.
  - **Outcome** offers only the words the list itself shows: "Fired" and "Quiet" for monitors,
    "Passed" and "Not passed" for phase gates, and "Error or refused" for every source. Changing
    Source resets an outcome the new source has no row for. The old "Errors only" checkbox is
    "Error or refused" now; the wire values stay `passed`, `failed` and `errorsOnly`.
  - **Filters** opens a popover with the rest: which monitor, gate or `decide` call (shown once a
    source with two or more is chosen; a `decide` call is named by its time and size), the turn
    (by its time and call count; a turn id the list does not name reads "An earlier turn", never
    the id), and a probability range, which applies as typed or on Enter. Its chip names what is
    set inside it ("Filters: monitor, turn"). A pick applies at once and leaves the popover open,
    and Escape in a nested menu closes only that menu and gives focus back to its chip; a second
    Escape closes the popover.
  - **Chips no picker names**: a `decide` card opens the view on its own batch ("This decide call
    only"), and a decision chat card's link on one call ("This call only"). Each is removable, and
    choosing another source or monitor lets it go too.

  The criterion and turn lists come from the summary, so they name what the chat actually has.
- **The list** is a `listbox` of 44px rows, newest first, loaded 100 at a time. Only the rows in
  view and eight on either side are in the DOM, placed at their index times 44 inside a box as
  tall as the loaded rows, and the next page is asked for once the last loaded row is within 20
  rows of the view. The arrow keys, Home, End and the page keys move the selection, which is the
  list's active descendant and is kept in the DOM wherever the window is.
  - **A `decide` call's rows stay together.** The daemon stores every row of a batch with the
    time its first row was stored (`insert_decision_into!` in
    `crates/biorouter/src/decision/log.rs`), so each chunk shows that one time, and the page order
    (`PAGE_ORDER` there) puts a batch's rows next to each other in chunk order, the last chunk on
    top as in the rest of the newest-first list. A row that names no chunk (a stopped batch's) sits
    above its chunks, and a vote's models answer the same chunk side by side. A monitor that
    checked while the batch ran lists above the whole batch. Before this, each row took the time
    it was stored, so chunks listed in the order they finished.
  - **It follows a running chat.** While the view is open and the window visible, it reads the
    list's own first call every 4 seconds (`FOLLOW_INTERVAL_MS`, the same filter, one row) and
    compares it with what the list's first page read. A new call reloads the list in place when
    the list is within 24px of the top; otherwise the preview bar offers "Show 3 new", so the rows
    under the reader never move. It compares with the list's own read, not with a separate summary
    read: a call that landed between those two reads used to stay missing from the list until
    Refresh. It counts only calls the filter shows.
- **A row** is the glyph, the title (a monitor by its name; a call that asked several monitors at
  once "2 monitors", with their names on the second line; a gate by its phase; a `decide` chunk by
  the id the agent gave it, else "Chunk 30"), and on the right the verdict ("Fired", "Quiet",
  "1 of 2 fired", "Passed", "Not passed") or, for a `decide`, chat or ad hoc call, the answer.
  The second line holds the time (and a several-monitor call's names) and, for a one-question
  call, a small bar with its named probability. When the rows loaded so far came from more than
  one model (a vote, or monitors on different models), the second line also ends with each row's
  model, with where it runs when that is short: "1:31:02 PM · joey-1 · Baseten",
  "1:31:02 PM · tev1:0.8b · this computer" (the longer place a `biorouter serve` browser shows
  stays under Details). The row's accessible name then ends with the model too (`rowModel` and
  `listsSeveralModels` in `DecisionHistory.tsx`). A chat whose calls all used one model keeps
  the model under Details, and the latency is always there.
- **The detail pane** shows, up front: the title once, one muted line with the kind and the time
  ("Monitor · 1:26:51 PM"), an error or a refusal and the privacy note when they apply, then per
  question the question in plain words (never its name, never in mono), the answer with its own
  probability, and the threshold line. When the probabilities are uncalibrated it says "This
  model's probabilities are its own estimate." with an InfoTip. Everything else is under a closed
  **Details**: the option bars with the threshold marked, the model (`tev1:0.8b · this computer`),
  "Served by" when Baseten served another model than asked, Model tier, Latency, Tokens,
  Certainty (the distribution's concentration, explained as not the probability of the answer),
  Probabilities (the calibration sentence), the chunk, and one switch between **What it saw**
  (the exact context it was sent, from the detail route, cut at 64 KB and said so) and **Raw
  JSON**, which is the only place the turn and batch ids appear. Details stays open, and keeps
  the switch's choice, while the arrows move through the rows. An error row can carry tokens: an
  answer the service could not read, and a late or abandoned answer, were answered and billed, so
  their tokens are shown and counted
  ([Usage and billing](../agent-loop/decision-models.md#usage-and-billing)).
- **Model tier, not Tier.** The detail pane labels the decision model's tier "Model tier"
  (`DECISION_HISTORY_COPY.tier`). It is the model's tier, not the chat's: a public chat that used a
  local decision model reads "Model tier: Private", and its content is still public. The decision
  chat's card and the model pickers say the same thing with a lock.

The list sits over the detail in a narrow panel and beside it from 720px (a container query in
`decisions/decisionsPanel.css`). In the narrow layout the list hugs its rows (the loaded rows plus
one while a page waits, at 44px each, at most 40% of the window), so ten calls leave no empty band
above the detail. The view only reads, so it works the same in a `biorouter serve` browser.

## The Monitors dialog

`components/decisions/MonitorsDialog.tsx` (styled by `monitorsDialog.css` and
`decisionsPanel.css`) holds the chat's default decision model and lists the chat's monitors, and
adds, edits or removes them. Its subtitle says the model first: "Choose the decision model this
chat uses, and add monitors that ask it about the chat as it runs." A monitor has a
name, a question, yes or no or one of several options (with the answer that fires it), a model,
an action (notify, tell the agent, stop the turn), when it checks (after each step, or when the
turn ends), an on switch, and a few settings most monitors leave at their defaults.

### Each change applies when it is made

The dialog is a manager, like the Knowledge bases manager: an editor's Save, a row's switch and
Remove each write at once, and there is no footer and no second Cancel. The dialog's × closes it,
also while a change is saving; the write finishes and the summary still updates. After each write
the foot row says "Saved" with a check (muted, `role="status"`), or "Saving…" while it runs, and a
refusal takes its place in danger ink. Opening an editor clears an earlier "Saved" or "Removed", which described that write and would read as if the new monitor were saved.

**Every write applies one change to what is stored now.** Before each write the dialog reads
`GET /sessions/{id}/decision-settings` again and applies only the person's one change to that
fresh list, by monitor id: add or replace the edited monitor, flip one switch, drop one monitor,
or, for Undo, put the removed monitor back at its old place when no monitor with its id exists by
then. It then writes the whole result with `PUT /sessions/{id}/decision-settings` and shows what
the write returned. So a monitor an agent's `workspace_monitor`, another window or
`biorouter session monitors` added while the dialog was open is kept, and Undo never undoes
someone else's change. A fresh read that fails falls back to the last list the dialog stored.
An editor Save whose monitor was removed elsewhere meanwhile adds it back as the person's.

| Person does | On success | On refusal |
|---|---|---|
| Save in the editor | the editor closes, the row shows, "Saved" | the editor stays open, the daemon's sentence above its buttons |
| A row's switch | "Saved" (an agent's monitor becomes the person's) | the switch returns to its stored state, the sentence under that row |
| `⋯` › Remove | the row goes, "Removed Flag medical advice.", and Undo beside it, which takes focus | the row stays, the sentence under that row |
| Undo | the row is back, "Saved" | the sentence in the status line |

While a write runs, every switch, Add, Undo and the editor's Save are disabled; a row's `⋯`
stays focusable but its items are disabled.

### A row

A row is the monitor's name, at most one owner badge, and its switch:

- **The owner badge.** "Set by the agent" for a monitor the parent agent put on this subagent,
  with an InfoTip ("The agent set this monitor. If you change it, it becomes yours and the agent
  can no longer change it."), and "Inherited" for a copy from the chat that started this
  subagent ("Copied from the chat that started this subagent."). A change you make to an agent's
  monitor (an edit, or its switch) makes it yours.
- **One muted line with three facts**, what it does, its rule in words and when it checks
  (`monitorRowLine`): "Stop the turn · when P(yes) is at or above 0.85 · after each step", or
  "… · when the turn ends". The line is cut with an ellipsis when it is too long, so the peek says
  the timing again. The rule reads the monitor's `option` as
  `MonitorCriterion::pass_threshold` does (`monitorThreshold` in `utils/sessionDecisions.ts`):
  "when P(no) is at or above 0.85" for a yes/no monitor that watches No, "when P(rrms) is at or
  above 0.70" for a choice, and "when P(mild or a higher level) is at or above 0.85" for a score
  monitor, whose level is named by its label whether the record stores the label or its index. A
  choice monitor with no option chosen yet reads "Notify me · choose the option it fires on ·
  after each step".
- **The peek.** That line is an InfoTip trigger, so hover peeks and a click pins
  ([Hover and click](shared-controls.md#hover-and-click)). The peek says, in at most two
  sentences, which model it uses, where it runs and when it checks ("Uses tev1:0.8b on this
  computer, after each step."), then what else is set: "Stops only once", "pauses if the model
  can't answer", "also watches subagents" (`monitorPeek`). The same words describe the switch to a
  screen reader. The peek opens below the row's line, where above it would cover the row's own name. It holds no actions, because an InfoTip holds plain text only.
- **Edit and Remove** are in the row's `⋯` and on right-click. Remove asks nothing, because it
  offers Undo.
- A stored monitor that is on but names a model the chat can no longer use shows the reason in
  danger ink under its row; it does not block writes to the other monitors.

With no monitors the dialog shows an empty state, "No monitors yet", with one "Add a monitor"
button. At 8 monitors Add is disabled and the status slot says "A chat can have at most 8
monitors.". After the last monitor is removed the empty state shows, and the foot row still
carries the status and Undo.

### Editing

The editor opens in place of the row it edits (a new monitor's at the end of the list), one at a
time; while it is open the other rows' controls and Add are disabled. It is the filled editor box,
with its own Cancel and Save, the dialog's only pair. Up front are only **Name**, **Question**
(with an InfoTip saying it is asked of the newest messages and tool results, and should be written
so yes means act) and **Then** (Notify me, Tell the agent, Stop the turn). A saved monitor is on,
and its row's switch is where it is turned off; the editor has no switch of its own.

Everything else is under **More options**, a disclosure whose closed summary states the defaults
it hides ("Yes or no · when P(yes) is at or above 0.85 · Default model"). It starts closed for a
new monitor and for one whose hidden settings all equal a new monitor's, and open for an edit of
a monitor that uses any of them (`usesHiddenSettings`). Inside, top to bottom:

1. **Answer type**: Yes or no, or One of several options. A score monitor, which the dialog
   cannot make but which can be set over HTTP, opens with a third, read-only Score segment, its
   levels listed lowest first and the level it fires on; editing its other fields saves the score
   question and its level unchanged, and only choosing another answer type converts it.
2. **Options**, for a choice: a name and an optional description for each ("What it means
   (optional)"). The description is saved to the option's `description`.
3. **Fires when**, one field: the answer it watches (Yes or No, or the option), At or above or
   Below, and the threshold slider of 21 stops (0.01, then 0.05 to 0.95 by 0.05, then 0.99), so a
   monitor can fire below 0.50. A stored value that is not a stop (0.33, set from the CLI) keeps
   its own stop for the whole edit and saves unchanged. One live sentence under it says the rule:
   "Fires when P(yes) is at or above 0.85." Switching the side clears any re-arm level.
4. **Model**, from the picker below.
5. **Check**: After each step, or When the turn ends.
6. **If the model can't answer**: Notify me (the default) or Pause the turn, with an InfoTip
   ("Pause the turn stops it whenever the decision model cannot answer, so a safety rule never
   goes unchecked.").
7. **Stop only once**, while Then is Stop the turn: checked, it sets the re-arm level at the
   threshold (or keeps the level the monitor already had), so the monitor stops the turn once and
   then stays quiet until P(yes) drops below the threshold again (rises to it, for a Below
   monitor); its InfoTip says which. Unchecked, a stop monitor stops every check while its answer
   holds ([latching](../agent-loop/decision-models.md#monitors-a-decision-model-watching-the-chat)).
8. **Also watch the subagents this chat starts**: copies the monitor onto every subagent this chat
   starts, and onto the ones running when it is saved. It is hidden on a subagent's own chat.

The parser and writer keep `direction`, `rearmAbove`, `setBy`, `setBySession`, `inheritedFrom` and
`applyToSubagents`, written only when they differ from the default, so a write from the dialog
never drops a field set elsewhere.

**Validation** is said by the field once Save is pressed: a missing name or question, too few or
duplicate options, no option chosen to fire on, and a public model on a private chat. When the
field is inside More options, Save opens it and focuses that field; when the model list could not
be read, Save opens it and focuses Model once before it writes. A refusal the daemon sends anyway
(a 400, or the save check's 422, "Nothing was saved: monitor "Done" could not run. …", given when
what the save turns on cannot run for this chat now; see
[What a save checks](../agent-loop/decision-models.md#what-a-save-checks)) is shown in the
daemon's words with backticks removed, and the editor stays open. A monitor that is off is never
held back for its model, so it can always be switched off.

### The model picker

`components/decisions/decisionModelOption.tsx` builds the rows of the Monitors picker and of the
phase gate editor's, so the two read one way. Each row is one line: the model's name, then muted
text saying where it runs. A private model carries a lock with a visually hidden "Private model";
a public one draws none, and the words "Private" and "Public" are not on the rows. The window is
named only when it is under 8,000 tokens (`SMALL_WINDOW_TOKENS`), for example
"this computer · 2K tokens" for `tev1:0.8b`, because a window that small is why a monitor on a
long chat may see only its newest messages. The default row reads "Default model", with the
stored default's alias or "the chat's decision model" beside it.

**Privacy.** The dialog asks for the catalog naming the chat (`GET /decisions/models?sessionId=`),
so each model carries the daemon's own reason the chat may not use it (its tier, an institution
the model does not cover, a Crew grant on the chat, or a local copy Ollama cannot load, whose
reason names the `ollama pull` or `ollama rm` that fixes it). A model with a reason is offered
disabled with the reason as its muted text, backticks removed, because Gate H or the Crew rule
refuses it before anything is sent, or Ollama would refuse to load it. A public model in a private
chat always reads one plain line, "Public model: it can't read a private chat.", whatever the
daemon's sentence was. In a Crew-scoped chat a model hosted elsewhere carries the Crew sentence,
and a model on this computer and the chat's default are offered. Once the chat's grant no longer
stands, every model and the default carry the catalog's `sessionUnavailableReason`, so the dialog
saves no monitor there that every check would refuse. Where the daemon gave no reason, a public
model is still refused for a private chat.

**A public model that makes the chat public is offered** (DR-33). A chat is as private as its least
private model, decision models included, so on a chat bound to a private model (Versa) that has no
message yet, or that the person made public from History, choosing joey-1 on Baseten makes the
chat public. The catalog marks such a model `chatEffect: "makesPublic"`; the row is enabled and its
muted text ends "Makes this chat public". The dialog never works this out itself. On a chat that
has messages and was never made public this way, the daemon's reason is "Public model: it can't
read a private chat. Start a new chat and choose this model before its first message, or make this
chat public from History."; the row keeps its first sentence, and because a disabled row cannot
hold a tip, the picker field carries the rest as its InfoTip (`anyPublicModelRefused`). On a
`biorouter serve` daemon whose operator did not allow it, those rows read "This server doesn't
allow public decision models in a private chat."
([SD-16](../deployment/serve-decisions.md#sd-16-a-keyless-daemon-makes-no-chat-public-by-a-decision-model-unless-its-operator-allows-it)).

**A model list that cannot be read is said under the picker.** A daemon without the route (404, an
older Biorouter service, which answers a bare 404) is told apart from an empty list
(`fetchDecisionCatalog`'s `missing`) and gets a sentence asking for a restart to finish updating.
The route's own 404 carries a sentence ("There is no chat with that id.", for a chat deleted in
another window) and is shown as a failure with that sentence instead. A 401 or 403 says Biorouter
refused to list the models, with the daemon's own sentence when it gave one, and any other failure
says the models could not be loaded, with the error's message or the status. Those two carry a
Retry link that reads the list again and keeps the monitor being edited.

### Getting in, reading first, and the proof

- **Two ways in.** The summary's Decisions section has a Monitors button, but that section is
  drawn only once a chat has a decision call or a monitor, so the composer's `+` menu also carries
  "Decision models and monitors…" (`composer/ComposerPlusMenu.tsx`), which is how the first
  monitor is added and how the chat's default decision model is chosen. The row names both because
  the dialog holds both; the dialog keeps the title "Monitors", the name of the summary's button.
  In a chat that exists both open the one dialog `BaseChat` mounts. Before the chat's first
  message (on Home, or in a new chat) the `+` row opens the dialog in draft mode, described in
  [Before the first message](#before-the-first-message). A decision chat draws no `+`.
- **It never writes what it has not read.** Every open reads the settings and keeps Add and every
  row's controls off until the answer arrives, even when the summary already holds a copy: the
  summary's copy of an idle chat can predate a write made with `biorouter session monitors` or in
  another window. The copy is shown meanwhile. Opened before anything has read the settings (from
  the `+` menu while the summary is folded away), a failed read keeps every control off and says
  "Could not read this chat's monitors. Close and open again to retry.", because an empty list
  written in its place would replace every stored monitor and the chat's default decision model.
  Between writes, while no editor is open, a newer copy from the summary's poll replaces the list.
  The value each write stores is handed back to the summary at once (`applySettings` in
  `hooks/useSessionDecisions.ts`), so it is current even while the summary reads nothing.
- **The proof.** A write needs the person's proof on a daemon that holds a user-action key (the
  desktop app's) and none on a daemon that holds none (`biorouter serve`), so the dialog works the
  same in a browser, steer monitors included. On a daemon with no key the one write refused is to
  a subagent's chat, with the sentence that tab's other controls give there
  (`DECISION_SETTINGS_SUBAGENT_NO_KEY`). A desktop daemon that was meant to receive a key and did
  not refuses the write, and the dialog shows that sentence. A saved change applies at the chat's
  next check, in the running turn too.
- **It moves the chat.** A person's change to a chat's monitors is activity, like a composer
  extension toggle, so the chat rises in the sidebar
  ([what moves a chat](sidebar-chat-list.md#what-moves-a-chat)).

With `readOnly` (for a surface that shows the monitors without letting them change) the rows draw
with their switch disabled and no `⋯`, and there is no Add and no editor.

### When a save makes the chat public

Every write answers with its `tierEffect` and the line to show for it, and the dialog draws that
line with one InfoTip (`decisions/TierEffectLine.tsx`; the words are `TIER_EFFECT_COPY` in
`decisions/decisionWords.ts`, shared by every surface below so no two say it differently):

- **While an edit would make the chat public**, one line sits above Save: "This chat will be
  public: joey-1 runs on Baseten.", with the InfoTip "A chat is as private as its least private
  model, so gpt-5.5 keeps answering but the chat can't use private extensions, knowledge bases or
  chats. Choose a model on this computer to keep it private." Save is the person's answer: it
  sends the write with `acceptPublicDecisionModels: true`. The line is worked out from the rows the
  daemon marked `makesPublic`, on the list as it would stand after the save.
- **When the daemon asks anyway** (409 `needsPublicDecisionConfirmation`, for a write that did not
  say it), the dialog shows the daemon's question with Make it public and Cancel, and Make it
  public sends the same write again with the answer.
- **A save that keeps it public** reads "This chat stays public: joey-1 runs on Baseten."
- **A save that removes the last public model** reads "This chat becomes private at your next
  message.", with the InfoTip "Public models can't be used in it after that unless you make it
  public from History." It needs no confirmation.

The phase gate dialog's editor does the same for a gate's model (or, for a gate with none, the
chat's default): the line above Save gate, and Save gate as the answer.

### The chat's default decision model

The top of the Monitors dialog, in a chat and before the first message alike, is one select,
"Decision model for this chat" (`DefaultModelField` in `MonitorsDialog.tsx`), with the picker's
own rows (`decisions/decisionModelOption.tsx`). Its first row, "Chosen automatically", clears the
default, so a decision that names no model is chosen within the chat's tier (a model on this
computer first). A monitor, a gate or a `decide` call that names no model runs on it. A pick is
applied at once, like a row's switch, and keeps the monitors as stored. A public row that would
make the chat public reads "Makes this chat public"; picking it shows "This chat will be public:
joey-1 runs on Baseten." under the select with Cancel and Make it public, and only the answer is
sent (`acceptPublicDecisionModels`), in draft mode to the draft. The daemon's line after a save
names a Baseten model by its alias even when the chat stored its slug ("joey-1 runs on Baseten",
never `decision-models/baseten-joey-2b-latest`; `LoweredBy::model_name`), as do `privacyBasis.model`
and every refusal. Live QA M1-1 (2026-11-11) found
the dialog offered monitors only, so a person could not set the default the CLI sets with
`biorouter session monitors <id> default`. Tests: `decisions/MonitorsDialog.tierEffect.test.tsx`.

### Before the first message

`decisions/DraftMonitorsDialog.tsx` is the Monitors dialog for a chat that has no session row yet,
shared by both composers (Home's `Hub` and a blank chat's `BaseChat`), which would otherwise
diverge. It edits the default decision model (the select above) and the monitors only, and writes
nothing to the daemon. So where a chat's dialog says "Saved" after a change, the draft dialog says
"Ready for the first message", with the same check, under the note "These monitors start with the
chat when you send its first message." The tip beside "This chat will be public" names the chat model the way the
model chip does (`gpt-5.5`, not `gpt-5.5-2026-04-24`), here and in a chat's own Monitors dialog. The draft lives in renderer memory beside the held chat model
(`decisions/draftDecisionSettings.ts`), keyed by the composer (a chat tab, or Home's draft key), so
it outlives a tab switch and a trip to Settings and is never written to disk. The list is read
with `GET /decisions/models?boundProvider=<p>&boundModel=<m>`, so each row carries what it would
do to a chat on the model the chat will start on.

Send carries the draft as `decisionSettings` on `POST /agent/start`, with the held chat model.
A draft the person saved under "This chat will be public" records the model it was accepted under
(`acceptedFor`), and Send passes `acceptPublicDecisionModels` only while the chat still starts on
that model. When the model changed after that, or the daemon asks for any other reason, Send
shows "Make this chat public?" with the daemon's line and Send and Cancel
(`decisions/usePublicDecisionConfirm.tsx`). Cancel is not a failure: nothing is reported and the
message goes back to the composer. The draft is dropped once the chat it started exists. Tests:
`Hub.draftMonitors.test.tsx`, `decisions/MonitorsDialog.tierEffect.test.tsx`,
`decisions/usePublicDecisionConfirm.test.tsx`.

### Where else it shows

The daemon computes why a chat is public (`privacyBasis` on `GET /sessions/{id}`, read by
`privacy/usePrivacyBasis.ts` while the chat is public and again whenever its decision settings
move); no surface folds it. A listing does not carry it, so the sidebar, History and the tab strip
show only the lock, as for any chat.

| Surface | What it shows |
|---|---|
| The chat's privacy pill (`SessionNamePill`, `PrivacyBadge`) | "Public", with the InfoTip "Public: monitor "Done" uses joey-1 · Baseten. A chat is as private as its least private model, so it can't use private extensions, knowledge bases or chats." For a subagent of a public chat: "Public: it is part of a public chat's work." The Private tip reads "Private. Biorouter only lets private models, decision models included, read this chat." |
| The model chip and its menu (`ModelEffortChip`, `ModelsBottomBar`) | Versa keeps its lock (the chip describes the model). The chat's line in its tooltip and menu header names the reason, as the pill's tip does: "Public chat: monitor "Done" uses joey-1 · Baseten." (`privacyBasisChipLine`). A chat in the tab strip draws no pill, so this is where the reason shows (live QA M1-2, M2-3). |
| The model switch (`SwitchModelModal`) | On a chat its decision models keep public, "This chat stays public: its decision models include joey-1 · Baseten." in place of "After your next message this chat becomes private" |
| A workflow's start | "Start this workflow?", "This workflow uses joey-1 · Baseten, so this chat will be public.", with Start it and Cancel |
| The declassify dialog (History) | The public monitors and default the declassification turns off ("Monitor "Done" uses joey-1 · Baseten and will be turned off."), with the InfoTip "After this, a public decision model you choose can read the whole chat." The lines come from the chat's settings and the catalog's tiers; the daemon turns them off whatever the dialog read |
| A refusal card | The daemon's sentence, which on a chat made public by a decision model names it: "This chat is public because monitor "Done" uses joey-1, a Baseten model, so it can't use the private extension OMOP. …" |

A gate's line in the plan text (the tool result card the person reads, `gate_words` in
`agents/orchestration_plan.rs`) names the gate's decision model when it names one: "1 question at
P(yes) >= 0.50 with joey-1 · Baseten, 0 of 3 attempts used, …" (live QA M2-4). The plan card does
not yet say which phase's gate is the last public one.

### The install panel

`components/decisions/DecisionModelInstall.tsx` offers to install a decision model on this
computer ([Installing a local decision model](../agent-loop/decision-models.md#installing-a-local-decision-model)).
It is mounted at the top of the Monitors dialog when the model list was read and no listed model
may read the chat, unless the chat's Crew grant no longer stands (installing cannot help there).
In the model picker it is mounted in a quiet form when "Ollama decision models" is chosen, and
shows something only when `tev1:0.8b` is not installed or Ollama is not running.

- **Its first sentence** says why: "This chat is private, so only a decision model on this
  computer may read it, and none is installed.", the same for a Crew chat ("This chat works under
  a Crew grant, so …"), or "No decision model is installed and no Baseten key is set." It stays
  visible, because it is a privacy disclosure. What the model is and its size sit in an InfoTip
  after the "Install tev1:0.8b" button, named "About tev1:0.8b": "About 1.5 GB, runs in Ollama on
  this computer." In a `biorouter serve` browser "this computer" reads "the computer Biorouter
  runs on".
- **Its states** follow `GET /decisions/models/install`: an Install button; "Downloading
  tev1:0.8b… 412 MB of 1.5 GB"; "Checking that Ollama answers decisions with it…"; "tev1:0.8b is
  installed."; a failure in the daemon's words with Retry; Ollama not running, with Get Ollama
  and Check again; and an Ollama on another computer, with the daemon's sentence alone. An install
  already running is joined. After an install the dialog reads the model list again.
- **An old failure is not shown over an installed model.** A failure is shown only when this panel
  watched that install, or while the model is still not installed, so a model installed meanwhile
  with `ollama pull` or `biorouter models decision pull` reads "tev1:0.8b is installed."

The panel's strings are `DECISION_INSTALL_COPY` in `DecisionModelInstall.tsx`.

## The gate chip on a phase row

A phase with a decision gate (`PhaseSnapshot.gate`, parsed tolerantly in
`utils/sessionSubagents.ts`, so an older daemon's snapshot still parses) shows one chip at the end
of its line in the summary's Phases section (`gateChipWords` in `summary/SummarySubagents.tsx`).
The chip is one word and a glyph; its tooltip, which is also its accessible name, is the full
sentence, with the probability named by its outcome:

| State | Chip | Tooltip |
|---|---|---|
| not asked yet | Gate | "Gate not asked yet · threshold 0.80" |
| passed | Passed | "Gate passed · P(yes) 0.91 · threshold 0.80" |
| not passed, attempts left | Not passed | "Gate not passed · P(yes) 0.62 · threshold 0.80 · attempt 2 of 3" |
| blocked, waiting for the person | Needs you (accent) | "Gate needs you: blocked after 3 attempts", or "…: could not be evaluated" |
| blocked, nothing waits for the person | Blocked | "Gate blocked: blocked after 3 attempts", or "…: could not be evaluated" |
| passed by the person | Passed | "Passed by you", or "Your pass stands until its subagents finish" |
| skipped by the person | Skipped | "Skipped by you", or "Your skip stands until its subagents finish" |

The reason "blocked after 3 attempts" is said only when the gate used every attempt. A gate blocked by
evaluation errors in a row, which use no attempt (the model was refused, unreachable or too small
for the text), reads "could not be evaluated", whatever attempts it had used before. Pressing the
chip opens the phase gate dialog. A phase from a daemon before gates has no `gate` and draws no
chip. The chip, the held line and "Add gate" are described with the rest of the rail in
[the Chat summary rail](chat-summary-rail.md).

## The phase gate dialog

`components/decisions/PhaseGateDialog.tsx`, mounted by `BaseChat`, is where the person controls a
gate ([Decision gates](../agent-loop/orchestration-phases.md#decision-gates)). It is a medium
(480px) dialog with the standard body and footer. Opened by the pointer or the keyboard, focus
rests on the dialog box, so Enter does nothing until the person moves to a button: its first
button overrides a gate.

- **Viewing a gate**, the body shows, in order: the status with its glyph ("Not asked yet",
  "Passed", "Not passed · 1 of 2 attempts used", "Blocked", "Blocked: needs you", "Your pass
  stands", "Passed by you" and the others; a phase that is closed reads as its gate did, "Passed" or "Not passed", and "Completed" only for a gate never asked), with an InfoTip saying what Pass and Skip each do;
  for a gate that did not pass or is blocked, the one number that explains it ("Lowest P(yes) 0.03
  · threshold 0.90"); a gate waiting for the person gets one warning line, "Waiting for you.", with a "Why it waits" InfoTip holding the reason ("Only you can move this phase on now: …" after every attempt failed, or the block's reason after an error block or the agent's own block); the other sentences that say what the person can do (what the agent can still do, "The last check decided nothing: …" after an error); the questions as a plain list;
  what it holds while the phase is open ("Review the probe waits until this gate passes."; a held
  phase the plan no longer names reads "a later phase", never its id); and a closed **Details**
  with the threshold ("Every question must reach P(yes) 0.90"), the last check when it is not
  already up front, the attempts, the model (`tev1:0.8b · this computer`, or "This chat's
  decision model, or its own model when it has none"), who gave the last verdict (Judged by:
  "gpt-5.5, the chat's model" or "joey-1, a decision model", and for the chat's model the
  sentence saying why, such as "This chat has no decision model it may use, so gpt-5.5, the
  chat's model, judged the gate."), "Accepts uncalibrated probabilities" when set, and who set
  it (The agent, You, The workflow).
- **The footer** holds Skip this phase and Pass this phase, each with a tooltip ("Lets the agent
  move on without this phase. It is marked skipped." and "Counts the phase as passed, as if its
  gate had passed."). Pass is the primary button only when the person is needed (blocked and
  waiting for them, or blocked by errors with no attempt left); in every other state it is
  secondary and the footer has no primary, because the safe course is to close. Skip is always the quiet (ghost) button. At its start a
  `⋯` named "More gate actions" holds Try again (once an attempt was used or the gate blocked),
  Edit gate, Decision history (which closes the dialog and opens the history on the gate) and,
  last after a separator, Remove gate…, which asks first ("Remove this gate?": its questions and
  attempts are discarded, and the agent can complete the phase without a check). A closed phase's
  footer holds Decision history alone; an open phase with no gate offers Add gate.
- **Each action** goes through the person's routes (`putPhaseGate`, `deletePhaseGate`,
  `decidePhaseGate` in `utils/phaseGates.ts`) with the person's proof. While one runs every
  footer control is disabled and the pressed one reads "Working". A result that applied and one
  that stands until the subagents finish read differently, in the daemon's words, and a refusal is
  shown in the daemon's words too, at the end of the body.
- **The editor** ("Add a phase gate" on a phase without one, or "Edit gate") replaces the body in
  the same dialog, with one Cancel and Save gate. Up front: "Saving makes this gate yours and
  starts its attempts over." when editing, 1 to 8 questions (with an InfoTip on how the model
  answers them), and the threshold slider from 0.05 to 0.99 by 0.01 (default 0.80) with its
  sentence, "Passes when every question's P(yes) is at or above 0.80." Below 0.50 the sentence is
  "Below 0.50 the gate passes answers the decision model leans against. That is your call to
  make." **More options**, with the summary "3 attempts · This chat's decision model" while
  closed, holds the attempts (1 to 5, default 3), the model (the shared picker rows; a model the
  chat may not use is offered disabled with its reason on the row, "Public model: it can't read a
  private chat." in a private chat) and "Accept uncalibrated probabilities". It starts open when
  the gate already uses one of them, and a refusal that names a field inside it opens it and
  focuses that field.

It works the same in a `biorouter serve` browser, where only the reach gate applies.

## A workflow's supervision

When a workflow declares `decision_settings` or `orchestration_plan`
([the workflow schema reference](../workflows/workflow-schema-reference.md#supervision-decision_settings-and-orchestration_plan)),
the warning shown before it starts (`components/ui/WorkflowWarningModal.tsx`) has a Supervision
panel: the default decision model, each monitor (its name, its model or "chat default", Public or
Private from the decision model list, and what it does), the plan and its gates ("2 questions at
P(yes) ≥ 0.80, 3 attempts"), "The agent cannot change, skip or remove these gates; only you can
pass, skip or retry one.", and "A monitor's decision model reads the newest messages of this
chat." The workflow editor (`components/workflows/CreateEditWorkflowModal.tsx`) shows the same
summary read only, with "Monitors and phase gates are edited in the workflow file; saving here
keeps them." The summary's Make workflow dialog
(`components/workflows/CreateWorkflowFromSessionModal.tsx`) does the same with the chat's
monitors and phase plan that `/workflows/create` captured: it shows them read only under the form
and writes them into the saved workflow. It used to drop both, so a workflow made from the desktop
lost the supervision `biorouter workflow create` keeps.

## The `decide` card in the transcript

A `decisions__decide` tool row (`components/decisions/DecideToolSummary.tsx`) reads the questions
the way the tool reads them (`decideArguments` in `decisionWire.ts`): `question` stands for
`instructions`, a question with `options` and no `type` is a choice and one with `levels` a score,
and the tool's synonyms (`multiple_choice`, `rating` and the others `question_type` in
`decisions_extension.rs` takes) count. Once the call answers, each question's kind is the one the
daemon ran.

- **The closed row** says what was checked, and its meta stays short, because a row's meta never
  truncates: "Checked 30 chunks against 1 question · 10 yes", or "… · 41 yes, 6 to review, 2
  errors". The yes count is given for one yes or no question only (from the daemon's four groups when it sends them, where a yes is a pass; "N yes by majority" for a vote); a choice, a score or several
  questions add only the error count, even when a threshold gives the daemon's groups for them. A call that escalated adds how many chunks it asked again, "… · 10 yes, 8 escalated". While the call runs it reads "Checking 30 chunks against 1
  question", or, for a call with `only_ids`, the chunks it names: "Checking 20 chunks against 1
  question".
- **The open well** starts with one muted line: the model and where it runs (`joey-1 ·
  Baseten`), a private model's padlock right after its name (in `PRIVATE_MARK_INK`, as on the decision card), the errors when there are any, and "598 of 600 chunks checked" only when fewer
  chunks were checked than sent and the errors do not account for it. When the batch is
  uncalibrated, the history's sentence follows ("This model's probabilities are its own
  estimate.", with its InfoTip). Then, for at most three questions, the question in plain words
  (never its name; "Question 1" when the call gave none) and one muted counts line: the answers, each with its number first, then the review count said as part of them, "1 yes · 19 no · 4 of them to review" (a yes is a pass; the review count is never a group beside the answers). A choice or a score gives each answer in option order ("300 rrms · 200 spms · 98 none · 12 of them to review"), with or without a threshold: the daemon sends a thresholded choice its four groups too, and the card used to draw them as "10 yes · 20 no", answers a three-way choice never offered (`decideCountsLine` reads the groups as yes and no only for a yes or no question, and for a choice or a score takes only the review count from them). A vote of several models opens its well with "Vote of 3 models: …" and counts the majority, "Majority 7 yes · 13 no · the models agreed on 19 of 20", with ties counted when there are any.
  A fourth question and beyond are "2 more questions in the decision history". Grouped errors
  follow, and the well ends with the history link, named "Open this decide call in the decision
  history". While the call runs only the question texts show.
- **A call that escalated** (`escalate`, see "Escalating the unsure answers" in [the agent-loop
  guide](../agent-loop/decision-models.md#the-decide-tool-screening-a-large-input)) says so in ONE
  line under the model, "8 of 30 escalated to clef-flash:latest", with the second model's padlock
  when it is private and "· 1 failed" when the second model failed some answers. An InfoTip
  ends the line (hover peeks, click pins): "Answers with a top probability under 0.80 were asked
  again of clef-flash:latest, and its answers are final.", then "It changed 3 of 8." or, when it
  failed some, "Where it gave no answer, tev1:0.8b's answer stands." Its per-chunk details sit in the
  open well, which is the row's own disclosure, under an "Escalated" section shaped like the
  errors: each chunk asked again by its number in the call, "Chunk 4", the number the decision
  history gives the same chunk (never the id the agent gave it), with its final answer beside
  the first model's ("no 0.90, was yes 0.60 by tev1:0.8b", or "yes by tev1:0.8b: the second model
  gave no answer" when it failed or the chunk was too large for its window), each answer prefixed by its question's words only when several questions were asked
  again. At most ten chunks are listed (`DECIDE_CARD_ESCALATED_SHOWN`), then "2 more chunks in the
  decision history"; the batch's other chunks are never listed. `decisionWire.ts` reads the
  daemon's `escalation` block into `DecideEscalation`, and `decideArguments` gives the call's
  chunk ids in order, from which the card takes each number.
- **What is not on the card.** The question type, the threshold, the passed count and the time
  are in the history's rows and detail. The chunks' text, which may hold clinical notes, the
  result's text for the model and the chunks' ids are never shown; the escalated chunks are
  named by their number in the call, in the open well only.

The to-review count is the chunks the agent has to read before it reports them (the result's
`review`, explained under "The review list" in
[the agent-loop guide](../agent-loop/decision-models.md#the-decide-tool-screening-a-large-input)).
`decisionWire.ts` reads it from `review` (with `reviewIds` and `moreReview` beside it) and never
from the retired `uncertain` field, which named a fixed 0.4 to 0.6 band. The well scrolls itself
into view once it opens (`useRevealWhenOpened`), so a call opened at the end of a chat is not left
under the composer.

## The turn marker

A turn that made decision calls ends with one marker row (`components/decisions/DecisionTurnMarker.tsx`,
`markerParts` and `markerSourceLine` in `decisionWire.ts`).

- **Closed**, it is a short summary with no names and no probabilities (a last probability does
  not say which outcome it is): one part per kind of source, counts first, such as "Monitors
  checked 10 times, none fired", "Monitor checked 5 times, quiet", "1 decide call read 30 chunks" (the calls are the sum of the items' `calls` from the daemon, because one item holds every `decide` call of the turn: three calls of 200 chunks read "3 decide calls read 600 chunks"),
  "Gate checked twice, not passed" or "Decision chat answered 3 times", each ending ", 2 errors"
  when it had any. The parts are ranked by what matters most (errors, then a gate not passed, then
  a monitor that fired, then the rest; ties keep monitors, decide calls, gates, the rest), at most
  two show, then "+1 more". Each part is its own span, so a part that does not fit drops whole and
  a number is never cut.
- **Its status**, read by a screen reader and drawn as the row's mark, is the first that applies:
  "A decision call failed this turn", "A phase gate did not pass this turn", "A monitor fired this
  turn".
- **Open**, it lists one line per source in the same order, by display name ("Done with shell:
  checked 3 times, fired twice", "Decide call: 30 chunks"). Each line is itself a history link in
  quiet ink, named "Open Done with shell in the decision history", and the well ends with a plain
  link to the whole turn. It scrolls itself into view once it opens.

## Decision history links

Every "Decision history" link in the transcript is one control,
`components/decisions/DecisionHistoryLink.tsx`: a `Button` in its link variant with the class
`.br-decision-link`, accent ink, no underline at rest, never centred and never filled, on the last
line of its block. Its accessible name, which its label tooltip repeats, says what it opens ("Open
this decide call in the decision history"), so a turn's link can be told from a card's before it
is pressed. The `decide` card and the turn marker use it, and a marker's source lines are the same
link in quiet ink (`.br-decision-link--line`). The history's Retry and Clear filters and the
Monitors dialog's Retry wear the same class. The decision chat's card keeps a link button of its
own at the end of its Details meta row. Inside a dialog footer or a menu the same destination is a
menu item or a button with the words "Decision history", as the phase gate dialog's `⋯` has it.
`DecisionHistoryLink.test.tsx` holds the look and guards the folder: no other file writes
"Decision history" as visible link text.

## A decision chat's composer

When a chat's model is a decision model (`isDecisionProvider` in `decisionWire.ts`), the composer
is reduced to what a decision model can use. `ChatInput` draws `DecisionComposer` above the text
box, and leaves out what means nothing there:

- **No `/` or `@` list.** A decision model reads the text and nothing else, so neither popover opens (a typed `/compact` still answers that there is nothing to compact). A send waits for up to 3 seconds for the question typed just before it to be stored (`settleDecisionQuestion` in `DecisionComposer.tsx`, awaited by `ChatInput`), while the composer clears at once.
- **The model menu names where the model runs.** For a decision provider the model dialog and the chip's menu read "Runs on this computer" or "Runs on Baseten" (`decisionModelPlace`), then the tier, on a line that wraps. The local affiliation's "On this machine" is not repeated, and the chip shows the alias (`joey-1`).
- **No `+` menu and no Tools chip**: a decision model reads the pasted text and calls no tools.
  The model menu (`settings/models/bottom_bar/ModelsBottomBar.tsx`) has no Effort, no "Lead and
  worker…" and no line saying the model can read files on this computer; its privacy line stays.
- **No context ring.** `ChatInput` passes `showContext={false}` to `ComposerFooter`, which keeps
  the working folder and the cost, and skips the context-window lookup. A decision chat has no
  running context: each message is asked on its own, so nothing adds to the chat's context, and a
  ring would show a total that never moves, and its Compact action would ask a model that cannot
  summarise. History's token count and the chat's cost still count each answer's billed tokens,
  as [Usage and billing](../agent-loop/decision-models.md#usage-and-billing) describes. A message
  too long for the model is refused with a sentence naming its window, and a typed `/compact`
  answers that there is no context to compact, as it does in the terminal UI and the classic CLI.

`components/decisions/DecisionComposer.tsx` holds the chat's question. Up front is one line: the
label "Question", an unfilled input like the composer's own text box, and at its end the status
("Saving…", then "Saved" with a check, or "Reading this chat's question…"). Its placeholder is an
example that cannot be mistaken for a kept value, and follows the answer kind: "For example: Does
this note mention a fever?", "For example: Which MS course does this note describe?", "For
example: How severe is the pain this note describes?". Focusing it lights the composer card's
edge as typing in the text box does. A longer message (a refusal, "Name at least 2 different
options.", "Give every level a label.", "The threshold is a number from 0 to 1.") takes its own
line under the question, in the same status node.

**Question options**, closed by default, holds the rest, with a summary in words while closed
("Yes or no", "Choice of 3", "Score, 4 levels", then the rule when a threshold is set, "· when
P(yes) is at or above 0.85"): the answer kind (Yes or no, Choice, Score), the options or the levels
lowest first (each with a placeholder, "Option 1", "Level 1"), and the threshold, with an InfoTip
("Optional. With a threshold, each answer shows whether it meets it.") and, once set, the sentence
"Meets the threshold when P(yes) is at or above 0.85." A rule set outside the chat that these
fields cannot show reads "Meets the threshold … , as set outside this chat. Clear the threshold to
set a new one." Before the chat's first message there are no fields, only the line "Start your
message with a question line, such as “Q: Is the form signed?”, then paste the text."

## The decision card in a decision chat

A decision chat's reply is a card (`components/decisions/DecisionCard.tsx`, styled by
`decisionsChat.css`). For each decision in it, up front: the question, muted; the answer once,
large, with its own probability ("No" then "P 0.88", the probability of no, never P(yes)); and, when the
question has a threshold, "Meets the threshold" with a check or "Doesn't meet the threshold" with
a neutral ring, both muted and never a cross. A decision chat's threshold marks an answer and is
not a gate, so a correct "No" under a P(yes) threshold never reads as a failure, and neither word
names a direction, because a below rule is met when P is under its number. An "Uncalibrated"
badge, when it applies, carries its meaning in an InfoTip (`calibrationHelp`): "The model wrote
this number itself, so read it as a rough guide." for a verbalized source, and a sentence about
the probability mass or the calibration otherwise. An error or a refusal is shown up front.

A closed **Details** holds the rest: for one decision, the bars (one per outcome, the answer's in
the accent, the threshold marked), the threshold in its record form ("Threshold: P(yes) ≥ 0.85")
and "Why: …" for a verbalized source with a reason. Several decisions in one record each get a
group under their own question, so a reader matches a headline to its bars. One meta row ends it:
the decision mark and the model's alias, where it runs, a lock with a hidden "Private model" for a
private model, the latency, and the "Decision history"
link, which opens the history on that one call.

## Decision models in the Usage panel

Settings > Usage counts decision-model usage in its totals and shows it on its own line
(`components/settings/usage/UsagePanel.tsx`):

- **A Decision models line** under the month's totals, with an InfoTip: "620 requests, 1.2M
  tokens, $0.12". The cost part (`formatDecisionCost`) says which requests had no price, from the
  subtotal's `hasUnmetered` and `hasUnpriced`: "Not metered" when every request went to a local
  model, "$0.04, some not metered" when local requests sit beside priced ones, "Unavailable" when
  the requests went to a Baseten model with no known price, which Baseten still billed, and
  "$0.04, some unavailable" when such requests sit beside priced ones. So the line never calls a
  billed request unmetered while that model's row reads "Unavailable". The cost part is dropped
  while costs are hidden.
- **In the per-model table** a decision model shows its short name (`joey-1`), the subtitle
  "Decision model · N requests", and a dash in Turns, which a screen reader reads as "Decision
  requests are not turns." An unpriced local decision model reads "Not metered" rather than
  "Unavailable". An unpriced Baseten decision model reads "Unavailable": Baseten billed it at a
  rate Biorouter does not know.
- **In the totals** decision tokens and priced dollars count, and so does the monthly budget.
  Decision requests are not turns. A local decision model counts as $0 and does not mark the
  totals as partial, so a month of only local decisions reads $0.00, not "Unavailable". An
  unpriced Baseten decision model marks them partial like any other unpriced model.
- **In a chat's cost** under the composer, and its breakdown, the same rule holds: a local
  decision model's row reads "Not metered" and leaves the chat's cost exact
  (`isUnmeteredDecisionRow` in `components/bottom_menu/CostTracker.tsx`).

The panel reads the daemon's split (`decisionRequests` and `decisions` on each report row and on
the totals), so it needs no list of decision providers. On Home, Favorite model never picks a
decision model, and the Models tab lists one by its short name after the chat models, so the
tab's first row is the Favorite model (`isDecisionUsageRow`, `modelBreakdown` and
`decisionModelName` in `utils/usageStats.ts`). Where the numbers come from, and the prices, are in
[Usage and billing](../agent-loop/decision-models.md#usage-and-billing). A decision's usage never
moves its chat in the sidebar ([what moves a chat](sidebar-chat-list.md#what-moves-a-chat)).

## The count in History

A top-level chat row in History shows its decision-model call count after where it ran, read
from the row's `decision_count`, with the full sentence in its tooltip. A nested subagent run
never shows one, and the calls are never rows of their own. A decision chat's row draws no
extensions cell: its model calls no tools, so the extensions its session row happens to carry are
not something it used.

## Where it is pinned

| What | Test |
|---|---|
| Parsers, wire round trips, the Rust enum variants, one row per source for 5,000 calls, the refetch revision, the proof on every request, a Baseten model named by its alias | `utils/sessionDecisions.test.ts` |
| The shared words: a quiet monitor never "Failed" and never a cross, a `decide` "no" read as "no", P(no) beside "no", "2 monitors" and never "+", no provider key or slug in any output, and the Source to Outcome table row by row | `components/decisions/decisionWords.test.ts` |
| One history link: its text, name, tooltip, class and placement, and no other file in the folder writing its own | `components/decisions/DecisionHistoryLink.test.tsx` |
| The facet chip's class string and `data-engaged` | `components/ui/facet-chip.test.tsx` |
| The picker rows: where it runs, the lock and its hidden words, the window only under 8K, the private-chat line, backticks removed | `components/decisions/decisionModelOption.test.tsx` |
| A Baseten model named by its alias in the history rows and detail, the decision card and the Monitors list | `components/decisions/DecisionHistory.test.tsx`, `components/decisions/DecisionCard.test.tsx`, `components/decisions/MonitorsDialog.test.tsx` |
| The hook's gate, re-reads and polling | `hooks/useSessionDecisions.test.tsx` |
| The section's rows (one per monitor that exists now, a several-monitor call on each, no row for a removed monitor or a gate), the trailing words, the closed line, opening on a fire or an error, a person's toggle holding, the Monitors button and its absence in a decision chat, and the stylesheet rules | `components/summary/SummaryDecisions.test.tsx`, `components/summary/summaryDecisionRows.test.ts` |
| The section at 240px in Chromium: five 28px rows for 5,000 calls, no bar, the heading's 24px Monitors button, nothing past the card | `components/summary/summaryDecisions.browser.test.tsx` |
| The history view's window, paging, selection, filters (Source, Outcome per source, the Filters popover and its label, the chips no picker names), row titles and verdicts, the model on the second line only when the list holds several models, following new calls against the list's own read, the detail up front and under Details, and the Model tier label | `components/decisions/DecisionHistory.test.tsx` |
| A `decide` call's rows at one time and in chunk order, a vote's models side by side, pages walking them once each, and a cursor that resumes only from a row of this chat | `crates/biorouter/src/decision/log.rs` (`cargo test -p biorouter --lib decision::log`) |
| In Chromium: 2,000 rows scrolled to the end, ten calls hugged in a narrow panel, the Filters popover kept open by a nested pick, Escape closing one layer at a time, and the dark popover's strong step | `components/decisions/decisionHistory.browser.test.tsx` |
| The `decisions` kind in the side panel | `components/decisions/decisionsArtifact.test.tsx` |
| The Monitors dialog: each change written at once over a fresh read (an agent's monitor added meanwhile kept, Undo putting back only its own monitor), the status and Undo, rows and their peek, the editor in place with More options and its starting state, Fires when and the 21-stop threshold, Stop only once, choice descriptions, validation and the daemon's refusals, owner badges and the transfer to the person, privacy reasons and the Crew rows, the model list's missing route, refusal and failure with Retry, and the browser | `components/decisions/MonitorsDialog.test.tsx`, `utils/sessionDecisions.test.ts` |
| In Chromium: the six-model picker of a private chat fitting its menu without scrolling, segmented tracks hugging their segments, the editor in the edited row's place, and the × steady through a row toggle | `components/decisions/monitorsDialog.browser.test.tsx` |
| The composer's `+` row that opens the dialog, only in a chat that exists | `components/ChatInput.decisionMonitors.test.tsx` |
| The install panel's sentences and states, the About InfoTip, a busy install joined, an old failure hidden once installed, and the model picker's quiet form | `components/decisions/DecisionModelInstall.test.tsx`, `components/settings/models/subcomponents/SwitchModelModal.decisionModels.test.tsx` |
| The phase gate dialog: the footer and its primary only when the person is needed, the `⋯` items, Remove gate asking first, "Not passed", the last check up front, Details, the editor's More options and InfoTips, a disabled model's reason, and the person's routes and their results | `components/decisions/PhaseGateDialog.test.tsx` |
| In Chromium: a pointer or key open resting on the box, so Enter passes nothing, and the footer wrapping at 480px | `components/decisions/phaseGateDialog.browser.test.tsx` |
| The chip's words and sentences, the held line and "Add gate" | `components/summary/SummarySubagents.test.tsx`, `utils/sessionSubagents.test.ts` |
| The workflow warning's Supervision panel, the editor's read-only summary, and Make workflow keeping the monitors and plan | `components/ui/WorkflowWarningModal.test.tsx`, `components/workflows/CreateEditWorkflowModal.test.tsx`, `components/workflows/__tests__/CreateWorkflowFromSessionModal.test.tsx` |
| The History count on top-level rows only, and no extensions cell for a decision chat | `components/sessions/SessionListView.decisionCount.test.tsx`, `components/sessions/SessionListView.nested.test.tsx` |
| The `decide` card: the row's meta, the model line, question text and counts, at most three questions, the uncalibrated sentence, the review fields read under their own names, the questions read as the tool reads them, and its link | `components/decisions/DecideToolSummary.test.tsx`, `components/decisions/decisionWire.test.ts` |
| The turn marker: its parts and their order, the status words, and one link per source then the turn's | `components/decisions/DecisionTurnMarker.test.tsx`, `components/decisions/decisionWire.test.ts`, `components/decisions/decisionTranscript.test.tsx` |
| A decision chat: no `+` and no Tools chip, no Effort or files line, no context ring and no window lookup, the ring still drawn for a chat model, and the footer without its ring | `components/ChatInput.structure.test.tsx`, `components/settings/models/bottom_bar/ModelsBottomBar.decisionModel.test.tsx`, `components/ChatInput.decisionChatGauge.test.tsx`, `components/bottom_menu/ComposerFooter.test.tsx` |
| The decision chat's question line, its status, Question options and the placeholders | `components/decisions/DecisionComposer.test.tsx` |
| The decision card: the headline, "Meets the threshold" or "Doesn't meet the threshold" and never a cross, the Uncalibrated InfoTip, and in Chromium each question's bars under its own question once Details opens | `components/decisions/DecisionCard.test.tsx`, `components/decisions/decisionCard.browser.test.tsx` |
| The Usage panel's Decision models line (Not metered for local requests, Unavailable for an unpriced Baseten model), a decision model's row, and Home leaving decision models out of Favorite model and ranking them last in the Models tab | `components/settings/usage/UsagePanel.test.tsx`, `utils/usageStats.test.ts` |
| A local decision model leaves a chat's cost exact and reads "Not metered" in its breakdown; an unpriced Baseten one leaves it partial | `components/bottom_menu/CostTracker.test.ts`, `components/bottom_menu/CostTracker.render.test.tsx`, `components/bottom_menu/ModelBreakdownTable.test.tsx` |

Run them with
`cd ui/desktop && npx vitest run src/utils/sessionDecisions.test.ts src/hooks/useSessionDecisions.test.tsx src/components/decisions src/components/summary src/components/sessions src/components/ui/facet-chip.test.tsx src/components/ChatInput.structure.test.tsx src/components/ChatInput.decisionChatGauge.test.tsx src/components/ChatInput.decisionMonitors.test.tsx src/components/bottom_menu/ComposerFooter.test.tsx src/components/settings/models/bottom_bar/ModelsBottomBar.decisionModel.test.tsx src/components/settings/usage src/utils/usageStats.test.ts src/utils/sessionSubagents.test.ts src/components/ui/WorkflowWarningModal.test.tsx src/components/workflows src/components/settings/models/subcomponents/SwitchModelModal.decisionModels.test.tsx`.
Set `DECISIONS_SCREENSHOT_DIR` to a folder to have the summary's, the history's and the phase
gate dialog's browser tests save a picture.

## Related documentation

- [Decision models in the agent loop](../agent-loop/decision-models.md): monitors, the `decide`
  tool, phase gates and the decision chat, and the routes this page reads.
- [The Chat summary rail](chat-summary-rail.md): where the summary appears and the rules its
  content keeps.
- [Privacy tiers](../security/privacy-tiers.md): Gate H, which refuses a public decision model
  for a private chat.
- [The sidebar chat list](sidebar-chat-list.md): what moves a chat, which a decision's usage
  never does.
- [Orchestration phases](../agent-loop/orchestration-phases.md): the phases a gate sits on.
