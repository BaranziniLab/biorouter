# Narrow and short windows

> **What this is.** What gives way, and in what order, as the desktop window gets narrower or shorter: the sidebar, Home's composer and usage card, and the page bands, down to the window's minimum size.
> **Status:** Current (owner request of 2026-11-19, which lowered the minimum width from 1048px to 721px).
> **Audience:** anyone changing Home, the window's minimum size, the yield ladder, a page band, or the sidebar's narrow layout.

A split pane's composer narrows a long way and stays usable: at the 360px chat floor it is a 304px card with every control on its row and its footer. Home's composer could not narrow at all, because the window stopped at 1048px (the 288px default sidebar plus the 760px chat measure), a floor that existed only so that Home's usage heat map was never squeezed. And at the shortest window the usage card could run down to the composer. The rules below replace that.

## The order things give way

As the window narrows:

1. **The sidebar folds** at 1120px, rung 1 of the yield ladder (`SIDEBAR_COMPACT_WIDTH` in `ui/desktop/src/components/Layout/yieldLadder.ts`). It becomes an overlay, so it takes no width from the page. Nothing changed here.
2. **Home's composer narrows** once the window is under 808px (the 760px measure plus Home's two 24px gutters), down to 673px at the floor. It is the same `ChatInput` a chat uses, fluid inside its `max-w-measure-chat` column, so nothing about it is special to Home.
3. **Home's usage card steps aside** as soon as its column is narrower than the 760px measure. The card's heat map is the one thing on Home whose size is computed (one fluid column per week), so a narrower card is a smaller grid. The old floor kept it whole; now the card leaves instead of shrinking, and Home shows the greeting and the composer.

The window stops at **721px** (`WINDOW_MIN_WIDTH`, the `minWidth` in `ui/desktop/src/main.ts`). The number is derived: the narrowest window that still holds a two-pane split with each pane at the chat floor (2 x 360 plus the 1px splitter). So rung 4 never has to merge a split at the floor, and Home's narrowest composer (673px) is wider than a split pane's (304px). `styles/measures.test.ts` pins the literal to the constant and the constant to `layoutMinWidth`.

With the sidebar docked the reading column still has its whole measure at every sidebar width, because the widest sidebar plus the measure is exactly rung 1 (360 + 760 = 1120). That was the property the old floor protected, and it still holds.

## Short windows

The window's minimum height stays 600px. Home's column (the greeting and the usage card) is centred in the room above the composer, and the card yields before it can touch the composer:

- **Whole card** while the greeting, the card and a 24px gap above the composer all fit.
- **Tiles only** where they do not: the heat map and its footnote fold away. This is what the shortest window shows with the Larger text size, or with a greeting that wraps to two lines.
- **No card** where even the tiles would not leave the gap.

Below that, the existing container queries on `.biorouter-home-content` compact the greeting and then hide it (under a 340px room, which only a `biorouter serve` browser reaches), and Home scrolls as a last resort.

The decision is `homeUsageFit` in `ui/desktop/src/components/sessions/homeUsageFit.ts`, a pure function of measured boxes, kept by `useHomeUsageFit`. A folded card or heat map is clipped to zero height, never unmounted, so the card keeps its natural size and is measured in every state; the answer cannot change its own inputs, so there is no loop and no return buffer. A hidden card is also `inert`. The gap is the column's own 24px bottom padding.

## The sidebar below 930px

`useIsMobile` turns true under 930px, and in a browser (`biorouter serve`) the sidebar becomes a drawer there. The desktop window used to stop above that width; now it does not, and it keeps the overlay sidebar it has below rung 1 at every width it allows (`ui/desktop/src/components/ui/sidebar.tsx`). One way to fold, the titlebar controls in place, and the titlebar reserve every page band keys on (`[data-slot='sidebar'][data-state='collapsed']`) still in force. In the drawer that selector did not match, so page titles slid under the macOS traffic lights.

The sidebar starts open, so a window that opens below rung 1 collapses it on the first frame. `AppLayout`'s width watcher does that in a layout effect, before anything on the page measures itself. As a passive effect it ran after the chat groups' split watcher had already measured the page beside a docked column, and a reload at the floor merged a two-pane split that fits and then lost it. Its cleanup leaves the overlay class (`biorouter-sidebar-compact`) alone, because it re-runs on every toggle, and lifting the class mid-commit stopped ⌘B moving focus into the overlay; the class goes when the layout unmounts.

## Page bands at the floor

Every page band was measured at the floor with the sidebar folded and the 172px macOS titlebar reserve in force (Settings, Knowledge, Chat history, Extensions, Skills, Workflows, the Scheduler, Built apps, Dictionary, and Crew with no workspace). They fit, with one change: the Knowledge band. Crew with a workspace was not measured, because that needs a Crew server: at the floor its channel is 481px beside the 240px Crew column, which is under the 800px step where its details pane covers the conversation instead of pushing beside it, Crew's designed narrow mode. Below the 860px step it also carries the Sources/Graph pair, and with the widest labels (BioOKF, Private, four-digit counts) its counts painted over that pair below about 830px, even after the base's name had given way. While the reserve and the pair share the band, the counts now fold; the title, the base's format and tier, the pair and the actions keep their size, and the band fits down to about 700px. With the sidebar docked there is no reserve and the counts stay.

## Where it is pinned

| Test | Holds |
|---|---|
| `styles/measures.test.ts` | `minWidth` equals `WINDOW_MIN_WIDTH`, a two-pane split fits at it, rung 1 folds the sidebar before the composer narrows, Home's narrowest composer is wider than a pane's, and the 360 + 760 = 1120 identity. |
| `sessions/homeUsageFit.test.ts` | Every branch of the decision, unmeasured input, and the CSS it draws with (the clip, the 24px gap, the measured box). |
| `sessions/homeLayout.browser.test.tsx` | Home in Chromium at widths from the floor to 1048 and heights 600 to 900: the composer is fluid to the floor and centred, the card shows only at its measure, nothing above the composer touches it or is cut off, the Larger text case drops the heat map, and the pane composer at the chat floor is usable. |
| `Layout/windowFloor.browser.test.tsx` | The Knowledge band at the floor with its widest labels and the titlebar reserve: the counts fold and nothing overlaps or overflows; they stay from the 860px step and beside a docked sidebar. |
| `ui/sidebar.test.tsx` | A narrow desktop window keeps the overlay; the drawer belongs to the browser. |
| `Layout/AppLayout.test.tsx` | Below rung 1 the overlay class is on `<body>` before the page's first effects run. |

## Related documentation

- [When the app "stops scaling with the window"](window-scaling-regressions.md): why the window's minimum size is a floor, not a cap, and what it used to protect.
- [Compact panes](compact-panes.md): rung 5, what a pane in a split shows under the chat floor.
- [The chat composer](chat-composer.md): the one-line composer Home and every chat share.
- [The preview panel in a narrow pane](preview-panel/narrow-panes.md): rung 2, which a window near the floor reaches with a single pane.
