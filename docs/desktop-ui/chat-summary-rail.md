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
| Column chrome | 56px | The transcript's ScrollArea root `px-1` (2 × 4) plus its viewport `paddingX={6}` (2 × 24), both in `BaseChat.tsx` |
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
| Fits | Shows and hides the docked rail (the preference) | `aria-expanded`, `aria-controls` naming the rail; pressed fill `--background-medium`; tooltip "Hide summary" or "Show summary" |
| Does not fit | Opens the summary in a 280px popover anchored to the button | `aria-haspopup="dialog"`, `aria-expanded`, `aria-controls` naming the popover; tooltip "Chat summary" |

The popover is anchored with `PopoverAnchor` rather than triggered, because a Radix trigger would
stamp its own `aria-expanded` and open the popover in rail mode too. If the window widens while
the popover is open, the rail replaces it and the popover closes, and so does a popover left open
when the button turns into the rail's (the rail was closed, then the window widened).

While the rail is hidden and a to-do step is running, the button carries a 6px coral dot and its
name becomes "Chat summary, 2 of 5 to do items complete".

## The content

`components/ChatSummary.tsx` draws the same content in the rail and in the popover: a card with
a 12px radius, a 1px `--border-subtle` edge and the `--background-default` ground, with a
scrolling body over a pinned footer. Two type sizes only, 13px for rows and 12px for labels.

- **To do**, when there are items: a muted label with a tabular "2 of 5" count (announced as "2
  of 5 complete" through `aria-live`), a 4px progress bar, then 28px rows. Each status has its own
  glyph shape, an open circle (pending), a dashed circle in the accent ink (running), a check
  (complete) and an alert (blocked), so status never depends on colour. The status word is read
  to assistive technology ahead of the task text. There is no "no tasks" sentence; a failed
  refresh shows one line, "To do could not refresh.", with Retry.
- **Statistics**: a `dl` of 28px rows: Tool calls, Tokens (with an InfoTip saying what is
  counted), Artifacts, and Code as `+N −M` in success and danger ink, read as words.
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
  side preview the rail takes a column between the conversation and the preview. The rail cell
  is padded 12px except on the transcript's side.
- **State**: `hooks/useSummaryRail.ts` (measurement, timing, entrance and exit),
  `Layout/summaryRailPreference.ts` (the viewer's choice).
- **Wiring**: `BaseChat.tsx` spreads `summaryRail.splitPaneProps` beside the artifact panel's,
  merging the two `style` objects after both spreads, and renders one `ChatSummaryRail`.

## Where it is pinned

| Test | Holds |
|---|---|
| `components/Layout/yieldLadder.test.ts` | Each seam one pixel apart (1055/1056, 1067/1068), the width steps (1056 → 240, 1076 → 260, 1096 → 280), the full measure across every pane width, the side-preview subtraction (1975/1976 at the default preview), a stacked preview, an unmeasured pane, and the window table |
| `hooks/useSummaryRail.test.tsx` | No hidden-then-shown flash on mount, a narrowing hides at once, a widening shows after 160ms, a mount does not animate, the first turn enters (from the commit that creates a new chat, once) but a loaded history (fetched, cached or rejoining a turn) does not, the toggle's entrance, exit and glide, and one live observer under StrictMode |
| `components/summary/ChatSummaryRail.test.tsx` | The card mounts entered when geometry showed it, plays its entrance once, ignores its children's animations, reports its exit (or its cancelled exit) and re-arms the entrance when reopened mid-exit |
| `Layout/summaryRailPreference.test.ts` | The per-viewer store, its default and the `storage` event |
| `hooks/useColumnGlide.test.ts` | No glide while resizing, under reduced motion or under 8px; the composer left to its own FLIP; the scroll anchor |
| `hooks/useSessionTodos.test.tsx` | Fetches with the user proof only while shown, refreshes after to-do writes, keeps the last list with an error and Retry |
| `components/ChatSummary.test.tsx` | The content, the status words, the code delta in words, the row entrances (kept through the re-renders that follow), the glyph crossfade, the thin progress bar, the scrolling body |
| `components/summary/SummaryHeaderButton.test.tsx` | The button's two modes, its tooltip, focus return, and the dot with its count |
| `styles/summaryRail.test.ts` | The exact grid templates, the `:not([data-preview-layout])` key, unlayered rules, no condition, transition or animation in the geometry, the card's motion, one rail in BaseChat, and the `paddingX={6}` and `const contentClassName = 'px-1';` literals the 56px comes from |
| `components/BaseChat.summaryRail.test.ts` | BaseChat's wiring: the button, the to-do hook's order and argument, the popover closing for the rail, the first turn counted from the commit that creates a new chat, no titlebar padding under the header band, and the message list memoised against everything the rail, the popover and the preview change |
| `components/summary/summaryRailGeometry.browser.test.ts` | In Chromium: the grid at 1055/1056 and 1067/1068, the 760px column on both sides of each seam, the composer's left edge on the column's, and the column moving by half the rail |

## Related documentation

- [The preview panel in a narrow pane](preview-panel/narrow-panes.md): rung 2, which shares the
  split box and its grid.
- [Chat summary and academic figure acceptance](summary-and-figures-qa.md): the summary's
  acceptance contract.
- [Diagnostics and bug reports](../troubleshooting/diagnostics-and-bug-reports.md): the
  Diagnostics action in the summary's footer.
- [Implementation spec, section 3.5](../design/codex-simplicity-redesign/implementation-spec.md):
  the design this records.
