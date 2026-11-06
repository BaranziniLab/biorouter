# Decision models in the desktop app

> **What this is.** How the desktop app shows a chat's decision-model activity: the Decisions
> section of the Chat summary, the decision history view (a tab of the artifact side panel), the
> Monitors dialog, the gate chip on a phase row, the `decide` card in the transcript, a decision
> chat's composer, the Decision models line of the Usage panel, and the per-chat count in History.
> Covers what each one reads, the owner rule that keeps thousands of calls out of the chat, and the
> tests that hold each part.
> **Status:** Current. Built on the decision-model branch (`claude/decision-models`, November 2026).
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
| `GET /decisions/models` | the Monitors dialog's model list | `DecisionModelsResponse` of `DecisionModelInfo` (`crates/biorouter-server/src/routes/decisions.rs`, not this folder; it carries `unavailableReason`, the daemon's reason this chat may not use a model, and `sessionUnavailableReason`, its reason this chat may use none) |

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
- **A blocked gate says why.** Its row reads "blocked after 3 attempts" only when the gate used
  every attempt. A gate blocked by evaluation errors in a row, which use no attempt (the model
  was refused, unreachable or too small for the text), reads "blocked: could not be evaluated", whatever
  attempts it had used before (`gateBlockedUnevaluated` in `summary/copy.ts`).
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
- **Model tier, not Tier.** The detail pane labels the decision model's tier "Model tier"
  (`DECISION_HISTORY_COPY.tier`). It is the model's tier, not the chat's: a public chat that used a
  local decision model reads "Model tier: Private", and its content is still public. The decision
  card says the same thing as "Private model" or "Public model".

The list sits over the detail in a narrow panel and beside it from 720px (a container query in
`decisions/decisionsPanel.css`). The view only reads, so it works the same in a
`biorouter serve` browser.

## The Monitors dialog

`components/decisions/MonitorsDialog.tsx` lists the chat's monitors and adds, edits or removes
them. A monitor has a name, a question, yes or no or one of several options (with the answer
that fires it: Yes or No, or one option), a model, a threshold (a slider from 0.50 to 0.99), an
action (notify, tell the agent, stop the turn), when it checks (after each step, or when the turn
ends) and an on switch. Save writes the whole list with `PUT /sessions/{id}/decision-settings`.

- **Each row says what the daemon compares.** The line under a monitor's name reads its
  `option` as `MonitorCriterion::pass_threshold` does (`monitorThreshold` in
  `utils/sessionDecisions.ts`) and words it as the decision card words the same threshold:
  `P(yes) ≥ 0.85`, `P(no) ≥ 0.85` for a yes/no monitor that watches No, `P(rrms) ≥ 0.70`, and
  `P(level ≥ mild) ≥ 0.85` for a score monitor, whose level is named by its label whether the
  record stores the label or its index.
- **A score monitor keeps its score.** The dialog makes no score monitor, but one set over HTTP
  opens with a third Score segment selected, its levels listed lowest first and the level it
  fires on, all read only. Editing its other fields saves the score question and its level
  unchanged; only choosing Yes or no, or One of several options, converts it.

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
  so each model carries the daemon's own reason the chat may not use it (its tier, an
  institution the model does not cover, a Crew grant on the chat, or a local copy Ollama cannot
  load, whose reason names the `ollama pull` or `ollama rm` that fixes it). The list shows each
  model's tier, and a model with a reason is offered disabled, with the reason, because Gate H or
  the Crew rule refuses it before anything is sent, or Ollama would refuse to load it. In a
  Crew-scoped chat every model carries the Crew sentence, and so does "The chat's decision
  model", which the catalog's `sessionUnavailableReason` gives because no row stands for it, so
  the dialog saves no monitor there that every check would refuse. Where the daemon gave no reason, a public model is
  still refused for a private chat.
