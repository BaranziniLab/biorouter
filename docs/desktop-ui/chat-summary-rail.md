# The Chat summary rail

> **What this is.** Where the Chat summary appears in a chat: docked as a rail beside the
> conversation when the pane has room for both at the full 760px measure, and as a popover from
> the header button when it does not. Covers the width rule (rung 0 of the yield ladder), what
> decides whether the rail is shown, the header button's two modes, the content, the motion, and
> the tests that hold each part.
> **Status:** Current. Built in the Codex simplicity redesign (WS-SUMMARY, October 2026).
> **Audience:** developers working on the chat pane's layout, the summary, the artifact preview
> or the yield ladder.

The Chat summary holds a chat's to-do list, its statistics (tool calls, tokens, artifacts, code
delta) and two actions (Make workflow and Diagnostics). It used to live only in a 360px popover
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

The pressed fill fades only when the person pressed the button, opened or closed its popover, or
the first turn brought the rail in. When geometry flips it (a resize across a seam, the sidebar,
a preview), the button carries `data-still` for that one change and the fill snaps, as the grid
and the card do; the marker is cleared two frames later, which starts no transition because
nothing changes then. Before this, dragging the window across 1344px ran the button's colour
transitions twice while everything else in the chat snapped.

While the rail is hidden and a to-do step is running, the button carries a 6px coral dot and its
name becomes "Chat summary, 2 of 5 to do items complete". The dot's 2px ring is the ground under
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
hairline: the to-do list, the statistics and the footer sit 12px apart (22px between their text,
against 10px between rows). Where the list overflows, the edge that hides rows fades over 16px:
the bottom while there is more below, the top once the list has scrolled. The fade follows the
body's own scroll position (a CSS scroll timeline), so it needs no listener and moves nothing,
and a focused body drops it so its focus edge is drawn whole. Two type sizes only, 13px for rows
and 12px for labels.

