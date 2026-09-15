# The preview panel in a narrow pane

> **What this is.** The rule for where the artifact preview goes when a chat pane is too narrow to seat it beside the conversation, how that rule is built, and what it guarantees the later motion pass.
> **Status:** Current; measured 2026-09-14 in the dev app on `fix/preview-panel-narrow-panes`.
> **Audience:** contributors changing the artifact panel, the chat pane's layout or the split panes.

In a window split into two or three panes, opening an artifact used to cover the pane it was opened in: below 720px the panel floated over the transcript and the composer of the very conversation it belonged to. A preview is now always beside the conversation or above it, and never over it. This page is the rule, the parts that implement it, and the invariants the tests hold.

## The rule

The preview's shape is a pure function of the pane's width P, measured on the split box `[data-preview-split]` with a `ResizeObserver`. P is never the window's width and never a `vw` value. Neither shape resizes the box that is measured, with a 12px return buffer: side switches to stack below 800px; stack returns to side at 812px.

| Pane | Shape | Size |
|---|---|---|
| P ≥ 800 on first open; ≥ 812 when returning from stack | **Side**: a full-height column on the right, with a 44px strip continuing the chat header's band | Default `clamp(round(0.48·P), 360, max(360, min(920, P − 640)))`: the panel yields to 360 before the conversation narrows from 640. A drag clamps to `[360, min(920, P − 440)]`, so the conversation is never under 440. |
| P < 800 | **Stack**: a sheet directly under the chat header at the pane's full width. From the top: header, sheet, transcript, composer. The composer stays on the pane's bottom edge. | Not dragged: `clamp(min(round(0.5·H), content), min(200, content), H − F)`. Dragged: `clamp(ratio·H, 200, H − F)`. If `H − F` is under the sheet's floor, the sheet is `max(36, H − F)`. |

- **H** is the split box's height below its header band (the chat header, plus a subagent's second header when there is one).
- **F** is the conversation's floor: its measured chrome (the composer bar, 174px today; a replay's page header), the 8px resize edge, and 146px of transcript. It is measured, not assumed, so a composer holding a queued message counts.
- **content** is the whole sheet's natural height, strip included. It never drops below the 36px strip.
- **800** falls on no common split: a two-pane split reaches it from a 1601px window with the sidebar collapsed or an 1889px window with it open. The 720px rule it replaced sat exactly on a 1440px window.
- **440** is the first chat column in which no full line of real transcript text runs under 45 characters, at the transcript's own 14px/24px face.
- **360** is the panel's own floor: its tab strip, status strip and file text fit with no overflow. A 285px floor cut the tab label to "Lab |".

The conversation always wins a collision: the sheet gives way down to its strip, and never covers anything.

## How it is built

**One grid, authored in `main.css`.** `useArtifactPanel` stamps the split box with `data-preview-layout` (`side` or `stack`), `data-preview-folded`, `data-preview-measuring`, and the lengths `--preview-panel-width`, `--preview-stack-height`, `--preview-provisional-height` and `--preview-chat-floor`. The rules place each piece by its `data-preview-area`: `header`, `subheader`, `transcript` and `composer` in the live chat, and one `conversation` item for a replay's whole reading column. The chat's column and body are flattened with `display: contents` rather than re-parented. Every rule is plain and unlayered, keyed on attributes, because a newly written Tailwind class can silently fail to generate under `BIOROUTER_NO_HMR`.

**The decision is pure.** `previewPanelMode`, `previewSideWidth`, `previewSideMaxWidth`, `previewStackHeight`, `previewStackDrag` and `previewChatFloor` live in `components/Layout/yieldLadder.ts` as rung 2 of the yield ladder, with no React and no DOM.

**A fresh sheet takes no room until it knows its height.** While a newly opened sheet's content is still loading, the grid gives its row 0px and lays the panel out invisibly at the default height. A figure's frame needs a real viewport to lay out in. The sheet takes its row once, when the content reports, so the transcript moves once rather than to half and then to the content. A document that never reports gets the default after 600ms.

