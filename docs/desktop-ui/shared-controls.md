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
- Its name is "About" and the label's words. When the label is a node rather than a string, the words it shows are read from it (`plainTextOf`), and `Field` takes an explicit `infoLabel`. A field id is never a name: the workflow editor's "Initial prompt (optional)" was announced "About workflow-prompt" (V3-R3-3).
- Its focus mark is a 20px box 2px inside the 24px hit box (`::before`), not the hit box itself. The hit box starts 1px before the label's end, so a mark on it lay over the label's last letter (V4-R3-1). The OS ring under `prefers-contrast: more` and forced colours is drawn on the same 20px edge.

## Rows and row actions

A setting row is 40px at minimum with the label, an optional InfoTip, an optional status line and one control. Whatever the control's height (a 20px switch, a 28px select, a 32px field), a one-line row stays 40px: the control reaches into the row's padding with a negative margin.

In a `SettingRow` the value and the control sit on the label's line, not on the row's middle. A status line ("Restart to apply", "Checking for updates…") grows the row downward, and nothing in the row moves: centred on the grown row, the switch a person had just clicked slid 8px down under the pointer (V4-R3-5). A hand-built row with a permanent second line, such as a local model's facts, is not a `SettingRow` and keeps its control centred.

The divider is a straight line drawn with `::after` and inset by the row radius. A bottom border on a rounded box curls up at both ends, and forced colours show it plainly.

A list whose rows read apart by air and the hover wash drops the divider with `.br-rows-flat` on its shell or any ancestor (R-27: "row lists drop inter-row hairlines where spacing and the hover fill already separate rows"). Only the `::after` goes, so no row moves. Forced colours repaint the wash as Canvas, so the divider comes back there, and the last row still has none. The divider stays the default because it is Crew's recipe (`styles/crewParity.test.ts`); Workflows, the Scheduler, Built apps and the Settings dialogs opt out.

A washed row (under the pointer, or holding keyboard focus) is a fill, so its controls step back one ground, as they do in a filled card: a secondary button or a Select in it rests on the page ground and hovers on the strong step, and a neutral progress track in it steps one in. The wash is the page ink at 5%, which lands on the rest fill's own value in light, so without this a button lost its box the moment the pointer reached its row. A neutral badge steps back with them: it rests on the strong step, which the wash lands 1.10:1 off in light and 1.08:1 in dark, so a "Built-in" or a change's kind read as bare ink under the pointer (V3-R2-5). The badge paints through `.br-badge-neutral`, which reads `--br-badge-ground`, and the washed row sets that to the page ground (1.10:1 light, 1.14:1 dark off the wash). The privacy pills (`PrivacyBadge`, `AffiliationBadge`) rest on `--background-muted`, the rest fill's value, and take the badge's ground on a washed list row, so one row is one ground; not on a settings row, where Crew draws its one-piece "Private · UCSF" chip.

A row washes and firms its line for keyboard focus only: `:focus-visible` on the row, or `:has(:focus-visible)` inside it. It was `:focus-within`, which is true for a switch the person has just clicked, so the row stayed washed and its line dark until focus moved. A click on a switch leaves nothing; Tab, an arrow key and a click into a text field still show the treatment. A row's `disabled` disables its control and dims only the row's words, because the control already dims itself to 50% and the two together put a locked switch at 25%.

`RowActions` shows its buttons when the row is hovered, holds focus, or has its menu open, and always under `(hover: none)`. Only opacity changes, so the buttons stay in the tab order. The optional `meta` (a relative time) shares the buttons' grid cell and fades out as they fade in. It must keep `pointer-events: none`: below opacity 1 it becomes a stacking context that paints above the opacity-1 buttons, and the invisible time ate the click on the `⋯` button.

## Menus

One row for the dropdown, the context menu, the command list and the Select options: 32px at minimum, 12px of padding, 13px text, a highlight of `--overlay-hover`. A selected checkbox or radio item shows a 16px check at the trailing edge, with no gutter on the left. A group label is 12px, sentence case, muted.

A menu opens with opacity, a scale from .97 and a 4px move toward its trigger, from the trigger's own transform origin, over 175ms. It closes with opacity alone over 125ms. The Select menu is not a Radix surface, so it plays the same values as a keyframe (`.br-select-menu`).

After a pointer pick the menu does not hand focus back to its trigger. Radix does that after the picked item unmounts, and Chromium then reads it as keyboard focus and draws the focus surface on the trigger. A key pick, Escape and a Tab out still return focus.

