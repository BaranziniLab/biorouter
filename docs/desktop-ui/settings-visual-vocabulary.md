# The settings visual vocabulary

> **What this is.** The nine rules that govern how the desktop Settings view (Models, Chat, App) is built, and the two primitives they lean on — a living reference for anyone adding or changing a control there.
> **Status:** Current.
> **Audience:** contributors working on the desktop renderer.

Settings is a column of labelled rows. Almost everything in it is a section header over a
hairline list, a row with a control on its trailing edge, a strip of section-level
buttons, or an in-place note. Every rule below exists because a call site painted one of
those four things itself instead of reaching for the shared one, and the drift became
visible the moment two of them sat on screen together.

Nothing here is a new visual language. Each rule is derived from a document already in the
repo — [`design.md`](../../design.md) (the Parchment design system),
[`docs/design/astryx-adoption/astryx-ui-adoption-design.md`](../design/astryx-adoption/astryx-ui-adoption-design.md)
(the design of record for the token and primitive layer), and the settings block in
`ui/desktop/src/styles/main.css` with its own comments.

Six of the rules are enforced at the source by
`ui/desktop/src/components/settings/settingsVocabulary.test.ts`. That is a source test and
not a render test on purpose: jsdom never runs Tailwind, so a class string that paints a
row computes to nothing there and a render test passes whether the class is present or
not. Two of the rules are worse than invisible to a render test, because the defect only
appears in the cascade — see rule 1.

## The nine rules

### 1. A row's fill never depends on its state

`.biorouter-settings-row` owns its background, and it owns it for one purpose: the
hover/`focus-within` wash. A component may not paint a second fill on top to say "this one
is on" or "this one is selected". The switch, the radio dot or the checkbox states the
state; a fill restates it in a weaker language and turns a hairline list into a striped
one.

> design.md **P3** — "no coloured hover fills, no accent-tinted card backgrounds. Hover is
> a **neutral** tint"; astryx **§2.6 / A-06** — "selection is achromatic so theme accents
> stay reserved for CTAs".

There is a second, mechanical reason and it is decisive. `.biorouter-settings-row:hover` is
**unlayered**, so it beats a Tailwind utility inside `@layer utilities` whatever the
specificity says. A row painted `bg-background-medium/70` while switched on therefore
*lightened* to 38% under the pointer: hovering an "on" row visibly turned it off. That is
the same inversion `main.css` records above `.tint-selected.tint-interactive`, arriving by
a second route.

The one sanctioned exception, used nowhere in Settings today: where selection genuinely is
the only indicator — no switch, no radio, no checkbox — the wash is the composed pair
`tint-selected tint-interactive`, never a raw `bg-*` alpha. The hover rule was narrowed
from the `background` shorthand to `background-color` so that route is possible at all;
the shorthand reset `background-image`, which is where a tint lives.

### 2. One row

```
biorouter-settings-row flex min-w-0 items-center justify-between gap-3 px-3 py-2.5 text-text-default
  └ <div className="min-w-0 flex-1">
      <p className="text-label text-text-default">Title</p>
      <p className="mt-0.5 max-w-md text-supporting text-text-muted">Description</p>
  └ control (flex-shrink-0)
```

No `min-h-*` — the class already carries `min-height: var(--row-height)` — no `py-2`, no
`py-3`, no `items-start`, no per-row measure fork.

Rows are **direct children** of `.biorouter-settings-list`. This is not tidiness:
`.biorouter-settings-row:last-child` is relative to a row's own parent, so a per-item
wrapper breaks the trailing hairline in one of two directions. One row per wrapper makes
*every* row `:last-child` and suppresses every hairline in the section; several rows in one
wrapper hides the wrapper's last hairline mid-list. Both had shipped on the Chat tab. A
section component that has nothing of its own to declare therefore returns a **fragment**
of rows rather than a box; one that does — `ModeSection` needs `role="radiogroup"` on the
element containing its radios — owns the list itself.

A disclosure renders its trigger and its panel as sibling rows, not as a row plus a boxed
panel, for the same reason.

### 3. One section header, one section rhythm

```
<div className="biorouter-settings-section">
  <div className="biorouter-settings-section-header">
    <h2 className="text-caps text-text-muted">LABEL</h2>       {/* + mb-1 only if a description follows */}
    <p className="text-supporting text-text-muted">…</p>        {/* optional */}
  </div>
  <div className="biorouter-settings-list"> … rows … </div>     {/* or a control strip, rule 5 */}
</div>
```

