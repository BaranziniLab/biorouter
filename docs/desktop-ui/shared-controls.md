# The shared controls

> **What this is.** The numbers and rules the desktop app's shared controls are built to (switch, segmented control, InfoTip, setting row, row actions, menus, tooltips, fields, tabs, dialogs and sheets), and the traps each one has already hit.
> **Status:** Current
> **Audience:** developers working on the desktop renderer, and agents editing `ui/desktop/src/components/ui/`

Every screen in the app draws its controls from `ui/desktop/src/components/ui/`, and the CSS that cannot be left to utility classes lives in `ui/desktop/src/styles/main.css`, in the blocks marked `@ws WS-PRIMITIVES`. The reasons for the numbers are in section 2.6 of [the redesign spec](../design/codex-simplicity-redesign/implementation-spec.md). This page is for the person who has to change one of them: what the control promises, what pins it, and what went wrong the last time.

## One control for each job

| Job | Control | Rule |
|---|---|---|
| A change that applies now | `Switch` | 32 by 20, a 16px knob inset 2px in both states, on the right of its row. Named by the row's visible label, never "Toggle …". |
| Picking items, or an option sent with a form | `Checkbox` | A 16px box in a 24px hit target, the label 8px to its right. |
| 2 to 4 short values that need no explanation | `SegmentedControl` | A radiogroup. One thumb slides under the choice and never uses the accent. The thumb is a step lighter than its track in both modes (`--background-default` in light, `--background-strong` in dark, where the default ground is darker than the track and the choice would read as a hole). |
| 4 or more values, or values that each need a line | `Select` or a menu | The chosen row carries a trailing 16px check. |
| Sections of a page | `Tabs` | One underline that travels under the active tab. |
| An explanation | `InfoTip` | See below. |

## InfoTip

An InfoTip is a visible 14px glyph after a label. It opens on hover after 200ms, at once when Tab moves focus onto it, and on a click, which pins it. It closes on pointer leave (with a 100ms grace), blur, Escape and scroll.

The words are always in the page: a visually hidden node holds them and the glyph points at it with `aria-describedby`, so a screen reader hears the help when it reaches the control without any hover. `useInfoTipId()` lets the explained control point at the same node.

- Never inside a `<label>` and never inside a row whose click toggles something. It sits next to the label, and a click on it stops there.
- Never `title=`. The tooltip layer copies a title into `aria-label` and renames the control.
- Never the only place an error, a refusal, a privacy disclosure or the consequence of a destructive action is written.
- The `asChild` form makes an existing badge the trigger. The target carries `biorouter-focus-region` so the base focus rule (a fill and an inset edge for every `[tabindex]`) stays off it and its outline is the only mark.

## Rows and row actions

A setting row is 40px at minimum with the label, an optional InfoTip, an optional status line and one control. Whatever the control's height (a 20px switch, a 28px select, a 32px field), a one-line row stays 40px: the control reaches into the row's padding with a negative margin.

The divider is a straight line drawn with `::after` and inset by the row radius. A bottom border on a rounded box curls up at both ends, and forced colours show it plainly.

A row washes and firms its line for keyboard focus only: `:focus-visible` on the row, or `:has(:focus-visible)` inside it. It was `:focus-within`, which is true for a switch the person has just clicked, so the row stayed washed and its line dark until focus moved. A click on a switch leaves nothing; Tab, an arrow key and a click into a text field still show the treatment. A row's `disabled` disables its control and dims only the row's words, because the control already dims itself to 50% and the two together put a locked switch at 25%.

`RowActions` shows its buttons when the row is hovered, holds focus, or has its menu open, and always under `(hover: none)`. Only opacity changes, so the buttons stay in the tab order. The optional `meta` (a relative time) shares the buttons' grid cell and fades out as they fade in. It must keep `pointer-events: none`: below opacity 1 it becomes a stacking context that paints above the opacity-1 buttons, and the invisible time ate the click on the `⋯` button.

## Menus

One row for the dropdown, the context menu, the command list and the Select options: 32px at minimum, 12px of padding, 13px text, a highlight of `--overlay-hover`. A selected checkbox or radio item shows a 16px check at the trailing edge, with no gutter on the left. A group label is 12px, sentence case, muted.

A menu opens with opacity, a scale from .97 and a 4px move toward its trigger, from the trigger's own transform origin, over 175ms. It closes with opacity alone over 125ms. The Select menu is not a Radix surface, so it plays the same values as a keyframe (`.br-select-menu`).

