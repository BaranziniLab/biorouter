# The Chat summary rail

> **What this is.** Where the Chat summary appears in a chat: docked as a rail beside the
> conversation when the pane has room for both at the full 760px measure, and as a popover from
> the header button when it does not. Covers the width rule (rung 0 of the yield ladder), what
> decides whether the rail is shown, the header button's two modes, the content, the motion, and
> the tests that hold each part.
> **Status:** Current. Built in the Codex simplicity redesign (WS-SUMMARY, October 2026); the
> content was made calmer after the live QA of 2026-11-10 (collapsed sections, the subagent
> peek), then divided into labelled sections with the agent grid on 2026-11-11.
> **Audience:** developers working on the chat pane's layout, the summary, the artifact preview
> or the yield ladder.

The Chat summary holds a chat's to-do list, its subagents and phase plan, its decision-model
activity, the chat's own figures (model, privacy, tool calls, tokens, artifacts, code delta, in a
This chat section) and two actions (Make workflow and Diagnostics), in labelled sections. It used to live only in a 360px popover
behind a header button. Now, once a chat has had a turn and the window is wide enough, it docks
as a 240px to 280px card to the right of the conversation, under the chat header, and the
conversation moves left by half the rail's width. The column never gets narrower than 760px to
make room: where both do not fit, the rail folds away and the same content opens from the
header button.

## The width rule (rung 0)

The decision is a pure function, `summaryRailFit` in `components/Layout/yieldLadder.ts`. Rung 0
yields first: the rail goes before the sidebar, the preview or a split give up anything.

| Quantity | Value | Where it comes from |
|---|---|---|
| Column chrome | 56px | The transcript's ScrollArea root `px-1` (2 × 4) plus its viewport `paddingX={6}` (2 × 24), both in `BaseChat.tsx`. The composer bar takes the same 28px a side (`paddingInline: CHAT_COLUMN_CHROME / 2`), and a subagent's line the same (a `px-1` wrapper around its own `px-6`), so all three share the column's left edge below the full measure too |
| Full-measure cell | 816px | 760 (`--measure-chat`) + 56 |
| Rail width | 240px to 280px | `clamp(available − 816, 240, 280)` |
| Show threshold | 1056px | 816 + 240 |
| Return threshold | 1068px | 1056 + the same 12px return buffer as rung 2, applied after a measured hide |

`available` is the pane's width (the split box, measured with a `ResizeObserver`) minus the side
preview's width. A stacked preview hides the rail. An unmeasured pane keeps the previous answer,
so a first paint never flashes the rail away and back.

What that means for a window:

| Window | Sidebar | Pane | Rail |
|---|---|---|---|
| 1100 | Overlay (below 1120) | 1100 | 280 |
| 1280 | Open, 288 | 992 | None; the button opens the popover |
| 1344 | Open, 288 | 1056 | 240 |
| 1440 | Open, 288 | 1152 | 280 |
| 1920 | Open, 288 | 1632 | 280 |
| 1440 with a side preview | Open, 288 | 1152 | None |
| 2264 with a side preview | Open, 288 | 1976 | 240, beside a 920px preview |

Narrowing a 1300px window to 1100px makes the rail appear, because below 1120px the sidebar turns
into an overlay and the pane gets the whole window. Beside a default-width side preview the rail
needs a 1976px pane, so on most screens opening a preview hides the rail and the header button
reaches the summary instead.

## When the rail is shown

`shown = preference is open AND the chat is active AND it fits AND this is not a phone-width
browser` (`summaryRailVisible`).

- **Preference** is per viewer, not per chat: `biorouter:summary-rail` in local storage, `open` by
  default (`Layout/summaryRailPreference.ts`). Every mounted chat reads one store through
  `useSyncExternalStore`, and the `storage` event carries a change to a second window. A person
  who closed the rail is never overridden, and a new to-do item does not reopen it.
- **Active** means the chat is not the empty new-chat screen and has a message or a running turn
  (`summaryRailChat`). An empty chat shows no rail; the rail arrives with the first turn. A new
  chat's first turn counts from the commit that starts creating the chat, which is the commit in
  which the composer leaves the empty state; the message and the running stream arrive about
  300ms later. Waiting for them made the composer move twice: to the centre, then 140px left.
- **A new chat's first message is on screen from the send** (`components/summary/useFirstSend.ts`).
  A message typed into a chat that exists is in the transcript at once, but a chat's first
  message waits for the chat to be created (`/agent/start`), bound to its model, navigated to
  and loaded before its store takes it. Measured at 1440, the text left the composer at 81ms and
  reached the transcript at 753ms, so for 0.67s it was in neither place (vision V2-R8-2). From
  the commit that starts creating the chat, the transcript draws the text as sent through
  `FirstSendTranscript`, which is the transcript's own `ProgressiveMessageList`, so the stored row
  that replaces it is the same row in the same place; that list mounts with the row, so it is
  drawn at rest and does not play its entrance again. Images are left out of that row: only the
  store's `createUserMessage` reads their bytes. The send ends when the store holds the message,
  and also when the chat could not be created or bound, or the first submit is refused (each
  hands the text back to the composer); a turn error or a failed load withholds the row.
  The composer says "Ask a follow-up" from the same commit: `composerFollowsUp` (same file) is
  `ChatInput`'s `followUp`, true for a stored message, the first send's row or a loading chat's
  row count. It words the placeholder and nothing else; the first send's row never reaches
  `messagesLength`, which arms the turn tier probe. Before it, the placeholder followed the
  store's count and asked "Ask Biorouter anything…" under the drawn message for 0.7s.
  The send also moves the composer from the empty state's centred slot to the bar under the
  transcript, so React mounts a new `ChatInput`. `composer/composerHandOff.ts` hands the chips'
  settled state to it under `reasoningDraftKey`, which `BaseChat` derives from the tab alone, so
  the key is the same before and after the chat exists.
- **A chat whose transcript is loading** is active when its row says it has messages
  (`knownToHaveMessages`). While a chat loads, `BaseChat` holds no `session`, so it reads the
  row from the session listing, or the row alone (`GET /sessions/{id}?metadata_only=true` with
  the user-action headers, sent in the same moment as the composer's read of it, so the two are
  one request): `components/summary/useLoadingChatRow.ts`. Without it, every first open dropped
  the rail for the load, re-centred the column 140px and brought it back, and the composer said
  "Ask Biorouter anything…" with the new-chat model on its chip (vision V2-R5-6). The same row
  gives the composer "Ask a follow-up", the chip the chat's own model (through
  `usePinnedModel`'s one rule), and the context ring the chat's own figure. The rail's values stay
  empty until the transcript lands, and the to-do list is read only then.