- **To do**, when there are items: a muted label with a tabular "2 of 5" count (announced as "2
  of 5 complete" through `aria-live`), a 4px progress bar, then 28px rows. Each status has its own
  glyph shape, an open circle (pending), a dashed circle in the accent ink (running), a check
  (complete) and an alert (blocked), so status never depends on colour. The status word is read
  to assistive technology ahead of the task text. There is no "no tasks" sentence; a failed
  refresh shows one line, "To do could not refresh.", with Retry, whose label ends on the
  values' right edge. On the card the 4px track rests on `--background-fill-strong`: the
  primitive's own track is `--background-muted`, which is the fill itself and would vanish.
- **Statistics**: a `dl` of 28px rows: Tool calls, Tokens (with an InfoTip saying what is
  counted), Artifacts, and Code as `+N −M` in success and danger ink, read as words. A count of
  zero is muted, so a chat that touched no code shows no red. A figure that is not known yet is
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
- The scrolling body is a focus region with a tab stop (`.biorouter-focus-region`), so a
  keyboard user can scroll it.

The to-do list is fetched while the popover or the rail is shown, and while a turn runs so the
button's dot stays current (`hooks/useSessionTodos.ts`, with the user-action headers).

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
| `hooks/useCostTracking.ledger.test.tsx` | `ledgerSettled` false with no rows while the usage request is pending, settled on rows, on none and on a failure, never one chat's ledger for another's, and a refresh of the same chat keeps its answer |
| `components/BaseChat.costLedger.test.tsx` | On the real hook and the real footer `CostTracker`: no Tokens value and no cost while the ledger is pending, then exactly one reading ("N/A" and no cost for an uncertified row; "2.2M" and the priced counters for a ledger with no rows) |
| `components/ChatSummary.test.tsx` | The content, the status words, the code delta in words and its muted zeros, empty values (and `aria-busy`) for figures not known yet, the gap above the statistics, the row entrances (kept through the re-renders that follow), the glyph crossfade, the thin progress bar, the scrolling body |
| `components/summary/SummaryHeaderButton.test.tsx` | The button's two modes, its tooltip, focus return, the dot with its count, and which flips of the pressed fill snap (geometry) and which fade (a press, the popover, the first turn) |
| `components/summary/summaryPopover.browser.test.tsx` | In Chromium, with the whole stylesheet, in a band shaped like BaseChat's header: the old wiring landing on the band line and past the window (the instrument); the popover 4px below the band line and 8px above the window's bottom with its body scrolling, at 1300x900, beside a side preview, at a device pixel ratio of 2 and in a 600px window; a short list at its natural height |
| `styles/summaryRail.test.ts` | The exact grid templates, the `:not([data-preview-layout])` key, unlayered rules, no condition, transition or animation in the geometry, the card's motion, one rail in BaseChat, the `paddingX={6}` and `const contentClassName = 'px-1';` literals the 56px comes from, and the fill: the card's fill and transparent border, no hairline in the content, the track's ground, the scroll-edge fades, the pressed fill and the dot's ring |
| `components/BaseChat.summaryRail.test.ts` | BaseChat's wiring: the button, the to-do hook's order and argument, the popover closing for the rail, the composer bar's and the subagent line's gutters, the first turn counted from the commit that creates a new chat, no titlebar padding under the header band, the message list memoised against everything the rail, the popover and the preview change, a loading chat's row reaching the rail, the chip, the placeholder and the ring, the commit before the load counted as loading (rail and empty state), a pending chat's composer bar and subagent line at opacity 0 and never `inert`, the Tokens value and the footer cost waiting for the ledger, and the transcript's follow hook (its arguments, both props, above the early return, the clicked card's hold), and the list settling through `useTranscriptSettle` with no first-settle force of its own, and the first send: started in the commit that starts creating the chat, the chat named once created, ended wherever the composer gets the text back, drawn in the transcript slot, and the four figures from `summaryFigures` |
| `components/summary/useFirstSend.test.tsx` | A new chat's first send against a fake daemon (`/agent/start` after 500ms, a 280ms load, the ledger 50ms after the session), on the real hook, `summaryFigures`, `ChatSummary`, `FirstSendTranscript` and `ProgressiveMessageList`: the old wiring leaving the text off screen and reading Tokens "0", nothing, "N/A" (the instrument); the text in the first commit after the send and in every commit after it; Tokens from empty to its answer once, the counts held at zero; the stored row the same markup as the first-send row, at rest. Also the hook's life (begin, created, spent, another chat, a failure, attachments alone) and `summaryFigures`' cases |
| `components/summary/transcriptSettle.test.ts` | The settle rule: the first settle lands at once while following, nothing for a reader who scrolled up (first settle too), later settles smooth, the first counted at the first settle that has messages and spent on a reader who scrolled up |
| `components/summary/transcriptSettle.browser.test.tsx` | In Chromium, on the real `ProgressiveMessageList` and `ScrollArea`: the old handler pulling a reader who scrolled up during the load back to the end (the instrument); that reader keeping their rows through the settle and a later message, also when the messages land inside the list's first 50ms; the end reached when the reader did not scroll, came back before the settle, or the chat is short |
| `components/summary/transcriptFollow.browser.test.tsx` | In Chromium, on the real `ScrollArea`: the old stacked-only wiring ending 32px short when the subagent line mounts (the instrument); the end kept through that line mounting, a shorter window, a shorter and narrower one, and a narrower one that wraps the text taller (also with the layout read in the same task); a reader who scrolled up left where they are; a reader 100px up not pulled down by a width change and keeping their bottom line through a height change; a clicked card's place winning for one width change; a stacked preview still anchoring a reader who scrolled up |
| `components/summary/summaryRailGeometry.browser.test.ts` | In Chromium: the grid at 1055/1056 and 1067/1068, the 760px column on both sides of each seam, the composer's edges on the column's (also in cells under 816px, with and without a side preview), the subagent line on the column's left edge (with the rail, without it, beside a preview), the column moving by half the rail, 28px rows, the long list scrolling inside the card, the card's fill with no visible edge or inner hairline, and the scroll-edge fades at rest, mid-way and at the end, and the header button's `data-still` beating the Button's layered transition with its clearing starting none |

## Related documentation

- [The preview panel in a narrow pane](preview-panel/narrow-panes.md): rung 2, which shares the
  split box and its grid.
- [Chat summary and academic figure acceptance](summary-and-figures-qa.md): the summary's
  acceptance contract.
- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md): the
  Diagnostics action in the summary's footer.
- [Implementation spec, section 3.5](../design/codex-simplicity-redesign/implementation-spec.md):
  the design this records.
