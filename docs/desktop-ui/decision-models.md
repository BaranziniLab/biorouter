# Decision models in the desktop side panel

> **What this is.** How the desktop app shows a chat's decision-model activity outside the
> transcript: the Decisions section of the Chat summary, the decision history view (a tab of the
> artifact side panel), the Monitors dialog, the gate chip on a phase row, and the per-chat count
> in History. Covers what each one reads, the owner rule that keeps thousands of calls out of the
> chat, and the tests that hold each part.
> **Status:** Current. Built on the decision-model branch (`claude/decision-models`, November 2026).
> **Audience:** developers working on the Chat summary, the artifact side panel, History or the
> decision routes.

A decision model answers a typed question (yes or no, one of several options, a score level)
with a probability for every answer, in tens of milliseconds. A chat uses one in four ways: a
monitor that watches the chat and acts when a criterion is met, the agent's `decide` tool over
many chunks of input, a gate on an orchestration phase, or a decision model bound as the chat's
own model. The agent loop, the routes and the transcript's own cards are described in
[the agent-loop guide](../agent-loop/decision-models.md). This page covers the panel side.
The design builds on Albert Lee's prototype in PR #469, which first wired Baseten's decision
models into the loop as exit criteria.

## The owner rule: aggregate, never flood

Decision calls can number in the thousands in one chat (a monitor every iteration, a `decide`
batch over 600 notes). The rule that follows, binding on every surface:

| Surface | What it draws | Never |
|---|---|---|
| Chat summary | One row per source: each monitor, every `decide` call together, each phase gate, the decision chat | A row per call |
| Phase row | One gate chip: passed, failed with the attempt, or blocked | Rows for the gate's evaluations |
| History (the chat list) | One count on the chat's own row | Child rows, or a count on a subagent run |
| Decision history view | Every call, one row each, windowed and paged | |

The decision history view is the only place individual calls are listed.

## What the panel reads

Everything comes from the daemon's decision routes, through `utils/sessionDecisions.ts`:

| Route | Read by | Shape (`crates/biorouter/src/decision/`) |
|---|---|---|
| `GET /sessions/{id}/decisions/summary` | the summary section, the history view's filters | `SessionDecisionSummary` (`log.rs`) |
| `GET /sessions/{id}/decisions` | the history view's list, a page at a time | `DecisionPage` of `DecisionRecord` |
| `GET /sessions/{id}/decisions/{decision_id}` | the history view's detail pane | `DecisionDetail`, with the context |
| `GET` and `PUT /sessions/{id}/decision-settings` | the summary (monitors), the Monitors dialog | `DecisionSettings` (`settings.rs`) |
| `GET /decisions/models` | the Monitors dialog's model list | `DecisionModelsResponse` of `DecisionModelInfo` (`crates/biorouter-server/src/routes/decisions.rs`, not this folder; it carries `unavailableReason`, the daemon's reason this chat may not use a model) |

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

## The Decisions section of the Chat summary

`components/summary/SummaryDecisions.tsx`, drawn by `ChatSummary` between the subagents and the
statistics, in the docked rail and in the header popover alike.

- **Rows.** `decisionSourceRows` folds the summary's aggregates into one row per source:
  monitors in the order the settings list them (a monitor nothing has checked yet is still
  listed, as "Not checked yet"), then one `decide` row for every `decide` call together, the
  phase gates in plan order, the decision chat and ad hoc calls. A chat with 5,000 calls draws
  as many rows as it has sources.
- **A row** is the source's label and call count, then what it last answered and the last pass
  probability, with a 40px probability bar whose tick marks the monitor's or gate's threshold.
- **State, by shape.** An eye while a monitor watches, a raised flag once it fired, a struck eye
  when it is off, a warning after an error; a check, a cross or an alert for a gate that
  passed, failed or is blocked; the decision mark (a gauge) for calls that simply decided.
  The word is the row's accessible name, ahead of the label.