A dropdown menu opened with the pointer rings no row. The trigger opens on a cancelled `pointerdown`, so Radix's script focus on each row the pointer crosses read as keyboard focus and the row wore the 2px inset ring. The menu surface records how it is driven, `data-menu-input="pointer"` or `"key"` (from `ui/inputTrail.ts` when it opens, then from the keys and presses inside it), and a pointer menu's rows keep only their hover highlight. The first key pressed in the menu brings the ring back; a menu opened with Enter, Space or an arrow keeps it; `prefers-contrast: more` and forced colours keep their outline. A right-click context menu never had the problem and is unchanged.

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
- A dialog whose field takes React's `autoFocus` has focus inside before Radix's focus scope mounts, so Radix never fires `onOpenAutoFocus` and the opener was never read: Escape left focus on the page body (WS-SKILLS-2). The content reads its opener when its ref attaches instead, as the control focus left to reach the field (`focusArrivedFrom` in `inputTrail.ts`), resolved like any other opener, so a menu pick still returns to the menu's trigger. `ui/dialogAutoFocusReturn.browser.test.tsx` drives it; Radix's own content leaving the body is its instrument.
- A dialog opened from a menu item follows the menus' own pointer rule. The item is gone by the time the dialog closes, so the opener is the control that opened the menu: a DropdownMenu names its trigger (`aria-labelledby`, or `aria-controls` when a caller's own `id` replaced the one Radix uses), a submenu climbs to the root trigger, and a ContextMenu, which names nothing, returns to the control that held focus when it opened (`inputTrail.ts`). A pick made with the keyboard and closed with the keyboard gives focus back, so ⋯, Enter, Enter on "View info", Escape lands on the ⋯. A pick made with the pointer gives nothing back, and opens quietly: after a pointer pick every script focus matches `:focus-visible` in Chromium, so a Cancel button or a link wore the ring after a mouse click. Focus goes to the dialog box, which draws none (`.biorouter-modal-surface:focus`), unless the dialog opens on a text field, which is ringed in every modality anyway, or its caller cancels `onOpenAutoFocus` and places focus itself. `ModalShell` and `ConfirmationModal` both pass `onOpenAutoFocus` and `onCloseAutoFocus` through. `ui/dialogFromMenu.browser.test.tsx` drives it with real keys and mouse.
- Any pointer open is a quiet open, from a menu or not (V4-R5-1). A click on a trigger that already holds keyboard focus (Escape gave it back, ringed, and the mouse then opened the dialog again) changes no focus, so Chromium keeps the keyboard modality and Radix's auto-focus inherited the ring: Reset data's "Select all", Configuration's "Go to Privacy", Usage's InfoTip. So after a pointer press, `DialogContent` and `SheetContent` put focus on the box unless the first stop (the control Radix would focus: the first tabbable, links left out) is a text field. The shared `Select`'s typing input does not count as one, because it reads as a closed control. A key open keeps Radix's first stop, ringed, and a caller that cancels `onOpenAutoFocus` still places focus itself. `ui/dialogPointerOpen.browser.test.tsx` drives it; its instrument is Radix's own content ringing its first button on the same reopen.

## Dialog placement

`ModalShell` pins every dialog's top edge at `max(10vh, 48px)` and draws the header hairline, on any surface, because Crew's dialogs are the reference design. A dialog whose height changes while it is open (a tab, a result replacing a form, an error) grows downward instead of re-centring under the pointer. A dialog that reads wrong pinned to the top passes `anchor="center"`, with a one-line reason beside it, and one that wants no hairline passes `headerRule={false}`. A `ModalShellDefaultsContext` still overrides both for an area, and a prop beats the context. `ModalShell.test.tsx` pins the defaults, the opt-outs and the precedence.

An InfoTip beside a dialog's title goes in `titleInfo`, never inside `title`. The dialog is named by its heading, and an InfoTip inside the heading would add its "About …" name and its hidden help to the dialog's name. "Report a problem" (`ui/Diagnostics.tsx`) was on a raw `DialogContent`: centred, no header rule, a 36px warning tile before the title and 28px footer buttons (V4-R4-2). It is now `ModalShell` md like the rest, its footer is three word-only default buttons (with their glyphs they wrapped at 480px under Large text), and it opens with focus on the dialog box, because Radix's first tabbable there is the title's InfoTip. `ui/diagnosticsDialog.browser.test.tsx` measures it in Chromium.

