# Biorouter frontend design language

Read this before any UI change in `ui/desktop/src`. It is a short copy of the root `design.md`
(Parts 1 to 3, and Part 9 for the 2026-10 redesign), regenerated at the end of that redesign's
wave 2. When they disagree, `design.md` wins, and its Part 9 wins over every earlier part.

## Design philosophy

> A quiet instrument: a warm, low-chroma, flat research console. Colour is evidence, not decoration.

- **Simplicity first.** Codex's structure and restraint, Biorouter's coral kept. When unsure, copy
  Crew (`components/crew/`) and never edit Crew to match; promote a Crew recipe under a `br-` name.
- **One reading column, quiet chrome.** Every surface has one 44px band with a 14px title and 32px
  icon actions. Content sits in the 760px measure. Home is the one exception.
- **Say it once, then hide the explanation.** A visible line is a label, a value or a state word;
  explanations go in an `InfoTip`. Errors, refusals, privacy disclosures, the consequence of a
  destructive action and empty-state one-liners always stay visible.
- **Rows, not cards.** Anything enumerable is a hairline row list, one primary line and at most one
  secondary line. A card is for something that asks the person to act. Never a box inside a box.
- **One job, one control, one look.** A variant lives in the primitive, never in a `className`
  override. Show names, not IDs: machine IDs and paths go behind a "Copy …" item or a tooltip.
- **Simplicity never costs accessibility.** Everything hidden stays reachable by keyboard and by
  screen reader, and the contrast gates pass.

## Colour tokens (`ui/desktop/src/styles/main.css`)

Write the token, never a hex. The neutrals are shared by all three theme families (Parchment, Alma
Mater, Roche Limit); a family varies only its ink, accent and status hues. Values are Parchment's.

| Utility | Light | Dark | Use |
|---|---|---|---|
| `bg-background-canvas` | `#ffffff` | `#131312` | Main panel |
| `bg-background-default` | `#ffffff` | `#1b1b19` | Cards, popovers, dialogs |
| `bg-background-muted` | `#f4f4f2` | `#232320` | Page ground, one real step off the canvas |
| `bg-background-well` | `#f5f5f3` | `#232320` | Code blocks, inline code |
| `tint-interactive` | ink at 5% | ink at 5% | Hover (`--overlay-hover`); press is 10% plus scale 0.98 |
| `border-border-subtle` | `#e4e4e0` | `#302f2c` | The default hairline |
| `border-border-control` | `#86847f` | `#817f7a` | Resting edge of inputs, checkboxes, radios, selects |
| `text-text-default` | `#2a2520` | `#f4f0e6` | Primary text |
| `text-text-muted` | `#635c54` | `#b0a892` | Secondary text, metadata |
| `text-text-subtle` | `#6e6760` | `#9c937b` | Quietest text, the InfoTip glyph at rest |
| `bg-background-accent` | `#b85a32` | `#e8895f` | The one primary button; on switches and checkboxes |
| `--accent-bar` | `#cf6d47` | `#e8895f` | The 2px active-row rail and tab underline only |

Status is a wash with its own ink: `bg-wash-danger` with `text-text-danger`, and the same for
success, warning and info. A colour a utility names needs a `--color-*` mirror in `@theme inline`.
Families are generated from `ui/desktop/themes/*.theme.mjs` by `npm run themes`.

## Shadow policy

Depth is a 1px `--border-subtle` hairline plus a step of ground colour. Hairlines are straight: no
`border-bottom` on a rounded row.

| Token | Use |
|---|---|
| **No shadow** | Cards, rows, page sections, bands, panels, tab strips |
| `--shadow-composer` | The chat composer |
| `--shadow-popover` | Menus, popovers, toasts, the floating message actions |
| `--shadow-modal` | Dialogs and sheets |
| `--shadow-raised` | The segmented control's thumb |

## Type

Arial first, byte-equal to the website's `--font` (`landing/site.css`); one mono stack, for machine
strings only. Five sizes and two visible weights, 400 and 600 (Arial draws 500 as 400). Use a role
utility: `styles/typeScale.test.ts` fails on `text-xs` and its siblings, `text-[Npx]`, `tracking-*`,
a bare `uppercase` and `font-bold`.

| Utility | Size / line, weight | Use |
|---|---|---|
| `text-supporting` | 12/16, 400 | Metadata, times, counts, helper and tooltip text, table heads |
| `text-chip` | 12/16, 500 | Badges and chips, sentence case |
| `text-caps` | 12/16, 500, caps +0.04em | Page section labels only |
| `text-secondary` | 13/18, 400 | Sidebar and menu rows, tab labels, table cells, composer controls |
| `text-code` | 13/20 mono | Code and terminal |
| `text-body` / `text-label` | 14/20, 400 / 500 | UI prose and values / control text, row and band titles |
| `text-prose` | 14/21, 400 | Chat markdown, documents, Crew messages |
| `text-subheading` | 17/24, 600 | Dialog and empty-state titles, chat h1 and h2 |
| `text-title` | 24/32, 400 | Home greeting, document h1, stat values |

Counts, dates, costs and percentages use the sans face with `tabular-nums`. Chat code blocks and
tables use the shared `.br-md-code` and `.br-md-table` recipes.