- **Pressing a row** opens the decision history filtered to that source.
- **Monitors.** A row under the sources opens the Monitors dialog and says how many monitors
  are on.
- **Hidden** while the chat has made no decision calls and has no monitors.

The rows and the Monitors action are plain buttons in the section's own module, never the
`Button` primitive, so `ChatSummary.tsx` keeps exactly three (`styles/summaryRail.test.ts`).
The stylesheet, `summary/summaryDecisions.css`, keeps the summary's rules: two type sizes, rows
of at least 28px, no hairline. `SummaryDecisions.test.tsx` reads it to hold them.

### The hook

`hooks/useSessionDecisions.ts` reads the summary and the settings on the same gate as the
subagent roster: while the summary is shown or the chat's turn runs. It re-reads when a
`decide` or `workspace_orchestrate` tool call answers, or a decision notice or a decision chat's
reply lands (`decisionRevision`). It polls the summary every 3 seconds only while a turn runs in
a chat with a monitor switched on or with decision calls on record, and reads the settings once
per stretch rather than on every poll. A daemon without the routes answers 404, which reads as
nothing to draw, not as an error.

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

- **Filters.** Source, criterion (a monitor by name, a phase gate by its phase, a `decide` call
  by when it ran), outcome; under More filters, the turn, a probability range and errors only.
  The criterion and turn lists come from the summary, so they name what the chat actually has.
  A `decide` card opens the view on its own batch, which no select names, so it shows as a
  removable "This decide call only" chip; choosing another source or criterion lets it go too.
- **The list** is a `listbox` of 44px rows, newest first, loaded 100 at a time. Only the rows in
  view and eight on either side are in the DOM, placed at their index times 44 inside a box as
  tall as the loaded rows, and the next page is asked for once the last loaded row is within 20
  rows of the view. The arrow keys, Home, End and the page keys move the selection, which is the
  list's active descendant and is kept in the DOM wherever the window is.
- **The detail pane** shows the selected call in full: what it saw (the exact context it was
  sent, from the detail route, cut at 64 KB and said so), each question with its instructions,
  its threshold and whether it passed, every option with its probability bar and the threshold
  marked on the option it is about, the answer, its probability and the distribution's
  concentration (explained as not the probability of the answer), whether the probabilities are
  calibrated (always said in words when they are not), the model and its tier, latency, tokens,
  the chunk, batch and turn, and the raw JSON.

The list sits over the detail in a narrow panel and beside it from 720px (a container query in
`decisions/decisionsPanel.css`). The view only reads, so it works the same in a
`biorouter serve` browser.

## The Monitors dialog

`components/decisions/MonitorsDialog.tsx` lists the chat's monitors and adds, edits or removes
them. A monitor has a name, a question, yes or no or one of several options (with the option
that fires it), a model, a threshold (a slider from 0.50 to 0.99), an action (notify, tell the
agent, stop the turn), when it checks (after each step, or when the turn ends) and an on switch.
Save writes the whole list with `PUT /sessions/{id}/decision-settings`.

- **Two ways in.** The summary's Decisions section has a Monitors row, but that section is drawn
  only once a chat has a decision call or a monitor, so the composer's `+` menu also carries
  "Decision monitors…" (`composer/ComposerPlusMenu.tsx`), which is how the first monitor is
  added. The row is there only in a chat that exists (`ChatInput`'s `sessionId`), never on Home
  or in a new chat before its first message. Both open the one dialog `BaseChat` mounts.
- **It never saves what it has not read.** Every open reads
  `GET /sessions/{id}/decision-settings` and keeps Add, Save and every row's controls off until
  the answer arrives, even when the summary already holds a copy: Save replaces the whole record,
  and the summary's copy of an idle chat can predate a write made with
  `biorouter session monitors` or in another window. The copy is shown meanwhile, and used when
  the read fails; opened before anything has read the settings (from the `+` menu while the
  summary is folded away), a failed read keeps Add and Save off for good, because an empty list
  saved in their place would replace every stored monitor and the chat's default decision model.
  The value a save stores is handed back to the summary at once (`applySettings` in
  `hooks/useSessionDecisions.ts`), so it is current even while the summary reads nothing.

- **Validation is said by the field** before Save is pressed: a missing name or question, too
  few or duplicate options, no option chosen to fire on, and a public model on a private chat.
  A refusal the daemon sends anyway is shown above the footer, and the dialog stays open.
- **Privacy.** The dialog asks for the catalog naming the chat (`GET /decisions/models?sessionId=`),
  so each model carries the daemon's own reason the chat may not use it (its tier, or an
  institution the model does not cover). The list shows each model's tier, and a model with a
  reason is offered disabled, with the reason, because Gate H refuses it before anything is
  sent. Where the daemon gave no reason, a public model is still refused for a private chat.