- **Until the row answers, the column paints nothing** (`components/summary/useLoadingChat.ts`).
  After a reload the listing of every chat is still on its way and the row read waits behind the
  app's other requests, so a chat opened from the sidebar had no row for 140 to 430ms and was
  drawn as a new chat (vision V2-R5-6, round 7). A guess is wrong either way: a chat created from
  Workflows has no messages and no message on its way, and neither listing ever holds an empty
  chat. So the loading chat's look is `chat` (its row has messages), `new` (its row is empty, or a
  message or attachments are on their way: Home's composer, a launcher message), or `pending`. A
  pending chat holds the composer bar and the subagent line at opacity 0 with no rail, then
  appears once, in place and at rest. Opacity alone, never `inert` or `aria-hidden`: the composer
  keeps the focus it takes on mount and stays reachable (spec principle 12). Loading counts from
  the commit before the store starts the load (`awaitingFirstRead`): a store made for a chat this
  window has not opened starts at `Idle` with no session, and that commit used to be the empty
  state, greeting and all.
- **Timing** (`hooks/useSummaryRail.ts`): the pane is measured synchronously on mount, so a tab
  switch or a reload shows the rail at once and at rest. A narrowing hides it at once; a widening
  shows it only after the width has held still for 160ms, so a sidebar sweep does not make the
  column jump halfway. The rail never resizes the window.
- **The observer must survive StrictMode.** The renderer runs in `<StrictMode>`, which unmounts
  every effect once and mounts it again. The teardown therefore forgets the element it observed
  as well as disconnecting, so the attach effect observes it again. A teardown that only
  disconnected left the rail with no observer at all: it kept its mount-time width while the
  window, the sidebar and the preview moved, and never came back after a hide.
- **First turn or history load**: both turn a chat active, but only a turn that starts in this
  mount enters the rail. `BaseChat` passes the stream's phase (`loading`, `running` or `idle`),
  and a chat that becomes active while loading, or whose transcript lands with no turn running
  (a tab opened from History, a restart, the cache), appears at rest like a tab switch.

## The header button

One `PanelRight` button in the chat header's actions
(`components/summary/SummaryHeaderButton.tsx`), with two modes:

| Rung 0 | The button | Accessible state |
|---|---|---|
| Fits | Shows and hides the docked rail (the preference) | `aria-expanded`, `aria-controls` naming the rail; pressed fill `--background-fill-strong`, the band's fill step; tooltip "Hide summary" or "Show summary" |
| Does not fit | Opens the summary in a 280px popover anchored to the button | `aria-haspopup="dialog"`, `aria-expanded`, `aria-controls` naming the popover; tooltip "Chat summary" |

The popover is anchored with `PopoverAnchor` rather than triggered, because a Radix trigger would
stamp its own `aria-expanded` and open the popover in rail mode too. If the window widens while
the popover is open, the rail replaces it and the popover closes, and so does a popover left open
when the button turns into the rail's (the rail was closed, then the window widened).

The anchor is the band's bottom line at the button's right edge, not the button itself, and the
popover hangs 4px below that line and stays 8px off the window's edges (vision V2-R4-2). The
button is centred in the 44px band, so the primitive's 6px offset from the button put the
popover's top edge on the band's hairline, and with no collision padding a long list ran 1px past
the window's bottom edge, cutting its corners. The padding also comes off the available height,
so a long list stops 8px above the bottom edge and scrolls inside.

A toast does not cover the docked card or the popover (vision V2-R7-4). The toast stack finds them
by their class names, `.br-summary-card` (`ChatSummaryRail.tsx`) and `.br-summary-popover`
(`SummaryHeaderButton.tsx`), and stands 16px to their left; the code is WS-MOTION's
`components/alerts/toastClearance.ts`. Renaming either class, or adding another summary surface
that opens into the window's top-right corner, means updating `TOAST_OBSTACLES` in that file.

The pressed fill fades only when the person pressed the button, opened or closed its popover, or
the first turn brought the rail in. When geometry flips it (a resize across a seam, the sidebar,
a preview), the button carries `data-still` for that one change and the fill snaps, as the grid
and the card do; the marker is cleared two frames later, which starts no transition because
nothing changes then. Before this, dragging the window across 1344px ran the button's colour
transitions twice while everything else in the chat snapped.

While the rail is hidden and a to-do step is running, the button carries a 6px coral dot and its
name becomes "Chat summary, 2 of 5 to do items complete". Subagents that are queued or running
with the rail hidden light the same dot and add their count to the name: "Chat summary, 3
subagents running", or "Chat summary, 2 of 5 to do items complete, 1 subagent running" when both
apply. The dot's 2px ring is the ground under
it: the band at rest, the pressed fill while the popover is open. The terminal button beside it
presses on the same fill.

## The content

`components/ChatSummary.tsx` draws the same content in the rail and in the popover. In the rail
it sits on a card with a 12px radius and the subtle fill, `--background-fill`, with no outline:
fills before lines (owner message 13, R-27; `design.md` 9.11). The card keeps its 1px border,
transparent, so nothing moves and forced colours still draw its edge, and it is a
`.br-fill-surface`, so a filled control placed inside it steps back to the page ground. The
popover is a floating surface and keeps the shared popover hairline and shadow. Either way there
is a scrolling body over a pinned footer, and the parts are told apart by space, never by a
hairline: the sections and the footer sit 12px apart (22px between their text,
against 10px between rows). Where the list overflows, the edge that hides rows fades over 16px:
the bottom while there is more below, the top once the list has scrolled. The fade follows the
body's own scroll position (a CSS scroll timeline), so it needs no listener and moves nothing,
and a focused body drops it so its focus edge is drawn whole. Two type sizes only, 13px for rows
and 12px for labels.

The body is divided into sections a person can scan (owner request, 2026-11-11, modelled on
Claude Code's Background tasks panel): **To do**, **Plan** (or **Agents** when the chat has
subagents and no plan), **Decisions** and **This chat**. Every section opens on the same heading
(`summary/SummarySectionHeading.tsx`): a chevron in the rows' 14px glyph column that turns 90°
when open, the label in the app's one caps style (12px uppercase at +0.04em, muted, the
`text-caps` tracking), and the count or status on the right in muted figures. The label sits on
the rows' text edge, so a section reads as the heading of what is under it, and nothing inside a
section (a 13px phase line in sentence case, a row) can be mistaken for one. A section collapses
on its heading; once the person toggles one, their choice holds for as long as the summary stays
mounted for the chat.