The typed confirmation `ui/DangerousConfirmDialog.tsx` is a `ModalShell` sm too, like `ConfirmationModal` (V3-R7-1): "Make "Soul" public?" sat centred at y=262 in a 900px window with 24px insets and no header rule, while every other dialog opened from the Knowledge band sat at y=90. Its title is in the header, its message is the description in body text, and its focus rule is unchanged (Cancel after a key open, the dialog box after any other). Inside Crew it takes `CrewDialogs`' defaults, as `ConfirmationModal` does. `ui/dangerousConfirmGeometry.browser.test.tsx` measures it against `ConfirmationModal` in Chromium, light and dark. `ui/WorkflowWarningModal.tsx` moved onto `ModalShell` lg with a scrolling body in the same pass. A dialog a test finds by `data-testid` passes `testId` to the shell. The raw `DialogContent` sites left are in frozen privacy and model files; the `p-6` default goes once they move.

## A Button inside a Radix trigger

`Button` writes `data-slot="button"` after the props it receives. A Button wrapped by `<DropdownMenuTrigger asChild>`, `TooltipTrigger`, `DialogTrigger` or any other trigger is handed that trigger's own `data-slot` through Radix's Slot, and a slot written before the spread lost to it. The element was drawn as a button but no longer matched `[data-slot='button']`, which is where `main.css` puts the forced-colours edge, the link underline and the copy field's footer, so a filled trigger had no boundary in Windows High Contrast (about sixty sites). Nothing keys on a trigger's slot, so a Button is always a button. `button.test.tsx` pins the order of the two.

## A Button that cannot act

Colour on a button means "this will do something", so a button that cannot is not coloured (vision V4-R7-4, coordinator ruling 22). A disabled `default` (coral) or `destructive` (danger) `Button` draws what a disabled `secondary` draws: the control ground, the default ink and the 50% fade `Button` already gives every disabled variant. The two variants carry the class `.br-button-inert` (`button.tsx`); the rule is authored in `main.css` beside `.br-button-xs`, unlayered, because the variants' own `bg-*` and `text-*` utilities sit in the `utilities` layer and would beat a layered rule. Its ground is `--br-control-ground`, the variable a secondary rests on, so the neutral button steps back to the page inside a filled card and forward on a recessed ground exactly as the Cancel beside it does.