Sections are **siblings under one tab wrapper**, so
`.biorouter-settings-section + .biorouter-settings-section { margin-top: 10px }` actually
fires. Only the tab's outermost wrapper carries the tail `pb-8`. A bare `<div>` between two
sections breaks that adjacency exactly as a classed one does, so an intermediate wrapper
must be deleted rather than declassed.

A header may carry a right-hand control. It takes `mr-3` (or a `pr-3` wrapper) so its *box*
shares the rows' 12px inset while the `text-caps` label stays flush left with every other
header on the page, and it sits on the default 32px rung.

### 4. One note

Every in-place prose block — a warning, a disclosure, an inline error, an empty-store line,
a restart notice — is `<Note>`. See [the primitives](#the-two-primitives) below.

Banned in Settings from here on: `border-borderStandard` (**no such token exists**; the
border renders only because of the `@layer base` `border-color` fallback), `rounded-lg` (a
deprecated alias of the element radius), `text-iconStandard` (no definition anywhere, so
every use was a no-op that read as intent), and every hand-mixed alpha.

### 5. One control strip

A group of section-level buttons is `.biorouter-settings-control-strip` — flex, wrap,
`align-items: center`, 10px gap, no chrome. Never a hand-rolled `flex gap-2`; never nested
inside a `.biorouter-settings-row`, which would give the buttons the row's hover wash;
never wrapped around a whole multi-row panel, which shrink-wraps it to content width.

### 6. Type roles, not sizes

| Where | Class |
|---|---|
| Section label | `text-caps text-text-muted` |
| Row title, every control's text, field label | `text-label` |
| Description, metadata, note body, caption, count, status line | `text-supporting` |
| Inside a `Badge` | `text-chip` (supplied by the primitive) |
| A control inside a ≤36px dense strip | `text-secondary` — the one sanctioned exception |
| Filesystem paths, ids, versions | add `font-mono`, keep the role |

No bare `text-xs`, `text-sm`, `text-sm font-medium`, `text-base` or `text-[11px]`, and **no
`leading-*` beside a role** — each role carries its own line-height, and stacking one on top
is how three line-heights ended up on one 12px role.

> `main.css` — "These are ROLES, not sizes … a call site can no longer pick 14px and forget
> the 500 weight that makes it a control label"; astryx **§2.2 / A-02**.

### 7. Button variant and size by role

| Role | Spelling |
|---|---|
| The one committing action of a view | `variant="default"`, default rung |
| Section action (opens a dialog or panel) | `variant="secondary"`, default rung, **no `className`** |
| Row-trailing labelled action | `variant="secondary"` or `"outline"`, default or `sm` rung |
| Row-trailing glyph-only action | `variant="ghost" shape="round"` (32×32) |
| Quiet destructive row action | `variant="ghost" className="text-text-danger"` |
| Loud destructive | `variant="destructive"` |
| Dialog footer | `variant="outline"` dismiss, `default`/`destructive` confirm |
| Inline text link | `variant="link"` on a real `<Button>` |

`size="xs"` is the 24px compact tier and is for a **glyph-only** control in an already-dense
cluster — `--control-compact`'s own comment says "a control carrying a label never uses it".
Nothing in these three tabs is that control.

A Button never carries `flex items-center gap-2`: the cva base already emits `inline-flex
items-center justify-center gap-2`, and a bare `flex` **flips that `inline-flex` through
tailwind-merge**, which is what rendered one destructive row action as a full-width red bar
across the reading column. A Button never carries `h-*`/`w-*`/`p-*` geometry, and never a
`hover:bg-*` — `tint-interactive` owns hover and press.

### 8. Reuse the primitive, always

`Badge` for a chip. `Skeleton` for a loading placeholder. `CustomRadio`'s ring construction
for a radio, `Checkbox` for a checkbox. `ConfirmationModal` (or `Dialog`) for a
confirmation — never `window.confirm`, which is theme-blind, unstyleable, and was the one
control in Settings that could not be read in dark mode. `MODAL_SIZE` from
`components/ModalShell.tsx` for a dialog width — never a pixel literal.
`useResolvedTheme()` for light/dark — never a `MutationObserver` on `<html>`.

> design.md **P4** — "If a surface needs a variant, the variant lives in the primitive, not
> in a `className` override at the call site."

⚠ Inlining a primitive's construction rather than mounting it is occasionally right — the
mode rows own their `role="radio"` semantics, which `CustomRadio`'s `<label>` would
duplicate — but copy the construction exactly. In particular the `peer` input must stay
inside the same box as the elements it styles: `peer-checked:` compiles to a sibling
combinator, so a ring that is a descendant of the peer's sibling silently never fills.

### 9. When the words are load-bearing, change the shape only

Some copy in Settings is pinned by a one-definition rule or served by the daemon, and none
of it moves for a style change:

1. `PrivacyPanel`'s DR-17 disclosure — every word comes from the daemon, there is no
   fallback string, it renders in **both** toggle positions and it sits **above** the
   switch. It also must not clamp: hiding half of a mandated disclosure behind "Show more"
   defeats the requirement, which is the entire reason `<Note unclamped>` exists.
2. The privacy disable-confirmation copy and the P-05 refusal placement — the typed phrase
   is compared **exactly** by the daemon, and the refusal is rendered at section level
   because both directions of the switch can be refused while only one opens a
   confirmation. ⚠ Do not move that confirmation into a modal: an open Radix
   `DialogContent` marks the rest of the document `aria-hidden`, so the refusal would sit
   behind the dialog that caused it.
3. `HOST_MANAGED_MODEL_SHORT` / `_REASON` and the placement comments at each call site —
   the short/long choice per surface is a decision, not a style.

**Read the comment before you touch the element.**

## The two primitives

### `components/ui/note.tsx`

The one in-place prose block. One shape: `rounded-element`, `px-3 py-2.5`,
`text-supporting`, an optional 16px top-aligned glyph, an optional single action.

- `neutral` is `border border-border-subtle bg-background-muted text-text-muted` — it has
  no hue to wash, so it takes a real surface step plus a hairline, the same exception
  `badge.tsx` documents for its own neutral.
- `info` / `warning` / `danger` / `success` are `bg-wash-{tone} text-text-{tone}` and carry
  **no coloured border**: status is a translucent fill at 22% with tinted ink, never an
  outline (astryx §2.5). `--wash-*` is derived per family and per mode, so a note reads
  correctly in Parchment, Alma Mater and Roche Limit, light and dark, with no `.dark` fork
  and no hardcoded rgba.
- **It has a ceiling.** Past `--note-max-height` (148px = eight lines of
  `--text-supporting` plus the note's own padding) it folds behind a fade with a "Show
  more" control, the way `utils/messageClamp.ts` folds a long message. Above that a notice
  has stopped being a notice — which is the reported defect, "banners that are essentially
  just a long block of text".
- The fade terminates in `--wash-solid-*`, the opaque companion of each wash. A wash cannot
  end a gradient: fading toward `--wash-warning` paints a second wash over the note's own
  and the bottom reads a step darker, while fading toward `--background-default` discards
  the tint and reads a step lighter.
- `className` at a call site is **layout only** (`mt-*`, `mb-*`, `min-w-0`) and merges.

The clamp itself is authored CSS (`.biorouter-note-clamp` in `main.css`), not a Tailwind
arbitrary value, for the reason `.br-swatch-ring` already records: under
`BIOROUTER_NO_HMR` the renderer runs `watch: { ignored: ['**'] }`, which is the same signal
Tailwind's scanner uses to notice new class strings, so a freshly written utility can
silently fail to generate. A clamp that fails to generate does not degrade — it prints the
whole document.

### `components/privacy/HostManagedModelNote.tsx`

One definition of the sentence a browser-served session is shown instead of a 409, and now
one definition of its shape too. `className` **merges** rather than replacing the
component's own type classes; `variant` is `note` (the boxed neutral `Note`) or `inset`
(flush, hairline-separated, for inside a dropdown's item list); `testId` is overridable so
one surface can mount several. It renders nothing on the desktop, so every call site mounts
it unconditionally.

## What this does not cover

Extensions, the provider-configuration page, the permission modals, dictation, the tunnel
and session sharing share these primitives and will inherit the rules, but were not swept
when the vocabulary landed. They are named in `OUT_OF_SCOPE` in
`settingsVocabulary.test.ts`; deleting a name from that list is how the work gets finished.

Also deliberately outside it: the 40px → 36px row retune (astryx A-03, still a later
phase), promoting a `ghost-danger` variant into `buttonVariants`, and adding `size` /
`purpose` props to `DialogContent` — all repo-wide primitive changes rather than settings
ones.

## Related documentation

- [`design.md`](../../design.md) — the Parchment design system: tokens, the type ramp, the radius ladder, rows-not-cards, and the calm register these rules serve.
- [Astryx UI adoption design](../design/astryx-adoption/astryx-ui-adoption-design.md) — the design of record for the token and primitive layer, including the density ladder and the status-wash formula.
- [Where a generated artifact is displayed](artifact-display-surfaces.md) — the sibling one-rule document for the artifact side panel, enforced the same way.
- [Renderer testing traps](renderer-testing-traps.md) — why several of these rules can only be asserted at the source.
