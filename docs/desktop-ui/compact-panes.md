# Compact panes

> **What this is.** What a pane in a split shows when it is too small for its chat, why the thresholds are where they are, and how the chat survives being hidden.
> **Status:** Current (from the merge of `feat/compact-panes`).
> **Audience:** anyone changing the chat pane's layout, the yield ladder, or `ChatGroupsShell`.

A window split into four panes of 140 to 290px used to draw four broken chats: bubbles one word wide, a composer cut to a sliver, tool rows reading "Re…" and a greeting on seven lines. Rung 5 of the yield ladder replaces a pane that is too small with a quiet placeholder that names its chat, says whether it needs the person, and offers one way to get it back. The chat itself is never unmounted: it stays laid out behind the placeholder and returns exactly as it was. Design record: [design.md, D-45](../../design.md#917--compact-panes-2026-11-16).

## The rule

A pane in a split is **compact** when it is narrower than **360px** or shorter than **270px**. It is full again at **372 by 282**: the same 12px return buffer rungs 0 and 2 use, so a slow splitter or window drag across the seam does not flicker. A pane that is alone in the window is never compact.

| Transition | Width | Height |
|---|---|---|
| full to compact | under 360 | under 270 |
| compact to full | 372 or more | 282 or more (both axes must pass) |
| held, no change | 360 to 371 | 270 to 281 |

The decision is `paneFit` in `components/Layout/yieldLadder.ts`, a pure function of the pane's own box, whether it is alone, and the previous answer. It never reads the window: `window.innerWidth`, `vw` and Tailwind's `sm:` follow the window, and a window rule is what drew a chat into a 140px column in the first place. An unmeasured box (a first paint, a hidden window) keeps the previous answer, as `summaryRailFit` does. There is no feedback loop: a pane's box is set by its flex share in the split, never by its content, and the swap changes only what is inside the box.

Rung 4 is unchanged. On a width crossing it still merges a split the window can no longer hold, and the lone pane left behind is full. Rung 5 covers the splits rung 4 leaves alone, including every split the person builds by hand.

## Why 360 by 270

**360 wide** is `CHAT_MIN_WIDTH`, the number every rung yields to (D-32, D-36) and rung 4's floor per pane, so rungs 4 and 5 agree about what a usable pane is. Every breakpoint found in the content sits below it: the composer row breaks at about 250px, the band at about 250px, the footer folder at about 330px, tool rows at about 300px and bubbles turn one word wide at about 220px. The composer is proven intact at 360px in Chromium.

**270 tall** is the 44px band, the resting composer bar and four 24px lines of transcript: 44 + 130 + 96. The composer bar was **measured at 130px on 2026-11-16** (28px top padding, the 50px one-line card, a 4px gap, the 24px footer line and 24px bottom padding). It was 174px before the one-line composer of 2026-11-13. Below 270 the transcript is a sliver over the composer, and at about 174px the composer itself starts to clip. Two stacked panes in the 600px minimum window are about 299px each and stay full; three are about 199px and compact.

Text size is not a multiplier. Every width rung is in CSS px, and scaling only this one would compact splits rung 4 keeps. What has to hold is that the composer never clips above the threshold at any text size: at Larger text (scale 1.15) the bar measured 149px, and 44 + 149 is well under 270.

## What the placeholder shows

The placeholder is the pane's own 44px band over a calm body holding one quiet button. The band is the chat header's band exactly (the sidebar ground and the same bottom hairline), so the window's top edge stays one line across compact and full panes.

The button holds, top to bottom:

- the chat's kind glyph in a 32px filled tile, with the lock badge for a private chat and the running dot on the tile's other corner while a turn is live;
- the chat's title, on one line;
- one state line: **Listening…** while the pane's own composer is dictating, else **Waiting for you** while the chat waits on an approval or an answer, else **Running**, else what the pane is short of (**Too narrow to show**, **Too short to show** or **Too small to show**). Listening and Waiting for you step from muted to default ink, and nothing is ever red;
- the action words: **Make wider**, **Make taller** or **Make larger**, or **Merge panes** when the layout has no room.

The button's accessible name is the verb and the title ("Make wider, Cohort query"), and its description is the glyph's own name, the state and the help ("Private chat. Running. The chat is hidden, not closed. It comes back as it was when the pane is wider."). The help is also in its tooltip, which a hover peeks and programmatic focus never opens. One `role="status"` line per compact pane announces "*title* is waiting for you" once, on the way into waiting, and is silent on mount and for every other change, so several running panes never talk at once.

Three densities, chosen in JavaScript from the placeholder's own box and drawn by authored CSS:

| Density | When | What shows |
|---|---|---|
| card | 176 wide and 188 tall, or more | the tile, the title, the state line and the action words |
| glyph | 64 wide and 108 tall, or more | the tile alone, in a 48px square button |
| band | anything smaller | an empty body; the action is a 24px button in the band |

The button is the same element in card and glyph density, so a density change during a drag never moves focus. When a change of size takes away the control that holds focus, focus moves to the action wherever it now is: from the body to the band, or from the ▾ when the band drops it. When the band has no room for the action either, the placeholder itself holds focus, with the shared focus edge, and carries the action's description, so focus never falls to the page.

The band holds, at its inline end, the band action (band density only) and then the ▾ tab menu when the pane holds two or more tabs, each only when it fits after the titlebar reserve. The first pane on macOS keeps 172px free for the floating titlebar controls while they are drawn over it, so with the sidebar collapsed a first pane narrower than 208px shows no ▾; its body action still restores it. Choosing a tab from the ▾ switches the pane's tab and the pane stays compact. There is no `+`, because a new tab would open hidden. The empty band moves the window and zooms it on a double-click by pointer gesture, as the tab strip's band does; it declares no app region, so it can never cover the titlebar controls (#74).

## What stays mounted, and how

`ChatPaneFrame` wraps everything the pane draws (the band with the tab strip, the chat or Home, the pane's terminals) in one always-rendered content box, the pane root's first child. A change of fit never changes the element type or position above `BaseChat`, so React never remounts it. `BaseChat.tsx` is not edited.

While the pane is compact, `compact-pane.css` does this to the content box, and each declaration has a reason:

- **`position: absolute` at `max(100%, var(--pane-min-width))` by `max(100%, var(--pane-min-height))`.** The chat leaves the flow and keeps a box of at least 360 by 270 that stops changing while the pane keeps shrinking. Every observer inside it (the transcript's follow, the composer's growth, the artifact panel, the strip's overflow, the terminal's fit) sees one stable box near its last size, never 0 and never 140, and none of them needs a guard.
- **`visibility: hidden`** keeps layout and every scroll offset and skips paint.
- **`opacity: 0`** hides any descendant that sets `visibility: visible` (the fade-out keyframes do), and makes the box a stacking context that holds `BaseChat`'s own `z-index: 60` under the placeholder.
- **`inert`**, set by the frame, blocks focus, hit testing, selection and find in page, and takes the box out of the accessibility tree. `visibility: hidden` already keeps ordinary content from taking focus; `inert` is what holds a descendant that sets `visibility: visible`, which `opacity: 0` hides only from the eye. `aria-hidden` is not added: `inert` already does it, and Chromium refuses `aria-hidden` on an ancestor of the focused element, which is exactly the moment a pane compacts.
- **`overflow: hidden` on the pane**, and the frame resets the pane's own scroll to 0. The hidden box can be wider or taller than the pane, and a `scrollIntoView` inside it (the strip's reveal, a transcript jump) would otherwise scroll something outside the pane.

Three ways of hiding the chat are wrong. **`display: none`** hands every observer a 0px box and a 0 scroll offset for as long as the pane is compact. **Unmounting** (a conditional render) loses the draft, the stream and the terminal. **`overflow: clip`** on the pane skips the pane as a scroll container, so a `scrollIntoView` inside the hidden box looks for the next scrollable ancestor.

What survives, measured in Chromium: the same DOM nodes before and after, the transcript's scroll offset, the draft with its selection, focus (a pane that held focus puts it on its action, or on the placeholder itself when the band has no room for one, and gives it back to the same element or the composer on return; a pane that did not hold focus never takes it), a running turn that keeps streaming into the hidden transcript, and an open artifact, which keeps its tabs in stack mode and fires no reload. A terminal's container never drops below 360px wide, so its PTY never resizes toward 0 columns. One reflow happens on entry, from the last width to 360.

The `visibility` change is inherited, so a box inside the chat whose transition list covers `visibility` (`transition: all`, as the terminal dock has, or Tailwind's `transition`) would animate it instead of switching. On the way back such a box still computes `hidden` in the frame that gives focus back, and refuses it: the live pass found focus in a terminal falling to the page. So the frame finishes those `visibility` transitions on every crossing (and only those; every other transition runs on), and when the element that held focus still refuses it, focus goes to the composer instead.

The transition never shows a frame of the broken chat. `usePaneFit` samples the pane in its own `ResizeObserver` callback and commits a crossing with `flushSync`, so the placeholder lands before that frame paints. The first sample runs in a layout effect, so a reload into a narrow layout shows the placeholder from the first frame. A splitter drag re-renders the chat once per crossing, never once per frame.

## Make wider and Merge panes

The action resizes the split so the pane is full again, with one batched set of `resizeBranch` dispatches after making the pane active. `growPanePlan` in `chatGroups/paneGrowPlan.ts` asks the layout for room in a fixed order and takes the first attempt that fits:

| Attempt | Target (width / height) | A readable other pane keeps | A compact other pane keeps |
|---|---|---|---|
| 1 | 496 / 320 | its current size | the smaller of its size and 176 / 188 |
| 2 | 496 / 320 | 362 / 272 | the smaller of its size and 176 / 188 |
| 3 | 376 / 286 | 362 / 272 | the smaller of its size and 176 / 188 |
| 4 | 376 / 286 | 64 / 88 | 64 / 88 |

Attempt 1 takes room only from panes that are already compact. Attempts 1 to 3 never turn a readable pane compact and keep every compact pane at card size or the size it already has. Attempt 4 is the last resort. Width changes only row branches and height only column branches, so the two axes never interact; a pane short on both needs a plan on both. The plan is made again from fresh boxes at the press, and a press whose label said grow never merges: if the room has gone, nothing happens and the label becomes Merge panes. The 4px over the exit threshold in 376 and 286 absorbs flex rounding.

The bounds, in the narrowest Electron tree (about 688 by 600): attempt 4 always succeeds for up to five panes in one row (376 + 4 × 65 = 636) and up to four stacked (286 + 3 × 89 = 553). Beyond that the button reads Merge panes.

**Merge panes** makes this pane active and runs the existing `mergeAllGroups`: every tab moves into this pane in leaf order, this pane's active tab stays active, and no tab closes. No snapshot is taken, so rung 4 never undoes a merge the person chose.

The splitter clamp changed with it (`clampPairShares` in `ChatGroupSplitter.tsx`): a pane that Make wider or a stored layout left under 12% of its pair no longer jumps to 12% on the first pixel of a drag. It can grow, and it never shrinks further.

## The lone pane

A pane alone in the window is never replaced, at any size: nothing in the app could give it room back, so a placeholder would trap the person (on a phone, or in a narrow `biorouter serve` window). Closing or merging the other panes brings the chat back at once. In Electron a lone pane cannot be under about 688px, so this matters only in `serve`; the composer's one-row case is held at 320px in Chromium with no sideways overflow of the card, the footer or the pane. Two stacked panes in a browser narrower than 360px are both compact by width, have no row to grow, and offer Merge panes.

## Tests that hold it

- `components/Layout/yieldLadder.test.ts`, "rung 5": `paneFit` one pixel either side of every threshold from both previous answers, the held band, the lone pane and unmeasured boxes; `compactPaneDensity`; `compactBandControls`, including the 208px titlebar case.
- `components/chatGroups/paneGrowPlan.test.ts`: the pixel model, the worked examples (four flat at 1152 give 218 / 218 / 218 / 496; nested 454 / 176 / 143 / 376; two at 700 give 203 / 496; three stacked at 600 give 156 / 286 / 156), the attempt order, the axes, and a seeded property test over every layout shape up to six panes.
- `components/chatGroups/ChatGroupSplitter.test.ts`: the pair clamp.
- `styles/paneFloor.test.ts`: the CSS floor equals the constants, the hiding declarations are present and `display: none` and `clip` are not, `compact-pane.css` uses tokens only and no danger tone, and `BaseChat` still has the bar padding and the dictation key the placeholder mirrors.
- `components/chatGroups/ChatGroupsShell.compactPane.test.tsx` (jsdom, with stubbed boxes): only the small pane compacts and its chat is inert, each chat mounts once across full, compact and full, focus moves and returns (to the placeholder itself when the band has no room for an action, away from a ▾ the band drops, and to the composer when the element that held it refuses it), the placeholder's own description only while it has no action, the run, waiting and dictation states, the announcement, Make wider and Merge panes dispatches, and the band's ▾.
- `components/chatGroups/CompactPaneView.test.tsx`: the words come from `copy.ts`, one action in each density, the same node across card and glyph, the name and description, the lock badge and "not yet known", and the dot.
- `components/chatGroups/compactPane.browser.test.tsx` (Chromium): the placeholder in 70 boxes in every family and mode (density, no overflow, one line each, the action inside the pane, no danger colour, the tile against the ground); the frame's crossings one pixel either side of 360, 372, 270 and 282, with the same nodes, the one mount, the scroll offset, the draft and its selection checked at every step, and the hidden chat held at 360 wide and 270 tall; focus kept on the placeholder when the band has no room for an action; focus given back to a control inside a box that transitions `visibility`, as the terminal dock does (with the precondition, that such a control refuses focus in that frame, measured first); `scrollIntoView` absorbed; the placeholder committed before paint; each hiding declaration (both `max()` floors, `opacity`, `inert`, the pane's `overflow` and `visibility`) removed in turn to show its defect, `inert` by a `focus()` inside the hidden chat that is refused until it is removed; and the titlebar reserve. `COMPACT_PANE_SHOTS=<dir>` writes a screenshot of every box. It needs Playwright's Chromium or Google Chrome. Without either it skips every test with a warning on a developer machine, and fails on CI (`CI` set), because no other suite checks this geometry.
- `components/composer/composerCard.browser.test.tsx`: the resting bar measures 130px within 2px, the band and the bar stay under 270 at Larger text, and the one-row case holds at 320px.

## Related documentation

- [When the app "stops scaling with the window"](window-scaling-regressions.md): the pane, not the viewport, and the ladder's measured numbers.
- [The preview panel in a narrow pane](preview-panel/narrow-panes.md): rung 2, which keeps its stack shape inside the hidden 360px box.
- [The Chat summary rail](chat-summary-rail.md): rung 0, the model for a rung that measures the pane and keeps a return buffer.
- [The chat composer](chat-composer.md): the one-line composer and its footer, whose height the 270px floor is built on.
- [The shared controls](shared-controls.md): "Hover and click" (D-43) and "Fills before lines".
- [design.md](../../design.md): D-32 and D-36 (the yield ladder and its 360px floor), D-37, D-43, D-44 and D-45.