- **The proof.** Save needs the person's proof on a daemon that holds a user-action key (the
  desktop app's) and none on a daemon that holds none (`biorouter serve`), so the dialog works
  the same in a browser. A desktop daemon that was meant to receive a key and did not refuses
  the write, and the dialog shows that sentence.

## The gate chip on a phase row

A phase with a decision gate (`PhaseSnapshot.gate`, parsed in `utils/sessionSubagents.ts`)
shows one chip under its heading in the summary's Phases section: "Gate 0.91 ≥ 0.80 passed",
"Gate 0.62 < 0.80, attempt 2 of 3", "Gate blocked", or "Gate at 0.80" before it has run. Its
state is a glyph and words. Pressing it opens the decision history filtered to that gate. A
phase from a daemon before gates has no `gate` and draws no chip.

## The count in History

A top-level chat row in History shows its decision-model call count after where it ran, read
from the row's `decision_count`, with the full sentence in its tooltip. A nested subagent run
never shows one, and the calls are never rows of their own.

## Where it is pinned

| What | Test |
|---|---|
| Parsers, wire round trips, the Rust enum variants, one row per source for 5,000 calls, the refetch revision, the proof on every request | `utils/sessionDecisions.test.ts` |
| The hook's gate, re-reads and polling | `hooks/useSessionDecisions.test.tsx` |
| The section's rows, states, hiding, actions and stylesheet rules; the gate chip | `components/summary/SummaryDecisions.test.tsx` |
| The section at 240px in Chromium: six rows for 5,000 calls, at least 28px, nothing past the card | `components/summary/summaryDecisions.browser.test.tsx` |
| The history view's window, paging, selection, detail and filters | `components/decisions/DecisionHistory.test.tsx` |
| 2,000 rows scrolled to the end in Chromium: every page loaded, 44px rows, a bounded window | `components/decisions/decisionHistory.browser.test.tsx` |
| The `decisions` kind in the side panel | `components/decisions/decisionsArtifact.test.tsx` |
| The Monitors dialog's validation, save, privacy reasons, the browser, its read on every open (a caller's older copy is not what it saves) and its filled rows | `components/decisions/MonitorsDialog.test.tsx` |
| The composer's `+` row that opens the dialog, only in a chat that exists | `components/ChatInput.decisionMonitors.test.tsx` |
| The History count on top-level rows only | `components/sessions/SessionListView.decisionCount.test.tsx` |

Run them with
`cd ui/desktop && npx vitest run src/utils/sessionDecisions.test.ts src/hooks/useSessionDecisions.test.tsx src/components/decisions src/components/summary src/components/sessions`.
Set `DECISIONS_SCREENSHOT_DIR` to a folder to have the two browser tests save a picture of the
section and the view.

## Related documentation

- [Decision models in the agent loop](../agent-loop/decision-models.md): monitors, the `decide`
  tool, phase gates and the decision chat, and the routes this page reads.
- [The Chat summary rail](chat-summary-rail.md): where the summary appears and the rules its
  content keeps.
- [Privacy tiers](../security/privacy-tiers.md): Gate H, which refuses a public decision model
  for a private chat.
- [Orchestration phases](../agent-loop/orchestration-phases.md): the phases a gate sits on.