- **To do**, when there are items: the heading with a tabular "2 of 5" count (announced as "2 of
  5 complete" through `aria-live`), a 4px progress bar that stays under it open or closed, then
  28px rows. It starts collapsed when the chat has a phase plan, because the Plan section is then
  what the person opened the summary for, and open otherwise. Each status has its own
  glyph shape, an open circle (pending), a dashed circle in the accent ink (running), a check
  (complete) and an alert (blocked), so status never depends on colour. The status word is read
  to assistive technology ahead of the task text. There is no "no tasks" sentence; a failed
  refresh shows one line, "To do could not refresh.", with Retry, whose label ends on the
  values' right edge. On the card the 4px track rests on `--background-fill-strong`: the
  primitive's own track is `--background-muted`, which is the fill itself and would vanish.
- **Plan** or **Agents** (`components/summary/SummarySubagents.tsx`, the notation in
  `summary/agentGrid.ts` and `summary/AgentMark.tsx`, styled by `summary/summaryAgents.css` and
  `summary/summaryAgentGrid.css`), when the chat has started any subagent or declared a phase
  plan. Subagents run in the background by default, so this section is where a person finds them.
  Settings > App > General > "Open subagent tabs automatically"
  (`BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS`, off by default, read by the daemon) brings back a
  tab for every new subagent; a spawn that asks for a tab gets one either way. "Never open tabs
  automatically" wins over both on the daemon, so with it on no subagent tab opens at spawn; the
  switch is not disabled for it, and its help says so.
  - **The notation.** One small mark per piece of work. Its shape says what kind of model did
    it: a **square** for a language-model agent, a **circle** for decision-model work (a
    subagent whose own model is a decision model, a phase gate, and every Decisions row). Its
    fill says where it stands, so status never depends on colour alone: hollow (waiting to
    start), hollow with a core (running), half full (stalled, or waiting for the person), solid
    (finished), solid with a cross cut out (failed), hollow with a bar (no outcome: status
    unknown, skipped, off). Its ink comes from existing semantic tokens: running
    `--text-info` (the info blue), waiting `--text-warning`, failed `--text-danger`, finished a
    calm sage (`--text-success` greyed 60:40 with `--text-muted`, then set at 70% over
    `--background-fill`), and the rest `--text-muted`. Every ink clears 3:1 against the card's
    fill in both modes and every family (the sage is the lowest, 3.3:1 in Alma Mater light). A
    running child's core breathes (an opacity pulse over 1.6s), never under reduced motion. In
    forced colours every mark is the system ink (running the highlight ink) and the shapes still
    tell the statuses apart. The marks are SVG in `currentColor`; the cross is a hole cut out of
    the solid shape, so a failed mark reads the same on the card, a band or a popover.
  - **Which status is which mark** (`AGENT_STATUS_MARKS`): waiting to start is hollow; running
    and stopping are the core; completed is solid; a run that needs an answer, was stopped
    before it finished ("Stopped") or was interrupted is half full, as is an active child with
    no recorded activity for 180 seconds ("Quiet for 4m", never "stalled"; the threshold is the
    daemon's `QUIET_AFTER_SECONDS` in `orchestration_plan.rs`); failed is the cut cross; status
    unknown is the bar. The precise word ("Stopped", "Interrupted", "Needs an answer") is in the
    mark's name and its card.
  - **The size scales with the count** (`agentGridSize`). With four agents or fewer in the chat,
    every agent is a two-line row led by a 12px mark: the role the parent gave (else the session
    name) with the run time on the right, then in 12px muted ink the model's short name and what
    it is doing ("Working…", "Waiting to start", "Quiet for 4m", "Stopping…") or, once it ended,
    its result line (else its status word). With five to 99, each group draws a grid of 10px
    squares 4px apart, one square per agent in spawn order, so a square never moves while the
    group runs; under it, the first five agents as 28px rows (the mark, the name, the run time
    or "quiet 4m"), then "Show N more" for the rest and "Show fewer" back. From 100 the squares
    are 7px, 2px apart, in a dense block. The notation is the same at every size.
  - **The first five** (`inHeadOrder`): whoever needs a look first (waiting for the person or
    stalled, then failed, then running, each in spawn order), then everything else by when it
    last did something, newest first. A row whose card is kept, or that holds focus, stays drawn
    past the five, so a poll that ranks it lower never unmounts it under the person.
  - **With a phase plan**, the section is **Plan**: its heading counts the phases done ("1 of
    4", announced "1 of 4 phases complete"; its accessible name carries the plan's title), and
    each phase, in plan order, is a **band**: the control ground the fill rule gives it (the
    page ground inside the rail's filled card, the fill in the popover), with the element
    radius, 4px from the next. A band holds the phase's line (the chevron, the title, which wraps
    to a second line before it is cut, "done of total" on the right, and the gate's mark), then
    its grid, its first rows and "Show N more". A phase that used no subagents shows where it
    stands instead of "0 of 0" ("Pending", "Skipped"). The phase in progress (or blocked) is
    open, and in a plan of more than three phases the others start folded. A folded phase keeps
    its grid while anything in it is not finished, so its live state is still in view; a folded
    phase whose every agent finished is its line alone. Under the line, a muted line appears
    only when an earlier gate holds the phase ("Waits for Build's gate"). Its failed and blocked
    subagents are its grid's crosses and half marks, and words in its accessible name ("1
    failed"). Subagents in no phase the plan knows are a last band, **Other agents**.
  - **Without a plan**, the section is **Agents**, its heading "2 running · 4 of 10" (announced
    "4 of 10 done"), holding the whole roster as one group with no band.
  - **Phase gates** are decision-model work, so a gate is a circle at the end of its phase's
    line, filled by its state: hollow while not asked yet, solid once passed (or passed by the
    person), the cut cross when not passed or blocked, half full when it needs the person, the
    bar when skipped. Its word ("Needs you", "Not passed", "Blocked") is drawn beside it, in the
    circle's ink, only when the gate wants a look; otherwise the mark stands alone. Its tooltip
    and accessible name are the full sentence with the probability named ("Gate not passed ·
    P(yes) 0.62 · threshold 0.80 · attempt 2 of 3"). It opens the phase gate dialog, where the
    person passes or skips the phase, and its `⋯` menu holds Try again, Edit gate, Decision
    history and Remove gate ([the phase gate dialog](decision-models.md#the-phase-gate-dialog)).
    A gate is never a row of its own, here or in the Decisions section. An open phase with no
    gate shows a quiet 20px plus in the gate's place on hover and on focus, named "Add gate,
    Build" with the tooltip "Add gate", which opens the dialog on its editor; it is always in the
    arrow-key order, so the keyboard finds it the way a pointer does
    ([Decision gates](../agent-loop/orchestration-phases.md#decision-gates)).
  - **The card: hover peeks, click pins**
    ([Hover and click](shared-controls.md#hover-and-click)). A square and a row are two triggers
    of the same card (`useFooterCard`, inside one `FooterCardGroup`), so a hover previews it after
    300ms and a 150ms grace closes it, a Tab onto the roster previews the focused row's, and a
    click, tap, Enter or Space keeps it: a row is marked `data-pinned` and painted
    `--background-fill-strong`, a square is ringed in the default ink, and Enter or Space moves
    focus into the card. Escape (focus back on the trigger), a press outside or a second click
    closes it, and one card is open at a time. The card is the light popover card, to the left
    of its trigger (`summary/SubagentPeek.tsx`): the title with the subagent glyph; under it the
    model and where it runs (`sonnet-4-5 · Public`, the tier while privacy tiers are on, or for a
    decision model `Decision model · tev1:0.8b · this computer`) and, when any of the chat's
    monitors also watches its subagents (`applyToSubagents`, while on), "Watched by Scope
    drift"; the task when the title is a role; the result under the status word and run time
    ("Completed · 3s"), or "Working…", or "It left no result."; an **Open in a tab** action; and
    a closed Details with the extensions by display name (at most 8, then "and N more"), the
    skills, the knowledge bases by name, and what it used ("9.8k tokens · 0 tool calls · 3
    messages"). No provider key, dated model slug, raw extension key or knowledge base id
    appears. The card is built the first time it opens.
  - **A click never opens the tab.** "Open in a tab" in the card does: it opens the subagent as
    a tab titled by its role and makes it the active one (the tab strip's `openTab`, which
    activates a tab already open; from outside `/pair`, a navigation to
    `/pair?resumeSessionId=`). A run row in History still opens on click.
  - **Keyboard**: the roster is one tab stop (`ui/useRovingRows.ts`), on the first running row;
    the arrow keys, Home and End move between phase lines, gates, rows and "Show N more", and
    open nothing. A grid is ONE stop in that order, its current square carrying the group's
    `data-row`: Up and Down leave it for the line above and the row below, Left and Right
    (mirrored right to left) walk its squares, each a button named in words ("Running, Reader 2,
    sonnet-4-5, 4m").
  - The roster keeps showing a chat's last list while a re-read is out or has failed after a
    remount (`summary/rosterMemory.ts`), so a tab switch does not empty the Plan section. A
    failed refresh keeps the last list and shows "Subagents could not refresh." with Retry; a
    daemon without the route shows nothing.
- **Decisions** (`components/summary/SummaryDecisions.tsx`), when the chat has decision calls or
  monitors: one line by default ("1 fired", "None fired", "10 checks") that opens on its own only
  when a monitor fired or a check errored, then one 28px row per monitor that exists now and one
  for the agent's `decide` calls, each led by a circle of the notation (decision-model work) and
  ending in a word (Fired, Quiet, Not checked yet, Off, Error) or a count, with no probabilities
  and no bars. A monitor that fired is half full in the warning ink, one that checked and stayed
  quiet is solid (never the failed cross: it did nothing wrong), one not checked yet is hollow,
  one switched off has the bar, and an error is the cut cross; `decide` batches and a decision
  chat's answers are solid (`DECISION_STATE_MARKS`). The section stays aggregate: never one row
  per call. A Monitors icon button sits on its heading line. The rules are in
  [the Decisions section](decision-models.md#the-decisions-section-of-the-chat-summary).
- **This chat**, closed by default, whose heading carries the token figure alone ("392k tokens",
  also its accessible description; nothing while Tokens is not known). Open, a `dl` of 28px
  rows: Model (the chat's model by its short name, as the model chip names it), Privacy
  (Private or Public, while privacy tiers are on), Tool calls, Tokens (with an InfoTip saying
  what is counted), Artifacts, and Code as `+N −M` in success and danger ink, read as words. The
  chat's cost is not repeated here: the composer footer's cost card, right under the summary,
  is its one place. A count of
  zero is muted, so a chat that touched no code shows no red. Code is the lines this chat's
  successful `text_editor` edits added and removed (`summary/codeDelta.ts`), never anything read
  off text: it used to match diff-shaped strings anywhere in the transcript, so a chat whose
  script wrote a file read `+0 −0` and a chat that only read code read `+0 −9` (final check,
  2026-10-12). A direct call is measured from its own arguments; a Code Execution script carries
  `biorouter/code-delta`, which the daemon measures from each sub-call's full arguments
  (`agents/code_delta.rs`), because a sub-call's record keeps its arguments cut at 2 KB and a
  record past the meta budget is dropped. A file written by a shell command is not counted:
  nothing says how many lines it changed. A figure that is not known yet is
  an empty value in a row that keeps its label and its 28px, never a stand-in the load then
  replaces: all four while the transcript loads, and Tokens until the chat's usage ledger has
  answered (`useCostTracking`'s `ledgerSettled`, `summaryTokensValue`). Before that, the
  accumulated counters stood in, and a ledger row with no certified total then turned "2.2M"
  into "N/A" on every open (vision V2-R5-5). The composer footer's cost waits for the same
  answer (`footerCostInputs`). A chat created here for its first message is new and empty, so
  from the send through its load it shows its zeros (Tool calls 0, Artifacts 0, Code +0 −0),
  and its Tokens stay empty until its own ledger answers, which needs its session
  (`summaryFigures`). Before that, the rail read "Tokens 0" from the counters of no chat, then
  nothing for the load, then "N/A" (vision V2-R8-2).
- **Footer**: Make workflow (Workflow when the chat has one) and Diagnostics as full-width ghost
  rows.
- **In a decision chat** (its model is a decision model, `isDecisionProvider`, the predicate the
  composer uses), the summary offers only what such a chat has: This chat holds its model and
  Tokens only, the
  footer has no Make workflow, and the Decisions section no Monitors button. A decision model
  calls no tools and writes no files, so Tool calls, Artifacts and Code would only ever read
  zero.
- The scrolling body is a focus region with a tab stop (`.biorouter-focus-region`), so a
  keyboard user can scroll it.

The to-do list is fetched while the popover or the rail is shown, and while a turn runs so the
button's dot stays current (`hooks/useSessionTodos.ts`, with the user-action headers).

The usage ledger behind Tokens and the footer's cost is read again whenever the chat's counters
move, on its row or live (`hooks/useCostTracking.ts`). The row's counters move on an open or a
resume. In a live chat only the stream's `tokenState` moves, which `BaseChat.tsx` passes as
`liveTokens`: once per provider call that records usage, and at the turn's end, each after the
daemon has written the turn's ledger row. It is never read per token: every streamed frame
carries a copy of the last counters (BR-52), and the counters are compared by value, so a copy
asks for nothing. Keyed on the row alone, the ledger was read once on open, because in a live chat
the row is the one the chat was opened with, so the footer's cost and its By model list stayed
where they were until a reload. A refresh keeps the answer it has until a new one lands, and so
does a refresh that fails; only a chat's first read settles with no rows. A read that a newer one
has superseded never lands. The footer's cost card lists every token bucket the ledger recorded
(input, output, cache read and cache write), so on the ledger its figures add up to the Tokens
value here and to Settings > App > Usage.

The subagents come from `GET /sessions/{session_id}/subagents` (`hooks/useSessionSubagents.ts`,
parsed by `utils/sessionSubagents.ts`), read with the user-action headers because the daemon
omits a private child from a caller without the proof. The hook reads on the to-do list's gate
(shown, or a turn running) and again whenever a roster tool call answers: a spawn, the plan tool,
a watch or a close (`subagentRosterRevision`). It polls every 2 seconds while any child is
queued, running or stopping, or while the turn runs in a chat that has subagents, and stops when
none is active. That poll continues after the rail is hidden, so a background child that outlives
its parent's turn keeps the button's dot honest until it ends. A poll that would draw nothing new
sets no state, so it does not re-render the chat. While the summary is hidden only the children and
their states count for that, because an active child's run time, quiet time and token count change
on every read; showing the summary again puts the newest answer up at once. The parser never drops
a row for a status this build does not know: it reads as unknown.

A row the person is focused on, or whose card is kept, stays among a group's drawn rows whatever
a poll does to its rank, so a child that finishes while the person reads its card is not ranked
out of the first five under them. A row that a redraw does remount (a phase folded and unfolded
around it) gets focus back: `SummarySubagents` puts focus on the row's new button after the
render, but only when the focused element was removed, never when the person moved focus
elsewhere. A grid's squares never move, because they keep spawn order.

The rail and the popover behave the same in a browser served by `biorouter serve`: opening a
subagent there opens its tab read-only, as the existing subagent tab code does.

## Motion

The grid snaps; only `transform` and `opacity` animate.

| Cause | Grid | Transcript and composer | Rail card |
|---|---|---|---|
| The person opens it, or the first turn starts | Snaps | Slide from the old position over 300ms | Fades in and slides 12px over 175ms, after 60ms |
| The person closes it | Holds until the card has left | Slide back over 175ms | Fades out and slides 8px over 125ms |
| Resize, sidebar, preview, split, tab switch or reload | Snaps | None | None (`data-rail-still`) |
| Reduced motion | Snaps | None | A 125ms fade only |

The conversation's slide is a FLIP on the grid items `[data-preview-area='transcript']` and
`[data-preview-area='composer']` (`hooks/useColumnGlide.ts`), never on the column inside the
scroller. It is skipped while the window or sidebar is being resized, under reduced motion, for
a move under 8px, and when the window grew to seat a preview. On the first turn the composer
already moves from the centred empty state to its place, and that end place includes the rail,
so the glide moves only the transcript and leaves the composer to its own animation. The same
hook slides the conversation when a side preview opens or closes, and the card the person
clicked stays in view while the column narrows.

A toggle, a preview opening or closing, or the rail's exit re-renders the grid and the rail and
nothing in the transcript. `BaseChat` holds the message list as one memoised element whose
dependencies are only what the list reads, so React skips it on those commits; a long chat used
to re-render every row first, and the first frame came 200 to 250ms after the click. The panel
rebuilds its `openArtifact` whenever the shown artifact changes, so the list is handed a stable
wrapper that reads the latest one when a card is clicked.

New to-do rows enter with `.br-enter`, 30ms apart, at most five at once; rows that arrive with
the list (an open, a reload, a tab switch) do not animate. A status change crossfades the glyph
over 125ms. The list keeps the running step in view, scrolling smoothly unless motion is reduced
or the pointer is over the summary.

## How it is built

- **Grid**: `components/summary/summaryRail.css`, imported statically through
  `ChatSummaryRail.tsx` so the rules are in the document before the first chat renders. It uses
  the same flatten-and-place grid as rung 2: the split box becomes one grid, its column and body
  are flattened with `display: contents`, and each piece is placed by its `data-preview-area`.
  The rail-only template is keyed on `[data-summary-rail]:not([data-preview-layout])`; beside a
  side preview the rail takes a column between the conversation and the preview. A subagent's
  line (`subheader`) sits on the transcript's track, not across the rail, so its 760px box is
  centred on the column it belongs to; the rail starts beside it, directly under the header, as
  in every other chat. The rail cell is padded 12px except on the transcript's side.
- **State**: `hooks/useSummaryRail.ts` (measurement, timing, entrance and exit),
  `Layout/summaryRailPreference.ts` (the viewer's choice).
- **Subagents**: `hooks/useSessionSubagents.ts` (reads and polls the roster),
  `utils/sessionSubagents.ts` (the wire types, the parser, the counts, the labels and
  `openSubagentTab`), `components/summary/agentGrid.ts` (the notation: status to mark, decision
  work to circle, the size tiers, the head order, the groups by phase; no React, no DOM),
  `components/summary/AgentMark.tsx` (one mark), `components/summary/SummarySubagents.tsx` (the
  Plan or Agents section, the bands, grids and rows), `components/summary/SubagentPeek.tsx` (the
  card) and `components/summary/rosterMemory.ts` (the last list, kept across a remount). The
  card's "Watched by" comes from the chat's own decision settings, which `ChatSummary` already
  holds for the Decisions section (`subagentWatchers`), so no route was added. BaseChat calls the hook right after `useSessionTodos`, hands it to both surfaces in
  `summaryProps.subagents` with the tab opener, and gives the header button the running count.
  The hook calls the generated `getSessionSubagents`, and parses its answer itself rather than
  trusting the generated type: the parser's types mirror `subagent_roster.rs` and
  `orchestration_plan.rs`, and its test reads both files so a variant added there fails here.
- **Wiring**: `BaseChat.tsx` spreads `summaryRail.splitPaneProps` beside the artifact panel's,
  merging the two `style` objects after both spreads, and renders one `ChatSummaryRail`.
- **Following the newest line**: `components/summary/useTranscriptFollow.ts` gives the
  transcript's `ScrollArea` its `anchorBottomOnResize` (a stacked preview, or a reader following
  the newest line) and `onScrollChange`, and puts a reader who was at the end back at the end
  after a width change. Crew's timeline uses the same recipe; this one asks "at the end" rather
  than "following" for a width change, so the rail or a side preview changing the column's width
  never pulls a reader who is up to 200px above the end down to it (vision V2-R6-1).
- **When the list settles**: `useTranscriptSettle`, in the same file, is BaseChat's
  `onRenderingComplete`. It goes back to the end only while the reader follows (the first settle
  that has messages lands at once, later ones scroll smoothly) and leaves a reader who has
  scrolled up where they are, the first settle included. A live chat opens at its end and adds
  older batches above it (V2-R7-2), so a first-settle force would only pull a reader who scrolled
  up while those batches loaded back to the end (WS-TRANSCRIPT-1).
- **A row the person did not cause**: `components/summary/useLateRowHold.ts`. A chat whose model
  cannot be restored learns it from the agent load, the second `/agent/resume` an open makes,
  kept off the paint path on purpose; on a long chat that lands seconds after the transcript
  (measured on an 82-figure chat: drawn at 6.0 s, "Model could not be prepared" at 12.5 s), and
  following it moved every card on screen 62px up, under the pointer (vision V2-R9-2). It cannot
  be known before the transcript is drawn without holding every open for the agent load, so the
  open's own `session`-scope error, landing at least `LATE_ROW_GRACE_MS` (250ms) after the
  transcript was first drawn and before any turn ran, is added below the fold: the follow hook's
  `holdForLateRow` (decided in the layout phase of the commit that draws it, and only when the
  row landed out of view) stops `ScrollArea`'s growth follow (`followGrowth`) and the settle
  until the reader reaches the end or a send takes them there. A row inside the grace, in the
  transcript's first frame, after a turn, or of another scope is followed as before.
  BaseChat passes `followGrowth={transcriptFollow.followGrowth}` to the transcript's
  `ScrollArea` (the prop came from WS-PRIMITIVES, `WS-SUMMARY-3`); without it the hold covers
  the settle only and the growth follow still moves every card up.

## Where it is pinned

| Test | Holds |
|---|---|
| `components/Layout/yieldLadder.test.ts` | Each seam one pixel apart (1055/1056, 1067/1068), the width steps (1056 → 240, 1076 → 260, 1096 → 280), the full measure across every pane width, the side-preview subtraction (1975/1976 at the default preview), a stacked preview, an unmeasured pane, and the window table |
| `hooks/useSummaryRail.test.tsx` | No hidden-then-shown flash on mount, a narrowing hides at once, a widening shows after 160ms, a mount does not animate, the first turn enters (from the commit that creates a new chat, once) but a loaded history (fetched, cached or rejoining a turn) does not, a loading chat known to have messages holds the rail at rest from its first frame with no glide when it lands, the toggle's entrance, exit and glide, and one live observer under StrictMode |
| `components/summary/ChatSummaryRail.test.tsx` | The card mounts entered when geometry showed it, plays its entrance once, ignores its children's animations, reports its exit (or its cancelled exit) and re-arms the entrance when reopened mid-exit |
| `Layout/summaryRailPreference.test.ts` | The per-viewer store, its default and the `storage` event |
| `hooks/useColumnGlide.test.ts` | No glide while resizing, under reduced motion or under 8px; the composer left to its own FLIP; the scroll anchor |
| `hooks/useSessionTodos.test.tsx` | Fetches with the user proof only while shown, refreshes after to-do writes, keeps the last list with an error and Retry |
| `components/summary/useLoadingChatRow.test.ts` | A loading chat's row from a warm listing in the first render, from the row read (with the proof, preferred to the listing), from a listing that arrives later; nothing for another chat, a failed read, a chat that is not loading or a new chat |
| `components/summary/useLoadingChat.test.tsx` | The loading look (`chat`, `new`, `pending`) and the commit before the load; a cold first open paints nothing, then the chat with its rail at rest and its own model, and never the new-chat placeholder or default model; a warm listing gives the chat in the first commit; Home's cargo keeps the new-chat composer; a Workflows chat never draws the rail; nothing pending once loaded, for a new chat or after a failed load |
| `hooks/useCostTracking.ledger.test.tsx` | `ledgerSettled` false with no rows while the usage request is pending, settled on rows, on none and on a failure, never one chat's ledger for another's, and a refresh of the same chat keeps its answer. A live chat's counters refresh it: a re-read after each turn with the chat's row unchanged and nothing remounted, nothing asked by frames that only copy the counters it has, no superseded read landing, and a failed refresh keeping the answer while a failed first read settles |
| `components/BaseChat.liveCost.test.tsx` | On the real stream store (`useChatStream`), the usage hook, `footerCostInputs` and the footer `CostTracker`, wired as BaseChat wires them: By model after a new chat's first reply and the new total after every turn, two provider calls in one turn included, with no reload and one mount; a chat opened with a total moving on from it; and one ledger read per change of the counters, never one per streamed token, each with the user proof |
| `components/BaseChat.costLedger.test.tsx` | On the real hook and the real footer `CostTracker`: no Tokens value and no cost while the ledger is pending, then exactly one reading ("N/A" and no cost for an uncertified row; "2.2M" and the priced counters for a ledger with no rows) |
| `components/ChatSummary.test.tsx` | The content, the status words, the code delta in words and its muted zeros, empty values (and `aria-busy`) for figures not known yet, the statistics in a closed This chat whose heading carries the token figure as its description (none while Tokens is not known), To do collapsed by default with a plan and open without one, a person's close holding, a decision chat's This chat holding Tokens only with no Make workflow, the row entrances (kept through the re-renders that follow), the glyph crossfade, the thin progress bar, the scrolling body |
| `components/summary/codeDelta.test.ts` | The Code figure from edits only: a script's write counted from its record, the daemon's script total preferred over truncated or dropped records (and kept when the script failed later), nothing from diff-shaped text, a grep, a `git show` or a quoted snippet, no failed or unanswered call, an undo paired with its own file's last edit; and `codeDeltaCases.json`, the table the daemon's `agents::code_delta` tests also run, so the two tallies cannot drift. `tests/code_execution_integration.rs` `case31` holds the daemon's total against a real script whose write record is truncated |
| `components/summary/SummaryHeaderButton.test.tsx` | The button's two modes, its tooltip, focus return, the dot with its count (to-do steps and running subagents, alone and together), and which flips of the pressed fill snap (geometry) and which fade (a press, the popover, the first turn) |
| `utils/sessionSubagents.test.ts` | The contract read from the Rust side (every run status, phase kind and phase status, and the quiet threshold), the parser (every field, an unknown status kept as unknown, a row without an id dropped, malformed fields degraded, a plan with an unknown kind or status), the groups and counts, the layout switch at five, phase membership and default disclosure, the labels (short model names, run time, the second line), the roster revision (which tools move it, a script's sub-calls, no move for prose, an unanswered call or a replay) and `openSubagentTab` on and off `/pair` |
| `hooks/useSessionSubagents.test.tsx` | No read while hidden and idle, the read with the user proof once shown, a re-read when a roster tool answers and not for prose, the 2s poll while a child is active (stopping once none is), the poll outliving the hidden rail, the turn's poll only in a chat with subagents, no re-render for a hidden summary's ticking counters (and the newest answer shown at once when it is shown again), the last good data kept on a failed read, a missing route read as no roster, and a superseded read aborted and ignored |
| `components/summary/SummarySubagents.test.tsx` | The gate's words and sentences for every state, its circle filled by state with its word only when it wants a look, the person's decisions and "needs you", the gate opening the gate dialog, the held line, and the compact "Add gate" plus in the gate's place on hover and focus and in the arrow-key order (and none where the gate is not a control); and nothing drawn for an empty roster, rows in a chat of four or fewer (the square and the role or name, the run time, the model and the second line status by status, quiet read as waiting, the muted lock, a decision model's circle), a click keeping the card and not opening the tab, "Open in a tab" opening it with its role, five or more as a grid of 10px squares in spawn order named in words, the first five rows (waiting, failed, running, then the newest), a square that never moves when its child finishes, Show N more and Show fewer, folding the Agents section with its count kept, a square's card, the phases (a band with the title and count and no bar, failures in the name and not under the line, the phase's grid, no "0 of 0" for a phase with no subagents, default disclosure, a folded phase's grid kept while anything in it is unfinished, Other agents inside Plan, a plan with no subagents), the card's content (task, result, Open in a tab, the setup by name under Details, no provider key, dated slug, raw extension key or knowledge base id, a decision model with where it runs) and its lazy build, a Tab previewing with the row at rest, Enter keeping the card and moving focus in and Escape returning it, one tab stop with the arrows moving between rows and opening nothing, the grid as one stop that Left and Right walk, a kept card and its row kept through a poll that ranks the child out of the five, focus kept on a row whose child finishes or a fifth spawn turns into a grid (and left alone once moved out of the list), and a failed refresh with Retry |
| `components/summary/ChatSummary.subagents.test.tsx` | The Agents section between the to-do list and This chat, nothing for it without the prop or with an empty roster, a subagent's card naming the chat's monitors that watch its subagents, and BaseChat's wiring at the source: the hook's place and gate, both surfaces fed from `summaryProps`, the tab opener, and the button's running count only while the rail is hidden |
| `components/summary/summaryAgents.browser.test.tsx` | In Chromium, in the rail's own grid at 240px (a 228px card): 30 subagents with every row shown as 28px rows that scroll inside the card with the footer in view and nothing overflowing sideways (with names long enough to overflow, the instrument); three subagents as 44px two-line rows with one muted second line; a kept card to the left of its row, its top on the row's, the row filled and marked `data-pinned`, and one card open at a time; a 30-subagent phase plan with the same rows and width; a gated phase whose count stays on one line while its long title wraps to two |
| `components/summary/agentGrid.test.ts` | The notation without rendering: the size tiers (rows to 4, a grid to 99, a dense block from 100; 12, 10 and 7px), every run status's mark and a quiet child read as waiting, the pulse only on a running child, a decision-model subagent, a gate and every Decisions row as circles with their fills, the monitors that watch subagents, the head order and the five with kept rows past them, and the groups by phase in plan order with Other last |
| `components/summary/summaryAgentGrid.browser.test.tsx` | In Chromium, in the rail at 240px: one agent as a 44px row led by a 12px square and no grid; eight as one line of 10px squares 4px apart and five 28px rows with 10px marks; 150 as 7px squares in lines 9px apart, all inside the card, nothing sideways; a square's hover card to the left of the rail with its name, model and tier, status, Open in a tab and "Watched by", the square at rest until a click keeps it; each kind drawn by its own parts and ink, gates and Decisions rows as circles. With `SUMMARY_GRID_SHOTS=<dir>` it saves the review screenshots |
| `components/subagent/subagentStatus.test.ts` | Every run status has a glyph and a word, and a stopped run is never the running glyph |
| `components/settings/app/OpenSubagentTabsRow.test.tsx` | "Open subagent tabs automatically": the key `BIOROUTER_WORKSPACE_OPEN_SUBAGENT_TABS`, off by default, written on toggle and rolled back on a failed write, held unpainted until read, a later visit painted from the last answer (the `openSubagentTabs` key of `settings/lastKnown.ts`), and help that names "Never open tabs automatically" as the setting that wins |
| `components/settings/app/AppSettingsSection.test.tsx`, `components/settings/warmSettings.test.ts`, `components/settings/tabSwitchFrames.browser.test.tsx` | The switch named by its label, its place right after "Never open tabs automatically" and before Show costs, held with the other four until read; its key warmed while Settings opens; and in Chromium, painted with its saved value in the first commit and every frame of a visit to the App tab |
| `components/ToolCallWithResponse.subagentCopy.test.ts` | The delegation row naming the role, each `workspace_orchestrate` action's label and its done form, and no row claiming a tab |
| `components/summary/summaryPopover.browser.test.tsx` | In Chromium, with the whole stylesheet, in a band shaped like BaseChat's header: the old wiring landing on the band line and past the window (the instrument); the popover 4px below the band line and 8px above the window's bottom with its body scrolling, at 1300x900, beside a side preview, at a device pixel ratio of 2 and in a 600px window; a short list at its natural height |
| `components/alerts/toastClearance.test.ts` (WS-MOTION's) | The two class names the toast stack steps aside for, each held at the component that renders it; the stack joining the docked card's column (as wide as the card, right edges aligned, 8px under it, or at the rail top over a card that fills the rail), never left of it over the conversation; and 16px left of the popover |
| `styles/summaryRail.test.ts` | The exact grid templates, the `:not([data-preview-layout])` key, unlayered rules, no condition, transition or animation in the geometry, the card's motion, one rail in BaseChat, the `paddingX={6}` and `const contentClassName = 'px-1';` literals the 56px comes from, and the fill: the card's fill and transparent border, no hairline in the content, the track's ground, the scroll-edge fades, the pressed fill and the dot's ring; and for `summaryAgents.css`, the same two sizes, no line (every border a reset), 28px rows, the 12px gaps between sections, a phase's band on the control ground 4px from the next, the caps section label, the restated focus fill, and no `Button` in `SummarySubagents.tsx`; and for `summaryAgentGrid.css`, the status inks from semantic tokens, no type of its own, the system ink in forced colours, and the opacity-only pulse that stops under reduced motion |
| `components/BaseChat.summaryRail.test.ts` | BaseChat's wiring: the button, the to-do hook's order and argument, the popover closing for the rail, the composer bar's and the subagent line's gutters, the first turn counted from the commit that creates a new chat, no titlebar padding under the header band, the message list memoised against everything the rail, the popover and the preview change, a loading chat's row reaching the rail, the chip, the placeholder and the ring, the commit before the load counted as loading (rail and empty state), a pending chat's composer bar and subagent line at opacity 0 and never `inert`, the Tokens value and the footer cost waiting for the ledger, which is keyed on the live counters (`liveTokens: tokenState`) as well as the row, and the transcript's follow hook (its arguments, both props, above the early return, the clicked card's hold), and the list settling through `useTranscriptSettle` with no first-settle force of its own (declared after the follow hook, whose hold it asks), the late row hold's arguments, its `followGrowth` on the transcript's `ScrollArea` and the one `ChatTurnError` drawing the row it decided about, and the first send: started in the commit that starts creating the chat, the chat named once created, ended wherever the composer gets the text back, drawn in the transcript slot, the four figures from `summaryFigures`, and the composer's `followUp` from `composerFollowsUp`, with the row kept out of `messagesLength`, and the composer's `reasoningDraftKey` taken from the tab alone, so the hand-off across the first send's remount finds it |
| `components/summary/useFirstSend.test.tsx` | A new chat's first send against a fake daemon (`/agent/start` after 500ms, a 280ms load, the ledger 50ms after the session), on the real hook, `summaryFigures`, `ChatSummary`, `FirstSendTranscript` and `ProgressiveMessageList`: the old wiring leaving the text off screen and reading Tokens "0", nothing, "N/A" (the instrument); the text in the first commit after the send and in every commit after it; Tokens from empty to its answer once, the counts held at zero; the stored row the same markup as the first-send row, at rest; a placeholder that follows the store asking the open question under the drawn message (the instrument), and "Ask a follow-up" from the commit that draws it. Also the hook's life (begin, created, spent, another chat, a failure, attachments alone), `composerFollowsUp` and `summaryFigures`' cases |
| `components/summary/transcriptSettle.test.ts` | The settle rule: the first settle lands at once while following, nothing for a reader who scrolled up (first settle too) or while a late row is held, later settles smooth, the first counted at the first settle that has messages and spent on a reader who scrolled up |
| `components/summary/transcriptSettle.browser.test.tsx` | In Chromium, on the real `ProgressiveMessageList` and `ScrollArea`: the old handler pulling a reader who scrolled up during the load back to the end (the instrument); that reader keeping their rows through the settle and a later message, also when the messages land inside the list's first 50ms; the end reached when the reader did not scroll, came back before the settle, or the chat is short |
| `components/summary/useLateRowHold.test.tsx` | Which rows are held: the open's own `session` error at or after the 250ms grace (the measured 6.5 s case held), not inside it, not in the transcript's first frame, not after a turn ran, not of another scope, not a Retry's second row; released when the row goes, decided once per row, started over for another chat |
| `components/summary/lateRowHold.browser.test.tsx` | In Chromium, on the real `ProgressiveMessageList`, `ScrollArea` (with its `followGrowth`), follow, settle and hold hooks: the old wiring moving every row 62px up when the row lands (the instrument); the last message's top the same in every frame after it, also while older batches still land above, with the row 62px below the fold; followed again once the reader reaches it, after a send, for a row inside the grace, and for a short chat whose row lands in view and whose later messages are followed |
| `components/summary/transcriptFollow.browser.test.tsx` | In Chromium, on the real `ScrollArea`: the old stacked-only wiring ending 32px short when the subagent line mounts (the instrument); the end kept through that line mounting, a shorter window, a shorter and narrower one, and a narrower one that wraps the text taller (also with the layout read in the same task); a reader who scrolled up left where they are; a reader 100px up not pulled down by a width change and keeping their bottom line through a height change; a clicked card's place winning for one width change; a stacked preview still anchoring a reader who scrolled up |
| `components/summary/summaryRailGeometry.browser.test.ts` | In Chromium: the grid at 1055/1056 and 1067/1068, the 760px column on both sides of each seam, the composer's edges on the column's (also in cells under 816px, with and without a side preview), the subagent line on the column's left edge (with the rail, without it, beside a preview), the column moving by half the rail, 28px rows, the long list scrolling inside the card, the card's fill with no visible edge or inner hairline, and the scroll-edge fades at rest, mid-way and at the end, and the header button's `data-still` beating the Button's layered transition with its clearing starting none |

## Related documentation

- [Subagents](../agent-loop/subagents.md): how a chat delegates, and why a subagent runs in the
  background without a tab unless asked.
- [Orchestration phases](../agent-loop/orchestration-phases.md): the phase plan the Phases
  section draws, and how a phase hands its results forward.
- [Decision models in the desktop app](decision-models.md): the Decisions section, the gate chip
  and the phase gate dialog it opens.

- [The preview panel in a narrow pane](preview-panel/narrow-panes.md): rung 2, which shares the
  split box and its grid.
- [Chat summary and academic figure acceptance](summary-and-figures-qa.md): the summary's
  acceptance contract.
- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md): the
  Diagnostics action in the summary's footer.
- [Implementation spec, section 3.5](../design/codex-simplicity-redesign/implementation-spec.md):
  the design this records.