- **A model list that cannot be read is said under the picker.** The picker would otherwise offer
  only "The chat's decision model" with nothing to say why. A daemon without the route (404, an
  older Biorouter service, which answers a bare 404) is told apart from an empty list
  (`fetchDecisionCatalog`'s `missing`) and gets a sentence asking for a restart to finish
  updating. The route's own 404 carries a sentence ("There is no chat with that id.", for a chat
  deleted in another window) and is shown as a failure with that sentence instead. A 401 or 403 says Biorouter refused
  to list the models, with the daemon's own sentence when it gave one, and any other failure says
  the models could not be loaded, with the error's message or the status. Those two carry a Retry
  link that reads the list again and keeps the monitor being edited; a new read clears the
  sentence until it answers.
- **The proof.** Save needs the person's proof on a daemon that holds a user-action key (the
  desktop app's) and none on a daemon that holds none (`biorouter serve`), so the dialog works
  the same in a browser, steer monitors included. On a daemon with no key the one write refused is
  to a subagent's chat, with the sentence that tab's other controls give there
  (`DECISION_SETTINGS_SUBAGENT_NO_KEY`). A desktop daemon that was meant to receive a key and did
  not refuses the write, and the dialog shows that sentence. A saved change reaches the chat's
  next turn, never the one running.

## The gate chip on a phase row

A phase with a decision gate (`PhaseSnapshot.gate`, parsed in `utils/sessionSubagents.ts`)
shows one chip under its heading in the summary's Phases section: "Gate 0.91 ≥ 0.80 passed",
"Gate 0.62 < 0.80, attempt 2 of 3", "Gate blocked", or "Gate at 0.80" before it has run. Its
state is a glyph and words. Pressing it opens the decision history filtered to that gate. A
phase from a daemon before gates has no `gate` and draws no chip.

## The `decide` card in the transcript

A `decisions__decide` tool row's well (`components/decisions/DecideToolSummary.tsx`) shows the
questions the call asked, read the way the tool reads them (`decideArguments` in
`decisionWire.ts`): `question` stands for `instructions`, a question with `options` and no `type`
is a choice and one with `levels` a score, and the tool's synonyms (`multiple_choice`, `rating`
and the others `question_type` in `decisions_extension.rs` takes) count. Once the call answers,
each question's kind is the one the daemon ran. The card then shows a head line (chunks, the
model by the name the rest of the panel uses, `joey-1` for a Baseten slug, errors,
"uncalibrated" when it applies, the time) and one line per question with counts only:

