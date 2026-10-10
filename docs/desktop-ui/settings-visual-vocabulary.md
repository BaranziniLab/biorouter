# The settings visual vocabulary

> **What this is.** The thirteen rules that govern how the desktop Settings view (Models, Chat, App) — and, since 2026-09-07, the chat-history surfaces (`components/sessions/`), the Scheduler (`components/schedule/`) and the four component views Workflows / Extensions / Skills / Built apps — are built, and the primitives they lean on — a living reference for anyone adding or changing a control there.
> **Status:** Current.
> **Audience:** contributors working on the desktop renderer.

Settings is a column of labelled rows. Almost everything in it is a section header over a
hairline list, a row with one control on its trailing edge, or an in-place note. Since the
Codex simplicity redesign (2026-10-09, `docs/design/codex-simplicity-redesign/`) a row
carries no paragraph: its explanation is an InfoTip. Every rule below exists because a call site painted one of
those four things itself instead of reaching for the shared one, and the drift became
visible the moment two of them sat on screen together.

Nothing here is a new visual language. Each rule is derived from a document already in the
repo — [`design.md`](../../design.md) (the Parchment design system),
[`docs/design/astryx-adoption/astryx-ui-adoption-design.md`](../design/astryx-adoption/astryx-ui-adoption-design.md)
(the design of record for the token and primitive layer), and the settings block in
`ui/desktop/src/styles/main.css` with its own comments.

Eight of the rules are enforced at the source by
`ui/desktop/src/components/settings/settingsVocabulary.test.ts`, which walks a
`ROOTS` list — the settings directory, plus each surface since swept onto the
vocabulary. Adding a directory to that list is how a surface joins; one root per
line, so two sweeps of two different surfaces merge without touching each
other's. That is a source test and
not a render test on purpose: jsdom never runs Tailwind, so a class string that paints a
row computes to nothing there and a render test passes whether the class is present or
not. Two of the rules are worse than invisible to a render test, because the defect only
appears in the cascade — see rule 1.

## The thirteen rules

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

### 2. One row: a label, an optional InfoTip, one control

Every settings row is `SettingRow` (`components/ui/setting-row.tsx`). Nobody writes the row
by hand any more.

```tsx
<SettingRow label="Prevent sleep while running" help="The screen can still lock.">
  <Switch checked={on} onCheckedChange={setOn} />
</SettingRow>
```

```
biorouter-settings-row  (min-height var(--row-height) = 40px, padding 10px 12px, gap 12px)
  ├ <label htmlFor={controlId}>Label</label>   text-label, truncated
  ├ InfoTip                                    the label's SIBLING, never inside it
  ├ status line (optional)                     text-supporting muted, transient state only
  ├ value (optional)                           text-secondary muted, mono for versions and ids
  └ ONE control                                trailing edge, shrink-0
```

- **The label is a real `<label>`**, so a click on the words toggles a switch, and the row
  hands the control `id`, `aria-labelledby` (the label) and `aria-describedby` (the help and
  the status line). The control's accessible name is therefore the visible label, word for
  word: someone driving the app by voice says what they read. No "Toggle …" and no "Enable …"
  names. A button whose visible words differ from the row label ("Edit…", "Check for updates")
  names itself with those words (`aria-label="Edit tool permissions"`), so the visible text
  stays inside the name.
- **No description paragraph.** The explanation goes in `help`, an InfoTip (rule 12). A
  visible line under the label is allowed only as `status`, for transient state the person
  must see without hovering: "Restart to apply" after a change, "Up to date", a field's
  validation message.
- **One control per row.** A second control is a second row. Two related buttons (Feedback's
  "Report a bug" and "Request a feature") sit in one `role="group"` that the row names.
- No `min-h-*`, no `py-2`, no `py-3`, no `items-start`, no per-row measure fork. A hand-built
  row (a checkbox list inside a dialog, a row that is itself a button) uses the same
  `biorouter-settings-row` class with `px-3 py-2.5`.