Before it the app had two looks for "not yet". The provider dialogs draw a secondary until their fields are filled (first-run FR-R3-3: Configure's Save, Add provider's Create provider, Detect provider, Versa's Connect), while Tool permissions' "Save changes", Make workflow's "Create and run" and Reset data's "Reset" drew a pale coral or a faded danger wash. The two rules are one rule with two doors:

- **It cannot be pressed.** Set `disabled`. The primitive draws it neutral; no dialog needs a line for it.
- **It can be pressed and has something to say** (a click names the field that is missing, a browser hears the host-managed refusal). Pass `variant="secondary"` until it can act, as FR-R3-3 does, and leave it enabled.

A button that is WORKING and says so with `aria-busy="true"` keeps its colour: the composer's Send is disabled while a chat starts and has no label to say "Sending", only a spinner, so it stays the accent. A labelled button that is disabled while it works goes neutral like any other disabled button, and its label ("Saving…", "Resetting…") carries the progress. `aria-disabled="true"` is covered like `disabled`, for the buttons that fade without the attribute because a disabled button would hand its focus to `<body>`. The other variants are untouched: a disabled `ghost` stays transparent and a disabled `outline` is the secondary box it always was. Forced colours repaint the fill and the ink, so the system button edge every `[data-slot='button']` takes is all the rule leaves them.

## Fills before lines

Since owner message 13 (R-27, [design.md 9.11](../../design.md)) the in-page controls are a subtle solid fill with no resting outline. Floating surfaces (menus, popovers, tooltips, dialogs, toasts) keep their hairline and shadow, and the row lists keep their straight hairline, which is Crew's recipe, unless a list opts out with `.br-rows-flat` (Rows and row actions, above).

| Control | At rest | Hover and press | Edge |
|---|---|---|---|
| `Input`, `Textarea`, `FilterInput`, `SecretInput`, the `Select` trigger | `--br-control-ground` | `--background-fill-strong`, but not while focused or disabled | 1px, transparent; `--border-focus` while focused, `--border-danger` while invalid (it wins) |
| Button `secondary`, and `outline` (now the same box) | `--br-control-ground` (`.br-fill-rest`) | `--background-fill-strong` (`.br-fill-hover`), with the 0.98 press | none; `outline` keeps a transparent 1px border so nothing that used it reflows |
| `Badge` `neutral` | `--background-fill-strong` | | none |
| `Card`, a neutral `Note`, `.biorouter-modal-panel` and `-row` | `--background-fill` | | 1px, transparent |
| `EmptyState`'s mark tile (`.br-empty-tile`) | `--br-control-ground` | | 1px, transparent |
| `SegmentedControl` track | `--br-track-ground`, else `--background-fill` (the value `--background-muted` had) | | none; the thumb stays the raised surface |
| `Progress` neutral track (`.br-progress-track`) | `--br-track-ground`, else `--background-muted` | | none |
| `CopyField` | `--br-control-ground`, else `--background-well` | | 1px, transparent |

`--br-control-ground` is a private variable, `--background-fill` unless something sets it. A fill never sits on a fill: a `Card`, every `Note`, a dialog's inner panel and row, and any filled box that adds the class `br-fill-surface` set it to `--background-default` for their children, so a field or a secondary button inside rests one step back on the page ground and needs no class of its own. It is a private variable because the shared fill tokens are declared once per mode and nowhere else (`styles/themeNeutrals.test.ts`). Inside a dark popover the strong step is re-pointed one step deeper than the popover ground, which is why the filled button steps between two tokens instead of taking `tint-interactive`'s ink overlay: an overlay would lighten it into the ground.

The neutral badge rests on the strong step, not the rest fill, because a badge sits on rows that wash on hover and the hover wash has the rest fill's value. On the rest fill a "Built-in" badge vanished into its row the moment the pointer arrived. While its row is washed it steps back to the page ground (see Rows). The fill is the authored class `.br-badge-neutral`, layered in `components`, so a caller's own `bg-*` still wins and no ground needs a Tailwind class to have been scanned.

Every edge that is kept transparent is still a border, never `border-0`: forced colours repaint the fill as Canvas and draw a transparent border in the system ink, so the box keeps its outline there, and nothing moves when an edge appears. A field is not found by its fill alone: its label or placeholder and its focus edge carry it, which is the trade-off design.md 9.11 records.

Three private variables say what a ground asks of the primitives on it, and each primitive reads them with its old value as the fallback, so a plain page draws what it drew:

| Variable | Read by | Filled box (`Card`, `Note`, modal panel and row, `.br-fill-surface`, a washed row) | Recessed ground (`.br-ground-recessed`) |
|---|---|---|---|
| `--br-control-ground` | fields, the Select trigger, `secondary` and `outline` buttons, `CopyField`, `EmptyState`'s tile | `--background-default`, one step back | `--background-fill-strong`, one step in |
| `--br-control-hover` | a filled button's hover and press (`.br-fill-hover`) | `--background-fill-strong` | `--background-strong`, one deeper again |
| `--br-track-ground` | the segmented track, a neutral progress track | `--background-fill-strong`, one step in | `--background-fill-strong` |

A track goes one step in rather than back because a raised or coloured mark rides it: a white track would hide the white segmented thumb. A field's hover stays on the strong step on a recessed ground, where it equals the field's rest, because no deeper neutral keeps the placeholder at 4.5:1.

A recessed ground is one where the rest fill is the ground itself: the sidebar (1.03:1), `--background-muted` (1.00:1: first run, the Knowledge graph canvas, the terminal dock) and the page band. Put `.br-ground-recessed` on the ground; a filled box inside it sets its own grounds again for its children. `ui/groundsAndRows.browser.test.tsx` measures every cell of the table in light and dark.

Crew is not converted. Its own recipes (`crew-*` classes) keep their lines; the shared primitives it renders take the fill like everywhere else.

A `Note`'s action is centred on the note's first line (`.br-note-action`): a 28px row pulled in by half its difference from the 12/16 line, so a one-line note with a Retry is as tall as one without (36px) and the words sit level with the button instead of 6px above its centre.

Inside a dark popover the ground is lifted to `--background-medium`, so the hairlines inside it are lifted too: `--border-subtle` and `--border-default` point at `--border-strong` there. On the lifted ground the old separator measured 1.05:1 and vanished; `--border-strong` gives back the 1.29:1 it had on the page.

## Fields and the cascade

The field's fill and edges, and the defaults of a textarea (three rows, vertical resize), sit in `@layer components`. That matters in both directions. A layered rule lets a caller's `border-border-danger`, `bg-*`, `min-h-*` or `resize-none` win, as a caller expects. An unlayered rule outranks every utility whatever its specificity: the first version of the edge hid the red border four forms pass in, and the first `.br-textarea` ignored `resize-none` and kept the workflow editors at three rows. A caller's `bg-*` on a filled control also beats the layered hover, so a control given its own ground needs its own hover. The `Field` wrapper is `.br-form-field`, not `.br-field`, which belongs to the input alone.

The Select's trigger is react-select's, which moves focus to a hidden input, so `.br-field` cannot reach it: its fill, its hover and its edges are written in the styles API, as Emotion's unlayered rules.

react-select draws its own filled chevron and cross, which read heavier than every other glyph. The Select replaces both with the app's 16px stroke 1.5 icons (`ChevronDown`, `X`) through its `components` prop, ahead of whatever a caller passes, and the clear mark is muted until the pointer is over it.

react-select writes its own `display: block` on an option in an unlayered style sheet, which beats the row's `flex`. The Select sets `display: flex` in its styles API so the label and the check share a line.

Escape with a Select's menu open closes the menu and nothing else. A react-select menu is not a Radix layer, so inside a dialog the dialog is the top layer, and Radix listens for Escape in the capture phase on the document, before react-select sees the key: one Escape used to close Switch models with the provider menu still open. The Select claims that Escape first, from a window capture listener, when the key's target is its own combobox (`.br-select`) and the combobox says `aria-expanded="true"`. It cancels the event, which Radix and every other surface leave alone, and react-select closes the menu. The class scopes the claim, because cmdk's input says expanded for as long as it is mounted. A caller that passes its own `onKeyDown` would stop react-select closing the menu on that cancelled key; none does.

A pick leaves focus on the Select, as a Radix control gives focus back to its trigger. The wrapper used to turn react-select's `blurInputOnSelect` on for every caller, so a pick blurred the field, and inside a dialog Radix's focus trap moved the lost focus onto the dialog box, which shows no ring. In New schedule, ArrowDown and Enter on "AM or PM" picked AM and left focus nowhere a person could see, and the next Tab went back to Name (V3-R8-2). The blur is now off unless a caller passes `blurInputOnSelect`, and none does: the trigger keeps its focus edge and Tab goes on to the next control.

A group heading is drawn only for a group that has a name. react-select draws a `GroupHeading` for every group, and an unlabeled one is an empty 8px row above the first option: Switch models' Model field passes its options as one unlabeled group, so its menu opened with the first row 13px below the surface's top against the Provider menu's 5px (the 1px border and the 4px list padding) in the same dialog (V4-R8-1). The Select now returns no heading for an empty or blank label, and a caller's own `GroupHeading` still replaces it. `ui/selectGroupHeading.browser.test.tsx` measures a flat list, one unlabeled group (portalled, with the model picker's two-line option label) and named groups in Chromium; its instrument puts the library's heading back and reads the 13px. `ui/Select.groupHeading.test.tsx` is the jsdom half.

The filter field's tooltip ("Filter · ⌘F") is a hover hint for a field the person is not in. It never opens on focus, Tab included, or while the field holds focus, and focus closes it: it used to sit under History's band while the person typed. There is none when the placeholder already names the shortcut (History's "Search history… ⌘F") or when ⌘F is off. The shortcut reaches assistive tech as `aria-keyshortcuts`.

## How these are tested

jsdom lays nothing out and runs no Tailwind, so a component test can only read class names. The acceptance numbers are measured in Chromium:

- `ui/controlsGeometry.browser.test.tsx` bundles the real components with esbuild, styles them with the authored blocks of `main.css`, and measures the switch, the menu row, the segmented thumb, the InfoTip, the tabs, the setting rows and the `⋯` hit target. Several cases strip the rule they guard from the stylesheet and expect the defect, so the instrument is proved.
- `ui/filledPrimitives.browser.test.tsx` compiles the real `main.css` with Tailwind (the classes the bundle uses) and paints the filled primitives in light, dark and forced colours: the rest fill and the hover step, the focus and danger edges, the step back to the page ground inside a filled box, a badge on a hovered row, a button in a dark popover, the transparent border drawn in forced colours at no cost in size, and a note's action level with its first line. Each assertion was proved by removing the rule it guards from the compiled sheet.
- `ui/disabledPrimary.browser.test.tsx` compiles the real `main.css` the same way and paints a disabled `default` and `destructive` next to a disabled `secondary` (same ground, ink, fade and box, in light and dark), an enabled primary (still coral and danger), a working one (`aria-busy` keeps the colour, `aria-busy="false"` does not), the `aria-disabled` form, the ground in a filled card, on a recessed ground and in a popover, the variants the rule must not touch, and the system edge under forced colours. Seven mutations of the rule and the class each turn two to six of its cases red.
- `ui/dialogFromMenu.browser.test.tsx` bundles the real menus and dialogs and drives them with Playwright's keyboard and mouse: where focus lands when a dialog that a menu item opened closes, and whether its first stop wears a ring after a mouse pick.
- `ui/menuPointerRing.browser.test.tsx` drives a dropdown menu, a submenu and a context menu with the real mouse and keys under the compiled `main.css`: no ring on a row the pointer crosses, the ring back on the first key, kept for a keyboard-opened menu and for `prefers-contrast: more` and forced colours. Its first case runs without the rule and sees the ring.
- `ui/groundsAndRows.browser.test.tsx` compiles `main.css` the same way and measures the grounds table above on a page, in a card, on a recessed ground and in a card on it, a button, a neutral badge (with its painted contrast against the wash) and the real privacy pill in a washed list or settings row, and the flat rows' divider in light and forced colours. `ui/privacyPill.test.ts` holds the pill's one rule off settings rows; `ui/badge.test.tsx` holds the neutral tone to the ground-reading class with no fill utility.
- `ui/settingRowStability.browser.test.tsx` compiles `main.css` with Tailwind and the renderer's text-size pass and drives settings rows with the real keyboard and pointer, at standard and Large text: a Tab onto an InfoTip changes no pixel of the label in light, dark, `prefers-contrast: more` and forced colours, and a status line leaves the clicked switch, the value and the button where they were. Its instrument cases put the old rules back and see the overlap and the 8px slide.
- `ui/filter-input.browser.test.tsx` drives the filter field with the real pointer and keys: the hint on hover over a field the person is not in (the instrument case), and none after a click, a leave and a return, on a Tab, under a placeholder that carries the shortcut, or with ⌘F off. `ui/Select.escape.test.tsx` holds Escape to the open menu inside a dialog, and leaves a closed Select's Escape and another combobox's to the dialog. `ui/selectFocusAfterPick.browser.test.tsx` picks by key and by pointer in the real `ModalShell` and reads where focus lands; its instrument is the same Select with the blur passed, which reproduces the finding. `ui/Select.focusAfterPick.test.tsx` is the jsdom half.
- `ui/diagnosticsDialog.browser.test.tsx` bundles the real "Report a problem" dialog (daemon client stubbed, the private-chat answer released by the test), compiles `main.css` with Tailwind and the text-size pass, and measures the top edge before and after the private warning arrives, the header rule, the title at the gutter with no tile, and the footer's 32px buttons on one row at standard and Large text. Point `DIAGNOSTICS_SOURCE` at another version of the component to see the cases fail on it.
- `styles/crewParity.test.ts` holds each promoted recipe to its Crew original, with the differences the redesign made on purpose written down.
- `ui/controlsSourceGuard.test.ts` fails on a hand-rolled checkbox, radio or switch, an `@radix-ui/themes` import, a `title=` on a menu item and a Switch `variant` outside `ui/`. The `variant` prop itself is gone, so the type checker refuses it too. A file waiting on its owner is listed in `PENDING`, and the list only shrinks; it is empty now.

To look at the components under the whole stylesheet, compile `main.css` with Vite (`server.transformRequest('/src/styles/main.css?direct')` on a middleware-mode server that loads `vite.renderer.config.mts`), bundle a small page of components with esbuild, and drive it with Playwright's Chromium using real mouse and keyboard input. `element.click()` and `focus()` from a script do not reproduce focus rings, which depend on how the focus arrived.

## Related documentation

- [Implementation spec, section 2.6](../design/codex-simplicity-redesign/implementation-spec.md): the numbers above and why they were chosen.
- [The settings visual vocabulary](settings-visual-vocabulary.md): the rules Settings and the component views are built to, which lean on these controls.
- [Renderer testing traps](renderer-testing-traps.md): other ways a frontend test passes while the code it covers is broken.