After a pointer pick the menu does not hand focus back to its trigger. Radix does that after the picked item unmounts, and Chromium then reads it as keyboard focus and draws the focus surface on the trigger. A key pick, Escape and a Tab out still return focus.

## Motion

Layout snaps and content glides, so a control moves only when the person chose something.

- The segmented thumb and the tab underline are measured with a `ResizeObserver`. A measurement the observer takes (a resize, a tab added before the active one, a label changing) lands at rest, and both carry `data-motion-layout`, which the resize rule uses to stop them while the window or the sidebar is being dragged.
- The first placement never animates. Until it is measured the selected segment paints its own ground.
- The underline is measured in the list's own pixels. A dialog pops in at scale .98, and `getBoundingClientRect` inside it is scaled, so the unscaled width of the list is used to undo that. Measured raw, the underline stayed about 2% off after the dialog settled.
- A switch whose value arrives without a touch (it loaded, or another window changed it) snaps. Its commit carries `data-motion-still`.
- Every duration is a `--dur-*` token and the curve is `--ease-out`. Reduced motion is handled by the global reset and by explicit rests for the loops.

## Focus

Focus is a surface shift plus an inset 2px edge, never a ring. Four rules keep it from appearing when a mouse did the work:

- A tooltip (`Tooltip`, `AppTooltipLayer`, `InfoTip`) opens on focus only when Tab moved the focus, so a dialog or menu handing focus back never pops it.
- A menu skips its focus return after a pointer pick, as above.
- A list or settings row takes its wash and firmed line from `:focus-visible`, never from `:focus-within` (see Rows and row actions).
- `DialogContent` and `SheetContent` give focus back to the control that opened them. Radix returns focus to a dialog's own `Trigger` and to nothing else, so a dialog opened from state left focus on the page body after Escape. The opener is read in `onOpenAutoFocus`, because the wrapper is mounted while the dialog is still closed. A caller's own `onCloseAutoFocus` runs first and wins when it cancels the event (Crew's dialogs do), and an opener that is gone is left alone. Whether the returned focus shows a ring is the browser's call from how the person closed it: a click on Cancel shows none, Escape does.

## Fields and the cascade

Text fields rest on `--border-control`, which clears 3:1 on every ground. The edge, and the defaults of a textarea (three rows, vertical resize), sit in `@layer components`. That matters in both directions. A layered rule lets a caller's `border-border-danger`, `min-h-*` or `resize-none` win, as a caller expects. An unlayered rule outranks every utility whatever its specificity: the first version of the edge hid the red border four forms pass in, and the first `.br-textarea` ignored `resize-none` and kept the workflow editors at three rows. The `Field` wrapper is `.br-form-field`, not `.br-field`, which belongs to the input alone.

react-select writes its own `display: block` on an option in an unlayered style sheet, which beats the row's `flex`. The Select sets `display: flex` in its styles API so the label and the check share a line.

## How these are tested

jsdom lays nothing out and runs no Tailwind, so a component test can only read class names. The acceptance numbers are measured in Chromium:

- `ui/controlsGeometry.browser.test.tsx` bundles the real components with esbuild, styles them with the authored blocks of `main.css`, and measures the switch, the menu row, the segmented thumb, the InfoTip, the tabs, the setting rows and the `⋯` hit target. Several cases strip the rule they guard from the stylesheet and expect the defect, so the instrument is proved.
- `styles/crewParity.test.ts` holds each promoted recipe to its Crew original, with the differences the redesign made on purpose written down.
- `ui/controlsSourceGuard.test.ts` fails on a hand-rolled checkbox, radio or switch, an `@radix-ui/themes` import, a `title=` on a menu item and a Switch `variant` outside `ui/`. The `variant` prop itself is gone, so the type checker refuses it too. A file waiting on its owner is listed in `PENDING`, and the list only shrinks; it is empty now.

To look at the components under the whole stylesheet, compile `main.css` with Vite (`server.transformRequest('/src/styles/main.css?direct')` on a middleware-mode server that loads `vite.renderer.config.mts`), bundle a small page of components with esbuild, and drive it with Playwright's Chromium using real mouse and keyboard input. `element.click()` and `focus()` from a script do not reproduce focus rings, which depend on how the focus arrived.

## Related documentation

- [Implementation spec, section 2.6](../design/codex-simplicity-redesign/implementation-spec.md): the numbers above and why they were chosen.
- [The settings visual vocabulary](settings-visual-vocabulary.md): the rules Settings and the component views are built to, which lean on these controls.
- [Renderer testing traps](renderer-testing-traps.md): other ways a frontend test passes while the code it covers is broken.