Rows are **direct children** of `.biorouter-settings-list`. This is not tidiness: the row's
hairline is drawn by `::after` and suppressed on `:last-child`, which is relative to a row's
own parent, so a per-item wrapper breaks the hairline in one of two directions. One row per
wrapper makes *every* row the last child and suppresses every hairline in the section; several
rows in one wrapper hides the wrapper's last hairline mid-list. A component that contributes
rows to a section therefore returns a **fragment** of rows rather than a box.

### 3. One section header, one section rhythm

Every section is `SettingSection`:

```tsx
<SettingSection id={SETTINGS_SECTION_IDS.general} title="General" help?="…" action?={…}>
  …rows…
</SettingSection>
```

It renders a `text-caps` muted label (12px, the one caps style left in the app), an optional
InfoTip beside it, an optional action at the header's end inset like the rows, and the rows
inside `.biorouter-settings-list`. **No `<p>` under the header**: a section's explanation is
its `help`. A section never exists to hold one row; merge it into a neighbour (the old
one-row Workspace and Editor sections are rows of General and Display now).

**Deep links scroll to a section `id`.** `components/settings/settingsSections.ts` names every
section a link can land on (`SETTINGS_SECTION_IDS`) and the keys callers already pass
(`update`, `models`, `modes`, `styles`, `tools`, `app`, `chat`, `privacy`); `SettingsView`
selects the tab, scrolls the section into view (smoothly only without reduced motion) and
gives it the `.br-highlight` wash. A section another workstream renders puts the id on its own
root. Never rename or drop a deep-link key: four files outside Settings pass them.

Sections are **siblings under one tab wrapper**, so
`.biorouter-settings-section + .biorouter-settings-section` actually fires. Only the tab's
outermost wrapper carries the tail `pb-8`. A bare `<div>` between two sections breaks that
adjacency exactly as a classed one does, so an intermediate wrapper must be deleted rather
than declassed.

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
| The one committing action of a view or dialog | `variant="default"` (coral), default rung; at most one per view |
| Row-trailing action that opens a dialog | `variant="secondary" size="sm"`, a verb with an ellipsis ("Edit…", "Reset…") |
| Row-trailing action that does the thing | `variant="secondary" size="sm"`, its own words ("Check for updates") |
| Row-trailing glyph-only action | `variant="ghost" shape="round"` (32×32) with a Tooltip |
| Destructive row action (Danger zone) | `variant="destructive" size="sm"`, always behind a confirmation |
| Quiet secondary links in a row (Feedback) | `variant="ghost" size="sm"` |
| Dialog footer | `variant="outline"` dismiss, `default` or `destructive` confirm |
| Inline text link | `variant="link"` on a real `<Button>` |