**Content says how tall it is.**
- Text previews (a markdown body, a code view's `<code>`, a CSV table) are measured from the panel's own DOM, through elements marked `data-preview-intrinsic`.
- A figure or HTML page runs in a sandboxed frame the panel cannot read. So the panel injects `utils/previewSize.ts`, which posts the document's intrinsic height: where the body's children end, not the frame's viewport. Auto Visualiser's own size report includes the viewport, so a short figure could never report less than the half it had been given.
- An image, a directory tree, a live page, a notebook and an error card cannot say, and get half.

**The resize edge is the panel's sibling.** The panel clips its own paint, so an edge inside it could only lie over the preview's content. As a sibling, the grid places it on the seam: the panel's left 8px beside the conversation, and the transcript's 8px top padding under a sheet. It is drawn as a 1px `--border-strong` line on hover.

**Folding is the user's.** A sheet is never folded by default. The strip's chevron folds it to its 36px strip, and so does dragging the edge above the midpoint of [36, 200]. A tab click, a click on the bare strip or the chevron unfolds it. A drag that folds keeps the height the sheet had when the drag began.

**The transcript keeps its bottom edge.** `ScrollArea`'s `anchorBottomOnResize` writes `scrollTop` once per viewport resize so the line against the composer stays there when a sheet opens, folds, crosses to a column or closes. A scroll event that arrives together with a viewport resize is treated as layout, not the reader, so opening a sheet does not turn following off. Only the live chat's transcript opts in, and only while a stacked sheet is on screen, plus the one resize that ends it.

**Text reads at the chat measure.** `.br-preview-measure` holds markdown, plain text, code and logs to `calc(var(--measure-chat) + 2 * 16px)`, centred like the transcript: a 760px column of glyphs. It is never applied to a CSV or TSV, a frame, an image, a directory tree, a notebook or a document.

## What the motion pass can rely on

- **The panel stays mounted.** Each host renders one `<ArtifactViewer>` at a fixed position with no layout-dependent key. A side and stack crossing changes attributes, the custom properties and authored CSS only. Measured across 13 crossings and a fold: the aside, split box, header, transcript scroller, composer and figure frame are the same nodes; the frame fires 0 `load` events and keeps its `contentWindow`. The only DOM removals are the composer toolbar's rung 3b controls and Radix's auto-hiding scrollbar, both independent of the panel.
- **The box settles immediately; its contents animate.** The existing preview body translates 32px and fades with the Web Animations API: entrance 300ms, orientation 250ms, exit 125ms. A window resize does not cancel entrance. Reduced motion disables animation and cancels an active animation when enabled. The grid and conversation do not animate.
- **No per-frame JavaScript layout.** The shape comes from `ResizeObserver` callbacks that bail out on equal values. Content height is measured when content changes, never on the panel's own resize. The only `requestAnimationFrame` loop runs during a pointer drag.

## Where it is pinned

| Test | Holds |
|---|---|
| `components/Layout/yieldLadder.test.ts` | Every threshold on both sides (799/800, 999/1000/1001, 117.9/118, `H − F` = 199/200), the floors, fit-to-content, the dragged share, the fold, and a 1px sweep with one crossing and a 12px return buffer. |
| `styles/measures.test.ts` | The CSS literals against the ladder's constants (`minmax(440px, 1fr)`, `calc(146px + 8px)`, `--dock-height`, `--chrome-height`), the stack rules declared after the side rules, the rules unlayered, no container or media condition, no transitions, one unkeyed `<ArtifactViewer>` per host, the anchor scoped to the live chat, and the text measure's declaration and exact call sites. |
| `components/artifacts/useArtifactPanel.fold.test.tsx` | Folding by the chevron and by drag, unfolding by a tab and the bare strip, the drag clamps, the measuring phase, closing, and node identity across a crossing. |
| `utils/previewSize.test.ts` | The reporter measures the body's content, not the viewport. |

Each of those was made to fail once on purpose: 16 mutations, each turning a named test red.

## What is not built

- **A maximise control** for a sheet squeezed to its strip. The rule allows one, behind an explicit click with a labelled way back to the chat, and never automatic. It is not built.
- **The window still grows on open** for a single pane, targeting 1048px, exactly as before.
- **A replay stacks only in a narrow browser.** On the desktop, History's run detail and a shared chat are one pane at least 1048px wide, so they sit beside. Their stacked layout was checked by narrowing the pane in the running app.

## Related documentation

- [The preview panel as it stands today](current-state.md) — what the panel renders and every guard on the path.
- [Where a generated artifact is displayed](../artifact-display-surfaces.md) — why the panel is the only artifact surface.
- [Window-scaling regressions](../window-scaling-regressions.md) — the window-size impostors a measurement of this layout has to avoid.
- [Renderer testing traps](../renderer-testing-traps.md) — why a component test cannot see this layout, and what can.