- for a yes/no question, how many said yes; for a choice or score, each answer's count;
- how many passed, when the question has a threshold;
- **how many are to review**, the chunks the agent has to read before it reports them (the
  result's `review`, explained under "The review list" in
  [the agent-loop guide](../agent-loop/decision-models.md#the-decide-tool-screening-a-large-input));
- the highest probability.

For example "has_ms yes 41, 6 to review, max p 0.98" and "course rrms 300, spms 200, none 98, 280
passed, 12 to review, max p 0.71". The to-review count shows for any question that has one, so a
choice or score with a threshold carries it as a yes/no question does; a count of zero is left
out. Grouped errors follow, then a link to the batch in the decision history. The chunks, which
may hold clinical text, and the result's text for the model, which names chunk ids, are never
listed on the card. `decisionWire.ts` reads the count from `review` (with `reviewIds` and
`moreReview` beside it) and never from the retired `uncertain` field, which named a fixed 0.4 to
0.6 band.

## A decision chat's composer

When a chat's model is a decision model (`isDecisionProvider` in `decisionWire.ts`), `ChatInput`
passes `showContext={false}` to `ComposerFooter`, which then draws no context ring: the footer
keeps the working folder and the cost. A decision chat has no running context. Each message is
asked on its own and nothing adds to the chat's token total, so a ring would show a total that
never moves against a window from a static table, and its Compact action would ask a model that
cannot summarise. `ChatInput` also skips the context-window lookup for such a chat. A message
too long for the model is refused with a sentence naming its window, and a typed `/compact`
answers that there is no context to compact, as it does in the terminal UI and the classic CLI.

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
never shows one, and the calls are never rows of their own.

## Where it is pinned

| What | Test |
|---|---|
| Parsers, wire round trips, the Rust enum variants, one row per source for 5,000 calls, the refetch revision, the proof on every request, a Baseten model named by its alias | `utils/sessionDecisions.test.ts` |
| A Baseten model named by its alias in the history rows and detail, the decision card and the Monitors list | `components/decisions/DecisionHistory.test.tsx`, `components/decisions/DecisionCard.test.tsx`, `components/decisions/MonitorsDialog.test.tsx` |
| The hook's gate, re-reads and polling | `hooks/useSessionDecisions.test.tsx` |
| The section's rows, states, hiding, actions and stylesheet rules; a blocked gate's reason; the gate chip | `components/summary/SummaryDecisions.test.tsx` |
| The section at 240px in Chromium: six rows for 5,000 calls, at least 28px, nothing past the card | `components/summary/summaryDecisions.browser.test.tsx` |
| The history view's window, paging, selection, detail (with the Model tier label) and filters | `components/decisions/DecisionHistory.test.tsx` |
| 2,000 rows scrolled to the end in Chromium: every page loaded, 44px rows, a bounded window | `components/decisions/decisionHistory.browser.test.tsx` |
| The `decisions` kind in the side panel | `components/decisions/decisionsArtifact.test.tsx` |
| The Monitors dialog's validation, save, privacy reasons, the browser, its read on every open (a caller's older copy is not what it saves) and its filled rows | `components/decisions/MonitorsDialog.test.tsx` |
| The Monitors list naming P(no), a choice and a score level; a score monitor kept through an edit; Fires on Yes or No; the model list's missing route, refusal and failure with Retry | `components/decisions/MonitorsDialog.test.tsx`, `utils/sessionDecisions.test.ts` |
| The composer's `+` row that opens the dialog, only in a chat that exists | `components/ChatInput.decisionMonitors.test.tsx` |
| The History count on top-level rows only | `components/sessions/SessionListView.decisionCount.test.tsx` |
| The `decide` card's per-question line, the to-review count on a choice with a threshold, the review fields read under their own names, the questions read as the tool reads them (`question`, inferred and synonym types), and the model named by its alias in the head | `components/decisions/DecideToolSummary.test.tsx`, `components/decisions/decisionWire.test.ts` |
| No context ring and no window lookup in a decision chat, the ring still drawn for a chat model, and the footer without its ring | `components/ChatInput.decisionChatGauge.test.tsx`, `components/bottom_menu/ComposerFooter.test.tsx` |
| The Usage panel's Decision models line (Not metered for local requests, Unavailable for an unpriced Baseten model), a decision model's row, and Home leaving decision models out of Favorite model and ranking them last in the Models tab | `components/settings/usage/UsagePanel.test.tsx`, `utils/usageStats.test.ts` |
| A local decision model leaves a chat's cost exact and reads "Not metered" in its breakdown; an unpriced Baseten one leaves it partial | `components/bottom_menu/CostTracker.test.ts`, `components/bottom_menu/CostTracker.render.test.tsx`, `components/bottom_menu/ModelBreakdownTable.test.tsx` |

Run them with
`cd ui/desktop && npx vitest run src/utils/sessionDecisions.test.ts src/hooks/useSessionDecisions.test.tsx src/components/decisions src/components/summary src/components/sessions src/components/ChatInput.decisionChatGauge.test.tsx src/components/bottom_menu/ComposerFooter.test.tsx src/components/settings/usage src/utils/usageStats.test.ts`.
Set `DECISIONS_SCREENSHOT_DIR` to a folder to have the two browser tests save a picture of the
section and the view.

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