`outline` is no longer a secondary action on a page; it is the dialog dismiss. Each tab that
has destructive work ends in one **Danger zone** section (Crew's wording) holding one
`destructive sm` button per row, and the consequence is stated in the dialog that opens, at
the moment of decision, never as a paragraph beside the button. The page never shows two red
buttons side by side.

`size="xs"` is the 24px compact tier and is for a **glyph-only** control in an already-dense
cluster; `--control-compact`'s own comment says "a control carrying a label never uses it".
Nothing in these tabs is that control.

A Button never carries `flex items-center gap-2`: the cva base already emits `inline-flex
items-center justify-center gap-2`, and a bare `flex` **flips that `inline-flex` through
tailwind-merge**, which is what rendered one destructive row action as a full-width red bar
across the reading column. A Button never carries `h-*`/`w-*`/`p-*` geometry (a select's
fixed trigger width is the one exception, so a column of selects lines up), and never a
`hover:bg-*`: `tint-interactive` owns hover and press.

### 8. Reuse the primitive, always

`Badge` for a chip. `Skeleton` for a loading placeholder. `SettingRow` and `SettingSection`
for rows and sections, `InfoTip` for help, `SegmentedControl` and `SettingSelect` for
choices, `Switch` for on and off, `Checkbox` for picking items, `CustomRadio` for a radio
inside a dialog. `ConfirmationModal` (or `Dialog`) for a
confirmation — never `window.confirm`, which is theme-blind, unstyleable, and was the one
control in Settings that could not be read in dark mode. `MODAL_SIZE` from
`components/ModalShell.tsx` for a dialog width — never a pixel literal.
`useResolvedTheme()` for light/dark — never a `MutationObserver` on `<html>`.

> design.md **P4** — "If a surface needs a variant, the variant lives in the primitive, not
> in a `className` override at the call site."

The two inlined radio constructions that used to live in the Mode and Response styles rows
are gone with the rows: Approval mode is a select and Tool call details a segmented control.

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

### 10. One page header

Every top-level view's header is `components/Layout/PageHeader.tsx`, and no view writes its
own. Since 2026-10-09 it is a **44px band** (`--chrome-height`, Crew's channel header): a 14px
`<h1>`, the page's help in an InfoTip (never a paragraph), the actions on the right, and the
band's own hairline running edge to edge. The band is a window-drag region whose controls are
`no-drag`. This reverses the 2026-09-07 "actions on their own line" decision; the reversal is
recorded in astryx §4.2.

Eight views each had their own copy of this header before the primitive existed, and the
copies had already drifted in four ways (hairline, description role, padding, action
placement). `styles/measures.test.ts` asserts **at the source** that each of those views
imports `PageHeader`, so a ninth view cannot quietly grow a ninth copy.

**Settings puts its tabs in the band** (`PageHeader`'s `tabs` slot): Models, Chat and App, text
only, with the active underline landing on the band's hairline. The tab names are pinned by
daemon copy ("Settings > Models", "Settings > App > Privacy") and must not change. The body
under the band is one reading column on the chat measure, and a tab change starts the next
tab at its top.

**A tab in that strip takes no focus fill (2026-09-08).** D-15 makes focus a surface shift, and
`ui/desktop/src/components/ui/tabs.tsx` renders a `<button role="tab">` that Radix activates on
focus — so the focused tab is always the _active_ tab and `--background-focus` parked itself on it
permanently, a grey box around the accent underline. Measured in Parchment light: a mouse click read
`rgba(0, 0, 0, 0)`, but one arrow key inside the strip read `rgb(224, 224, 220)` and it then stayed.
The base rule in `styles/main.css` now excludes `[role='tab']`, and a focused tab firms its
underline instead — the `after:` bar goes 2px → 3px with the label at `--text-default`. The rule is
on the TRIGGER, not on this page: `settings/providers/ProviderCatalog.tsx` reuses the same
`.biorouter-settings-tabs` strip, and Knowledge's Sources/Graph strip uses the same primitive.
`styles/tabFocus.test.ts` guards it. Two traps recorded there: the exclusion must wrap the whole
`:where()` list (a trigger matches `button`, `[role='tab']` _and_ `[tabindex]:not([tabindex='-1'])`,
so removing one arm changes nothing), and the underline rule must be **unlayered** or the Tailwind
utility that sets the bar's height beats it silently.

**Neither does the PANEL under it (2026-09-08).** The same fill, on a bigger box, found while
verifying the paragraph above. Radix `TabsContent` renders `role="tabpanel"` with `tabindex="0"`, so
the `[tabindex]:not([tabindex='-1'])` arm reached it and one Tab out of the strip turned the whole
Settings body `rgb(224, 224, 220)` — measured over **712 × 2676 px** in Parchment light, and the same
element in the Provider catalog (712 × 1763) and in Knowledge (772 × 744). D-15 is written for
controls: a control's fill is the size of the thing you are about to operate, a region's is the size
of the page, so the panel is excluded from the fill and given no focus treatment at all — the next
Tab lands on the first control inside, which has its own. The trap, guarded in
`styles/tabFocus.test.ts`: excluding the panel also drops the block's `outline: none`, and Chrome's
own `:focus-visible` ring is underneath it, so a second rule has to put the suppression back or the
grey box becomes a ring around the same box. The `prefers-contrast` escape hatch still reaches the
panel and is deliberately untouched.

### 11. One spelling convention — American English

**Added 2026-09-09.** User-visible copy is American English: `behavior`, `color`, `center`,
`catalog`, `recognizes`, `canceled`, `judgment`. Unlike rules 1–10 this one is not about
Settings, and it is not about a shape — it governs every user-visible string the desktop
renderer ships, plus the shipped skill text under
`crates/biorouter/src/agents/builtin_skills/`. It is recorded here because rule 9 already
governs words, and because this is where a contributor looks before writing a label.

**Product names are proper nouns and are exempt.** **Auto Visualiser** keeps its British
spelling; so do Biorouter, BAAM, Knowledge and every provider and vendor name. So do wire
values — the daemon's `cancelled` status is a value, not a word.

The convention was measured before it was chosen, because a raw grep cannot decide it: over
`ui/desktop/src` the counts read `color`/`colour` 126/106 and `center`/`centre` 49/35, and
almost all of that is Tailwind classes, web-platform identifiers and comments. Counting only
strings a person reads, the landing site is 86 American to 6 British, the CLI 76 to 26, and
the renderer 11 to 9. The deciding argument is that the identifiers cannot move — `color`,
`center`, `dialog`, `catalog`, `license`, `artifact` are CSS, DOM, API and product
identifiers — so a British copy rule would put every label permanently at odds with the
symbol beside it. That seam had already split: the marketplace said "Marketplace catalogue"
in one component and "Loading catalog…" in two others.

⚠ **Its guard is a different file.** Rules 1–8 are enforced by
`components/settings/settingsVocabulary.test.ts`; this one by
`ui/desktop/src/test/uiCopySpelling.test.ts`, which reads copy out of the TypeScript AST and
carries the proper-noun allow-list. Adding a name to that allow-list is a claim that the
words are a name — not a preference for how a sentence reads.

See [`design.md` §3.10](../../design.md#310--spelling) for the decision record.

### 12. Help is an InfoTip, never a paragraph

**Added 2026-10-09** (owner, message 5: "hover-and-show helper text instead of showing that
text verbatim up front"). An explanation, a definition or a hint is `InfoTip`
(`components/ui/info-tip.tsx`): a visible 14px glyph in subtle ink after the label that opens
on hover (200ms), on Tab focus and on click, and whose text is always in the accessibility
tree through `aria-describedby`. `SettingRow help` and `SettingSection help` mount it for you.

- **These stay visible**, always: errors, refusals, privacy disclosures (the DR-17 statement,
  the privacy-off strip and the disable confirmation, rule 9), the consequence of a
  destructive action at the moment of decision (in its dialog), and empty-state one-liners.
- Plain text, at most two sentences, no links, no buttons, no bold. Anything longer belongs
  in a dialog or a `Disclosure`.
- Never `title=`: `AppTooltipLayer` copies a title into the control's name.

Enforced by **V9** in `settingsVocabulary.test.ts` (no `max-w-md text-supporting` description
paragraph in a row).

### 13. One control per job, one look per control

**Added 2026-10-09.**

| Job | Control | Never |
|---|---|---|
| On or off, applies now | `Switch`, trailing edge, named by the row label | a checkbox, a two-option segmented control |
| One of 2 to 4 short options | `SegmentedControl` (Theme, Color palette, Text size, Tool call details) | radio rows on a page, toggle-button strips, native radios |
| One of 4 or more, or options that need a line each | `SettingSelect` (Approval mode, tool rules) | radio rows with paragraphs |
| Pick several | `Checkbox` on the left of its label, inside a dialog (Reset data) | checkboxes on the page |
| A number | `Input type="number"`, `w-20` | a disclosure hiding one field |
| Open an editor | `secondary sm` button, "Edit…" | an icon-tile card |

The segmented thumb and the select's check are neutral; coral marks only a switch's on
state, a checked checkbox and the one committing button. Enforced by **V10** (no native
radio).

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

## What this covers beyond Settings

The rules were written for Settings and have since been applied, unchanged, to two more
surfaces. `settingsVocabulary.test.ts` walks one root per surface.

**The chat-history surfaces** (2026-09-07): the Chat history page, the saved transcript, the
shared transcript and the three dialogs they mount. Those views moved onto the 760px chat
measure in the same pass, and the two facts are related — a column of labelled rows is
exactly the shape both of these rules and that measure assume.

⚠ **Adding a root is not the same as deleting an exclusion.** The guard's original root is
`components/settings`, so the `sessions/` entry in *that root's* out-of-scope list means
`components/settings/sessions/` — the session SHARING section, still unswept — and not
`components/sessions/`, where chat history actually lives. Deleting that name sweeps session
sharing and leaves chat history uncovered, which is the opposite of what it looks like it
does. The exclusions are per root for exactly this reason.

**The component views** (`components/workflows/`, `components/extensions/`,
`components/skills/`, `components/applications/`, `components/apps/`) joined on 2026-09-07,
in the pass that also gave them the 760px chat measure and rule 10's shared header. The two
belong together: the vocabulary assumes a column of labelled rows, and the measure is the
width that shape reads at. Before the sweep these five views were the app's largest reservoir
of the constructions the rules ban — a `flex items-center gap-2` on every header button, a
bare "No extensions available" line for an empty state, a hand-rolled centred block with a
Retry button for another, `text-caps … uppercase` group headers where `text-caps` already
uppercases, and an accent-filled Run button living inside a workflow row.

⚠ **`components/settings/extensions/` is still out of scope**, and its exclusion is not a
sibling of these roots. That directory holds the extension ROWS, which `components/extensions/`
merely lays out; it stays in the settings root's own out-of-scope list for the reason the
original scope note gives — sweeping it triples the diff. The chat-measure pass touched it for
`min-w-0` / `truncate` only. Deleting its name from that list is how the work gets finished.

**The Scheduler** (`components/schedule/`) was swept onto it on 2026-09-07 as well. Two of its
constructions are worth naming because they are what the vocabulary looks like on a surface
that is not Settings:

- **A definition row is `.biorouter-settings-row`.** Astryx §4.5 asks the schedule detail
  for "definition rows"; rather than invent one, the label/value pair takes the row
  Settings already uses for a label and the control it names — muted label left, value on
  the trailing edge, `font-mono` where the value is a path, a cron expression or an id.
  A fact and a control are the same shape of thing on a page, and a near-miss of one row
  is how two rows drift.
- **A status is a dot plus a word, never a filled pill.** Running / Paused / Failed /
  Scheduled were `rounded-md` chips on hand-mixed `bg-background-{tone}` alphas — rule 4's
  banned construction, and a fill §2.5 reserves for a `Note` rather than for a word inside
  a row. `components/schedule/scheduleStatus.tsx` is the one definition both Scheduler
  surfaces read.

## What this does not cover

Extensions, the provider-configuration page, dictation, the tunnel and session sharing share
these primitives and will inherit the rules, but were not swept when the vocabulary landed.
The permission dialogs were swept onto `ModalShell` and `SettingRow` on 2026-10-09; their
folder leaves the exclusion list in the redesign's final sweep. They are named in the per-root `outOfScope` lists in
`settingsVocabulary.test.ts`; deleting a name from one of those lists — or adding a root
for a directory the walker has never reached — is how the work gets finished.

Two files in the chat-history root are excluded for a different reason: `SessionsInsights`
is the **Home** view and `UsageHeatmap` is its grid, whose `leading-*` values and per-mille
alphas are fitted cell geometry rather than prose styling. `ui/ConfirmationModal.tsx`'s
`sm:max-w-[425px]` is likewise left alone — it is a shared primitive, so moving it onto the
`MODAL_SIZE` ladder changes every confirmation in the app rather than one feature's.

Also deliberately outside it: the 40px → 36px row retune (astryx A-03, still a later
phase), promoting a `ghost-danger` variant into `buttonVariants`, and adding `size` /
`purpose` props to `DialogContent` — all repo-wide primitive changes rather than settings
ones.

## Related documentation

- [Codex simplicity redesign](../design/codex-simplicity-redesign/README.md) — the 2026-10-09 pass that removed row paragraphs, added rules 12 and 13 and moved the Settings tabs into the band (implementation spec §3.13).
- [`design.md`](../../design.md) — the Parchment design system: tokens, the type ramp, the radius ladder, rows-not-cards, and the calm register these rules serve.
- [Astryx UI adoption design](../design/astryx-adoption/astryx-ui-adoption-design.md) — the design of record for the token and primitive layer, including the density ladder and the status-wash formula.
- [Where a generated artifact is displayed](artifact-display-surfaces.md) — the sibling one-rule document for the artifact side panel, enforced the same way.
- [Renderer testing traps](renderer-testing-traps.md) — why several of these rules can only be asserted at the source.