## Spacing, rows and radii

| What | Value |
|---|---|
| Spacing | 2, 4, 6, 8, 12, 16, 20, 24, 32; 8 is the default gap and 16 the default inset |
| Bands (`h-chrome`) | 44px; the three shell bands move together |
| Dock strips (`h-dock`), tabs (`h-tab`) | 36px, 32px |
| Rows | `h-row-nav` 28 (app sidebar, 13px), `h-row-rail` 32 (menus, Crew), `min-h-row` 40 |
| Controls | `h-control-compact` 24, `h-control-sm` 28, `h-control-md` 32, `h-control-lg` 36 |
| Icons | 14 in a chip and the InfoTip, 16 in rows and controls, 20 in banners and empty states |
| Radii | `rounded-inner` 4, `rounded-element` 8, `rounded-container` 12, `rounded-full` |

A nested radius is the outer radius minus the padding. `rounded-surface` (16px) is the preview
sheet only; `rounded-2xl` is banned outside `artifacts/`.

## Controls: one job, one control

Use the primitive in `components/ui/`, never a raw `<button>` or `<input>` with its own classes.

- `Switch` when the change applies now, on the right, named by the row's visible label (no
  "Toggle …", no "Enable …"). `Checkbox` to pick items or for a form option, on the left.
- `SegmentedControl` for two to four short options; it never uses coral. `Select` for four or more,
  or options that need a line each. `CustomRadio` only in dialogs and forms. Underline `Tabs`.
- Menus: 32px rows at 13px, a trailing check on the selected item, sentence-case group labels.
- `Button`: `default` is the one coral button per view, `secondary` the usual second action,
  `ghost` for icon actions, `destructive` a danger wash. Fields are 32px with the control edge.
- `InfoTip` for help: a 14px glyph visible at rest, plain text, at most two sentences, never inside a
  `<label>`. Help text never goes in `title=`.
- `SettingRow` and `SettingSection`: a label, an InfoTip, one control at the trailing edge; a
  `text-caps` section label with no paragraph under it.
- `RowActions`: shown on hover, `:focus-within`, an open menu and always under `(hover: none)`;
  right-click opens the same menu. Only opacity changes.
- `FilterInput` (28px, "Filter", ⌘F), `PageHeader` for a view's band, `ModalShell` for dialogs.

Focus follows D-15: a control shifts its fill (`.biorouter-focus-surface`), a row takes
`--background-focus` and an inset 2px `--border-focus` edge, a tab firms its underline. Never write
`outline-none`, `focus:ring-*` or `focus-visible:ring-*` at a call site.

## Coral accent

Coral marks state, nothing else: the active-row rail, the focused field and composer edge, the one
primary button, a ready Send button, switches and checkboxes when on, the active tab underline, the
in-progress to-do glyph, the working sweep, links, selection, the heatmap ramp, "New" or unread
marks and the private-chat lock badge. Hover is always neutral. Never a coloured divider, section
header or card background.

## Motion

Layout snaps and content glides. Animate only `transform`, `opacity` and `clip-path`, with tokens:
`--dur-fast-min` 95ms, `--dur-fast` 125, `--dur-fast-max` 175, `--dur-med-min` 250, `--dur-med` 300,
`--dur-slow` 525 (ambient loops only); `--ease-out` for everything, `--ease-spring` for physical
gestures only. In TypeScript use `styles/motion.ts`; shared classes are `.br-enter`, `.br-crossfade`
and `.br-highlight`. Exits are one tier faster than entrances. Nothing animates during a resize, on a
first placement or after navigation, and a row never moves under the pointer. Reduced motion is
honoured in CSS, JS and the main process.

## Copy

Sentence case. The noun is "chat"; the names are "Biorouter" and "BAAM". American spelling and the
typographic ellipsis. No em or en dashes as punctuation, no marketing words. A one-line menu label
takes no trailing period. InfoTip text is at most two full sentences, about 160 characters.

## Checklist

- [ ] Tokens only: no hex, no `rgba()`, no Tailwind grey such as `gray-*` or `slate-*`.
- [ ] Type roles only, and `npx vitest run src/styles/typeScale.test.ts` passes.
- [ ] No shadow on a card, row or section, and no box inside a box.
- [ ] Coral only where the list above allows it; every hover neutral.
- [ ] Each job uses its one primitive from `components/ui/`, with no visual override.
- [ ] Explanations sit in an `InfoTip`; errors, refusals and privacy text stay visible.
- [ ] Hidden actions are reachable by keyboard and screen reader.
- [ ] Checked in light and dark, in all three families; `npm run lint:check` passes.
- [ ] A rule something depends on is authored CSS, not a new Tailwind class.

## Where things live

| What | Where |
|---|---|
| Tokens and shared classes | `ui/desktop/src/styles/main.css` |
| Primitives | `ui/desktop/src/components/ui/` |
| Icons | `components/icons/app-icons.tsx`; one glyph per concept in `entity-icons.ts` |
| The reference | `ui/desktop/src/components/crew/` |
| The full record | `design.md` Part 9 and `docs/design/codex-simplicity-redesign/` |
