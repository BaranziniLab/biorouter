# Biorouter Design System

**Status:** ✅ **Signed off 2026-07-09** · **Sidebar density addendum 2026-07-15** · **UI cohesion pass 2026-07-16 ([Part 6b](#part-6b--ui-cohesion-pass--2026-07-16))** · **Row density superseded 2026-08-02 (Astryx A-03)** · **Neutrals shared across all three families 2026-08-08 ([§3.1](#31--colour))** · **Copy is American English 2026-09-09 ([§3.10](#310--spelling))** · **Codex simplicity redesign 2026-10 ([Part 9](#part-9--2026-10-redesign-decisions))** · **Doc revision:** 1.87.2 (the app version at sign-off, not the current one) · **Owner:** Baranzini Lab, UCSF

> All 14 open decisions are settled — see [Part 6](#part-6--open-decisions). Recommendations were accepted for
> D-01 … D-11, D-13, D-14; **D-12 was refined to one fixed density profile — now 36px content rows and 32px sidebar
> navigation/session rows** (content rows were 40px from 2026-07-15 until Astryx **A-03** superseded that value on 2026-08-02).
> The [drift register](#part-7--drift-register) is now the active work backlog.

This is the single source of truth for how Biorouter looks and feels. It reconciles the design language *as documented*, *as implemented*, and *as it should be*. Where those three disagree — and they disagree often — this document names the winner.

> **Read [§3.0](#30--theme-families--this-document-describes-one-of-three) first.** This document describes
> **Parchment**, the base theme family. Two others ship alongside it (Alma Mater, Roche Limit), selected by
> a `data-theme` attribute. The *tokens* below are the system-wide contract; the *hexes* are Parchment's
> answer to it.

> **⚠ 2026-08-08 — most of the hexes below are no longer "Parchment's answer"; they are everyone's.**
> Backgrounds, greys, borders, the focus ring and fill, elevation and the scrim are now **one shared set**,
> byte-identical across all three families in both modes. A family varies in exactly two things: its **ink**
> (text and syntax colours) and its **accent**. Parchment's identity is now its warm ink `#2a2520` / `#f4f0e6`
> and its dark orange `#b85a32` / `#e8895f` — not its ground, which is white on `#f4f4f2` like everyone
> else's. The rule, the reasoning and the table of what is owned versus shared are in
> [theme system architecture §8](docs/design/theming/theme-system-architecture.md#8--shared-neutrals--one-scaffolding-three-inks).
> **Do not reintroduce a per-family grey**: if an ink fails on a shared ground, retune the ink.
>
> Current-state tables below were updated. **Historical passages were not** — the drift register (Part 7),
> the decision records (Part 6) and every "Was / Today" comparison keep the values that were true when they
> were written, because rewriting them would falsify the record. Where such a passage states a live value,
> it is annotated inline.

> **Companion artifact:** [`docs/design/design-system-gallery.html`](docs/design/design-system-gallery.html) — also hosted at
> **<https://claude.ai/code/artifact/0726814d-a317-4dd2-bf34-a447e5c6ae6d>** — renders every token, element, and state
> described here, in **both themes side by side**, and lets you pick between the open options in
> [Part 6](#part-6--open-decisions). Open it, make your choices, hit **Export decisions**, and paste the result back.
> Nothing in the codebase changes until you do.

---

## How to read this document

Each element spec carries three labels:

| Label | Meaning |
|---|---|
| **Canonical** | The target. Build to this. |
| **Today** | What the code did when the passage was written, with `file:line` evidence. |
| **Drift** | The gap. Every drift item has an ID (`DR-nn`) and lands in the [drift register](#part-7--drift-register). |

> **Line numbers in this document are as-audited, not current.** `main.css` has grown past 5,300 lines and most of the `file:line` citations below now land on unrelated code. Grep for the token or selector name instead; a name survives the file growing, a line number does not. The same applies to `input.tsx`, `button.tsx`, `dialog.tsx` and `Select.tsx`.

Where a genuine aesthetic trade-off exists, you'll see a **`Decision D-nn`** callout instead of a canonical value. Those are yours to settle.

**Precedence**, when sources conflict: your explicit instruction → this document → `main.css` tokens → component code → `.claude/commands/frontend-design.md` (which is now *downstream* of this file and should be regenerated from it).

---

## Part 1 · Identity

### The thesis

> **A quiet instrument.** Biorouter is a warm, low-chroma, flat-surface research console — paper-like rather than glassy, dense rather than airy, and confident enough to stay silent. Colour is evidence, not decoration.

This is a tool that clinicians and computational biologists keep open for eight hours. It sits next to a terminal, a genome browser, and a stack of PDFs. It must never look like it is trying to sell them something.

### Adjectives (in priority order)

1. **Calm** — nothing pulses, glows, or gradients without a reason grounded in state.
2. **Precise** — hairlines, tabular numerals, consistent optical alignment. The UI should feel machined.
3. **Warm** — the neutral ramp is biased toward paper and bone, never toward blue-grey steel.
4. **Dense** — information-per-pixel is a virtue. Rows beat cards. Text beats iconography.
5. **Trustworthy** — status is legible at a glance and never ambiguous. Destructive actions look destructive.

### Lineage

Biorouter sits between three named traditions, and borrows deliberately from each:

- **The scientific instrument panel** (Tektronix, LabVIEW, IGV) — dense readouts, monospace as a first-class citizen, colour reserved for signal.
- **The editorial reading surface** (Readability, iA Writer) — warm paper ground, generous measure in prose, restrained typography.
- **The modern developer console** (Linear, Zed, Codex) — flat surfaces separated by hairlines, keyboard-first, near-zero chrome.

It is explicitly **not** a consumer chat app. The chat surface is an instrument readout that happens to accept prose.

### Anti-patterns — what Biorouter must never look like

- **Glassmorphism.** No frosted panels floating over blurred wallpaper. (The one legitimate blur is the modal scrim.)
- **Multi-layer drop shadows** used to fake depth on ordinary content. Elevation is reserved for things that genuinely float above the page.
- **Gradient heroes** or a saturated brand colour used as a background fill.
- **Colour as decoration.** A coloured divider, a coloured section header, or a coloured hover state that doesn't encode state is a bug.
- **Cold neutrals.** `#f5f5f5`, `#e5e7eb`, and every other blue-grey Tailwind default are foreign bodies here.
- **Emoji as UI.** Icons are line-drawn and monochrome.
- **Bouncy motion.** No spring overshoot, no `ease-elastic`. Motion is short, linear-ish, and informative.

---

## Part 2 · Principles

Each principle is stated with the concrete rule it forces. A principle that forces nothing is a platitude.

### P1 — Surfaces, not elevation
> **Hairline superseded 2026-10 ([Part 9.11](#911--fills-before-lines-owner-message-13)).** In-page cards, panels and controls are now a subtle solid fill with no resting outline (`--background-fill`). Lines stay on structural edges and on floating surfaces, which keep a hairline and a shadow. The no-shadow half of P1 still holds.

Depth is communicated by a **1px hairline and a change of ground colour**, not by a shadow.
**Forces:** cards, rows, page sections, panels, and tab bars carry `border: 1px solid var(--border-subtle)` and **zero** box-shadow. Shadow is permitted on exactly four things: the modal surface, the popover/dropdown surface, the toast, and the floating chat composer. Nothing else. ([Elevation scale](#25--elevation))

### P2 — Rows, not cards, for anything enumerable
A list of sessions, extensions, files, models, or skills is a **row list**: full-bleed, hairline-separated, hover-tinted.
**Forces:** a card is only correct for a *standalone* object — a single metric tile, a provider you are choosing between, an empty state. If you can scroll past three of them, they are rows.

### P3 — Colour is evidence
The coral accent means *"this is the primary action"* or *"this is live."* The status hues mean *danger / success / warning / info*. Nothing else is coloured.
**Forces:** no coloured section dividers, no coloured hover fills, no accent-tinted card backgrounds, no rainbow tag palettes. Hover is a **neutral** tint. ([Colour](#21--colour))

### P4 — One control, one look
Every text input in the app is the same control. Every dropdown is the same control. If a surface needs a variant, the variant lives in the primitive, not in a `className` override at the call site.
**Forces:** `<Button>` not `<button>`; `<Input>` not a bespoke `<input className="...">`. Overrides that change colour, radius, height, or border are forbidden — they indicate a missing variant. ([Drift register](#part-7--drift-register) tracks 103 raw `<button>` elements across 58 files.)

### P5 — Every interactive element is reachable, and shows it
A visible focus indicator is not optional and is not a design compromise.
**Forces:** one focus treatment app-wide — a 2px ring at 2px offset, in a colour that clears **3:1** against every ground it can appear on. Today the ring is `#f4f0e6` on white: **1.14:1**. That is not a subtle ring; it is an invisible one. ([Focus](#28--focus))

### P6 — The monospace layer is part of the design system
Terminals, code blocks, diffs, logs, and tool output are the app's most-read surfaces. They get the same care as chat.
**Forces:** the code theme and the terminal palette are *tokens*, derived from the neutral ramp, and they **have a dark variant**. ✅ Both now do — and both are generated per family from `themes/<id>.theme.mjs`, so there are six code palettes and six terminal palettes (3 families × light/dark), each gated on contrast at generation time. ([Part 5](#part-5--the-monospace-layer))

### P7 — Both themes ship, or neither does
Dark mode is not a filter applied to light mode. Every surface, overlay, inset highlight, syntax colour, and status hue is authored twice.
**Forces:** no hardcoded `rgba(255,255,255,…)` inset highlights, no `#fffdf7` backgrounds, no `color: var(--color-black)` on a surface that inverts. A token that has no `.dark` value is an incomplete token.

### P8 — Density is a budget, not an accident
The row rhythm, the type scale, and the spacing scale are fixed. You spend from them; you don't invent new values.
**Forces:** 4 radii, 6 spacing steps, 7 type sizes, 3 motion durations. Any new value requires deleting an old one.

---

## Part 3 · Foundations

### 3.0 · Theme families — this document describes one of three

Everything below used to be *the* design system. It is now **Parchment**, the base family — one of three
that ship, selected by a `data-theme` attribute on `<html>`, orthogonal to light/dark.

| Family | `data-theme` | Character | Reference |
|---|---|---|---|
| **Parchment** | *(none — the base)* | Warm paper and terracotta. **The subject of this document.** | this file |
| **Alma Mater** | `alma-mater` | UCSF brand — navy ground, teal accent | [`docs/design/theming/alma-mater-theme-tokens.md`](docs/design/theming/alma-mater-theme-tokens.md) |
| **Roche Limit** | `roche-limit` | JupyterLab-inspired white / grey / orange | [`docs/design/theming/roche-limit-theme.md`](docs/design/theming/roche-limit-theme.md) |

The architecture — how a family is defined, generated and guarded — is
[`docs/design/theming/theme-system-architecture.md`](docs/design/theming/theme-system-architecture.md). Two facts from it
change how you read the rest of Part 3:

**1 · The tokens are the contract; the hexes are one family's answer to it — or, increasingly, everyone's.**
Every family re-colours the *same* semantic token set. Since the 2026-08-08 neutral unification the
backgrounds, greys, borders, focus surfaces, elevation and scrim are **identical across all three**, so a
rule stated as "hover is `#ecece9`" is now as portable as one stated as "hover is `--background-medium`".
What is still per family is the **ink** and the **accent**: a rule stated as "the accent is `#b85a32`" or
"body text is `#2a2520`" is Parchment's answer and two other families answer differently. **Prefer the
token regardless.** A hex that is shared today is shared by decision, not by definition, and writing the
token is what keeps a component correct if that decision is ever revisited.

**2 · A theme is now ONE file.** `ui/desktop/themes/<id>.theme.mjs` is the source of truth for a family:
its light and dark token maps, its syntax palette, its ANSI-16 terminal palette, its boot-splash stops,
its picker label and swatch. `npm run themes` (`scripts/generate-themes.mjs`) generates everything
downstream — the `:root[data-theme='…']` / `.dark[data-theme='…']` CSS blocks in `main.css`,
`src/styles/themes.generated.ts`, the family list, the picker entry, and the splash block in
`index.html` — into fenced `THEMES:GENERATED:*` regions. Do not hand-edit a generated region.

The one hand-written exception is **Parchment's own `:root` / `.dark` blocks in `main.css`**. They stay
hand-authored because they are the *base layer*: they carry the structural tokens every family inherits
and never restates (radii, motion, z-index, row height — `STRUCTURAL_TOKENS` in
[`scripts/lib/theme-contract.mjs`](ui/desktop/scripts/lib/theme-contract.mjs)), plus the raw neutral and
coral ramps the other families' generated blocks are overrides *against*. `parchment.theme.mjs` still owns
Parchment's syntax, terminal, splash and picker data.

The generator is also a **gate**: it refuses to emit a theme whose syntax palette falls below 4.5:1 on
that family's `--background-code`, or whose terminal palette falls below its per-slot floor
(`TERMINAL_FLOORS` — see [§5.2](#52--the-terminal)). A theme that fails contrast cannot be written to disk.

### 3.1 · Colour

#### The neutral ramp — SHARED by every family

Warm neutral, very lightly paper-biased. It is deliberately quieter than the ramp Parchment used to
carry: a ground that three different inks and three different accents all have to sit on cannot afford
a strong cast of its own.

| Token | Hex | Role |
|---|---|---|
| `neutral-50` | `#fafaf9` | Lightest tint |
| `neutral-100` | `#f4f4f2` | Page ground (`--background-muted`) |
| `neutral-200` | `#e4e4e0` | Light hairline |
| `neutral-300` | `#d2d2cd` | Light strong border |
| `neutral-400` | `#a9a7a1` | Mid grey |
| `neutral-500` | `#84827c` | Mid grey |
| `neutral-600` | `#5c5a55` | Light-mode focus ring |
| `neutral-700` | `#3e3d39` | Dark strong border |
| `neutral-800` | `#2c2c29` | Dark hover fill |
| `neutral-900` | `#1b1b19` | Dark surface / card |
| `neutral-950` | `#131312` | Dark canvas |

**Canonical, and no longer Parchment's alone.** This is Roche Limit's ramp, adopted wholesale as the one
neutral set on 2026-08-08. Editing a value here moves all three families and requires a matching edit in
`parchment.theme.mjs`, `alma-mater.theme.mjs` and the hand-written `:root` / `.dark` blocks in `main.css`.

> **What it replaced, and why.** Parchment ran a warm cream ramp (`#faf8f3` / `#f4f0e6` / `#e8e1d2` /
> `#d4cab6` / … / `#0d0a06`) whose hue drifted from bone toward umber, and Alma Mater a cool blue-grey one.
> Of the 35 background / border / sidebar keys, the three families agreed on **four**. Three ramps meant
> three times the surface for one bug — `--background-canvas` and `--background-muted` were byte-identical
> in three of the six family/mode scopes, so anything that used `bg-background-muted` to lift off the page
> was invisible there, and only the family that happened to pick different numbers escaped. The families
> were never actually told apart by their greys; they are told apart by ink and accent, and that is now the
> only place they differ.

#### The accent

Biorouter has exactly one brand hue: a terracotta coral.

| Token | Hex | Contrast facts |
|---|---|---|
| `--coral-500` | `#cf6d47` | 3.54:1 on white — **passes 3:1 for UI/large text, fails 4.5:1 for body text**. 5.58:1 on `neutral-950`. |
| `--coral-600` | `#b85a32` | 4.62:1 on white — white text on this fill **passes AA**. |
| `--coral-400` | `#e8895f` | 7.69:1 on `neutral-950` — the dark-mode accent. |

> **This is the single most consequential fact in this document:** white text on `#cf6d47` is **3.54:1** and fails AA. If the primary button is coral, its fill must be `#b85a32`, not `#cf6d47`. See **[Decision D-01](#d-01--primary-cta-colour)**.

**Today:** ✅ **fixed — D-01 shipped.** `--background-accent` **is** the coral: `var(--color-coral-600)` = `#b85a32` in light, `var(--color-coral-400)` = `#e8895f` in dark ([`main.css:104`, `233`](ui/desktop/src/styles/main.css#L104)). White on the light fill is **4.62:1**; `#16120c` on the dark fill is **7.26:1** — both clear AA. `--accent-bar` (the sidebar rail, tab underline and status dots) carries `--color-coral-500` `#cf6d47` in light, which is 3.30:1 on the shared sidebar `#f7f7f5` — fine as a non-text indicator, and the reason `#cf6d47` is *only* ever a bar and never a text or button fill.

The legacy aliases survive but are now honest about what they hold: `--color-block-teal: var(--color-coral-500)` and `--color-block-orange: var(--color-coral-600)` ([`main.css:45–46`](ui/desktop/src/styles/main.css#L45)) — the misleading *names* remain, so `DR-02` stays open; the near-black primary button (`DR-01`) does not.

> This paragraph previously read *"the coral is not reachable through any semantic token … `--background-accent` is near-black, not coral. The existing design doc claims primary CTAs are coral. They are not."* That was true when audited and has been false since D-01 landed. It is corrected rather than deleted because the claim was quoted downstream.

#### Semantic status colours

**Today, light and dark share the same hues.** `--text-danger` maps to `red-200` (`#e85252`) in light mode *and* `red-100` (`#f07575`) in dark. Those hues were tuned for a dark ground. On white they all fail AA as text:

| Role | Today (light) | Ratio on white | Canonical (light) | Ratio | Canonical (dark) | Ratio on `#131312` |
|---|---|---|---|---|---|---|
| Danger | `#e85252` | **3.65 ✗** | `#b3261e` | 6.54 ✓ | `#f07575` | 6.65 ✓ |
| Success | `#5bbe5e` | **2.34 ✗** | `#1f7a3d` | 5.37 ✓ | `#7ac87c` | 9.19 ✓ |
| Warning | `#e8b830` | **1.85 ✗** | `#8a5a00` | 5.93 ✓ | `#f0c84a` | 11.57 ✓ |
| Info | `#5892ee` | **3.11 ✗** | `#1e5fbf` | 6.10 ✓ | `#7aabf5` | 7.94 ✓ |

**Canonical.** Split each status into two tokens — a **fill** (for badge/banner backgrounds, where the text on top is what must pass) and a **text/icon** colour (which must itself pass 4.5:1 on the page ground). Conflating them is the root cause. `DR-03`.

> **The status hues stay PER FAMILY**, and that is a deliberate exception to the shared-neutrals rule
> (2026-08-08). They are neither neutral scaffolding nor the family accent, so the rule does not reach
> them; and Alma Mater's are UCSF institutional brand values rather than a palette choice. They are
> re-verified against the shared grounds by the contrast guard like everything else — the dark column
> above was recomputed when the dark canvas moved from `#0d0a06` to the shared `#131312`.

#### The usage-heatmap ramp

> **Level 0 and Parchment's dark level 1 superseded 2026-10 ([Part 9.5](#95--colour-and-contrast-additions)).** The idle day is the shared neutral `#eeeeea` (light) and `#32312e` (dark, 1.33:1 on the card); Parchment's dark level 1 is `#573e2a`.

The Home page's usage heatmap is the one place a **sequential** scale is correct — it encodes a
quantity, not a category. It is derived from the warm ramp and the accent, monotonic in luminance so
the five steps read in order even in greyscale.

| Level | Light | Dark | Adjacent step |
|---|---|---|---|
| 0 (idle) | `#ece5d6` | `#241f16` | — *(1.18:1 / 1.21:1 vs the canvas, so the grid is visible when idle)* |
| 1 | `#e9c9ab` | `#4a3524` | 1.25 / 1.42 |
| 2 | `#dda27a` | `#7a4d2e` | 1.41 / 1.60 |
| 3 | `#c6774c` | `#b0653a` | 1.55 / 1.63 |
| 4 | `#a04a27` | `#e8895f` | 1.75 / 1.71 |

Shading is **relative to the visible window** — the quartiles of its active days — not to the window
maximum. Dividing by the maximum saturates: `ln` compresses so hard that nearly every active day lands
at 0.75–1.0 of the max, so a 4k-token day renders as dark as a 250k-token day and level 1 goes unused.
This is the same convention GitHub's contribution graph uses. Absolute values live in the tooltip.

#### Text colours

**This is Parchment's axis.** Now that the grounds are shared, the ink below is most of what makes the
family. It was not retuned by the 2026-08-08 unification — every value held on the shared grounds, and
the minima moved only because the grounds did.

The **audit** columns are the original 2026-07 finding, measured on the grounds that existed then
(sidebar `#f3ede1` light, `#282217` dark). They are left at their measured values — a historical failure
does not stop having happened because the ground later moved. The **min ratio** column is live, over the
five grounds `check-contrast.mjs` enumerates today.

**Light**

| Role | Was | Audit: on white | Audit: on sidebar `#f3ede1` | **Canonical** | Min ratio (today) |
|---|---|---|---|---|---|
| `--text-default` | `#2a2520` | 15.17 ✓ | 13.01 ✓ | `#2a2520` *(unchanged)* | 13.78 |
| `--text-muted` | `#7a736c` | 4.67 ✓ | **4.01 ✗** | **`#635c54`** | 5.98 |
| `--text-subtle` | `#948d83` | **3.28 ✗** | **2.82 ✗** | **`#6e6760`** | 5.06 |

**Dark**

| Role | Was | Audit: on sidebar `#282217` | **Canonical** | Min ratio (today) |
|---|---|---|---|---|
| `--text-default` | `#ffffff` | 15.77 ✓ | **`#f4f0e6`** *(warm bone; pure white is harsh)* | 13.85 |
| `--text-muted` | `#b0a892` | 6.66 ✓ | `#b0a892` *(unchanged)* | 6.65 |
| `--text-subtle` | `#88806a` | **4.01 ✗** | **`#9c937b`** | 5.16 |

> The minima barely moved across the unification, which is the point worth noticing: the shared
> `--background-muted` `#f4f4f2` is very close in luminance to the warm sidebar `#f3ede1` that used to be
> Parchment's darkest light ground, and the shared dark grounds are *darker* than the ones they replaced.
> Parchment's ink needed no retuning at all. (Its terminal `yellow` and `cyan` did — see [§5.2](#52--the-terminal).)

Three findings the arithmetic forced, none of which the original audit caught:

1. `--text-muted` passed on white but **failed on the two-tone sidebar** — the warm-beige reskin darkened the ground without re-checking the text on it. `DR-04`
2. `--text-subtle` failed *everywhere* in light mode, and also failed on the dark sidebar. `DR-05`
3. **The first draft of this document had `subtle` darker than `muted`** — an inverted hierarchy. The values above restore the correct order: `default` → `muted` → `subtle`, each step lighter, all clearing 4.5:1 on the darkest ground either can land on.

> **"Min ratio" means: across the grounds body text may legitimately land on** — `--background-app`,
> `--background-canvas`, `--background-default`, `--background-muted`, `--sidebar` — which is exactly the
> set `check-contrast.mjs` enumerates (`TEXT_GROUNDS`). It is *not* a minimum over every fill in the
> system. `--text-default` on `--background-strong` (`#dcdcd8`) is **11.04:1**; an earlier revision of this
> table printed that kind of number in the sidebar column, which is how a value measured against a
> pressed-state fill came to be documented as the sidebar ratio. The sidebar figure is **14.15:1**.

All of these are asserted by [`ui/desktop/scripts/check-contrast.mjs`](ui/desktop/scripts/check-contrast.mjs), which parses the real
token file, resolves the `var()` chains and fails `npm run lint:check` on any regression. It runs across
**all three theme families × light/dark** and currently makes **330 assertions**. Re-measure that figure
rather than quoting it — it grows whenever an assertion is added, and it was 324 before the canvas/muted
step was guarded.

#### Border tokens

| Token | Light today | Dark today | Problem |
|---|---|---|---|
| `--border-default` | `neutral-100` `#f4f0e6` | `neutral-900` `#16120c` | Dark value is **1.06:1** against the `#0d0a06` canvas — invisible. |
| `--border-input` | `neutral-100` | `neutral-800` | — |
| `--border-strong` | `neutral-100` | `neutral-700` | **Identical to `--border-default` in light mode.** |
| `--border-subtle` | `neutral-200` `#e8e1d2` | `neutral-800` | The only one doing real work. |

In light mode `--border-default`, `--border-input`, and `--border-strong` are **all `neutral-100`**. Every documented distinction between them — "hover thickens the border to `border-strong`" — is a silent no-op. `DR-06`

It is in fact *worse* than a no-op. `--border-strong` (`neutral-100 #f4f0e6`) is **lighter** than `--border-subtle` (`neutral-200 #e8e1d2`). So `hover:border-border-strong` applied to a `border-border-subtle` surface makes the border **weaker** on hover — the exact opposite of the documented intent. Dark mode spreads the three tokens correctly, so the two themes don't mirror each other's intent either. `DR-51`.

**Canonical** — and **shared by every family** since 2026-08-08; borders are scaffolding, not identity:

| Token | Light | Dark |
|---|---|---|
| `--border-subtle` (hairline; the default) | `#e4e4e0` (`neutral-200`) | `#302f2c` |
| `--border-strong` (hover, emphasis) | `#d2d2cd` (`neutral-300`) | `#3e3d39` (`neutral-700`) |
| `--border-input` (control resting edge) | `#c9c9c3` | `#4a4945` |

Three real tokens. Note that `--border-input` is now its own value rather than an alias of the
hairline — a control's resting edge is one step firmer than a rule between rows.

**Shipped, and the rule was knowingly not taken in full.** `--border-strong` is `neutral-300` and
`--border-subtle` `neutral-200`, so `strong` is *darker* than `subtle` and `hover:border-border-strong`
firms the edge instead of weakening it (`DR-06`, `DR-51` closed). The ordering is asserted in all six
theme/mode scopes, so the shared ramp cannot silently invert it again.

But **`--border-default` was not deleted.** It is retained as a deprecated alias —
`--border-default: var(--border-subtle)` ([`main.css:144`](ui/desktop/src/styles/main.css#L144)) — with a
comment recording why: **45 call sites still name it**, and a rename touching 45 files to eliminate one
`var()` hop buys nothing the alias doesn't already buy. The harmful part is gone: the token no longer
resolves to a near-invisible `neutral-100`.

The `@layer base` global also **stays**, deliberately, and was re-pointed rather than removed
([`main.css:749`](ui/desktop/src/styles/main.css#L749)):

```css
*, ::before, ::after { border-color: var(--border-subtle); }
```

Tailwind v4's preflight defaults `border-color` to `currentColor`, so deleting the global would paint
every un-coloured border in the **text** colour — strictly worse than what it replaced. It now names the
hairline directly instead of `--border-default`. `DR-45` closed.

#### The two-tone canvas

The sidebar is a distinct, slightly deeper surface against the lighter main canvas. **Shared by every
family** — the two-tone is a structural idea, and what varies on it is the ink and the nav-icon colour.

| | Light | Dark |
|---|---|---|
| Window ground (`--background-app`) | `#ffffff` | `#131312` |
| Main panel (`--background-canvas`) | `#ffffff` | `#131312` |
| Page ground (`--background-muted`) | `#f4f4f2` | `#232320` |
| Sidebar (`--sidebar`) | `#f7f7f5` | `#171716` |
| Sidebar hover | `#efefec` | `#232320` |
| Sidebar active | `#eaeae6` | `#2e2e2a` |

**Canonical.** Keep the two-tone. But see [Decision D-09](#d-09--sidebar-treatment) — and fix `--text-muted` on it (`DR-04`).

> **The dark ladder changed direction on 2026-08-08.** Parchment dark used to run its canvas at
> `#282217` — *lighter* than its `#16120c` cards, so the page lifted and cards recessed into it — and
> Alma Mater dark did the same in navy. The shared set runs the other way: canvas darkest, cards a step
> up, muted a step above that. One neutral set cannot carry two contradictory orders, and this is the
> largest visible change the unification made.
>
> `--background-canvas` and `--background-muted` are also now guaranteed to be a **real step apart** in
> every family and mode. They were byte-identical in three of the six scopes, which made anything using
> `bg-background-muted` to lift off the page invisible there — the composer's chips, most recently.
> `check-contrast.mjs` asserts the step.

#### Four tokens this document had never named

They ship, they are theme-aware, and the guard checks three of them. Undocumented tokens get
re-invented as literals, which is how the scrim and the code ground got duplicated in the first place.

| Token | Light | Dark | What it is for |
|---|---|---|---|
| `--sidebar-icon` | `#635c54` | `#b0a892` | The nav-icon colour, split from the label. Every family points it at its own `--text-muted`, the ink a nav row's label and a chat row's glyph wear at rest, so the sidebar's 16px icon column is one ink (vision finding V1-R2-5, 2026-10-11; it was `--sidebar-foreground`, the default ink, which read heavier than the labels beside it). Before 2026-10-10 Alma Mater and Roche Limit painted nav icons in their accent, which put the accent outside the redesign's principle 7 list (V1-M3). `themeNeutrals.test.ts` holds both. The token stays per family. Declared in every family so `check-contrast.mjs` can assert it on `--sidebar`, `--sidebar-hover` and `--sidebar-active` (floor 3:1, a non-text indicator). |
| `--background-code` | `#f5f5f3` | `#1b1b19` | The ground code actually paints on, and the ground the [§5.1](#51--code-blocks) syntax palette is measured against. Introduced by **D-20**: light wants a panel a hair off the page, dark wants the card ground, so no existing token expressed it and the value had been copy-pasted into `codeTheme.ts`, `main.css` and `InAppTerminalDock`. **Shared** since 2026-08-08 — every family's syntax palette is measured against these two values. |
| `--scrim` | `rgba(31,30,28,.18)` | `rgba(0,0,0,.48)` | The modal / diagnostics backdrop. Was a hardcoded warm-brown `rgba()`, so **Roche Limit — added later — silently wore Parchment's brown scrim.** Tokenised so a family *could* tint its own ink — Alma Mater ran a navy scrim for a while. **Shared** since 2026-08-08: a full-screen backdrop is a background, and backgrounds no longer vary. The token stays so a future family can re-point one name. |
| `--sidebar-ring` | `#5c5a55` | `#a5a39d` | `--ring` scoped to the sidebar; tracks `--ring`, which is neutral by design and therefore **shared**. |

#### The boot splash

The splash paints **before the app**, before React and before `main.css`, so it cannot read a token.
Its values are therefore literals, generated into a `THEMES:GENERATED:SPLASH` region in
[`ui/desktop/index.html`](ui/desktop/index.html) and cross-checked against `THEME_FAMILIES` by
`boot-splash.test.ts` so a new family cannot silently miss this screen.

**What it draws (2026-10-11, first-run vision FR-R1-7, R-29, R-34).** The Biorouter wordmark, at rest, centred, with
a thin sweep under it. It used to draw the BR monogram with a staggered blur-slide entrance, which made the
monogram the first logo a new person saw and a second logo before the wordmark the first-run page leads with.
The lockup is static geometry (a viewBox, two bar rects and a text element in the bundled Inter at weight 600)
copied from a rendered `<BioRouterWordmark/>` at 160 by 49 in Chrome on macOS. `bootSplash.browser.test.ts` holds
it, on every platform, to the same pixels as the component drawn at the same numbers, and on macOS also to the
numbers the component measures and to the same pixels unpinned. That second half is gated to macOS: Chrome on Linux
measures text through FreeType, and on CI it put the box's x 0.12 units (about 0.04 px) off the markup (2026-10-12),
so regenerate the numbers on macOS. The splash waits for Inter (up to 250 ms) before it shows, so
the first frame is the whole logo rather than a bar with letters following. A launch that finishes inside 400 ms
shows nothing.

**The hand-off (2026-10-11, FR-R4-1).** The splash fades out over 260 ms in the frame the app mounts, in light as well
as in dark. That depends on `<html>` carrying a mode class before React runs: the theme script in `index.html`
stamps `dark` or `light`, never neither, and `ThemeContext` decides whether a mount is a theme change by asking
whether the page is dark. A mount that counted as a change would put `biorouter-theme-switching` on `<html>` for two
frames, and that class stills every transition on the page, the splash's own fade included, so the wordmark would
vanish in one frame and the empty loader would show before the band painted. That is what a light launch did until
this fix, because a light page had no class to match. `bootSplash.browser.test.ts` mounts the real `ThemeProvider`
and the dismissal that `renderer.tsx` runs after it, and asserts the opacity transition is running.

**Nothing opens over the splash (2026-10-11, FR-R5-1).** The splash is the topmost thing on the page
(`z-index: 2147483000`) until it is removed, which is up to about 300 ms after the app mounts, so a dialog opened in
that window had the wordmark drawn across its body text. The controller therefore says when it has left:
`window.__brBootSplash.isGone()` is false until the element is removed (after the fade, or at once when the splash was
never drawn), and `window` gets one `br-boot-splash-gone` event at that moment. `components/privacy/firstRunNoticeHandoff.ts`
reads both; the day-one privacy notice is the one dialog that can be due at launch, and it waits for the splash and
for the first page to draw (the guard stopping its empty loader) before it opens, with a 3 s bound so a missed signal
cannot hide a disclosure. Any other surface that can open at launch asks the same question. `index.html` and the
module restate the event's spelling, and `boot-splash.test.ts` pins them together.

| Token | Light | Dark | Role |
|---|---|---|---|
| `--br-bg` | `#ffffff` | `#131312` | The ground, **`--background-canvas`**: the ground the first-run page, Home and the loader paint, so the splash fades onto its own colour. It was `--background-muted`, one step off the page, and a launch that crossed the threshold flashed grey before the white. Shared by every family. |
| `--br-navy` | `#052049` | `#18a3ac` | The `Bio` letters and the left half of the underline (D-39). **The same in every family**: it is the base family's mark ink. |
| `--br-coral` | `#b85a32` | `#b85a32` | The `Router` letters and the right half of the underline. The same in every family. |
| `--br-track` | `#e4e4e0` | `#302f2c` | The 120 by 2px sweep track, a hairline, so shared (`--border-subtle`). |

**The inks do not follow the family, because the wordmark does not.** `BioRouterWordmark.tsx` is deliberately fixed to
the brand colours. A splash that took Alma Mater's teal or Roche Limit's orange from the family's own mark would hand
off to a differently coloured logo one frame later, so the generator gives every family the base family's mark inks
and the family's own `mark` now drives only the in-app BR monogram. `boot-splash.test.ts` reads the wordmark's
`NAVY`, `CORAL` and `TEAL` constants from its source and fails if a splash rule disagrees. The generator still
asserts the navy half at 3:1 against the splash ground, now each family's canvas (Parchment dark: 6.08:1, light: 16.04:1).

**Drift found while documenting this, since fixed (`DR-61`).** `--br-navy` did not flip for dark mode in any family,
and the mark sits directly on `--br-bg` with **no cream plate**, so on every dark boot splash the navy half was
invisible: `#052049` measured **1.02:1** on Parchment dark `#282217`, **1.12:1** on Alma Mater dark `#0d2a50`,
**1.02:1** on Roche Limit dark `#232320` (the grounds of the day). The user saw a lone coral `R` above half an
underline. **D-39 had already decided this case**: "on a dark app surface UCSF navy all but vanishes, so the navy
role becomes UCSF teal `#18A3AC`". The dark inks now flip, and the generator holds the line.

Motion: the lockup has none. The sweep is a CSS animation on `transform` alone and stops under
`prefers-reduced-motion`; under forced colours both inks take `CanvasText`, as the in-app wordmark does.

---

### 3.2 · Typography

> **Scale superseded 2026-10 ([Part 9.3](#93--type)).** The app now renders five sizes, 12 · 13 · 14 · 17 · 24, in two visible weights, as named roles (`text-supporting`, `text-secondary`, `text-body`, `text-label`, `text-prose`, `text-subheading`, `text-title`, `text-caps`). The table under "Canonical type scale" records the scale before the redesign.

**Today:** `--font-sans` aliases the `--font-body` role, an Arial-first native stack (`Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif`), and `--font-mono` is `ui-monospace, 'SF Mono', SFMono-Regular, 'Cascadia Mono', Menlo, Consolas, 'Liberation Mono', monospace`. **No webfont is loaded anywhere**; both stacks are the OS's own faces.

`TERMINAL_FONT` in `InAppTerminalDock.tsx` is that mono stack byte-for-byte, at 13/20, with a comment requiring it to stay identical, because xterm measures glyph widths itself and cannot read `var(--font-mono)`. So a command pasted from a chat code block renders in exactly the same face as the terminal. `DR-07` described the mismatch these used to have and is **fixed**.

Arial is a defensible choice for a tool that must render identically on a lab Windows box. It is also, bluntly, the reason the UI reads as slightly dated. See **[Decision D-06](#d-06--typeface)**.

#### Canonical type scale

The values below are the Standard baseline. Settings → App → Appearance offers Standard (the default, 1×), Large (1.07×), and Larger (1.15×). Scale typography and absolute line heights together, including code and terminal text. Padding, gaps, positive margins, and shared control/row heights follow the same preference so larger text has room; pane widths, viewport geometry, and window zoom remain independent. Relative em spacing already follows its text and must not scale twice. Sidebar navigation and recent-conversation rows share the same scalable 32px baseline height and 2px row gap; Components children keep their indentation within that rhythm. The sidebar wordmark uses a 24px base height multiplied by the same preference, with its aspect ratio preserved.

| Role | Size / line-height | Weight | Tracking |
|---|---|---|---|
| Page title | 24 / 30 | 600 | −0.01em |
| Section title | 18 / 26 | 600 | −0.005em |
| Body | 14 / 21 | 400 | 0 |
| Body emphasis | 14 / 21 | 500 | 0 |
| Secondary / metadata | 13 / 18 | 400 | 0 |
| Caption | 12 / 16 | 400 | 0 |
| Section label (all-caps) | 11 / 14 | 500 | +0.08em |
| Metric readout | 30 / 34 | 300, mono | −0.02em |
| Code / terminal | 13 / 20 | 400, mono | 0 |

**Rules.** Prose (chat messages, docs) is capped at **68ch** measure. Anywhere digits align in a column — token counts, durations, table cells, metric tiles — use `font-variant-numeric: tabular-nums`.

**Drift:** `<Input>` sets `text-base` (16px) with `md:text-sm` (14px) above the 930px breakpoint ([`input.tsx:11`](ui/desktop/src/components/ui/input.tsx#L11)), so inputs are 16px on narrow windows and 14px on wide ones, while the `Select` control is 14px always. `DR-08`.

---

### 3.3 · Spacing & layout

Six steps. Nothing between them.

| Step | Value | Use |
|---|---|---|
| `1` | 4px | Icon-to-label |
| `2` | 8px | Inside a control |
| `3` | 12px | Between rows in a group |
| `4` | 16px | Between groups |
| `5` | 24px | Section separation |
| `6` | 32px | Page gutter |

#### Page shell

| Property | Value |
|---|---|
| Page horizontal gutter | 32px (`px-8`) |
| Page top padding | 48px (`pt-12`) |
| Header bottom padding | 24px (`pb-6`) |
| Header separator | 1px `--border-subtle` |
| Chat / prose measure | 760px flat (`--measure-chat`) |
| Page measure | `clamp(1120px, 88%, 1720px)` (`--measure-page`) |
| Row height | 36px content · 32px sidebar navigation/session ([D-12·C](#d-12--row-density)) |
| Sidebar width | 216–360px, 288px default (user-resizable; `components/ui/sidebarWidth.ts`) |

**Today:** the documented flat header (`px-8 pt-12 pb-6 border-b`) is contradicted by `.biorouter-page-header`, which sets `border-bottom-color: transparent !important` and replaces the hairline with a gradient wash plus `box-shadow: var(--shadow-modal-chrome-bottom)` ([`main.css:519`](ui/desktop/src/styles/main.css#L519)). So the "flat header with a bottom border" is actually a shadowed, gradient header with no border. `DR-09`. See **[Decision D-05](#d-05--elevation-policy)**.

---

### 3.4 · Radius

**Was, at audit: seven distinct radii in TSX** — `rounded-md` (127×), `rounded-full` (90×), `rounded-lg` (85×), `rounded-xl` (54×), `rounded-2xl` (17×), `rounded-sm` (7×), `rounded-none` (6×) — plus raw `border-radius` values of 4/8/16px in `main.css`, and no radius token at all. `DR-10` is **fixed**: a semantic ladder ships, named for the role a corner plays rather than a t-shirt size.

**Canonical: four roles, plus `full`.**

| Token | Value | Applies to |
|---|---|---|
| `--radius-inner` | 4px | Inline code, chips, checkbox, swatches, nested inside a control |
| `--radius-element` | 8px | Buttons, inputs, selects, tabs, list rows, menu items |
| `--radius-container` | 12px | Cards, panels, dialogs, popovers, toasts, code blocks, the composer |
| `--radius-surface` | 16px | RESERVED: the artifact / preview sheet only |
| `--radius-full` | 9999px | Status dots, avatars, pills, toggle knobs |

The nesting rule is normative: an element inside a container takes the next step **down**, and when a container's padding is `p`, its inner radius is `outer − p`.

The old size names survive as deprecated aliases, and two of them do **not** resolve to what their name suggests: `--radius-sm` → `inner` (4px), `--radius-md` → `element` (8px), `--radius-lg` → `element` (**8px**, not 12px; it was already a silent alias of `md`), `--radius-xl` → `container` (**12px**, not 16px), `--radius-2xl` → `surface` (16px). Write the role name at new call sites.

See **[Decision D-04](#d-04--radius-scale)** for the one genuinely contested value: list rows at 8px (current `.biorouter-list-row`) vs 12px (documented `rounded-xl`).

---

### 3.5 · Elevation

**Today:** `@theme { --shadow-*: initial; }` ([`main.css:15`](ui/desktop/src/styles/main.css#L15)) unsets Tailwind's entire shadow namespace, and only `--shadow-default` is re-registered. I verified this by compiling the real Tailwind config:

```
DEAD      shadow-sm      DEAD      shadow-md
DEAD      shadow-lg      DEAD      shadow-xl
GENERATED shadow-none    GENERATED shadow-default
```

There are **22 usages of `shadow-sm` / `shadow-md` / `shadow-lg` / `shadow-xl` in TSX that render no shadow at all.** They are dead classes. Some of them are on elements that the design doc says should be flat anyway — so the bug and the rule accidentally agree. `DR-11`.

**Canonical.** Four elevation tokens, and a closed list of what may use them.

| Token | Value (light) | Permitted on |
|---|---|---|
| `--elev-0` | none | Cards, rows, page sections, tab bars, panels, headers |
| `--elev-composer` | `0 2px 6px -1px rgba(32,25,15,.08), 0 1px 2px rgba(32,25,15,.05)` | The floating chat composer, only |
| `--elev-popover` | `0 8px 24px rgba(32,25,15,.10), 0 0 1px rgba(0,0,0,.15)` | Dropdowns, popovers, tooltips, toasts |
| `--elev-modal` | `0 22px 60px -18px rgba(32,25,15,.22), 0 8px 24px -18px rgba(32,25,15,.16), 0 0 0 1px rgba(32,25,15,.045)` | Modal + sheet surfaces |

Dark-mode variants deepen opacity and swap the hairline ring to `rgba(255,255,255,.055)`.

**Also fix:** `.biorouter-modal-panel` and `.biorouter-page-block` apply `box-shadow: inset 0 1px 0 rgba(255,255,255,0.42)` with **no dark override** ([`main.css:494`, `529`](ui/desktop/src/styles/main.css#L494)) — a white glare line across the top of dark panels. `DR-12`.

---

### 3.6 · Motion

> **Tiers superseded 2026-10 ([Part 9.6](#96--motion)).** The ladder is now 95, 125, 175, 250, 300 and 525ms; `--ease-spring` is sanctioned for transforms of physical gestures; `--ease-in` is deleted; the `--motion-*` names below were aliases of the `--dur-*` tiers and are deleted too (fast is `--dur-fast`, base is `--dur-fast-max`, slow is `--dur-med`).

**Today:** six durations (`150` ×43, `300` ×31, `200` ×28, `500` ×3, `75` ×1, `100` ×1) and eight keyframe animations. `prefers-reduced-motion` is honoured for exactly two of them (`.sidebar-item`, and one block at `main.css:1091`). `DR-13`.

**Canonical: three durations, two easings.**

| Token | Value | Use |
|---|---|---|
| `--motion-fast` | 125ms (`--dur-fast`) | Hover, focus, colour transitions |
| `--motion-base` | 175ms (`--dur-fast-max`) | Popover / dropdown / tooltip enter, tab switch |
| `--motion-slow` | 300ms (`--dur-med`) | Modal enter, route transition, drawer |
| `--ease-out` | `cubic-bezier(0.24, 1, 0.4, 1)` | Everything entering |
| `--ease-in` | `cubic-bezier(.4,0,1,1)` | **Deprecated**: defined, never used; deleted in the sweep |

The three `--motion-*` tokens were aliases onto the `--dur-*` ladder, re-pointed rather than retuned, so call sites shifted by at most 5ms (the exception was `--motion-slow`, which was 260ms and became the 300ms medium tier it was always describing). Each owner then moved its call sites to `--dur-*` and the aliases were deleted in wave 2, with a source test that fails if a `var(--motion-*)` returns.

No spring, no overshoot, no bounce. Exit is always shorter than enter.

**Rule:** every animation must be nulled under `@media (prefers-reduced-motion: reduce)`, applied once, globally — not per-class.

---

### 3.7 · Z-index

**Today:** twelve ad-hoc layers — `z-50` (18×), `z-10` (14×), `z-[1000]` (7×), `z-[60]` (4×), `z-[400]` (4×), `z-20`, `z-0`, `z-[9999]` (2×), `z-[1210]`, `z-[100]`, `z-40`, `z-[999]`. React-select portals at `z-[9999]`; the dialog sits at `z-[1200]`/`z-[1210]`. A select opened inside a modal therefore paints above the modal's own close button. `DR-14`.

**Canonical: a six-stop scale.**

| Token | Value | Layer |
|---|---|---|
| `--z-base` | 0 | Page content |
| `--z-sticky` | 100 | Sticky headers, the composer |
| `--z-dropdown` | 200 | Selects, dropdowns, popovers, context menus |
| `--z-overlay` | 300 | Modal scrim |
| `--z-modal` | 400 | Modal / sheet surface (dropdowns *inside* a modal re-portal to 500) |
| `--z-toast` | 600 | Toasts, and nothing else |

---

### 3.8 · Focus

This was the most serious defect in the audited system. It is **fixed**: D-15 ships, and the shipped answer is stated first below.

**Today:** focus is D-15's surface shift. `main.css` owns it globally for every text field, and `input.tsx` says so in as many words ("D-15: focus is a surface shift, never a ring"); the `--background-focus` / `--border-focus` tokens are defined in all six family/mode scopes. `--ring` survives only for the `prefers-contrast: more` and `forced-colors` paths and is now `--color-neutral-600` (`#5c5a55`), which clears 3:1 against every ground.

**Was, at audit:** `--ring` was aliased to `--border-strong` at `neutral-100`, `<Input>` focused to `ring-2 ring-border-strong`, `<Button>` to `focus-visible:ring-ring/50 focus-visible:ring-[1px]`, and the dialog close button to `focus:ring-ring focus:ring-2`.

| Ground | Ring colour | Contrast | Required |
|---|---|---|---|
| White surface | `#f4f0e6` | **1.14:1** | 3.0:1 |
| `#faf8f3` canvas | `#f4f0e6` | **1.07:1** | 3.0:1 |
| Dark `#0d0a06` | `#403928` | **1.72:1** | 3.0:1 |

**The focus indicator was invisible in both themes**, and the app was not keyboard-navigable in any practical sense. `DR-15` is fixed.

Compounding it: **six different focus treatments** were in use across the codebase — `focus:outline` (44×), `focus:ring` (34×), `focus:border` (32×), `focus-visible:ring` (12×), `focus-visible:outline` (8×), `focus-visible:border` (1×). `DR-16` is fixed by the same decision.

**Canonical — focus is a surface shift, never a ring.** *(D-15, superseding the original D-03 answer.)*

A 2px coral outline drawn around every focused control reads as an alarm on a surface whose entire
thesis is calm. Worse, **browsers treat text fields as permanently `:focus-visible`**, so the outline
fired on an ordinary mouse click, wrapping the chat composer in a bright orange rectangle. The
indicator was correct by the letter of WCAG and wrong for this product.

The focused control instead **deepens its own fill by one step** and firms its existing edge. Nothing
is added around it.

```css
/* Controls: the fill steps past hover, and the label firms up.
   A TAB TRIGGER and a REGION are exempt — see the three amendments below. */
:where(:is(a, button, summary, [role='button'], [role='menuitem'],
           [role='option'], input[type='checkbox'], input[type='radio'],
           [tabindex]:not([tabindex='-1'])):not([role='tab'], [role='tabpanel'],
           [role='application'], .biorouter-focus-region)):focus-visible {
  outline: none;
  background-color: var(--background-focus);
  color: var(--text-default);
}

/* A region draws nothing, so the UA ring has to be put back down by hand.
   `.biorouter-focus-region` is how a region with no role to key on opts out. */
:where([role='tabpanel'], .biorouter-focus-region):focus-visible {
  outline: none;
}

/* Text fields already own a border, so they shift fill AND firm that edge.
   No new strip is introduced. */
:where(input:not([type='checkbox']):not([type='radio']):not([type='range']),
       textarea, select):focus,
:where(…):focus-visible {
  outline: none;
  background-color: var(--background-focus);
  border-color: var(--border-focus);
}

/* A container holding a focused field takes the shift, so the inner control
   never draws a box of its own. */
:where(.biorouter-chat-composer, .biorouter-list-row,
       .biorouter-settings-row):focus-within {
  border-color: var(--border-focus);
}
```

**Amendment, 2026-09-08 — a tab trigger is exempt from the fill; its underline is the indicator.**
Radix `TabsTrigger` activates on focus, so the focused tab is always the _active_ tab and the fill
parked itself permanently on it: a grey box around the accent underline, reported against Settings
("that weird shade") and equally present on the Provider catalog strip. Measured in Parchment light
before the change: a mouse click read `rgba(0, 0, 0, 0)` (Chrome withholds `:focus-visible` from a
mouse-clicked button) but one arrow key inside the strip read `rgb(224, 224, 220)` = `--background-focus`,
and it then stayed for as long as the strip kept focus. D-15 forbids a ring and the fill is what was
rejected, so what remains is the indicator the component already draws: `:focus-visible` takes the
`after:` bar from 2px to 3px and firms the label to `--text-default`, in
`ui/desktop/src/styles/main.css` beside the block above and guarded by `styles/tabFocus.test.ts`.
Two things about the exclusion are load-bearing. It is a `:not([role='tab'])` around the **whole**
list, because three arms match one trigger — it is a `<button>`, it carries `role="tab"`, and Radix's
roving tabindex gives the active one `tabindex="0"` — so deleting the obvious arm changes nothing on
screen. And the underline half is **unlayered**, because the bar's height and colour come from
Tailwind utilities and a layered rule would lose to them silently. `.br-tab` (chat header, artifact
panel, terminal dock) is untouched: it draws no underline and owns its own `::after`.

**Amendment, 2026-09-08 (second) — D-15 is written for CONTROLS, so a focusable REGION takes no fill
either.** A control deepens its own fill because the fill is the size of the thing you are about to
operate; a region is entered rather than operated, and its fill is the size of the page. The arm that
reaches one is `[tabindex]:not([tabindex='-1'])`, which cannot tell an 8px checkbox from a document.
Found while verifying the amendment above: Radix `TabsContent` carries `role="tabpanel"` with
`tabindex="0"`, so one Tab out of the Settings strip painted `--background-focus` over the whole
**712 × 2676 px** body — measured `rgb(224, 224, 220)` in Parchment light, with the same rule named by
CDP's `CSS.getMatchedStylesForNode`. `role="application"` (the knowledge graph canvas, the app's one
instance) measured the same and is exempt for the same reason. Two things are load-bearing. Dropping
the panel from the block also drops its `outline: none`, and the UA `:focus-visible { outline: auto }`
is underneath — so the fill would be traded for a ring around that same box unless the suppression is
restored on its own, which is the second rule above; `role="application"` needs no such rule because
it writes `outline-none` plus a 2px inset `--border-accent` ring as utilities. And the
`prefers-contrast: more` / `forced-colors` block is deliberately **not** narrowed by any of this, so a
user who asked their OS for a stronger signal still gets the ring on the panel. `[role='tablist']` is
not exempt: it matches, but focusing it redirects into the active trigger within the same event, so
the rule never paints it.

**Amendment, 2026-09-08 (third) — a region with NO role opts out with `.biorouter-focus-region`.**
Both exemptions above key on a role, and a scroll container given a tab stop so a keyboard user can
scroll it has none to key on. The chat summary's To Do list (`components/ChatSummary.tsx`) is an
`<ol tabIndex={0}>`; measured in Parchment light on the real popover, opened with the keyboard, it
read `rgb(224, 224, 220)` over the whole **334 × 240 px** list, with the D-15 rule named by CDP's
`CSS.getMatchedStylesForNode`. It cannot take a role: an `<ol>`'s implicit `list` role is what makes
its rows list items to a screen reader, so `role="region"` would silence the fill and take the rows'
semantics with it, and an explicit `role="list"` would be a redundant attribute carried only so a
stylesheet could see it. So the hook is a class — an authored selector in the same `:not()`, never a
`focus-visible:bg-*` utility at the call site, for the reason `.biorouter-focus-surface` gives (a newly
written Tailwind class can silently fail to generate; a hook that needs nothing generated cannot) — and
the UA-ring restoration names the class beside the panel, or the grey list would have become a ringed
one. After the change the focused list reads `rgba(0, 0, 0, 0)` with outline and box-shadow `none` in
all three families in both modes; the next Tab lands on the summary's own "Make workflow" button, which
shows its focus surface; and under `prefers-contrast: more` the list takes the `2px solid --ring`
outline through the same `[tabindex]` arm. Guarded in `styles/tabFocus.test.ts` beside the other two.

**Amendment, 2026-09-08 (fourth) — that region gets a quiet inset edge, because "no treatment" is not
an indicator.** The third amendment put the class into BOTH arms — the fill exemption and the
`outline: none` restoration — so the UA ring was suppressed with nothing in its place: measured on the
running app (Parchment dark, the real popover, focus reached with a real Tab so `:focus-visible` was
Chrome's own verdict), `background-color`, `outline`, `box-shadow`, `border-width` and every `<li>` were
byte-identical focused and unfocused. That is WCAG 2.4.7 with nothing on screen. The tabpanel's argument
— the next Tab lands on a control inside — does not reach a role-less region: the To Do list has **zero**
focusable children and is its own scroll container, so it is the thing being operated. So
`.biorouter-focus-region:focus-visible` takes `box-shadow: inset 0 0 0 1px var(--border-accent)` — one
pixel, inset, in the composer focus edge's own token, needing no border and no geometry change, and
painted on the element's box so it stays put while the list scrolls. `--border-accent` is the only accent
that clears the 3:1 non-text floor against the popover's `--background-default` ground in all three
families in both modes (Parchment 4.62 / 6.71, Alma Mater 4.56 / 9.47, Roche Limit 3.09 / 5.58);
`--accent-bar` falls to 2.66:1 in Alma Mater light. The rule is **unlayered**, for the same reason the
tab underline is: inside `@layer base` a `:where()` rule sits at specificity 0 and every Tailwind
utility beats it. The panel keeps the third amendment's treatment unchanged, the `outline: none` stays
in the layered rule so there is exactly one indicator, and `prefers-contrast: more` is still untouched.

All three tokens are **shared** — the focus treatment was always explicitly neutral rather than accented
(D-15), so unifying it changed its values without changing its intent.

| Token | Light | Dark | Checked |
|---|---|---|---|
| `--background-focus` | `#e0e0dc` | `#35342f` | text-default on it: 11.46:1 / 10.96:1 ✓ |
| `--border-focus` | `#6b6963` | `#9c9a93` | **4.15:1** / 4.43:1 against the focused fill ✓ (floor 3:1) |
| `--ring` *(escape hatch only)* | `#5c5a55` (`neutral-600`) | `#a5a39d` | ≥5.82:1 / ≥5.55:1 on the ring grounds ✓ (floor 3:1) |

The token is spelled **`--ring`**. There is no `--focus-ring` — where this document says `--focus-ring`
(§4.1, §4.6, §4.7, §5.2) it means `--ring`, and those specs are in any case superseded by D-15: the
default treatment is the fill shift above, not a ring.

**"On every ground" is a defined set, not a flourish.** `check-contrast.mjs` measures `--ring` against
`RING_GROUNDS` = the text grounds plus `--background-medium`: `--background-app`, `--background-canvas`,
`--background-default`, `--background-muted`, `--sidebar`, `--background-medium`. The ring is drawn
*outside* the control at 2px offset, so the page ground is what it lands on — not the control's own fill.
Across that set the light floor is **5.82:1** and the dark floor is **5.55:1**, both set by
`--background-medium`. (Because the ring and the grounds are now all shared, these two numbers are the
same in every family — a small, real simplification.)

Two fills sit outside that set and are worth stating so the "≥" is not read as a universal claim: on
`--background-strong` the ring measures **5.01:1** light and **4.53:1** dark, and on `--background-focus`
**5.20:1** light and **4.95:1** dark. All clear the 3:1 floor a non-text indicator owes.

Focus must also be distinguishable from **hover**: the focused fill sits one step past the hover fill
(1.20:1 light, 1.19:1 dark) — perceptible without being loud.

> **An honest trade-off, deliberately taken.** A colour shift of this softness cannot meet WCAG's 3:1
> non-text-contrast floor for a state indicator (SC 1.4.11). The calm is worth it for the default
> experience, but users who have asked their operating system for a stronger signal get the ring back:
>
> ```css
> @media (prefers-contrast: more), (forced-colors: active) {
>   :where(a, button, input, textarea, select, summary,
>          [tabindex]:not([tabindex='-1'])):focus-visible {
>     outline: 2px solid var(--ring);
>     outline-offset: 2px;
>   }
> }
> ```
>
> This is a product decision, not an oversight. Revisit it if the app is ever procured under a strict
> Section 508 / EN 301 549 review.

**Never** write `outline-none`, `outline-hidden`, `focus:ring-*`, or `focus-visible:ring-*` at a call
site. All 68 such classes and all 54 focus-scoped ring utilities were removed; the treatment is global.

---

### 3.9 · Iconography

> **Custom set and sizes superseded 2026-10 ([Part 9.8](#98--icons)).** Hand-drawn glyphs now sit beside Lucide in one registry, and the default icon size is 16px (14 in a chip, 20 in a banner or empty state).

**Today:**
- `app-icons.tsx` declares a `light()` wrapper whose contract is *"all icons render at `strokeWidth=1.5`."* But **~15 files import from `lucide-react` directly**, bypassing the wrapper, so those icons render at Lucide's native `strokeWidth=2`. Two icon weights coexist on screen. `DR-53`.
- `react-icons` and `@radix-ui/react-icons` are in `package.json` but imported in **zero** files — dead dependencies, not a consistency problem. Remove them. `DR-17`.
- `ui/icons.tsx` exports 11 icons; `components/icons/` holds ~40 hand-authored SVG components (including six `Bird1–6` decorative marks); there are **96 inline `<svg>` literals** scattered through TSX. `DR-18`.

**Canonical.**

| Property | Value |
|---|---|
| Library | `lucide-react` for everything with a Lucide equivalent |
| Custom set | `components/icons/` only for domain marks (Biorouter logo, provider logos, SPOKE) |
| Sizes | 16px (inline/dense), 20px (default), 24px (page-level) |
| Stroke | 1.5px at all sizes |
| Colour | `currentColor`, always. Never a hex. |
| Optical alignment | Icons sit on the text baseline box, not centred on the cap-height |

Inline `<svg>` literals in view components are forbidden; promote to `components/icons/`.

#### The logo

`components/icons/Biorouter.tsx` draws the wordmark with gradient stops at `#EC5D2A` (20×) and `#57B9AF` (20×) — an orange and a teal that **exist nowhere else in the system** and are not the token coral `#cf6d47`. `DR-19`. See **[Decision D-02](#d-02--brand-mark-palette)**.

### 3.10 · Spelling

**Decided 2026-09-09.** User-visible copy is **American English** — `behavior`, `color`, `center`, `catalog`, `recognizes`, `canceled`, `judgment`. This governs the desktop renderer's copy and the shipped skill text; it does not govern this repository's contributor prose (`CLAUDE.md`, `docs/`, code comments), which stays as written.

The choice was measured, not assumed. Across the product's user-visible strings: the landing site is 86 American to 6 British, the CLI 76 to 26, the desktop renderer 11 to 9. Two further reasons the American column was kept: the identifiers cannot move — `color`, `center`, `dialog`, `catalog`, `license`, `artifact` are CSS, DOM, API and product identifiers, so a British copy rule would put every label permanently at odds with the symbol beside it, and that seam had already split (the marketplace said "Marketplace catalogue" in one component and "Loading catalog…" in two others); and UCSF and this repository's operator write American English, so it is the convention that gets written by hand rather than remembered.

**Product names are proper nouns and are exempt** — **Auto Visualiser** keeps its British spelling, as do Biorouter, BAAM, Knowledge and every provider and vendor name.

Enforced at the source by `ui/desktop/src/test/uiCopySpelling.test.ts`, which reads user-visible strings out of the AST (JSX text, a `{…}` JSX child, the copy-bearing attributes and properties, toast arguments) rather than grepping — a grep over `src/` is dominated by class strings, web-platform identifiers and comments, and cannot answer a question about copy. Flipping the convention is one word in `spellingVariants.ts`.

---

## Part 4 · Element specifications

Every element below is rendered live, in every state, in [`docs/design/design-system-gallery.html`](docs/design/design-system-gallery.html).

### 4.1 · Buttons

**Canonical.** Five variants. Four sizes. One radius (`--radius-md`, 8px). No shape variant.

| Variant | Fill | Text | Border | Hover | Use |
|---|---|---|---|---|---|
| `primary` | accent | `--text-on-accent` | none | darken 6% | The one committing action per view |
| `secondary` | `--background-medium` | `--text-default` | none | `--background-strong` | Everything else |
| `outline` | transparent | `--text-default` | 1px `--border-strong` | `--background-medium` | Secondary action on a tinted ground |
| `ghost` | transparent | `--text-default` | none | `--background-medium` | Icon buttons, toolbar, row actions |
| `danger` | `--fill-danger` | white | none | darken 6% | Destructive, irreversible |

| Size | Height | Padding-x | Text | Icon |
|---|---|---|---|---|
| `xs` | 24px | 8px | 12px | 14px |
| `sm` | 32px | 12px | 13px | 16px |
| `md` (default) | 36px | 16px | 14px | 16px |
| `lg` | 40px | 24px | 14px | 18px |

Icon-only buttons are square at the same heights, radius `--radius-md`.

**States.** Rest → Hover (fill step) → Active (`translateY(1px)`, no scale) → Focus-visible (2px `--focus-ring`, 2px offset) → Disabled (`opacity: .5`, `pointer-events: none`) → Loading (label stays, a 14px spinner replaces the leading icon; width does not change).

**Today** ([`button.tsx`](ui/desktop/src/components/ui/button.tsx)):
- `default` variant fills with `bg-background-accent` = `neutral-900` — **near-black, not coral.** `DR-01`
- `outline` has **no border**: its class string is `bg-background-medium text-text-default hover:bg-background-strong` — byte-for-byte `secondary` plus `active:translate-y-px`. Two variants, one appearance. `DR-20`
- `shape: 'pill'` (the default) maps to `rounded-md`. It is not a pill. `shape: 'round'` also maps to `rounded-md`. The `shape` prop changes only padding. `DR-21`
- `destructive` sets `focus-visible:ring-destructive/20` and `aria-invalid:border-destructive` — **`destructive` is not a defined colour**; both compile to nothing. `DR-22`
- `size: 'xs'` uses `![&_svg:not([class*="size-"])]:size-3` — a leading `!` is Tailwind v3 important syntax. In v4 the modifier is a trailing `!`. Dead class. `DR-23`
- Sizes set height but no width for round shape until a compound variant patches it; `default` is `h-9` (36px) ✓.

**103 raw `<button>` elements across 58 files** bypass the component entirely (vs 276 `<Button>` usages). `DR-24`

---

### 4.2 · Pop-ups — modal, sheet, confirmation

**Canonical.**

| Property | Value |
|---|---|
| Scrim | `rgba(32,25,15,.18)` light / `rgba(0,0,0,.48)` dark, `backdrop-filter: blur(8px)` |
| Surface | `--background-default`, 1px `--border-subtle`, `--radius-container` (12px), `--elev-modal` |
| Width | `ModalShell`'s three, never wider than `100vw − 32px`: `sm` 400 (confirmations and short forms: Transfer ownership, Make private, Create team, Create channel, Rename, Share path), `md` 480 (longer forms: Keys, Connection settings, Join, Invite, Let in, Edit profile, Add people), `lg` 640 (lists, multi-step and the terminal: Workspace settings, Host, Sign in). Crew's dialogs use all three |
| Padding | 24px |
| Header | Title 18/26 600, description 13/18 `--text-muted`, 8px gap |
| Footer | Right-aligned, 8px gap, `secondary` then `primary`; stacks reversed on narrow |
| Close | 32px ghost icon button, top-right, **16px inset** (`absolute right-4 top-4`), 16px icon centred |
| Enter | `--motion-base` `--ease-out`, `opacity 0→1`, `scale .96→1` |
| Exit | 120ms `--ease-in` |
| Dismiss | Escape ✓, backdrop click ✓ (except when a form is dirty), focus trap ✓, focus restored on close |
| Z | scrim `--z-overlay`, surface `--z-modal` |

**Today.** The Radix `Dialog` is the canonical implementation ([`dialog.tsx`](ui/desktop/src/components/ui/dialog.tsx)): `rounded-2xl p-6`, `z-[1200]`/`z-[1210]`, `zoom-in-95`, `duration-200`. `.biorouter-modal-surface` supplies `border-radius: 16px` and `--shadow-modal`. This is close to canonical and mostly correct.

But **two parallel modal systems exist.** 25 files use the Radix `Dialog`; roughly **17 modals are hand-rolled** as `biorouter-modal-overlay fixed inset-0` + `biorouter-modal-surface` divs with no Radix at all — `BaseModal.tsx`, `ConfigureApproveMode.tsx`, `WorkflowInfoModal.tsx`, `DependencySetup*`, `ScheduleModal.tsx`, `WorkflowWarningModal.tsx`, and more. The hand-rolled ones have **no focus trap, no `role="dialog"`, no `aria-modal`**, and inconsistent Escape/backdrop dismissal. `DR-49`

Their z-indices span `z-40` → `z-[9999]` with no scale, so stacking order contradicts intent: `BaseModal` (`z-[9999]`) paints **above** the canonical Radix dialog (`z-[1210]`), while `WorkflowWarningModal` (`z-50`) paints **below** it. `DR-14`

`Diagnostics.tsx` uses its own `.biorouter-diagnostics-surface` — hardcoded `background: var(--color-white); color: var(--color-black)` with **no dark override**, and a `.dark .biorouter-diagnostics-overlay` that keeps the *cream* `rgba(246,243,237,0.7)` scrim ([`main.css:447–466`](ui/desktop/src/styles/main.css#L447)). In dark mode the diagnostics panel is a white card behind a cream haze. `DR-25`

The dialog close button focuses to `focus:ring-ring focus:ring-2 focus:ring-offset-2` where `ring-offset-background` is undefined and `--ring` is invisible. `DR-15`, `DR-26`

---

#### The close affordance — one geometry, everywhere

A dismiss control is the same object regardless of what it dismisses. Seven different insets were in
use (`top-5 right-4`, `top-4 right-4`, `top-3.5 right-3`, `top-1.5 right-1`, `right-2 top-2`,
`right-2 top-1`, `top-12 right-4`).

| Context | Button | Inset | Icon |
|---|---|---|---|
| Modal, sheet, drawer, full popover | 32px ghost, `rounded-md` | `right-4 top-4` (16px) | 16px |
| Toast, inline banner, compact popover | 20px ghost, `rounded-sm` | `right-2.5`, vertically centred | 14px |
| Chip / tag / attachment | 14px, inline (not absolute) | — | 11px |

The icon is **optically centred** in its button, never flush to a corner. An absolutely-positioned
close button must not overlap its own content: the container reserves `button width + 2 × inset` of
padding on that side. (`react-toastify` needed `padding-inline-end: 38px` for exactly this reason.)

**Overlay action clusters** (the copy/expand controls that fade in over an artifact or an image) sit at
`right-2 top-2` (8px), are 28px square, and reveal on `:hover` *and* `:focus-within` — never hover alone.

---

### 4.3 · Toasts and inline alerts

**Canonical.**

| | Toast | Inline alert |
|---|---|---|
| Surface | `--background-default`, 1px `--border-subtle`, `--radius-container` (12px), `--elev-popover` | `--fill-{status}` at 8% over the page ground, 1px `--border-{status}` at 30% |
| Text | 13/18 | 13/18 `--text-{status}` |
| Icon | 16px, `--text-{status}` | 16px, `--text-{status}`, top-aligned |
| Accent | 3px left bar in `--text-{status}` | none |
| Duration | 5s (error: sticky) | — |
| Motion | slide-in 8px + fade, 180ms | none |
| Z | `--z-toast` | — |

**Today.** Toasts are `react-toastify`, restyled in `main.css:383–432`. But `toastClassName` is a **static** string — `text-white bg-neutral-800/95 backdrop-blur-md border border-white/10 shadow-lg rounded-xl` ([`App.tsx:575–580`](ui/desktop/src/App.tsx#L575)). The toast is hardcoded dark in **both** themes, uses a raw Tailwind neutral instead of a token, and is `rounded-xl` (12px) where the modal is 16px. The pop-up users see most often is the only surface that never adapts. `DR-48`

Inline alerts are ad-hoc: `text-destructive bg-destructive/10 rounded-lg px-4 py-3` — **both `text-destructive` and `bg-destructive` are undefined**, so error banners render as unstyled inherited text on a transparent background. Eight such call sites. `DR-22`

⚠ **2026-09-07 — the canonical inline-alert row above is superseded, and part of "Today" is closed.** The shipped primitive is `components/ui/note.tsx`: a **22% status wash with tinted ink and no border**, per the astryx §2.5 formula, rather than an 8% fill inside a 30% status border — one formula for chips, badges, notes and the destructive button, instead of two that drift. It also carries a `--note-max-height` ceiling the canonical row has no equivalent for. It is adopted across Settings (Models, Chat, App); the remaining ad-hoc alerts live in Extensions, the provider page and the modal subtrees. See [the settings visual vocabulary](docs/desktop-ui/settings-visual-vocabulary.md).

---

### 4.4 · Tooltips

**Canonical.** `--background-inverse` fill, `--text-inverse` at 12/16, 6px×8px padding, `--radius-sm`, no arrow, 8px offset, 500ms open delay / 0ms close, 120ms fade. Never contains interactive content. Never the only source of an action's meaning.

**Today.** A `Tooltip.tsx` primitive exists, but native `title=` attributes are also used, which render an OS tooltip with different timing and styling. Normalize. `DR-27`

---

### 4.5 · Dropdown menus & popovers

**Canonical.**

| Property | Value |
|---|---|
| Surface | `--background-default`, 1px `--border-subtle`, `--radius-container` (12px), `--elev-popover` |
| Padding | 4px |
| Item | 32px tall, 8px×12px, `--radius-md`, 13/18 |
| Item hover | `--background-medium` |
| Item selected | `--background-medium` + 16px check, leading |
| Item danger | `--text-danger`; hover `--fill-danger` @ 8% |
| Section label | 11px caps, `+0.08em`, `--text-muted`, 8px×12px |
| Separator | 1px `--border-subtle`, 4px margin |
| Offset | 6px from trigger |
| Motion | `--motion-base` `--ease-out`, `opacity` + `scale .96→1` from the trigger edge |
| Z | `--z-dropdown` |

**Today.** `.biorouter-popover-surface` gives border + shadow and is applied to both the Radix popover and the react-select menu — good. But the react-select menu is `rounded-xl` (12px) while its own control is `rounded-md` (6px), and the Radix dropdown items use a different height. `DR-28`

---

### 4.6 · Text inputs

**Canonical.**

| Property | Value |
|---|---|
| Height | 36px (`sm`: 32px) |
| Padding | 8px 12px |
| Radius | `--radius-md` |
| Fill | `--background-default` |
| Border | **1px solid `--border-input`** — a real border, not a ring |
| Text | 14/21 (fixed; no breakpoint switch) |
| Placeholder | `--text-muted`, normal weight |
| Hover | border → `--border-strong` |
| Focus | border → `--focus-ring`, plus `outline: 2px solid --focus-ring; outline-offset: 2px` |
| Invalid | border → `--text-danger`; message 12/16 `--text-danger`, 4px below |
| Disabled | `--background-muted` fill, `--text-muted`, `cursor: not-allowed` |

**Today** ([`input.tsx:11`](ui/desktop/src/components/ui/input.tsx#L11)): `border-0 … ring-1 ring-border-input`, focusing to `ring-2 ring-border-strong`. Borders are simulated with rings, so the *focus* ring and the *resting* border are the same visual channel — and in light mode `--border-strong` equals `--border-input` equals `neutral-100`. **Focusing an input changes its appearance by 1px of a colour that is 1.14:1 against the field.** `DR-06`, `DR-15`

Worse: because the base sets `border-0`, any `border-*` **colour** utility a caller passes survives `twMerge` but has zero border-width to paint. Every hand-written invalid/error border on an `<Input>` is a silent no-op. Validation feedback does not render. `DR-50`

Also `text-base md:text-sm` — 16px below 930px, 14px above. `DR-08`

`placeholder:font-light` renders Arial Light, which most systems substitute with regular; on those that don't, placeholders are visibly thinner than the `Select` placeholder. `DR-29`

---

### 4.7 · Textarea & the chat composer

**Canonical.** The composer is the one element permitted `--elev-composer`.

| Property | Value |
|---|---|
| Surface | `--background-default`, 1px `--border-subtle`, `--radius-xl`, `--elev-composer` |
| Min height | 52px; grows to 40vh, then scrolls |
| Padding | 12px 12px 8px |
| Text | 14/21 |
| Focus | border → `--focus-ring` (no outline; the composer *is* the focus target) |
| Toolbar | 32px row beneath the text, ghost icon buttons at 28px |
| Send | 28px round `primary`, disabled until non-empty |
| Attachment chip | 24px, `--radius-sm`, `--background-medium`, 12px label, ×-to-remove |

---

### 4.8 · Select / combobox

**Canonical.** The trigger is visually identical to a text input (same height, radius, border, focus), plus a 16px trailing chevron in `--text-muted` that rotates 180° over `--motion-fast` when open. The menu is the [dropdown surface](#45--dropdown-menus--popovers). Selected option shows a leading check, not a fill.

**Today** ([`Select.tsx`](ui/desktop/src/components/ui/Select.tsx)) wraps `react-select` with `unstyled` + Tailwind `classNames`, which is the right approach. Issues:

- Control is `rounded-md`, menu is `rounded-xl` — the trigger and its own menu have different radii. `DR-28`
- Selected option fills with `bg-background-accent text-text-on-accent` — a **near-black block** in a menu of neutral rows. Loud, and it disagrees with the Radix dropdown's checkmark convention. `DR-30`
- Menu portals to `z-[9999]`, above the modal surface at `z-[1210]`. `DR-14`
- A five-line comment in the file documents an Emotion-vs-Tailwind specificity fight over `fontSize` — evidence that `react-select`'s style injection is structurally at odds with the token system. See **[Decision D-08](#d-08--select-implementation)**.

---

### 4.9 · Switch, checkbox, radio

> **Superseded 2026-10 ([Part 9.7](#97--interaction-states-and-controls)).** The switch is 32×20 with a 16px knob that does not grow, off on `--control-track-off` with a `--border-control` ring; checkbox and radio rest on `--border-control`; the name of a switch is its row's visible label.

**Canonical.** Track 36×20px, `--radius-full`; knob 16px white, 2px inset, 120ms translate. Off: `--background-strong`. On: accent. Focus: standard outline on the track.
Checkbox 16px, `--radius-sm`, 1.5px border, accent fill + white check when set.
Radio 16px, `--radius-full`, 1.5px border, 6px accent dot when set.
All three: 8px gap to a 14/21 label; the **label is part of the hit target**; minimum hit target 32×32px.

---

### 4.10 · Tabs

**Today: three incompatible implementations.**

1. **Radix** ([`tabs.tsx`](ui/desktop/src/components/ui/tabs.tsx)) — a *segmented* control: `TabsList` is `rounded-md bg-background-default p-1`, triggers are `rounded-lg` with `data-[state=active]:bg-background-medium data-[state=active]:shadow-sm`. Three bugs in nine lines: the trigger radius (8px) exceeds the list radius (6px), so active triggers overflow the container's corners; `shadow-sm` is a dead class; and `text-muted-foreground` is an undefined token. `DR-31`
2. **Underline tabs** — `border-b-2` used in 7 places.
3. **Left-bar tabs** — `InAppTerminalDock.tsx` uses `before:absolute … w-0.5 … bg-[#b98b52]`, a hardcoded bronze that exists nowhere in the token system, plus `shadow-sm` (dead). `DR-32`

**Canonical.** Pick one — see **[Decision D-07](#d-07--tab-indicator)**. Whichever wins, the spec is: 36px tall triggers, 13/18 text, `--text-muted` at rest → `--text-default` when active, 180ms indicator transition, full keyboard support (←/→, Home/End), and `role="tablist"`.

---

### 4.11 · Sidebar & navigation

> **Row density superseded 2026-10 ([Part 9.1](#91--decisions-where-the-reports-disagreed), [9.4](#94--spacing-rows-bands-radii)).** App sidebar rows are 28px at 13px with a 2px gap (`--row-height-nav`), the width is 216 to 360 (default 288), and group and bucket labels are sentence case. Crew's 32px rail is unchanged.

**Canonical.** 240px expanded, 60px collapsed. The responsive overlay also stays at the fixed 240px expanded width; it never grows to fit content. Surface `--sidebar`, right edge 1px `--sidebar-border`. The first navigation action is **New Session**, followed by Home, Workflows, Scheduler, Extensions, Skills, Knowledge, Applications, and conditional Apps. A date-grouped Recents list follows it, with **View all chat history** attached to that section; Settings remains fixed in the footer.

| | Value |
|---|---|
| New Session action | Same standard 32px navigation-row treatment; 12px horizontal inset, `--radius-md`, 16px plus icon + 14/20 label; no border, boxed ground, or special weight |
| Navigation row | 32px, 12px horizontal inset, `--radius-md`, 8px gap, 16px icon + 14/20 label; no inter-row gap |
| Recent session row | 32px, 12px horizontal inset, plain 14/20 title text; no leading icon; single-line ellipsis at the fixed sidebar width; running indicator only when live; full title and metadata remain available on hover/focus |
| Compact titlebar controls | Three 32px controls in an explicit non-drag layer above the chat header; the session title starts 8px after their measured endpoint and moves beyond the 240px sidebar when it overlays the canvas |
| Rest | transparent, `--text-muted` icon + label |
| Hover | `--sidebar-hover` |
| Active | `--sidebar-active`, `--text-default`, **plus a 2px accent bar on the leading edge** |
| Section label | 11px caps `+0.08em` `--text-subtle`, 8px inset |

The accent bar is the *only* place the coral appears in the sidebar. Active state must not rely on background alone (that fails 3:1 against the hover state).

**Today.** `--text-muted` on `--sidebar` is **4.01:1** — fails AA. `DR-04`. Sidebar rows animate in with a staggered `sidebar-item-in` keyframe per `:nth-child(1..7)` — a decorative entrance on a nav that the user sees hundreds of times a day. It is correctly disabled under `prefers-reduced-motion`. Consider removing it outright. `DR-33`

---

### 4.12 · Page header & layout

> **Superseded 2026-10 ([Part 9.1](#91--decisions-where-the-reports-disagreed)).** A page header is a 44px band with a 14px `<h1>`, actions on the right and the description in an `InfoTip`. The 24px title, paragraph and own-line action strip below are the pre-redesign canon.

**Canonical.**

```tsx
<header className="px-8 pt-12 pb-6 border-b border-border-subtle">
  <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>
  <p className="mt-1 text-sm text-text-muted">{description}</p>
</header>
```

Flat. A hairline. No gradient, no shadow, no card wrapper. Primary action, if any, sits right-aligned on the title row.

**Today: three competing page headers.**

1. **Flat + hairline** — list views (Workflows, Schedules, Sessions, Skills, Knowledge, Apps). This is the documented one.
2. **Gradient + shadow** — detail views (`SessionHistoryView`, `SharedSessionView`) use `.biorouter-page-header`, which sets `border-bottom-color: transparent !important` and substitutes a gradient wash plus `box-shadow: var(--shadow-modal-chrome-bottom)` ([`main.css:519`](ui/desktop/src/styles/main.css#L519)).
3. **Bespoke** — Schedule detail hand-rolls `px-8 pt-6 pb-4 border-b` with an `h1 pt-8`.

`DR-09` — resolve via **[Decision D-05](#d-05--elevation-policy)**.

**Content width is also unstandardised.** `ReadableContent` offers `text` = 1120px, `wide` = 1280px, `graph` = 1440px; Knowledge uses `graph`; Apps and Dashboard never wrap in `ReadableContent` at all and run full-bleed. Switching tabs visibly reflows the content column. `DR-52`

---

### 4.13 · Cards

`--background-default`, 1px `--border-subtle`, `--radius-container` (12px), 20px padding, **no shadow**. Hover (only if the card is a link): border → `--border-strong`. Metric tiles: 30px mono-light value over an 11px caps label in `--text-muted`.

Contextual summaries are not dashboard metric tiles. The chat-summary popover uses 12px sentence-case labels and 14px medium-weight values with tabular numerals, the standard compact controls, and no nested filled cards. Its To Do section is an ordered, connected step list with explicit statuses and a completion count; it is omitted when no checklist exists, including plan-only chats. Connections show checklist order, not inferred dependencies.

**Today.** The base `Card` component ships `[box-shadow:var(--shadow-default)]` ([`card.tsx:10`](ui/desktop/src/components/ui/card.tsx#L10)) — every card is elevated by default, and callers must manually cancel it. This directly contradicts the flat-surface rule. `DR-47`

`.biorouter-page-block` adds `box-shadow: 0 16px 34px -28px …, inset 0 1px 0 rgba(255,255,255,.44)` with no dark override. `DR-12`

---

### 4.14 · List rows

Full-bleed inside a `--radius-lg` shell. **36px tall** ([D-12·C](#d-12--row-density)), 8px×16px. Bottom hairline `--border-subtle`; last child none. Hover `--background-medium` at 42%. Focus-within: same, plus the standard outline. Right-side actions fade in on hover **but remain in the tab order and visible on focus-within** — a hover-only affordance is a keyboard trap.

**Today.** `.biorouter-list-row` is `border-radius: 8px` with a bottom border; the design doc says `rounded-xl` (12px). Both patterns exist in the codebase. `DR-34` → **[Decision D-04](#d-04--radius-scale)**.

---

### 4.15 · Badges, pills, chips

**Canonical.** One `Badge`. Height 20px, `--radius-sm`, 11px 500 weight, 6px padding-x. Tones: `neutral` (default), `success`, `warning`, `danger`, `info`, `accent`. Fill = `--fill-{tone}` at 12%; text = `--text-{tone}`.

**Today.** `Pill.tsx` ships `variant: 'default' | 'glass' | 'solid' | 'gradient' | 'glow'` and `color: 'blue' | 'green' | 'amber' | 'red' | 'purple' | 'slate'` — glassmorphism, gradients, glow, and a six-colour decorative palette including `purple`, which is not in the design system at all. It defaults to `glass`.

It has **zero call sites.** Delete the file. `DR-35`

---

### 4.16 · Status dots

8px circle, `--radius-full`. `--fill-success` connected · `--fill-warning` degraded · `--fill-danger` error · `--background-strong` idle. A *live* dot gets a 2px halo pulsing at 2s (disabled under reduced motion). Colour is never the only signal — always paired with a text label or an `aria-label`.

**Today.** `Dot.tsx` takes a `size` prop and renders `width: size * 2` px — a caller asking for `size={8}` gets a 16px dot. `DR-36`

---

### 4.17 · Tables

> **Markdown tables superseded 2026-10 ([Part 9.1](#91--decisions-where-the-reports-disagreed)).** Chat, documents, CSV and notebook tables share Crew's recipe: a 1px `--border-subtle` frame, radius 8, top rules only, a `--background-muted` header row at 12px muted 600, cells 8px 12px with tabular numbers and no bold first column.

Header row 32px, 11px caps `--text-muted`, bottom hairline. Body rows 36px ([D-12·C](#d-12--row-density)), hairline between. Numeric columns right-aligned with `tabular-nums`. No zebra striping, no vertical rules. Sortable headers show a 12px chevron on hover and when active.

**Markdown tables (chat and document previews).** Use a soft gray `#ecece9` header with bold dark `#292927` sentence-case labels (dark mode: `#353533` with `#eeeeeb` text), 13px base type, and 11px × 16px cell padding. Separate body rows with fine horizontal rules, emphasize the first column, and preserve authored numeric alignment. Wide tables scroll inside a keyboard-focusable region; never split a numeric value to squeeze it into a narrow pane. This markdown treatment supersedes the earlier D-21 no-header-fill rule for these surfaces.

---

### 4.18 · Chat messages

> **Headings and prose superseded 2026-10 ([Part 9.3](#93--type)).** Markdown has no coral bar beside h1 and h2; chat h1 and h2 are 17px, a document h1 is 24px at weight 400, and prose is 14/21. Blockquotes are a 2px `--border-strong` edge with no tint. Code blocks follow Crew's head-row recipe.

The chat surface is an instrument readout, not a messaging app.

| | User | Assistant |
|---|---|---|
| Alignment | Right, `max-w-[85%]` | Left, full measure |
| Container | `--background-medium` fill, 1px `--border-subtle`, `--radius-xl`, 10px×16px | **Bare** — no bubble, no fill |
| Text | `--text-default` | `--text-default` |
| Measure | 68ch | 68ch |
| Avatar | none | none |
| Actions | copy / edit, revealed on hover, kept in tab order | copy / retry / branch |

> **D-16.** The user's turn is **never** an accent surface. It shipped as
> `bg-background-accent text-text-on-accent`, so the moment D-01 made the accent coral, every user
> message became a solid orange block — the single loudest thing on a canvas whose thesis is calm.
> The tint exists only so the eye can find the turn boundary when scrolling.

Assistant prose is the page. Wrapping it in a bubble would halve the effective measure and add visual noise to the app's most-read surface. The user's turn is tinted only so the eye can find the boundary when scrolling.

Markdown responses and document previews share one typography recipe: bold headings with a small accent beside H1/H2, consistent paragraph/list spacing, softly tinted blockquotes, and distinct code headers. Chat titles use a 24px base; document titles use 28px. Use [the synthetic markdown template](docs/desktop-ui/markdown-showcase.md) to inspect headings, tables, quotes, code, math, Unicode, and nested lists together.

An empty chat's welcome sentence belongs to its tab. Select and animate it once, preserve it through tab switching, split/move/merge layouts and Settings visits, and start a fresh lifetime in a new tab or renderer. Keep the selected tab's complete frame and close control visible; use subtle neutral tab boundaries and mark the active tab with a bold label and thin bottom accent, without an accent border or surrounding shadow.

> **Superseded 2026-10 ([Part 9.7](#97--interaction-states-and-controls)).** Message actions float above the message and reserve no row; the footer described in the next paragraph is the pre-redesign canon.

Message timestamps and actions sit in a reserved footer with an 8px baseline top gap that follows the size preference. Reveal the footer when its own message is hovered or contains keyboard focus; keep it hidden at rest without shifting the conversation. On touch devices without hover, keep actions reachable. Allow wrapping at narrow widths and larger sizes.

Streaming: a 2px × 1em caret in `--text-muted`, blinking at 1s; removed on completion.

---

### 4.19 · Tool-call lines

> **D-17.** A tool call is a **line in the transcript, not a card.** An outline around every one of them
> turns a quiet conversation into a stack of boxes, and — because the app's focus treatment is now a
> surface shift — a persistent 1px rectangle around a row is read by users as a *stuck focus ring*.

**Collapsed.** A transparent, borderless row with muted text and the tool icon alone, without colored status dots. Hover and keyboard focus bring the label to `--text-default` (dark ink in light mode, light ink in dark mode), without a background wash. Running text pulses gently; reduced motion disables the pulse.

**Expanded.** One click reveals the inputs, generated code, and results together, separated by fine rules and short labels. Short content is fully visible. Long scripts, logs, and outputs show a few lines with an explicit Show more / Show less control. Arguments remain literal, selectable monospace text. No nested disclosure is required just to inspect an ordinary command and its output.

**Failure.** Keep the failure label and diagnostic message, with no red background. The words Ran, Working on, and Problem with communicate status without badge overlays; preserve any actionable permission or recovery control beneath the diagnostic.

---

### 4.20 · Empty states, skeletons, spinners

**Empty state.** Centred, max 320px: a 24px `--text-subtle` line icon, a 14/21 500 title, a 13/18 `--text-muted` sentence, and at most one `secondary` button. No illustrations, no birds.

> `components/icons/` contains `Bird1.tsx` … `Bird6.tsx`. Decorative avian marks are not part of this design system. `DR-37`

**Skeleton.** `--background-medium` fill, `--radius-sm`, shimmer 1.4s linear infinite; disabled under reduced motion (falls back to a static fill). Only for content whose shape is known.

**Spinner.** One implementation: 1.5px stroke, `currentColor`, 700ms linear rotation, sizes 14/16/20/24.

---

### 4.21 · Scrollbars

10px wide, transparent track, `--radius-full` thumb in `--border-strong`; hover `--text-subtle`. `scrollbar-gutter: stable` on scroll containers so content doesn't shift.

**Today.** `::-webkit-scrollbar-track` and `::-webkit-scrollbar-thumb` are declared **twice** with different values ([`main.css:692–731`](ui/desktop/src/styles/main.css#L692) and [`735–753`](ui/desktop/src/styles/main.css#L735)); the second silently wins. There is also a `scrollbar-width: auto !important` override. `DR-38`

---

## Part 5 · The monospace layer

Terminals, code, diffs and logs are where this app earns its trust. They were once the **least** designed
surfaces in the app — one hardcoded light code theme, one hardcoded light terminal palette, neither with a
dark variant. Both are now generated, per family, and gated on contrast. This part is the spec they are
generated *to*.

### 5.1 · Code blocks

> ✅ **Code renders in monospace.** This blockquote used to read *"Code in Biorouter does not render in a
> monospace font"* — `MarkdownContent.tsx` set `fontFamily: 'var(--font-sans)'` on the fenced-code
> renderer, and `--font-sans` was Arial, so every code block, diff and inline path was proportional and
> columns did not align. Both halves are fixed: `codeTheme.ts` sets `var(--font-mono)`, and D-06 replaced
> Arial with a native stack (`ui-monospace, 'SF Mono', SFMono-Regular, 'Cascadia Mono', Menlo, Consolas,
> 'Liberation Mono', monospace` — [`main.css:705`](ui/desktop/src/styles/main.css#L705)). The **same**
> `--font-mono` string is used verbatim by the xterm terminal, so a pasted command and its output are set
> identically (`DR-07`). `DR-46` closed — the drift register has recorded it as fixed for some time; this
> paragraph simply had not been updated to match.

**Today (colour).** ✅ **Superseded by `codeTheme.ts`.** The two divergent hand-tuned themes are gone:
`MarkdownContent.tsx` no longer imports `oneLight`, and `ArtifactViewer.tsx`'s separate static
`previewCodeTheme` is gone with it. One builder — `build(palette, tint)` in
[`ui/desktop/src/styles/codeTheme.ts`](ui/desktop/src/styles/codeTheme.ts) — produces **six** Prism themes
from the generated per-family syntax palettes (3 families × light/dark), and leaf components select one
via `useResolvedTheme()` + `useThemeFamily()`. Code blocks paint on `--background-code`, not
`--background-medium`, which is the value the palette below is actually measured against (D-20). The
`prose-invert` complaint no longer applies: the prose tokens are remapped to the Parchment tokens outright
(D-19). `DR-39` closed.

**Canonical markdown blocks.** Use a 10px-radius panel with a 1px `--border-subtle` edge and no shadow. A header at least 36px tall carries the language in its authored case and the existing copy/run controls on `--background-muted`. The body uses `--background-code`, the shared mono theme, and 14px × 16px padding. Document code preserves lines and scrolls horizontally; chat code wraps long lines. Unlabelled fences use the same card with a `text` label and Copy control.

The syntax palette is derived from the system, not imported:

The palette itself is **Parchment's ink** and is unchanged by the neutral unification; the two grounds
it is measured on are the shared `--background-code`.

| Token | Light (on `#f5f5f3`) | Dark (on `#1b1b19`) |
|---|---|---|
| Plain / punctuation | `#2a2520` | `#e8e1d2` |
| Comment | `#6f6659` *(italic)* | `#8d8266` *(italic)* |
| Keyword | `#a94f2a` | `#e8895f` |
| String | `#22784f` | `#7fbf6a` |
| Number / constant | `#8a5a00` | `#d9a441` |
| Function | `#255fb5` | `#8fb8e8` |
| Type / class | `#7847b8` | `#b98ad6` |
| Operator | `#6e6760` | `#b0a892` |
| Deleted | `#b3261e` on `#b3261e`@9% | `#f07575` on `#f07575`@10% |
| Inserted | `#1f7a3d` on `#1f7a3d`@9% | `#7ac87c` on `#7ac87c`@10% |

The two grounds above are `--background-code`, light and dark. Every foreground clears 4.5:1 on its
stated ground — verified: the tightest stops are `inserted` at **4.92:1** light and `comment` at
**4.53:1** dark. This is not a claim taken on trust: `generate-themes.mjs` recomputes all ten stops
against `--background-code` and **refuses to write the theme** if any falls below 4.5.

> The dark `comment` figure is the tightest number in the whole palette and it got tighter when the
> grounds were shared — Parchment dark code used to sit on `#16120c`, and `#1b1b19` is lighter. It still
> clears AA, and it is a stop to leave alone rather than nudge: it has 0.03 of headroom, so the generator
> will catch the next person who moves either the ink or the ground.

The diff-row tints are **9% light / 10% dark**, applied as
`color-mix(in srgb, <stop> <tint>, transparent)` over the whole line
([`codeTheme.ts:133–134`](ui/desktop/src/styles/codeTheme.ts#L133)). This document previously said 8% for
light; the dark figure was correct.

See **[Decision D-10](#d-10--code-theme)**.

**Inline code.** `--radius-sm`, `--background-medium` fill, 0.9em, 2px×5px padding, no border. (`.bg-inline-code` currently uses `::before`/`::after` pseudo-elements to inject backticks — remove them; the fill already communicates it. `DR-40`)

### 5.2 · The terminal

**Today.** ✅ **Superseded.** The single unconditional `terminalTheme` with the bespoke `#fffdf7` cream
background is gone. There are now **six** terminal palettes (3 families × light/dark), generated from each
family's `terminal` block and selected at runtime via `useResolvedTheme()` + `useThemeFamily()`;
`grep -r fffdf7 src` returns 0 hits. `DR-41` closed.

Which token the dock paints is **recorded per family rather than assumed** — `terminalGround` in the
theme definition. All three families now answer **`--background-muted`** in both modes (`#f4f4f2` /
`#232320`). Parchment used to answer `--background-code` in dark, which was a genuine difference while
the two tokens held different per-family values; once the neutral set became one set, a dock painting a
different token would have been the only surface still varying by family. The declaration stays in the
contract — the point of writing it down was that the choice be *measured*, and that is still worth doing
when the answer agrees. Parchment's dark ANSI palette was re-verified against the ground it moved to
rather than assumed compatible with it.

**Canonical.** Font: `--font-mono` at 13px/1.5 — literally the same stack and size as code blocks, so a
pasted command and its output look identical. Cursor: **the accent** — `#b85a32` light / `#e8895f` dark
(`--background-accent`), block, 1.2s blink; 4.19:1 and 6.13:1 on their grounds, clearing the 3:1 a
non-text indicator owes. Selection: **`#e4d9c3`** light and **`#403928`** dark, opaque — **no alpha**.
Those two are among the few surfaces still tinted per family: the terminal's selection wash sits beside
the cursor, and both are affordances a family tints rather than app chrome. *(This paragraph previously
specified the cursor as `--focus-ring` — a token that does not exist — and the selection as
`--background-strong` at 60%. Neither was ever what shipped.)*

#### The bright slots do not owe 4.5:1, and here is why

This table used to be introduced as *"the shipped values, with `blue` and the bright set corrected."*
**That was false.** Nothing had been corrected, because the terminal palettes had no test at all — the
claim sat in the document while five stops shipped below AA. Both halves are now addressed in code, but
not in the same way, and the difference is the point:

- **`cyan` was `#16818c` — 4.35:1, under AA.** It is a **base** slot, so it owes the full 4.5. It was
  fixed properly to `#107e89`, and darkened again to **`#107a85`** (**4.60:1**) when the dock ground
  moved to the shared `#f4f4f2`. `yellow` took the same treatment in the same pass: `#9b6818` →
  **`#976517`** (**4.55:1**). Both were tuned against Parchment's own cream ground and fell to
  4.37 / 4.35 on the shared one. **This is the sanctioned repair for a shared-ground regression —
  retune the family's own ink, never reintroduce a family-specific background.**
- **`brightRed` 3.86 · `brightBlue` 4.27 · `brightMagenta` 3.98 · `brightCyan` 3.18** are now held to a
  documented **3:1** floor, not 4.5. On a **light** ground, "bright" and AA are mutually exclusive: the
  ANSI convention is that a bright variant is *lighter* than its base, and lighter on a light ground means
  *less* contrast. Forcing 4.5 would either invert the convention or collapse the pairs — `brightCyan`
  would have to darken to roughly `#007e8a`, a hair from `cyan`'s `#107e89`, making the two
  indistinguishable. So the bright slots hold the **3:1** graphical / large-text floor, which every one of
  them clears, and the **base** slots — which carry the bulk of terminal output — hold the full **4.5**.

That is a deliberate, recorded relaxation, not an oversight. The floors and the reason for each live in
`TERMINAL_FLOORS` in
[`ui/desktop/scripts/lib/theme-contract.mjs`](ui/desktop/scripts/lib/theme-contract.mjs), and
`generate-themes.mjs` enforces them per slot at generation time — a palette that misses its floor cannot
be written to disk. Two further slots are relaxed there for reasons of their own: `black` (floor 1.0 — an
ANSI dim slot that is a lifted *ground* by convention, not text) and `brightBlack` (floor 3.0 — the
dimmed/comment slot; it measures 5.13:1 light and 4.14:1 dark, so light clears 4.5 anyway and dark
genuinely spends the relaxation).

**ANSI 16, light** — the shipped Parchment values, on the shared dock ground `#f4f4f2`. Ratios verified;
**bold = below 4.5, held to the 3:1 bright floor**:

| | Normal | | Bright | |
|---|---|---|---|---|
| Black | `#2d2a26` | 12.97 | `#6f6659` | 5.13 |
| Red | `#b63f3f` | 5.06 | `#d45252` | **3.72** |
| Green | `#22784f` | 4.93 | `#1f7a3d` | 4.88 |
| Yellow | `#976517` | 4.55 | `#8a5a00` | 5.38 |
| Blue | `#255fb5` | 5.64 | `#2f75d6` | **4.11** |
| Magenta | `#7847b8` | 5.62 | `#9462d6` | **3.83** |
| Cyan | `#107a85` | 4.60 | `#1f9aa6` | **3.06** |
| White | `#574f46` | 7.30 | `#2d2a26` | 12.97 |

Foreground `#2d2a26` (12.97:1). Note that Parchment's light `brightGreen` and `brightYellow` are *darker*
than their bases — a deliberate inversion of the convention in the two slots where it costs nothing, which
is why they clear 4.5 while the other four do not.

**ANSI 16, dark** — now on `--background-muted` `#232320` rather than `--background-code` `#16120c`.
Every value still clears its floor, because on a dark ground "brighter" and "higher contrast" point the
same way; but note the whole column dropped about 15%, since the shared muted ground is lighter than the
code ground the palette was originally measured on. `brightBlack` at 4.14 is the tightest, and it is a
3:1 slot:

| | Normal | | Bright | |
|---|---|---|---|---|
| Black | `#3a3324` | *1.26 — dim slot, floor 1.0* | `#8d8266` | 4.14 |
| Red | `#e2665c` | 4.72 | `#f0857b` | 6.26 |
| Green | `#7fbf6a` | 7.18 | `#9ad686` | 9.26 |
| Yellow | `#d9a441` | 7.01 | `#ecc063` | 9.22 |
| Blue | `#6f9fd8` | 5.72 | `#8fb8e8` | 7.65 |
| Magenta | `#b98ad6` | 5.75 | `#d0a6e8` | 7.73 |
| Cyan | `#5fb8b8` | 6.78 | `#7fd0d0` | 8.88 |
| White | `#d4cab6` | 9.70 | `#e8e1d2` | 12.10 |

Foreground `#e8e1d2` (12.10:1). `black` is the one relaxed slot: at 1.26:1 it is not text but a *lifted
ground*, which is what the ANSI dim slot is for. See **[Decision D-11](#d-11--terminal-ground)**.

---

## Part 6 · Open decisions

> ## ✅ Signed off — 2026-07-09
>
> | ID | Decision | Resolved value |
> |---|---|---|
> | D-01 | Primary CTA colour | **Coral** — light `#b85a32`, dark `#e8895f` (ink `#16120c`) |
> | D-02 | Brand-mark palette | **Retune gradient** `#cf6d47` → `#b85a32` |
> | D-03 | Focus ring | **Accent ring** `#b85a32` / `#e8895f`, 2px @ 2px offset |
> | D-04 | Radius scale | **Rows 8px · cards 12px · modals 16px** (+ 4px chips, `full`) |
> | D-05 | Elevation | **Hairline only** — delete header gradient + `Card` shadow |
> | D-06 | Typeface | **System stack** — `ui-sans-serif` / `ui-monospace` |
> | D-07 | Tab indicator | **Underline**; left bar retained for *vertical* lists only |
> | D-08 | Select | **Migrate to Radix `Select`** (+ `Command` for searchable) — *staged, see note* |
> | D-09 | Sidebar | **Keep two-tone**; `--text-muted` → `#6e6760` |
> | D-10 | Code theme | **Custom warm theme**, light + dark, token-derived |
> | D-11 | Terminal ground | **`--background-muted`** — today `#f4f4f2` / `#232320`, shared by all three families. *(Decided against Parchment's then-values `#faf8f3` / `#16120c`; the dark answer drifted to `--background-code` and was returned to `--background-muted` on 2026-08-08 — see [§5.2](#52--the-terminal).)* |
> | D-12 | Row density | **36px content · 32px sidebar navigation/session** — one fixed profile, no density setting ⚠️ *refined 2026-07-15* ⚠️ *content rows 40 → 36px, 2026-08-02 — option **B** superseded by **C** (Astryx **A-03**)* |
> | D-13 | Status colours | **Split `--fill-{s}` from `--text-{s}`**, per theme |
> | D-14 | Decorative motion | **Delete** sidebar entrance + `Bird1–6` |
>
> **Added 2026-07-09, after seeing the accent land in the running app:**
>
> | ID | Decision | Resolved value |
> |---|---|---|
> | D-15 | Focus indication | **A surface shift, not a ring.** No outline anywhere. Focused fill — as decided `#e4dcc9` / `#4d4430`, **now the shared `#e0e0dc` / `#35342f`**; the ring returns only under `prefers-contrast: more`. *Supersedes the D-03 answer.* **Amended 2026-09-08: a `[role='tab']` trigger takes no fill** — it activates on focus, so the fill sat permanently on the active tab as a grey box; its underline firming (2px → 3px, label at `--text-default`) is its focus indicator instead. **Amended 2026-09-08 (fourth): a keyboard-scrollable region with no role (`.biorouter-focus-region`) takes a 1px inset `--border-accent` edge** — it was exempted from the fill *and* from the UA ring, leaving no indicator at all. |
> | D-16 | The user's turn | **Tinted, not accent.** `--background-medium` + hairline + `--text-default`. A solid coral block shouted. |
> | D-17 | Tool calls | **Lines, not cards.** No outline, collapsed or expanded. Failure = a clear notice, with no red wash. A persistent rectangle reads as a stuck focus ring. |
> | D-18 | Hairlines | **One value.** `border-border-subtle` at full strength. Eight alpha-diluted variants (`/35`…`/70`) made adjacent panels' edges read at different weights, so they never visually aligned. |
>
> The [drift register](#part-7--drift-register) is now the active backlog. The options below are retained as
> the rationale record.

Each was rendered side-by-side in [`docs/design/design-system-gallery.html`](docs/design/design-system-gallery.html).

Ordered by blast radius.

---

#### D-01 · Primary CTA colour
The design doc says primary buttons are coral. The code makes them near-black. Only one can be true.

| | Option | Exact values | Consequence |
|---|---|---|---|
| **A** ★ | **Coral primary** | light fill `#b85a32`, dark fill `#e8895f` w/ `#16120c` text | The brand shows up where it matters. `#cf6d47` **cannot** be used — white on it is 3.54:1 and fails AA. |
| B | **Keep near-black** | `#16120c` / `#f4f0e6` | Maximum contrast (18.65:1), maximum restraint. Coral survives only as the accent bar and status dots. |
| C | **Coral outline** | transparent fill, `#b85a32` border + text | Softest; risks reading as a secondary button. |

★ Recommended: **A**. A research tool with a brand hue that never appears on the action the user came to click is a brand that doesn't exist. *Touches: `button.tsx`, `main.css`, ~276 call sites (no per-site edits).*

---

#### D-02 · Brand-mark palette
The wordmark's gradient uses `#EC5D2A` → `#57B9AF` (orange → teal). Neither is in the token system.

| | Option | Consequence |
|---|---|---|
| **A** ★ | Retune the gradient to `#cf6d47` → `#b85a32` | Mark and UI finally share a hue. Loses the teal. |
| B | Keep the mark; add `#57B9AF` as an official secondary | Introduces a second brand colour to a system built on one. |
| C | Keep the mark as-is, unmanaged | Documented exception; logo drifts from the UI forever. |

★ **A**. *Touches: `components/icons/Biorouter.tsx`.*

---

#### D-03 · Focus ring
Non-negotiable that it becomes visible. Negotiable what colour.

| | Option | Min contrast | Consequence |
|---|---|---|---|
| **A** ★ | Accent ring — `#b85a32` / `#e8895f` | 3.96:1 / 7.26:1 | Ties keyboard nav to the brand; never confused with a border. |
| B | Neutral ring — `#403928` / `#d4cab6` | 8.7:1 / 9.1:1 | Higher contrast, zero brand. |
| C | Coral `#cf6d47` both themes | **3.04:1** on the sidebar | Passes, barely. One sidebar tint change breaks it. |

★ **A**. *Touches: `main.css` (`--ring`), and the 6 competing focus treatments across 97 usages.*

---

#### D-04 · Radius scale
Seven radii today. Four proposed. The contested value is the list row.

| | Option | Consequence |
|---|---|---|
| **A** ★ | Rows 8px, cards 12px, modals 16px | Rows read as rows; the nesting reads correctly. Matches `.biorouter-list-row` today. |
| B | Rows 12px, cards 12px | Matches the old doc; rows and cards become indistinguishable. |
| C | Rows 0px (full-bleed, hairline only) | Densest, most "instrument." A real option for this app. |

★ **A**. *Touches: ~280 `rounded-*` usages.*

---

#### D-05 · Elevation policy
`.biorouter-page-header` replaced the documented hairline with a gradient + shadow. Which is the system?

| | Option | Consequence |
|---|---|---|
| **A** ★ | **Hairline only.** Delete the header gradient and `--shadow-modal-chrome-*`. | Honours P1. Flat, calm, cheap to render. |
| B | Keep the gradient wash | Softer, more "app-like." Costs a principle and two shadow tokens. |

★ **A**. Also re-register real `--shadow-*` values so the 22 dead `shadow-sm/md/lg/xl` usages either work or get deleted. *Touches: `main.css`, ~22 call sites.*

---

#### D-06 · Typeface
Today: Arial, and bare `monospace`.

| | Option | Consequence |
|---|---|---|
| **A** ★ | **System stack** — `ui-sans-serif, -apple-system, "Segoe UI", Roboto, sans-serif`; mono `ui-monospace, "SF Mono", "Cascadia Mono", Menlo, monospace` | Zero bytes, native rendering, instantly modern. Slight per-OS variance. |
| B | Ship a webfont (Inter / IBM Plex Sans) | Pixel-identical everywhere. +180 KB, and an `@font-face` pipeline the Electron build doesn't have. |
| C | Keep Arial | Honest, boring, dated. |

★ **A**. The mono choice also fixes the terminal/code-block font mismatch (`DR-07`). *Touches: `main.css:56`, `InAppTerminalDock.tsx:176`.*

---

#### D-07 · Tab indicator
Three implementations exist. One must win.

| | Option | Consequence |
|---|---|---|
| **A** ★ | **Underline** — 2px accent bar under the active label | Scales to any width, reads as navigation, already used 7×. |
| B | Segmented pill (the Radix default) | Reads as a *filter*, not navigation. Good for 2–3 short options only. |
| C | Left bar (the terminal dock's) | Correct for vertical lists; wrong for horizontal tabs. |

★ **A** for horizontal navigation, **C** retained only for vertical lists. Delete B. *Touches: `tabs.tsx`, `SettingsView.tsx`, `InAppTerminalDock.tsx`.*

---

#### D-08 · Select implementation
`react-select` injects Emotion styles that fight Tailwind (there is a comment in `Select.tsx` documenting the fight).

| | Option | Consequence |
|---|---|---|
| **A** ★ | Migrate to Radix `Select` + `Command` for searchable cases | One surface spec for dropdowns, popovers and selects. Removes a 30 KB dep and the z-index conflict. Real work: ~15 call sites. |
| B | Keep `react-select`, normalize its `classNames` | Cheap. The Emotion specificity problem stays. |

✅ **A** chosen, and it is being delivered in two stages.

**Stage 1 — done.** The visual and layering defects are fixed in place: the trigger now matches `<Input>`
exactly (36px, real 1px border, accent focus edge), the menu is the 16px popover surface, the selected
option is *emphasised* rather than inverted to a near-black block, and the menu z-index moved from
`z-[9999]` to `--z-modal-dropdown` (500) so it can no longer paint over a dialog's own close button.

**Stage 2 — not started.** The library swap itself. Of the four call sites, three are plain selects that
Radix `Select` covers directly. The fourth — the model picker in `SwitchModelModal.tsx` — is a **typeahead
combobox** (`onInputChange` + `inputValue` + live filtering) that Radix `Select` cannot express; it needs
`cmdk`, which is not a dependency. Swapping it blind, without being able to launch the GUI, would risk a
core flow for no visual gain, since Stage 1 already delivers the cohesion. Stage 2 should add `cmdk`, port
the three simple selects to Radix `Select`, and port the model picker to `Command`.

---

#### D-09 · Sidebar treatment
The warm two-tone sidebar shipped in v1.87.1.

| | Option | Consequence |
|---|---|---|
| **A** ★ | Keep two-tone; darken `--text-muted` to `#6e6760` | Fixes the 4.01:1 failure, keeps the look. |
| B | Flush — sidebar shares the canvas, separated by a hairline | Calmer, flatter, more Zed-like. Loses spatial hierarchy. |

★ **A**.

---

#### D-10 · Code theme
| | Option | Consequence |
|---|---|---|
| **A** ★ | **Custom warm theme**, light + dark, derived from the neutral ramp (palette in [5.1](#51--code-blocks)) | Code stops looking pasted-in. Every colour verified ≥4.5:1. ~60 lines of Prism token CSS. |
| B | `oneLight` + `oneDark`, switched on theme | One line of work. Cold blue-greys against warm cream, forever. |

★ **A**.

---

#### D-11 · Terminal ground
| | Option | Consequence |
|---|---|---|
| **A** ★ | Terminal bg = `--background-muted` (`#faf8f3` / `#16120c`) | Terminal, code blocks and page share one ground. |
| B | Keep the bespoke `#fffdf7` and add a dark twin | Preserves a paper-white terminal that is *slightly* brighter than the page. Deliberate, but one more untokenised colour. |

★ **A**.

---

#### D-12 · Row density

> **Sidebar value superseded 2026-10 ([Part 9.1](#91--decisions-where-the-reports-disagreed)).** The persistent sidebar row is now 28px (`--row-height-nav`). Content and settings rows are unchanged at `--row-height`; A-03's 36px is not part of the redesign.
| | Option | Consequence |
|---|---|---|
| A | 44px default, 36px compact (user-togglable) | Comfortable default, dense on request. |
| B ⚠️ | 40px content; compact sidebar rhythm | 40px content rows and 32px sidebar navigation/session rows; no setting. *Chosen and refined 2026-07-15; **superseded 2026-08-02** by **C**.* |
| **C** ✅ | **36px content; compact sidebar rhythm** | The same single profile, one grid step tighter: 36px content rows, 32px sidebar navigation/session rows; still no setting. **← chosen 2026-08-02 (Astryx [A-03](#a-03-supersedes-d-12b--2026-08-02))** |

✅ **C.** There is one authored density profile and no user-facing compact mode or density setting.
Table body rows, content lists, and settings rows use the 36px rhythm. The persistent sidebar is the
deliberate exception: primary navigation, the New Session action, and recent-session rows are 32px,
and adjacent navigation rows have no added gap. This keeps the always-visible rail dense without
changing the established type scale, icon sizes, or hit-target clarity.

##### A-03 supersedes D-12·B — 2026-08-02

The operator approved Astryx **A-03**: content list rows and table rows move from **40px to 36px**.
D-12·B's *principle* is what was signed off and it survives untouched — one authored rhythm, no compact
mode, no density setting. Only the value moves, by one 4px grid step, and the app gains roughly 10% more
rows per screen. B is retained above as the rationale record, not as the target; build to **C**.

The sidebar rhythm is unchanged at 32px, and so is the type scale. The **`lg` control height stays 40px**
([§4.1](#41--buttons)): A-03 rules on row heights, not on the control ladder. The two happened to share the
value 40px, which is exactly why this needs saying — a sweep that replaced every 40 would have moved a
control size nobody ruled on.

The full density ladder this value belongs to — control sizes, chrome bar, tabs, terminal dock strip,
icon-label gap, sidebar and reading widths — is specified in **§2.3 of
`docs/design/astryx-adoption/astryx-ui-adoption-design.md`**, which is where to read it. Only **A-03** has
been ruled on; the rest of that ladder (including its own answer for `lg`) is still proposed, so where it
disagrees with this document, this document holds until each item is settled. *(That spec is still on the
unmerged `worktree-heatmap-responsive-home` branch — read it with
`git show worktree-heatmap-responsive-home:docs/design/astryx-adoption/astryx-ui-adoption-design.md`
until it lands.)*

---

#### D-13 · Status colour architecture
| | Option | Consequence |
|---|---|---|
| **A** ★ | Split each status into `--fill-{s}` (backgrounds) and `--text-{s}` (text/icons), authored per theme | Fixes all four AA failures at the root. |
| B | Darken the shared hue until it passes on white | Makes dark mode muddy. |

★ **A**.

---

#### D-14 · Decorative motion
| | Option | Consequence |
|---|---|---|
| **A** ★ | Delete the staggered `sidebar-item-in` entrance and the `Bird1–6` marks | The nav stops performing. Honours "calm." |
| B | Keep them | A 400ms animation the user sees on every cold start. |

★ **A**.

---

## Part 6b · UI cohesion pass — 2026-07-16

> **Companion artifact:** [`docs/design/ui-overhaul/ui-cohesion-redesign.html`](docs/design/ui-overhaul/ui-cohesion-redesign.html) —
> renders each item below on real BioRouter chrome, in both themes, with a
> **Current ⇄ Redesigned** toggle. Implemented; gates: `tsc` clean, `eslint` 0
> warnings, **140/140** contrast assertions, **1004/1004** unit tests. *(Those are that pass's numbers,
> recorded as of 2026-07-16. The assertion count has since risen to **228** with the third theme family
> and `--sidebar-icon` — see [§3.0](#30--theme-families--this-document-describes-one-of-three).)*

The through-line is not new. **D-15** made focus a surface shift rather than a
ring, **D-17** made tool calls lines rather than cards, **D-18** collapsed
hairlines to one value. Outlined tabs and bordered buttons were the outliers.
The rule, stated plainly: **BioRouter signals state with fills, not outlines.**

| ID | Decision | Resolved value |
|---|---|---|
| D-19 | **Prose tokens** | `--tw-prose-*` remap to the Parchment tokens. Nothing had ever remapped them, so every element the class list forgot fell back to Tailwind's **cold blue-grey** on the warm ground: `<strong>`/h3/h4 at `#101828`, `<hr>` and table hairlines at **`#e5e7eb`** — a hex [Part 1's anti-patterns](#anti-patterns--what-biorouter-must-never-look-like) names a *"foreign body"*. A bold word was literally a different hue than the sentence around it. |
| D-20 | **The code ground** | New `--background-code` (as decided: light `#faf8f3` / dark `#16120c`; alma `#f2f3f4` / `#08213f` — **now the shared `#f5f5f3` / `#1b1b19` for every family**). The syntax palettes in [§5.1](#51--code-blocks) are verified against those values, but code blocks painted `--background-muted`, which was `#282217` in dark — so dark code rendered on a ground its palette was never measured on and `comment` sat at **4.15:1, under AA**. Not expressible with existing tokens (light wants a panel a hair off the page, dark wants the card ground), which is exactly why the value was duplicated across `codeTheme.ts`, `main.css` and `InAppTerminalDock`. `check-contrast.mjs` now guards it. |
| D-21 | **Tables** | [§4.17](#417--tables) enforced: hairline rows, no vertical rules, no header fill, 11px caps header, 13px body, `tabular-nums`, header and body both `middle`. They previously drew a 1px box on all four sides of every cell. |
| D-22 | **Heading scale** | h1 18/26 · h2 16/24 · h3 15/22 · h4 13/18 muted. h3 and h4 were both 14/600 — the same object twice. Leadings pinned, since `text-lg/base/sm` each ship a line-height that collided with the plugin's, source-order-dependently. Blockquotes no longer inject the plugin's curly quotes. |
| D-23 | **Document tabs** | Shared `.br-tabstrip` / `.br-tab`, after Safari. The strip is `--sidebar` — the **same** surface as the sidebar's titlebar band — so the window's top edge is one continuous ground and never a dark slab. Only the ACTIVE tab is painted (a floating `--background-default` pill); inactive tabs are plain text; the divider is a hairline **in the gap** that retracts around the cursor; no tab carries an outline. **Qualifies D-07:** underline stays for *navigation* tabs (settings sections); the pill is for *document* tabs (preview, terminal, and later chat groups) — things you switch between, as in Safari and VS Code. **Tab labels are `--font-sans` at 13px** (§3.2 metadata) — see D-31. |
| D-24 | **One floating surface** | `.biorouter-popover-surface` takes a **full-strength** `--border-subtle` hairline (it diluted it to 44%, breaking D-18) and the `--shadow-popover` token instead of a hand-rolled shadow — which let a bespoke Alma Mater shadow override be deleted, since it existed only to compensate for bypassing the token. Popover/dropdown radius **16 → 12px** ([§4.5](#45--dropdown-menus--popovers)); `sideOffset` 4 → 6. |
| D-25 | **Buttons** | `outline` → `secondary` on the Session-review popover: a 1px box drawn around the panel's quietest actions was the heaviest line in it. [§4.1](#41--buttons) already specified a fill. `outline` survives only for a secondary action on an already-tinted ground. |
| D-26 | **Metric tiles** | [§4.13](#413--cards)'s 30/34 mono-light readout; the fills are gone (four boxes inside an already-rounded popover). |
| D-27 | **Toast** | **The code wins; this spec changes.** [§4.3](#43--toasts-and-inline-alerts) asked for a 3px left status bar. The shipped toast carries a **tinted icon chip** on a neutral surface and is better: it reads at a glance, survives both themes, and doesn't paint a coloured stripe on a calm surface. Radius **12px**, matching every other floating surface, not §4.3's 8px. One geometry, two densities: the chip and close stay optically centred on the **first line**, so a title-only toast is a tidy 48px bar and a two-line one grows downward from the same top edge. |
| D-28 | **Titlebar** | The Dashboard control is removed (the mode is being discontinued): the strip narrows **96 → 64px** and the session-title reserve **204 → 172px**, now *derived* from `getTitlebarControlReserve()` rather than a second magic literal. The strip stays a floating `no-drag` layer **outside** the sidebar — inside it, collapsing would take the un-collapse button with it. New-window gets its own glyph; `Plus` now means only New Session. |
| D-29 | **Sidebar** | The wordmark leaves its floating `pt-10` position for a 32px row aligned (measured, not eyeballed) to the nav labels' text edge; a `Menu` section label; a full-width divider and an inset Recents well; Recents gains a disclosure so the rail can be folded away. **Amends [§4.11](#411--sidebar--navigation):** history rows now carry a leading glyph — the "no leading icon" rule is overridden deliberately, because a wall of identical text is what made history unreadable. Branch detection is title string-sniffing: `SessionSummary` exposes no kind/branch field. A real field is the right fix. |
| D-30 | **Panels de-boxed** | The preview went *panel → p-3 → bordered card → header → code*; it is now *panel → status strip → content on the ground*. The terminal's xterm host was a `rounded-md` bordered box painted `bg-background-muted` sitting inside a gutter **also** painted `bg-background-muted` — a hairline between a surface and itself. |
| D-31 | **Tab labels: sans, not mono** | **Overrides D-23's first cut, 2026-07-16, on user review.** Tab labels shipped in `--font-mono` at 12px — borrowed from otty.sh, which sets its UI labels in JetBrains Mono, and defended as on-thesis because [P6](#p6--the-monospace-layer-is-part-of-the-design-system) calls monospace "a first-class citizen". In the running app it read as *a special thin font that belongs to nothing else on screen*: the sidebar, the nav, the chat and the session title are all `--font-sans`, so the tab strip — sharing the same 52px bar with all of them — was the only element speaking a different language. **That is precisely the incoherence this pass exists to remove**, so the rule is now: `--font-sans` at 13px (§3.2 "Secondary / metadata"), normal tracking. P6 is unchanged and still right — the monospace layer keeps the jobs it earns (code, the terminal, paths, aligned figures). A tab label is none of those; it is a name. Mono for *data*, sans for *chrome*. |
| D-32 | **The yield ladder** | **The active chat always wins. Everything else yields, in a fixed order, and the order is the design.** The app already had two isolated rules — the sidebar auto-collapses under 1120px (`AppLayout.tsx:15`) and the artifact panel overlays under `MOBILE_BREAKPOINT` 930px (`use-mobile.ts:3`) — but nothing said what should happen between them, so a narrow window squeezed the chat instead of shedding chrome. The ladder, widest-yields-first: **(1)** the sidebar collapses to an overlay (< 1120, already shipped and correct); **(2)** the preview panel narrows to its 360px floor, then yields entirely rather than starve the transcript; **(3)** tab labels shrink to their 88px floor, then scroll, then collapse into a ▾ overflow menu — **never wrap** (a wrapped second row moves every tab under the cursor); **(4)** a split merges back to one group rather than render two useless slivers. Floors, not guesses: a chat pane below ~360px cannot hold a 68ch measure and stops being a chat. Nothing in this ladder is new chrome — each rung is a thing the app can already do; the decision is only *what order they let go in*. |
| D-33 | **Mono for data, sans for chrome** | **Generalises D-31 from tab labels to every surface, 2026-07-17, after the user reported the "wrong font" still present in the chat *and preview*.** D-31 fixed the tab strips (both of them — the preview panel shares chat's `.br-tabstrip`/`.br-tab__label`, so one line covered both). But the preview's status strip still set *all* of its text in mono, so the panel's chrome kept reading in a different voice from the chat beside it: the same drift, one surface over. The rule that resolves it, and the test to apply per-usage rather than per-file: **monospace is a claim that the glyphs matter** — either you will read this character by character, or the digits must not jitter. Under that test, in `ArtifactViewer`: the file path (`l` vs `1` vs `I`), the git ref, and a `tabular-nums` count all **keep** mono; the language chip ("TYPESCRIPT") and the git-status legend ("Modified") are prose that names a thing, and go **sans**. `STRIP_META_CLASS` had been doing double duty for both kinds, which is how the drift hid — it split into `STRIP_IDENT_CLASS` (mono, earned) and `STRIP_META_CLASS` (sans). [P6](#p6--the-monospace-layer-is-part-of-the-design-system) is unchanged and still right; this is what "the jobs it earns" means in practice. |
| D-34 | **A tab opens already named** | **2026-07-17, on user report:** clicking a chat in Recents opened a tab titled "New Session", which sat there ~1s and then popped to the real name. The name was never missing — the sidebar is *rendering it on the row being clicked*. It simply wasn't handed over, so the tab was born with a placeholder and waited for `BaseChat` to fetch the session and fire `onSessionUpdate` → `renameTab`: a round-trip to the server for a string already on screen. **The name now travels with the click** (`RecentChats.onOpen(id, name)` → route state → `UrlOpenRequest.title` → the `openTab` payload). The late rename **stays** and is not a fallback nobody hits: a deep link, a reload, a fresh chat and any external nav carry no state and still open on the placeholder; a session renamed mid-turn still has to propagate; and `SessionSummary`, the list payload, has no `user_set_name`, so the load remains the only authority on that flag. **This removes the flash, not the correction.** The general rule: *if the opener already knows a value, hand it over — don't make the openee re-derive it and repaint.* |
| D-35 | **The tab model is a browser's, and so is the keyboard** | **2026-07-17, on user request:** once the chat area became tabs (D-23), the tabs had to *behave* like the ones everyone already knows. Four bindings, no invention: **Cmd/Ctrl+T** new tab = new chat (a tab with no session — which is already exactly what the centered empty composer renders), **Cmd/Ctrl+N** new window, **Cmd/Ctrl+W** close tab with **Shift+Cmd+W** close window (Safari's split, landed earlier), **Ctrl+Tab / Ctrl+Shift+Tab** cycle left-to-right, wrapping. The preview panel cycles its own stack on the same key, and **focus decides which strip answers** — one predicate consulted by both, because both listen on `window` in the capture phase and capture order is *mount* order, which is no basis for a keyboard contract. **Why Ctrl+Tab and not Cmd+Tab, on a Mac:** Cmd+Tab is the macOS application switcher. The window server claims it before any application is consulted, so it is not ours to take — Safari and Chrome both cycle tabs with Ctrl+Tab on macOS for that exact reason. Ctrl+Tab is not a Windows habit leaking onto the Mac; it *is* the Mac convention. **And the trap this pass exists to record:** an Electron menu accelerator is consumed by the menu *before the renderer ever sees the keydown*, so a key the menu owns cannot be answered in React no matter how the listener is written. Cmd+W was silently owned by `role: 'close'` (no visible `accelerator:` line); Cmd+T was silently owned by "Go → New Chat", which merely navigated Home. Both had to move to menu item + IPC, with the renderer deciding what the key *means*. Ctrl+Tab is the opposite case — a dump of the built menu shows nothing claims it — so it is an honest DOM listener. The rule, and it is cheap: **dump the built menu before writing a renderer key handler**; a unit test cannot see a menu and will pass while the key never arrives. Ctrl+Tab takes **no text-input guard** — it has no editing meaning in a text field, and a browser switches tabs whether or not you are typing; guarding on "focus is in a textarea" would kill the shortcut exactly where the user lives. Plain Tab still moves focus, because the predicate requires Ctrl. |
| D-36 | **The yield ladder's floors are on the PANE, and the 68ch rationale was aspirational** | **Corrects D-32's own arithmetic, 2026-07-17, on measurement.** D-32 justified its floor with "a chat pane below ~360px cannot hold a 68ch measure and stops being a chat". Implementation measured the thing D-32 only asserted: **a pane spends ~56px on chrome**, so the 360px floor delivers a **~304px column**, and a *column* of 360 would need a *pane* of ~416. Worse for the stated reason: 68ch of the body face is ~500px, so **no floor near 360 was ever going to hold a 68ch measure** — the rationale was reasoning backwards from a number that felt right. The floor stays at 360 (rung 2 yields the preview's column entirely at pane < 720 = 360 + 360, which measures well and is what shipped), but it is now defended honestly: **below ~360 a pane stops being usable, which is not the same claim as "holds 68ch"**. Recorded rather than quietly re-numbered — the fix for a wrong rationale is a right one, not a silent edit. |
| D-37 | **A split the user made by hand is never merged away** | **2026-07-17, decided on implementation.** Rung 4 merges a split back to one group when the window shrinks past what the layout needs — but only on a **crossing**. A split *created* at a narrow width is left alone, so a 4-up dragged out at 1400px sits at 169px panes and stays there. This is D-32's own wording ("merges *back*") and the [sidebar bug](#the-sidebar-bug)'s lesson applied: a watcher that dissolved a split the instant you made it would be **fighting the drop that just happened**, and the user would lose an argument they never knew they were having. If slivers should never exist at all, the fix belongs in the **drop** — refuse the split, at the moment of the gesture, where the user can see why — not in a watcher that undoes it afterwards. Not implemented; a deliberate open question, not an oversight. |
| D-38 | **A composite mark is centered by its UNION, not its type** | **2026-07-17, on the BR app-icon.** The proposed `BR` monogram (navy `#052049` B, coral `#b85a32` R, a split navy/coral underline on the `#faf8f3` plate) read as *sitting low*. Cause: the **letters** were centered and the underline hung **below** them, so the mark's real mass dropped — the underline was never counted. The rule: **when a mark is more than its letters, center the union of every part as one body.** Corollary for sizing: express size as the union's **share of the plate**, not a font-size in px — a px size means different footprints in different faces, but a *share* is font-independent and survives the round-trip from a mock to the real asset. Both are computed live from rendered geometry (`getBBox`), so the "center" and "size" defaults are **measured, not guessed**, and self-calibrate to whatever font renders. The interactive studio is [`docs/design/branding/logo-icon-studio.html`](docs/design/branding/logo-icon-studio.html) (dials: vertical position, mark size, underline gap; text export/import). **This BR mark is now the shipped identity** — the abstract circle glyph is retired (D-40). Finalized square icon **(set in Inter, 2026-07-18 re-tune — see [D-41](#d-41--the-brand-font-is-inter))**: mark size **60%**, vertical offset **−10**, underline gap 2%, on the cream plate, inset to the macOS icon safe-area (plate 824/1024, ~100px margin) so it does not read oversized in the dock. |
| D-39 | **The BioRouter wordmark** | **2026-07-17, approved.** The horizontal logo, same family as the D-38 square mark: **`Bio`** in UCSF navy `#052049`, **`Router`** in coral `#b85a32`, over a **short** two-tone bar that sits **between the o and the R** (underlining the "oR" pair, split navy/coral at the o\|R seam) — *not* a rule under the whole word. Approved ratios: **weight 600, letter-spacing 0, underline gap 2% of cap height, thickness 10%, width 100% (the oR pair).** On a dark app surface UCSF navy all but vanishes, so the navy role — the `Bio` letters + the navy half of the bar — becomes **UCSF teal `#18A3AC`**; `Router` stays coral. On a light *plate* the letters stay navy regardless of theme (the plate is its own light ground). Shipped as `<BioRouterWordmark>` / `<BioRouterMark>` (`components/icons/`), which **measure their geometry at runtime** (`getBBox` + `getStartPositionOfChar`) rather than baking coordinates, so the underline stays right in any font. Studio: [`docs/design/branding/logo-wordmark-studio.html`](docs/design/branding/logo-wordmark-studio.html). **Runtime-measurement gotcha (learned the hard way):** measuring in a dependency-less `useLayoutEffect` loops forever — `getBBox` jitters sub-pixel between calls, so a stringify guard never matches; measure **once** on mount + on `fonts.ready`, never per render. jsdom has no `getBBox` at all, so this bug is invisible to unit tests and only shows in a real engine. |
| D-40 | **The abstract circle glyph is retired** | **2026-07-17.** The old mark was an abstract circle-and-nodes glyph, rendered as a mono `currentColor` mask (`glyph.svg`) and rasterized into the app icon. It is replaced everywhere: the **app icon** (`icon.icns`/`.ico`/`.png`) is now the BR mark on the cream plate, **inset to the macOS icon safe-area** (the plate is 824/1024 ≈ 79% of the tile, ~100px transparent margin, so it doesn't read oversized next to native icons — a full-bleed plate looked bigger than Chrome); the **menu-bar template** (`glyph.svg` → 22px `iconTemplate`) is a **monochrome BR**; and every in-app lockup (sidebar brand row, welcome, loader) flies the two-tone `<BioRouterWordmark>`/`<BioRouterMark>`. Rasters are built by resizing a **browser-rendered** PNG master, never by `sips`-rasterizing the text SVG — `sips` flattens font-weight, turning the heavy 800 mark medium. |
| <a name="d-41--the-brand-font-is-inter"></a>D-41 | **The brand font is Inter, not SF Pro** | **2026-07-18.** The mark shipped in the native UI stack (`-apple-system` → **SF Pro** on macOS), but Apple's font license forbids SF Pro "in app icons, logos, or any trademark use" — fine as live UI text, wrong as a brand mark baked into cross-platform icon rasters and a served landing SVG; a native stack also renders the logo in a *different* face per OS. From a 7-font study (all **SIL Open Font License**, which explicitly permits logos, embedding, and outlining) the user chose **Inter** — closest to SF Pro's face with a rounder feel. Shipped as a latin-subset **variable** `@font-face` embedded (data URI) in `main.css` — a deliberate **logo-only exception to D-06's** native-stack rule, used only by `<BioRouterWordmark>`/`<BioRouterMark>` (native stack stays as the load fallback); the icon SVGs embed a weight-800 Inter `@font-face` so they rasterize Inter without it installed. Re-tuned in the (now-Inter) studios: the **BR icon** to size **60%** / offset **−10** / gap 2% (supersedes D-38's 70/−20), the **wordmark** ratios unchanged (D-39). Every raster + the CLI/landing logos re-propagated via `prepare.sh`. |

**Deferred, and specified:** the tabbed + splittable chat area (Y/Z/Æ/Ø in the
companion) maps BioRouter onto VS Code's layout — primary sidebar → the rail,
editor group → chat group, panel → terminal dock, secondary side bar → preview
panel; the preview follows the **active** group, the terminal stays global. It
needs multi-session state and drag/drop, so it is a feature, not a re-skin. The
shared tab component it will use is already in place (D-23).

---

## Part 7 · Drift register

> ### Implementation status — 2026-07-09
>
> The decisions in Part 6 are signed off and the fix pass has run. Everything below is **verified on
> disk**, not asserted:
>
> | Check | Command | Result |
> |---|---|---|
> | Dead Tailwind classes (`shadow-sm/md/lg/xl/2xl/inner`) | `rg -oI '\bshadow-(sm\|md\|lg\|xl\|2xl\|inner)\b' src` | **0** |
> | Undefined tokens (`destructive`, `muted-foreground`, `bg-primary`, `bg-app`, `animate-indeterminate`, `textPlaceholder`, `ring-offset-background`) | `rg -oI …` | **0** |
> | Classes that defeat the focus ring (`outline-none`, `outline-hidden`) | `rg -oI 'outline-none\|outline-hidden' src` | **0** *(was 68 across 40 files)* |
> | Undefined CSS variables in the stylesheets | var-vs-`var()` diff | **0** *(was 4)* |
> | Legacy brand hex (`#EC5D2A` / `#57B9AF`) | `rg -oI` | **0** |
> | WCAG contrast assertions | `npm run check:contrast` | **228 / 228 pass** *(58 at sign-off; grew with the code ground (D-20), `--sidebar-icon`, and ×3 theme families)* |
| Theme palette contrast | `npm run themes` | **gate** — refuses to emit a theme whose syntax palette misses 4.5:1 on its `--background-code`, or whose terminal palette misses its `TERMINAL_FLOORS` slot floor |
> | Code-palette contrast | `vitest src/styles/codeTheme.test.ts` | **7 / 7 pass** |
> | TypeScript | `npx tsc --noEmit` | **clean** |
> | Unit tests | `npx vitest run` | **600 pass**, 1 pre-existing failure (`dashboardStorage`, unrelated) |
> | ESLint | `npx eslint src` | **no new errors** (43 → 43; the +1 is pre-existing uncommitted test code) |
>
> A permanent guard, [`ui/desktop/scripts/check-contrast.mjs`](ui/desktop/scripts/check-contrast.mjs),
> parses the real token file, resolves the `var()` chains, and fails `npm run lint:check` if any
> contrast pair regresses.
>
> **Still open (deliberately):**
> - `DR-24` — 104 raw `<button>` elements in 58 files still bypass `<Button>`. Bulk conversion changes
>   layout and cannot be verified without launching the GUI; the dead classes and focus rings on those
>   elements *are* fixed.
> - **D-08 stage 2** — the `react-select` → Radix + `cmdk` swap. See the note under
>   [D-08](#d-08--select-implementation).
> - `DR-49` — the ~17 hand-rolled modals now have correct surfaces and z-indices, but still lack a
>   focus trap and `role="dialog"`.
> - `DR-52` — content max-width still varies (1120 / 1280 / 1440 / uncapped).

> **The register carries a status column, and it is load-bearing.** This table
> was written as a point-in-time audit with no status, so it kept reading as
> *live* long after items were fixed — `DR-07`, `DR-32`, `DR-41` and `DR-48`
> were all closed months before anyone noticed the register still indicted
> them, and their `file:line` evidence now points at unrelated code. A backlog
> that cannot say "done" sends people chasing ghosts. **Every row must carry a
> status; every status must be re-verified against the code, not against this
> document.** Statuses below were re-verified on **2026-07-16**, and again on
> **2026-09-19** for `DR-01`, `DR-03`, `DR-04`, `DR-06`, `DR-10`, `DR-15`,
> `DR-16`, `DR-17`, `DR-19` and `DR-22`, all of which the Astryx work closed and
> which are now marked fixed with current evidence. **The remaining `open` rows
> carry their 2026-07-16 status and have not been re-checked since**, so several
> of them are likely closed too; confirm a row against the tree before acting on
> it.
>
> **A row's `file:line` evidence is as-audited, not current.** `main.css` has
> grown past 5,300 lines and most of the line numbers in this document, in Part 7
> and elsewhere, now point at unrelated code. Grep for the token or selector
> name; a name survives the file growing, a line number does not.

The original backlog, as audited:

| ID | Sev | Status | What | Evidence |
|---|---|---|---|---|
| `DR-01` | High | ✅ fixed | Primary button fill is near-black, not the documented coral | `main.css:628` sets `--background-accent: var(--color-coral-600)`; §3.1 records D-01 shipped. |
| `DR-02` | Med | open | `--color-block-teal` holds a coral (`#cf6d47`); `--color-block-orange` holds a deep coral | `main.css:20–21` |
| `DR-03` | **High** | ✅ fixed | All four light-mode status text colours fail AA (1.85–3.65:1) | `main.css:711` sets `--text-danger: var(--color-red-600)`, 6.54:1 on white, the canonical §3.1 value. |
| `DR-04` | High | ✅ fixed | `--text-muted` on the sidebar is 4.01:1 — fails AA | `main.css:705` sets `--text-muted: #635c54`, min 5.56:1. |
| `DR-05` | High | open | `--text-subtle` is 3.28:1 on white — fails AA everywhere in light mode | `main.css:86` |
| `DR-06` | High | ✅ fixed | `--border-default`, `--border-input`, `--border-strong` are all `neutral-100` in light mode | `main.css:689-692` holds three distinct values; the comment beside them records DR-06/DR-51. |
| `DR-07` | Low | ✅ fixed | Terminal font (`Menlo` 12.5px) ≠ code-block font (`monospace`) | Both are `--font-mono` at 13/20 — `InAppTerminalDock.tsx:60` vs `codeTheme.ts:21`. |
| `DR-08` | Low | open | `<Input>` is 16px below 930px, 14px above; `Select` is always 14px | `input.tsx:11` |
| `DR-09` | Med | open | `.biorouter-page-header` nulls the documented hairline and adds a shadow | `main.css:519` |
| `DR-10` | Med | ✅ fixed | Seven border radii in TSX; no `--radius` token exists | `main.css:37-49` defines the `--radius-inner/element/container/surface` ladder, with the old size names as aliases. |
| `DR-11` | **High** | open | `--shadow-*: initial` makes `shadow-sm/md/lg/xl` dead — 22 usages render nothing | `main.css:15` (verified by compiling Tailwind) |
| `DR-12` | Med | open | `inset 0 1px 0 rgba(255,255,255,.42)` glare line on dark panels | `main.css:494`, `529` |
| `DR-13` | Low | open | Six transition durations; `prefers-reduced-motion` covers only 2 of 8 animations | 107 usages |
| `DR-14` | **High** | open | `react-select` portals to `z-[9999]`, above the modal at `z-[1210]` | `Select.tsx:27`, `dialog.tsx:52` |
| `DR-15` | **High** | ✅ fixed | Focus ring is 1.14:1 (light) / 1.72:1 (dark) — invisible | `main.css:763` ships D-15's `--background-focus`; `input.tsx:25` says "D-15: focus is a surface shift, never a ring". |
| `DR-16` | High | ✅ fixed | Six competing focus treatments across 131 usages | Closed by the same decision: one treatment, owned globally by `main.css`. |
| `DR-17` | Low | ✅ fixed | `react-icons` + `@radix-ui/react-icons` in `package.json`, zero imports | Neither package is in `ui/desktop/package.json`. |
| `DR-18` | Med | open | 96 inline `<svg>` literals in view components | app-wide |
| `DR-19` | Med | ✅ fixed | Logo gradient uses `#EC5D2A`/`#57B9AF`, in no token | `grep -r '#EC5D2A\|#57B9AF' ui/desktop/src` returns 0. |
| `DR-20` | Med | open | Button `outline` variant has no border; identical to `secondary` | `button.tsx:15–18` |
| `DR-21` | Low | open | `shape="pill"` renders `rounded-md`; `shape="round"` also renders `rounded-md` | `button.tsx:28–31` |
| `DR-22` | **High** | ✅ fixed | `destructive` is undefined: `bg-destructive` (8×), `text-destructive`, `border-destructive`, `ring-destructive` all dead. Error banners render unstyled. | `grep -r bg-destructive ui/desktop/src` returns 0. |
| `DR-23` | Low | open | `![&_svg…]` uses Tailwind v3 important syntax under v4 | `button.tsx:23` |
| `DR-24` | High | open | 103 raw `<button>` in 58 files bypass `<Button>` | app-wide |
| `DR-25` | High | open | Diagnostics panel is white-on-cream in dark mode | `main.css:447–466` |
| `DR-26` | Low | open | `ring-offset-background` undefined (3 usages) | `dialog.tsx:58` |
| `DR-27` | Low | open | Native `title=` tooltips coexist with `Tooltip.tsx` | app-wide |
| `DR-28` | Med | open | Select control 6px radius, its own menu 12px | `Select.tsx:14,26` |
| `DR-29` | Low | open | `placeholder:font-light` diverges from `Select`'s placeholder | `input.tsx:11` |
| `DR-30` | Med | open | Selected select-option fills near-black | `Select.tsx:33` |
| `DR-31` | Med | open | Radix tab trigger radius (8px) > list radius (6px); `shadow-sm` and `text-muted-foreground` both dead | `tabs.tsx:25,41` |
| `DR-32` | Med | ✅ fixed | Terminal tabs use hardcoded `#b98b52` | Tab indicator uses `bg-accent-bar`; `grep -r b98b52 src` → 0 hits. |
| `DR-33` | Low | open | Staggered per-`nth-child` sidebar entrance animation | `main.css:338–372` |
| `DR-34` | Med | open | Two list-row radii (8px vs 12px) coexist | `main.css:543` vs doc |
| `DR-35` | Low | open | `Pill.tsx` ships glass/gradient/glow + 6 decorative colours; zero call sites | `ui/Pill.tsx` |
| `DR-36` | Low | open | `Dot.tsx` renders `size * 2` px | `ui/Dot.tsx` |
| `DR-37` | Low | open | `Bird1–6.tsx` decorative marks | `components/icons/` |
| `DR-38` | Med | open | `::-webkit-scrollbar-thumb` declared twice with different values | `main.css:704` & `740` |
| `DR-39` | **High** | ✅ fixed | Code blocks are `oneLight` in **both** themes; no dark theme imported | `codeTheme.ts` ships per-family light+dark palettes. |
| `DR-40` | Low | open | `.bg-inline-code` injects backticks via `::before`/`::after` | `main.css:868–905` |
| `DR-41` | **High** | ✅ fixed | Terminal has one hardcoded light theme applied unconditionally | Six palettes (3 families × 2 modes), generated from `themes/*.theme.mjs` and switched via `useResolvedTheme` + `useThemeFamily`; `grep -r fffdf7 src` → 0 hits. |
| `DR-42` | Med | ✅ fixed | Terminal `blue` 4.45:1; `brightBlack` (dim text) 4.32:1; 6 bright colours fail AA | Re-measured on the *shipped* ground: `blue` `#255fb5` is 5.85:1 and `brightBlack` `#6f6659` 5.32:1 — both clear AA (the audited figures were against the retired `#fffdf7` cream). The one real base-slot failure, `cyan` `#16818c` at 4.35:1, is now `#107e89` at **4.53:1**. The four sub-4.5 *bright* slots are held to a documented **3:1** floor with the reason recorded — see [§5.2](#52--the-terminal) and `TERMINAL_FLOORS`. Enforced per slot by `generate-themes.mjs`. |
| `DR-43` | Med | open | 88 distinct hardcoded hex values in TSX | app-wide |
| `DR-44` | Med | open | Twelve ad-hoc z-index layers | app-wide |
| `DR-45` | Med | ✅ fixed | Global `* { @apply border-border-default }` paints every element's border colour | The global is **kept deliberately** and re-pointed: `*, ::before, ::after { border-color: var(--border-subtle) }` ([`main.css:749`](ui/desktop/src/styles/main.css#L749)). Deleting it would let Tailwind v4 preflight default every un-coloured border to `currentColor` — the *text* colour — which is worse. The defect was the near-invisible `neutral-100` it resolved to, and that is gone. |
| `DR-46` | **High** | ✅ fixed | **Code renders in Arial.** Fenced blocks, the wrapper, inline code and `prose-code` are all `font-sans` | Code renders in `var(--font-mono)` (`codeTheme.ts:20`). |
| `DR-47` | High | open | Base `Card` ships `[box-shadow:var(--shadow-default)]`; every card is elevated by default | `card.tsx:10` |
| `DR-48` | High | ✅ fixed | Toast is a static `text-white bg-neutral-800/95` — hardcoded dark in both themes, raw neutral, 12px radius | `App.tsx` uses `TOAST_SURFACE_CLASS_NAME` (`alerts/NotificationSurface.tsx`) — tokenised + theme-aware; `grep -r "bg-neutral-800/95" src` → 0 hits. |
| `DR-49` | **High** | open | ~17 hand-rolled modals with no focus trap, no `role="dialog"`, no `aria-modal` | `BaseModal.tsx:17` + 16 others |
| `DR-50` | High | open | `<Input>` is `border-0`, so caller-supplied `border-*` colours never paint — validation borders are no-ops | `input.tsx:11` |
| `DR-51` | High | ✅ fixed | `--border-strong` is *lighter* than `--border-subtle`; hover **weakens** the border | `--border-subtle` = `neutral-200` `#e8e1d2`, `--border-strong` = `neutral-300` `#d4cab6` ([`main.css:141–142`](ui/desktop/src/styles/main.css#L141)). `strong` is now darker, so `hover:border-border-strong` firms the edge. Guarded: `check-contrast.mjs` asserts `border-strong vs subtle` ≥ 1.1 in all six theme/mode scopes. |
| `DR-52` | Med | open | Content max-width varies 1120 / 1280 / 1440 / uncapped; tabs reflow the column | `ReadableContent.tsx:9`, `AppsView.tsx:133` |
| `DR-53` | Med | ✅ fixed | ~15 direct `lucide-react` imports bypass the `light()` wrapper → `strokeWidth` 2 vs 1.5 | The 15 direct `lucide-react` importers now route through `app-icons`' `light()` wrapper; the 2 remaining are the wrapper itself and a type-only import. |
| `DR-54` | Med | open | Undefined tokens referenced app-wide: `--color-primary`, `--color-textPlaceholder`, `--color-background-subtle`, `--color-background-app` (`bg-app`), `animate-indeterminate` | `ToolCallWithResponse.tsx:858,862`; `PasteTextBox.tsx:36`; `ProviderSelector.tsx:89` |
| `DR-55` | Med | open | `search.css` references `--text-standard` / `--text-prominent`, defined nowhere; highlight is a hardcoded yellow with no dark variant | `search.css:4,12` |
| `DR-56` | Med | ✅ fixed | Two accent utility families: `bg-accent` (constant `#16120c`, never flips) vs `bg-background-accent` (flips per theme) | The constant `--accent` token is gone and `grep -r 'bg-accent\b' src` → **0** usages. What remains is `--accent-bar` (`#cf6d47` / `#e8895f`), which is a *different thing* with a defined job — the sidebar rail, tab underline and status dots — and does flip per theme. |
| `DR-57` | Low | open | `--breakpoint-md` is 930px, but hardcoded `768px`/`767px` media queries remain | `main.css:6,660,805` |
| `DR-58` | Med | open | `InAppTerminalDock` carries 9+ raw warm-beige hex literals where sidebar tokens already encode the palette | `InAppTerminalDock.tsx:302,396,401,415,416,424,436,474` |
| `DR-59` | Med | open | Empty / loading / error states hand-rolled in all 7 list views; icon sizes, alignment, text sizes and heights all differ | `WorkflowsView.tsx:651`, `SessionListView.tsx:763`, +5 |
| `DR-60` | Med | open | Light-mode `--text-default/-muted/-subtle` are raw hex outside the neutral ramp; dark mode derives from it — asymmetric | `main.css:83–86` vs `151–153` |
| `DR-61` | Med | ✅ fixed | Boot splash `--br-navy` `#052049` never flipped for dark mode, and the mark has no plate — the navy half of the BR mark was invisible on **every** dark splash: **1.02:1** (Parchment, then `#282217`), **1.12:1** (Alma Mater, then `#0d2a50`), **1.02:1** (Roche Limit `#232320`). D-39 had already decided this case for `<BioRouterMark>`; the splash was a separate literal path that never got the rule. The dark inks now flip (Parchment and Alma Mater to teal `#18a3ac`, Roche to its light ink), and `generate-themes.mjs` asserts the navy half at 3:1 against the splash ground — **5.16:1** on the now-shared dark ground `#232320`. | `index.html` `THEMES:GENERATED:SPLASH`; `themes/*.theme.mjs` `mark.navy` |
| `DR-62` | Med | open | Content rows, settings rows and table body rows still render **40px** against the 36px canonical ([D-12·C](#d-12--row-density), 2026-08-02). One token plus one markdown-table rule cover almost all of it; three call sites hardcode the height instead. | `main.css:97` (`--row-height`), consumed at `main.css:1528` (`.biorouter-list-row`) and `main.css:1618` (`.biorouter-settings-row`); `main.css:1918` (`.prose tbody td`, comments at `1881`/`1914`); `SessionListView.tsx:251`, `ExportAppDialog.tsx:36`, `ModelBreakdownTable.tsx:63` |

**Totals:** 62 items — **18 high, 28 medium, 16 low.**

- **Dead code / silent no-ops (14):** `DR-11`, `DR-17`, `DR-21`, `DR-22`, `DR-23`, `DR-26`, `DR-31`, `DR-35`, `DR-40`, `DR-50`, `DR-51`, `DR-54`, `DR-55`, `DR-56`
- **WCAG AA failures (5):** `DR-03`, `DR-04`, `DR-05`, `DR-15`, `DR-42`
- **Dark mode never authored (6):** `DR-25`, `DR-39`, `DR-41`, `DR-48`, `DR-55`, `DR-61`
- **Accessibility beyond contrast (2):** `DR-49` (no focus trap on 17 modals), `DR-16` (six focus treatments)
- **Duplicate implementations (7):** modals, tabs, page headers, empty states, focus, code themes, selects

---

## Part 8 · Governance

1. **New colour?** It goes in `main.css` as a semantic token with a light *and* dark value, and its contrast is verified against every ground it can appear on. No hex in a `.tsx` file, ever.
2. **New variant?** It goes in the primitive (`button.tsx`, `input.tsx`). A `className` override that changes colour, radius, height, or border is a bug report against the primitive.
3. **New animation?** It uses one of the three durations and two easings, and it is nulled under `prefers-reduced-motion`.
4. **New surface?** It is flat unless it is a modal, a popover, a toast, or the composer.
5. **CI should enforce** what review can't: no hex literals in `.tsx`; no `<button>`/`<input>`/`<select>` outside `components/ui/`; no `shadow-*` outside the four permitted tokens; contrast assertions on the token pairs in [3.1](#31--colour).

`.claude/commands/frontend-design.md` is now downstream of this file. Regenerate it from Parts 1–3 once Part 6 is settled.

---

## Part 9 · 2026-10 redesign decisions

> **What this is.** The record of the Codex simplicity redesign (2026-10): the decisions it made where
> the research disagreed, the principles it binds, and the tokens and control specs it sets.
> **Status:** Current. Where this part and an earlier passage disagree, **this part wins**. The
> passages it supersedes carry an inline note pointing here, and are otherwise left as written
> because they record what was true when they were signed off.
> **Source:** the implementation spec,
> [`docs/design/codex-simplicity-redesign/implementation-spec.md`](docs/design/codex-simplicity-redesign/implementation-spec.md)
> (sections 0, 1 and 2, plus the owner decisions in section 6), and the requirement list `R-01` to
> `R-24` in
> [`requirements-and-plan.md`](docs/design/codex-simplicity-redesign/requirements-and-plan.md), plus the
> owner message 13 rulings (spec section 9, `R-25` to `R-27`), recorded in 9.11. Numbers below were
> checked against `main.css` and the contrast checker when this part was written.

The owner's brief, in one line: **simplicity above all**, Codex's structure and restraint, Biorouter's
coral accent and personality kept, Crew as the in-house reference, the website's Arial face with few
sizes, hover help instead of paragraphs, one control style, and a sidebar as dense as Codex's or
Claude Code's.

### 9.1 · Decisions where the reports disagreed

| Question | Decision | Why |
|---|---|---|
| UI face | **Arial first**, byte-equal to the website's `--font` (`landing/site.css`). Inter is the wordmark and the BR mark only. One mono stack. | The owner settled it on 2026-10-09. Arial draws only 400 and 700, so 500 renders as 400 and 600 as 700. `styles/fontStacks.test.ts` pins the stack to the website's. |
| Visible text sizes | **12 · 13 · 14 · 17 · 24.** Retire 10, 11, 15, 16, 19, 20, 28 and 30. | Crew already runs on 12/13/14 with 17 for one title. 24 is the one large step. Dropping 20 keeps the count at five. |
| Sidebar row height | **28px rows, 13px text, 2px gap**, for the app sidebar only (`--row-height-nav`). Every other part of the row follows Crew's recipe. Crew keeps its 32px rail untouched. | The owner asked for a compact, Codex-like sidebar. On `/crew` at wide windows two pitches sit side by side, which is a known cost. |
| Section labels | **One caps style survives, for page section labels only**: `text-caps` is 12/16/500 at +0.04em. Sidebar groups, menu group labels and bucket labels are sentence case at 12px. | Crew and Settings use caps section labels. The change reaches Crew's seven labels as a 1px change. |
| Summary rail and the transcript | **The grid snaps and the transcript and composer cells FLIP.** Only `transform` and `opacity` animate. Crew's width tween stays inside Crew. | The chat transcript is not virtualized, so a width tween would re-lay out every frame. |
| Overview and Models control | **Underline `Tabs`** for panel switching, **`SegmentedControl`** for a range. | Panels keep tab semantics (D-07). A range is a value, so it is a radio group. |
| Help primitive | **`InfoTip`** (`components/ui/info-tip.tsx`). | One name across the research. |
| Primary buttons | **Coral stays**, one coral-filled button per view. | The owner: retain the colour accent. |
| Focus indicator | **D-15 stays**: a surface shift for controls, an inset edge for rows. | Signed off, and nothing in the requirements reopens it. |
| Menu row height | **32px menu rows at 13px**, one recipe. The check and the value sit at the trailing edge. | Crew's menus are 32px. |
| Extension provenance | **Keep the three privacy-tiers 13.5 strings visible.** Only the built-in sentence moves into a tooltip. | A privacy mandate cannot become tooltip-only. |
| Page headers | **A 44px band with a 14px `<h1>`**, actions on the right, the description in an `InfoTip`. This reverses the 2026-09-07 "actions on their own line" decision. | Crew's channel header is the reference. |
| Markdown code blocks and tables | **Copy Crew's recipes** as shared `br-md-*` classes, one look app-wide. Code block: `--background-well`, 1px `--border-subtle`, radius 12, a head row (language, Copy, Run) over a hairline, body 10px 12px mono 13/20. Table: 1px frame, radius 8, top rules only, header row `--background-muted` at 12px muted 600, cells 8px 12px, tabular numbers, no bold first column. Chat headings stay 17px. | Chat answers are documents. Two table looks, one in Crew and one elsewhere, was the alternative. |
| Names of controls | A switch is named by its visible row label everywhere. No "Toggle …" and no "Enable …". | The spec contradicted itself in two places. |

Owner decisions that shape the rest: large titles are **400**, 17px headings stay at 600 (6.2); the
InfoTip glyph is **visible at rest**, in subtle ink (6.8); success toasts for toggles are dropped, error
toasts stay (6.9); the Settings App section keeps Configuration then Privacy first (6.5); content rows
stay at `--row-height` 40px, because A-03's 36px is not part of this pass.

### 9.2 · Principles

1. **One reading column, quiet chrome.** Every surface has one 44px band holding a 14px title and a
   cluster of 32px icon actions. Content sits in the 760px measure. Every band is a window drag region
   and every interactive thing in it is `no-drag`. Home is the one exception.
2. **Say it once, then hide the explanation.** A visible line is a label, a value or a state word.
   Explanations live in an `InfoTip`. These always stay visible: errors, refusals, privacy disclosures,
   the consequence of a destructive action at the moment of decision, and empty-state one-liners.
3. **Five sizes, two visible weights, two families.** Regular (400) and strong (600). 500 never carries
   a state alone. Mono is for machine strings only. Counts, dates, costs and percentages use the sans
   face with `tabular-nums`.
4. **Rows, not cards.** Anything enumerable is a hairline row list with one primary line and at most one
   secondary line. A card is for something that asks the person to act, or a standalone object. A box
   never sits inside a box.
5. **Each job has one control, and each control has one look.** `Switch` when the change applies now, on
   the right of the row. `Checkbox` to pick items or for an option submitted with a form, on the left.
   `SegmentedControl` for two to four short options that need no explanation. A select menu for four or
   more options, or options that each need a line of explanation. Radio rows only inside dialogs and
   forms. A trailing check inside menus. Underline `Tabs` for sections.
6. **Actions appear when wanted.** Row actions show on hover, on `:focus-within` and while their menu is
   open, and always under `(hover: none)`. Right-click, Shift+F10 and the ContextMenu key open the same
   menu as `⋯`. Only opacity changes, so the actions stay in the tab order.
7. **The accent marks state and adds zest, nothing else.** Coral appears only on the 2px active-row rail,
   the focused field and composer edge, the single primary button of a view, a ready Send button, the
   on-state of switches and checkboxes, the active tab underline, the in-progress to-do glyph, the
   composer's working sweep, links, text selection, the heatmap ramp, "New" or unread marks, the lock
   badge on private chat glyphs, the Update row's download glyph, and the landing site's coral rule.
   Hover is always neutral.
8. **Fills come before lines** (owner message 13; 9.11). In-page cards, panels, buttons, chips, tabs and
   fields rest on a subtle solid fill (`--background-fill`) with no resting outline. Lines stay only where
   they are structural (the band edge, pane dividers, resize handles, table rules inside a filled table)
   and on floating surfaces (menus, popovers, tooltips, toasts, dialogs), which keep a hairline and a
   shadow. Hairlines are straight: no `border-bottom` on a rounded row. This replaces the first wording,
   "borders come before shadows".
9. **Layout snaps and content glides.** Animate only `transform`, `opacity` and `clip-path`, with tokens
   only. The named exceptions, and no others: the Disclosure's height, the Progress fill's width,
   absolutely positioned indicators that move no other box (the segmented thumb, the tab underline), the
   long-message `max-height`, the sidebar's own gap width, and Crew's pane width. Never animate against a
   resize of the window, a split, the sidebar or the preview. A surface that mounts because of
   navigation, a reload, a tab switch or a history load appears at rest, and so does the first placement
   of a sliding indicator. Never move a row under the pointer. Reduced motion is honoured in CSS, in JS
   and in the main process.
10. **Show names, not IDs.** Machine IDs and file paths go behind a "Copy …" item or a tooltip.
11. **When unsure, copy Crew, and never edit Crew to match.** Promote Crew's recipes under `br-` names.
    Never borrow a `crew-*` class outside `crew/`.
12. **Simplicity never costs accessibility.** Everything hidden stays reachable by keyboard and by screen
    reader, and the contrast gates pass, including the 3:1 control assertions in 9.5.

Copy rules: sentence case, the noun is "chat", "Biorouter" and "BAAM", the typographic ellipsis, American
spelling, no em or en dashes as punctuation, no marketing words. A one-line menu label takes no trailing
period. InfoTip text is full sentences, at most two (about 160 characters).

### 9.3 · Type

| Role (utility) | Size / line | Weight | Tracking | Use |
|---|---|---|---|---|
| `text-supporting` | 12 / 16 | 400 | 0 | Metadata, times, counts, helper text, tooltip text, sidebar bucket and group labels, menu group labels, table heads, footnotes |
| `text-chip` | 12 / 16 | 500 | 0 | Badges and chips, sentence case |
| `text-caps` | 12 / 16 | 500 | +0.04em, uppercase | Page section labels only |
| `text-secondary` | 13 / 18 | 400 | 0 | Sidebar rows, menu rows, tab labels, segmented labels, dense controls in strips of 36px or less, tool-call rows, table cells, composer controls |
| `text-code` | 13 / 20 mono | 400 | 0 | Code and terminal text |
| `text-body` | 14 / 20 | 400 | 0 | UI prose and values |
| `text-label` | 14 / 20 | 500 | 0 | Control text, row titles, band titles, field labels |
| `text-prose` | 14 / 21 | 400 | 0 | Chat markdown, document prose, Crew messages |
| `text-subheading` | 17 / 24 | 600 | 0 | Dialog titles, empty-state titles, markdown h1 and h2 in chat, h2 in documents |
| `text-title` | 24 / 32 | 400 | −0.01em | The Home greeting, the document h1, the preview front-matter title, stat values (with `tabular-nums`) |

Markdown: chat h1 and h2 are 17/24 600, h3 to h6 are 14/21 600, prose is 14/21. A document (`[data-variant='document']`)
has an h1 of 24/32 400, an h2 of 17/24 600, and prose at 14/21 (was 15). The 3px coral bar on h1 and h2,
the tinted blockquote (now a 2px `--border-strong` left edge, muted ink, no fill) and the −0.015em
tracking are gone. When front matter supplies a title, the first `# H1` takes the h2 style.

Deleted from call sites: every `tracking-*` utility except those a role carries, `uppercase` beside
`text-caps`, `font-bold` and CSS 700 (both become 600), and `leading-*` that only restates a role.
`text-display` and `text-heading` are deleted; a title is `text-title` and a dialog or section heading is `text-subheading`.
`styles/typeScale.test.ts` pins the roles and guards the call sites.

The document default is `text-body`: `body` sets `font-size: var(--text-body)`, 14px, where it used to be
16px (`calc(1rem * scale)`). Anything that names no size (a dialog's paragraph, a raw input, a menu's text)
now lands on the scale. Only the size moved: the line height is still the inherited unitless 1.5, because a
fixed 20px would cramp every larger size set without a line height, and `html` stays 16px, so rem spacing
is unchanged. The Text size preference still scales it, through the token. It moved last, at the end of
wave 2 and before the vision loop, so the loop reviews the final sizes.

### 9.4 · Spacing, rows, bands, radii

| Token | Value | Use |
|---|---|---|
| Spacing scale | 2, 4, 6, 8, 12, 16, 20, 24, 32 | 8 is the default gap and 16 the default inset. Padding that centres content in a token height, hairline offsets, negative hit-box margins, the heatmap's 3px gap and values copied from a Crew recipe are derived, not chosen, and are exempt. |
| `--chrome-height` | 44 | Every band: sidebar titlebar band, chat header, artifact strip, page bands, Knowledge band, pane headers. The three shell bands move together or not at all. |
| `--dock-height` | 36 | Dock strips (terminal, Knowledge facet strip) and the usage card header row |
| `--tab-height` | 32 | Tabs in strips |
| `--row-height-nav` | 28 | App sidebar rows only (scales with Text size) |
| `--row-height-rail` | 32 | Crew rail and menu rows |
| `--row-height` | 40 | Content and settings rows (A-03's 36px is not part of this pass) |
| Controls | 24 `compact`, 28 `sm`, 32 `md`, 36 `lg` | Unchanged |
| Icons | 14 chip and inline help glyph, 16 rows and controls, 20 banners and empty states | `--icon-chip`, `--icon-row`, `--icon-banner` |
| Radii | 4 inner, 8 element, 12 container, 16 surface (the preview sheet only), full | A nested radius is outer minus padding. `rounded-2xl` is banned outside `artifacts/`. |
| `--measure-chat` | 760 flat | Unchanged |
| Sidebar width | 216 to 360, default 288 | `minWidth` 1048 is 288 + 760 |

### 9.5 · Colour and contrast additions

- `--border-control` is the resting edge of every input, checkbox, radio, select trigger and the switch
  knob ring: `#86847f` in light (3.74:1 on the page and the card), `#817f7a` in dark (4.31:1). It replaces
  `--border-emphasized`, which measured 1.6:1.
- `--control-track-off` is the Switch's off track: `#ebebe8` light, `#33332f` dark. The off knob ring
  clears 3:1 against it (3.13 and 3.17), which is why the track sits a quiet step off the page (1.19 and
  1.36) rather than a mid-grey slab. Both are **hex literals declared once per mode in the base blocks**
  because `check-contrast.mjs` resolves hex only; no family restates them. A dark popover sinks the track to
  `#1b1b19` (9.11), where `#33332f` measured 1.05:1 on a highlighted row.
- Dark `--heat-0` is `#32312e` (was `#1e1d1b`, 1.02:1 on the card, so a quiet day vanished): 1.33:1 now.
  Light is unchanged at `#eeeeea`. Each family's dark `--heat-1` moved one step where the lifted zero would
  have swallowed it (Parchment `#573e2a`, Roche Limit `#5c3410`), so the first active step stays 1.2:1 or
  more above zero.
- `--background-fill` and `--background-fill-strong` are the fills of 9.11: `#f4f4f2` and `#eaeae6` in light,
  `#232320` and `#2c2c29` in dark. Shared neutrals, hex, declared once per mode in the base blocks like the
  two above, with `bg-background-fill` and `bg-background-fill-strong` utilities.
- New assertions in `scripts/check-contrast.mjs`, all six scopes: control edge on the default and card
  grounds at 3:1; the off knob ring on its track at 3:1; the off track a step off the page; the on knob on
  the accent at 3:1; the zero day on the card (1.3 dark, 1.1 light); the first heat step above zero (1.2
  dark, 1.1 light); the lock badge ink (`--text-accent`) on `--sidebar`, `--sidebar-active` and
  `--background-default` at 3:1; and the fill set listed in 9.11.
- Any new colour a Tailwind utility names needs a `--color-*` mirror in `@theme inline`
  (`npm run check:tokens`). Prefer authored CSS for controls.

### 9.6 · Motion

| Token | Value | Use |
|---|---|---|
| `--dur-fast-min` | 95ms | Row and menu-item hover, row-action reveal, tooltip exit |
| `--dur-fast` | 125ms | Control state changes, and every exit of a small surface |
| `--dur-fast-max` | 175ms | Entrances of small surfaces, tab enter, slide and settle, rail content, crossfade in, chevrons |
| `--dur-med-min` | 250ms | Dialog enter, progress fill, side and stack crossfade |
| `--dur-med` | 300ms | Panels: preview open, rail open, the conversation glide, sidebar open, disclosure open |
| `--dur-slow` | 525ms | The period unit for ambient loops only (`calc(var(--dur-slow) * n)`) |
| `--ease-out` | `cubic-bezier(0.24, 1, 0.4, 1)` | Everything |
| `--ease-spring` | `cubic-bezier(0.34, 1.56, 0.64, 1)` | Transforms of physical gestures only: tab slide and settle, drag lift, the segmented thumb |

Constants: pop-in scale 0.97 (dialogs 0.98); travel 4px (menus, row insert), 8px (crossfade rise, cover
pane, toast), 32px in and 16px out (preview body); stagger 30ms with at most five items animated; the
resize settles 180ms after the last event. Exits are one tier faster than entrances and travel half the
distance. `--dur-med-max`, `--ease-in`, `--motion-fast`, `--motion-base` and `--motion-slow` are deleted
(`styles/motion.test.ts` fails if any of them comes back).

`styles/motion.ts` exports `DUR`, `EASE_OUT`, `EASE_SPRING`, `MOTION`, `TRAVEL`, `prefersReducedMotion()`,
`isWindowResizing()` and `motionDuration()`; `styles/motion.test.ts` pins every number to `main.css`. In
CSS, `body.biorouter-window-resizing [data-motion-layout]` and the sidebar equivalent set
`transition-duration: 0ms` and `animation: none`. Three shared classes, each with a reduced-motion rest:
`.br-enter` (rise 4px and fade over 175ms), `.br-crossfade` (stack in one grid cell: the incoming layer
rises 8px over 175ms, the outgoing fades over 125ms) and `.br-highlight` (an `--overlay-selected` wash that
clears over three `--dur-slow` periods). Ambient loops run on multiples of `--dur-slow`; the tool pulse is
four.

### 9.7 · Interaction states and controls

Hover is `--overlay-hover` (ink at 5%). Press is `--overlay-pressed` plus the `--press-scale` 0.98 transform.
Focus follows D-15: controls take the surface shift (`.biorouter-focus-surface`), rows take
`--background-focus` plus an inset `0 0 0 2px var(--border-focus)`, tabs take no fill and their underline
firms from 2px to 3px. Programmatic focus never matches the visible-focus treatment, and the tooltip layer
opens on focus only after a Tab.

- **Switch.** One style, no variants. Track 32×20, `--radius-full`; off `--control-track-off`, on
  `--background-accent`. Knob 16px, inset 2px on every side in both states, travel 12px, no growth; off it is
  the page ground (`--background-default`) ringed by `--border-control`, in light and dark, so the accent
  track alone carries on; on, the knob is `--text-on-accent`. A dark `--text-muted` off knob was dropped
  (coordinator ruling 8): it was the brightest thing in the control, so off read as on.
  Knob and track animate over 125ms; a value that loads asynchronously does not animate
  (`data-motion-still`). Named by the row's visible label through `aria-labelledby`. Stays a Radix
  `button[role=switch]`.
- **Checkbox.** A 16px box inside a 24px hit target, radius 4, 1.5px `--border-control` edge. Checked is an
  accent fill with a 10px check in `--text-on-accent`; indeterminate is an 8×2px bar. The fill transitions
  over 95ms. The label sits 8px to the right.
- **Radio.** A 16px ring on the same edge token with a 6px dot that scales from 0.6 to 1 over 125ms.
- **SegmentedControl.** `role="radiogroup"` with roving arrow keys. Track 28px tall, 2px padding,
  `--radius-element`, `--background-fill` (9.11; the same value `--background-muted` had), no border.
  Segment 24px tall, radius 6, `text-secondary`, resting ink `--text-muted`, selected ink `--text-default`.
  One thumb (`--background-default`, `--shadow-raised`, `--inset-hairline`) slides with `translate` over
  175ms `--ease-spring`. It never uses coral, and its first placement does not animate.
- **Menus.** One row for DropdownMenu, ContextMenu, Command, Select options and composer popups. Surface
  `.biorouter-popover-surface`, radius 12, 4px padding, no row gap, width 180 to 320. Row 32px, padding 0 12px,
  radius 8, `text-secondary`, a muted leading 16px icon, highlight `--overlay-hover` over 95ms. A selected item
  shows a trailing 16px check; a submenu shows a chevron. Group label `text-supporting` muted, sentence case.
  Open: opacity, `scale(.97)` and 4px toward the trigger over 175ms. Close: opacity only over 125ms.
- **Tooltip.** One app-level provider (500ms delay, 300ms skip). Inverse fill, radius 12, padding 6×8, 12/16.
  Fade in over 125ms, out over 95ms. Help text never goes in `title=`.
- **InfoTip.** A 14px `Info` glyph in a 24×24 hit box, `--text-subtle` at rest and visible. Opens on hover
  after 200ms, on Tab focus and on click; closes on leave, blur, Escape and scroll. The same text is always
  rendered in a visually hidden node that `aria-describedby` points to. Plain text, at most two sentences,
  no links. Never inside a `<label>`.
- **SettingRow and SettingSection.** A row is `min-height: var(--row-height)`, padding 10px 12px, gap 12px:
  a real `<label>` in `text-label`, the InfoTip as its sibling, an optional transient `status` line, an
  optional `value`, and exactly one control at the trailing edge. A section is a `text-caps` muted label with
  an optional InfoTip and action; no paragraph under the header. The row hairline is straight and inset by
  the row radius, drawn with `::after`, and absent on the last row.
- **RowActions.** A primary icon plus `⋯`, revealed over 95ms on row hover, `:focus-within` and an open menu,
  always shown under `(hover: none)`. The same items open as a context menu on right-click.
- **Fields and buttons.** `Input`, `Select` and `Textarea` are 32px tall (textarea at least three rows),
  radius 8, resting on `--background-fill` with a transparent 1px border (9.11); the focus treatment
  draws the edge. Checkboxes, radios and the switch knob keep their `--border-control` edge. Buttons keep the 24, 28, 32 and 36px ladder, radius 8, 12px
  padding and `text-label`; `secondary` is the default secondary action and `destructive` is a 22% danger wash
  with danger ink.
- **A button that cannot act is not coloured** (V4-R7-4, coordinator ruling 22). Coral and danger mean "this will
  do something", so a disabled `default` or `destructive` draws what a disabled `secondary` draws: the control
  ground, the default ink and the 50% fade every disabled button takes (`.br-button-inert`, `main.css`, applied by
  the primitive, so no dialog restates it). A primary that can be pressed and has something to say, such as a Save
  whose click names the missing field, stays `secondary` until it can act and stays enabled (first-run FR-R3-3).
  A button that is working and says so with `aria-busy="true"` keeps its colour: the composer's Send has only a
  spinner to say "Sending". A labelled button that is disabled while it works goes neutral and its label carries
  the progress.
- **Dialogs and sheets.** A `ModalShell` takes Crew's placement by default: its top edge is pinned at
  `max(10vh, 48px)` so it grows downward only, and a hairline sits under the header. `anchor="center"` is
  the exception, with a reason at the call site. The overlay fades over 250ms; the panel goes from opacity
  0 and scale 0.98 to 1 over 250ms; exit is opacity and scale 0.98 over 125ms. A sheet is 300ms in and 125ms out. Tabs keep 36px, 2px
  `--accent-bar` underline and a 20px gap; the underline is one indicator that travels (175ms).
- **Filter field.** One 28px list filter for every view band, with a leading 16px `Search` glyph, the
  placeholder "Filter", and ⌘F to focus it.
- **Message actions.** They float: a compact cluster (24px icon buttons and the time in 12px muted tabular
  figures) sits 12px above the message's top right on the popover ground with the popover shadow, so no row
  is reserved for it. It shows when its own message is hovered, holds keyboard focus or has its menu open,
  and it only fades, so it stays in the tab order. On touch devices without hover it sits in the flow under
  the message, always shown. The user message has Edit and Copy; the answer has Copy and a ⋯ menu with
  "Diverge from here". Copy confirms in its tooltip, so nothing changes width. The rules live in
  `components/message-meta.css`.

### 9.8 · Icons

- One registry: `components/icons/app-icons.tsx`. Lucide through `light()`, plus hand-drawn glyphs with the
  same contract (24 grid, stroke 1.5, `currentColor`): `ChatBubble`, `CrewChat`, `Agent`, `KnowledgeGraph`,
  `Skill` and `Copilot`. Each stamps `br-icon br-icon-<name>`. This supersedes the "custom set is only for
  domain marks" line in §3.9.
- One glyph per concept: `ENTITY_ICONS` (`entity-icons.ts`) holds workflow, knowledge, extension, skill,
  application, schedule, crew, agent, chat, model and folder. Chat kinds and tool rows read their glyphs from it.
- New chat is `NewChat` (SquarePen); `Plus` means add. Slash commands are `SquareSlash`. Resume is `Play`.
  Writing or editing a file is `FilePen`; reading is `FileText`.
- Chat kinds (`chats/chatKind.ts`): chat, Crew task, sub-agent, branch, app, scheduled run, terminal. Private
  is a shape on every kind: the lock badge, in the family accent (`.br-icon-lock-badge`), over a body that is
  muted at rest and default ink on the active row, always in a 16px slot.
- Tool rows (`utils/toolGlyph.ts`) use a seven-rung ladder. Specific glyphs exist only for Biorouter's own
  extensions; every external extension draws the Puzzle; the wrench is for an unknown tool.
- Sizes: 14 chip and inline help glyph, 16 rows and controls, 20 banners and empty states.

### 9.9 · Where this part supersedes earlier passages

| Earlier passage | What changed |
|---|---|
| §3.1 heatmap ramp | Level 0 is the shared neutral `#eeeeea` light and `#32312e` dark; Parchment's dark level 1 is `#573e2a` |
| §3.2 type scale | Replaced by 9.3 |
| §3.6 motion | The tiers are 95, 125, 175, 250, 300 and 525ms; `--ease-spring` is sanctioned for physical gestures; the `--motion-*` aliases are deleted |
| §3.9 iconography | Hand-drawn glyphs sit beside Lucide, default 16px; see 9.8 |
| §4.9 switch, checkbox, radio | Replaced by the specs in 9.7 |
| §4.11 sidebar | Rows are 28px at 13px with a 2px gap; section labels are sentence case |
| §4.12 page header | A 44px band with a 14px `<h1>`; the description moves into an `InfoTip` |
| §4.17 markdown tables | Crew's table recipe, top rules only, one look for chat, documents, CSV and notebooks |
| §4.18 chat headings | No accent bar; chat h1 and h2 are 17px, document h1 is 24px at 400 |
| D-12 | The sidebar row is 28px; content rows are unchanged at `--row-height` |
| §4 message timestamps and actions | A floating cluster above the message instead of a reserved footer; see 9.7 |
| P1 surfaces, and 9.2 principle 8 as first written | Fills before lines: in-page boxes and controls are a subtle fill with no resting outline; see 9.11 |
| 9.1 code block and table frames, §4.17 note | The code well and the table frame take the fill with no frame; the table's internal rules stay; see 9.11 |
| 9.7 fields, SegmentedControl track | Fields rest on the fill with a transparent border; the track is the fill |
| §4.7 the chat composer | Stands as written: the card is the page ground with its resting hairline, not a fill (R-32); see 9.11 |

### 9.10 · Open at the time of writing (2026-10-10)

Moves that were still waiting on other owners, and where each stands at the end of wave 2: the
`--motion-*` aliases and `text-heading` are deleted, and `styles/typeScale.test.ts` is strict (no guarded
file may carry a legacy type class or a px font size; `ModelAndProviderContext.tsx` and the other frozen
files are out of scope). `body` moved from 16px to `var(--text-body)` (9.3) before the vision loop, which is
the visual sweep the move waited for. `.claude/commands/frontend-design.md` was regenerated from Parts 1 to 3
and Part 9 at the end of wave 2 (2026-10-10), with the coordinator's approval, and brought in line again
after the body move and the dialog placement ruling, and again for the fill rule (9.11); if the vision loop
changes this part, bring that file back in line with it.

### 9.11 · Fills before lines (owner message 13)

The owner, after vision round 1: use "more subtle solid fill rather than these line draw ui". The coordinator's
ruling R-27 (spec section 9) makes it a general rule, and it replaces 9.2 principle 8 as first written.

**What takes the fill, with no outline:**
- Cards and boxed panels: the summary rail card, the Home usage card, approval, elicitation and secret request
  cards, artifact cards, notes, empty-state boxes, boxed settings groups, code blocks, table frames and tool
  wells. Same radius as before.
- Controls: secondary and outline buttons, chips, the tab strip's tabs, the segmented track, filter and search
  inputs, text fields and selects.
- Row lists drop the hairlines between rows wherever spacing and the hover fill already separate them.

**What keeps a line:** structural edges (the 44px band's bottom edge, pane dividers, resize handles, table
rules inside a filled table) and floating surfaces (menus, popovers, tooltips, dialogs and toasts keep a
hairline and a shadow, so they stay visible in dark). Crew is the reference and is not converted; shared
tokens may reach it.

**The one box the rule does not reach: the composer card** (owner message 17, R-32, which supersedes R-27 for
this card only, in a chat and on Home). It is the page ground (white in light, `--background-default` in dark)
with its resting 1px hairline, `border-border-subtle/60`, not the grey fill. It carries no `.br-fill-surface`,
so a control inside it (the quiet Send, a chip, a queued message's edit field) rests as it does on any page
ground. Its coral focus edge and its working sweep turn that same 1px border, as they did before the fill rule,
and Crew's card already wrote the same pair. A reviewer who sees a white card with a resting hairline there is
looking at the specified design, not at a missed fill. The card's classes live in
`components/composer/composerCard.ts`; `composer/composerCard.browser.test.tsx` paints them under the real
stylesheet.

**The tokens.** Two shared neutrals, declared once per mode in the base `:root` and `.dark` blocks
(`theme-contract.mjs` lists them as structural, so no family restates them):

| Token | Light | Dark | Use |
|---|---|---|---|
| `--background-fill` | `#f4f4f2` | `#232320` | The resting fill on the page grounds (canvas, default surface, card) |
| `--background-fill-strong` | `#eaeae6` | `#2c2c29` | Hover and press; the resting fill on a recessed ground |

The step was matched in lightness, not in ratio: the fill sits about 3.9 CIELAB L* off the card in both modes
(1.10:1 in light, 1.09:1 in dark), because contrast ratios compress toward black and an equal ratio is a smaller
visible step in dark. On the dark canvas the transcript paints, the fill is 1.18:1.

**How to apply it:**
- Write `bg-background-fill border border-transparent`, and `hover:bg-background-fill-strong` where the box
  responds to the pointer. Keep the 1px border and make it transparent, never `border-0`: no geometry moves
  between the bordered and the filled look, and forced colours draw a transparent border in the system ink.
- On a recessed ground (the sidebar, `--background-muted`), rest on `--background-fill-strong`: the rest fill
  equals the muted value there (1.00:1, and 1.03:1 on the sidebar), the strong step is 1.10:1 and 1.12:1 off
  them. Put `.br-ground-recessed` on the ground itself (today the sidebar, first run, the Knowledge graph
  canvas and pane header, and the page band) and the shared primitives on it take the strong step with no
  class of their own; a filled button there hovers one deeper, on `--background-strong`.
- A box never sits inside a box, so a fill never sits on a fill. A control inside a filled card (a field or
  a secondary button in an approval card) rests on `--background-default`, one step back toward the page, and
  hovers on `--background-fill-strong`. `Card`, every `Note`, a dialog's inner panel and row, and a washed
  list or settings row do this for their children already; a caller's own filled box adds `.br-fill-surface`.
- Three private variables carry what a ground asks of the primitives on it: `--br-control-ground` (where a
  field, the Select trigger, a secondary button, a `CopyField` or an empty-state tile rests),
  `--br-control-hover` (where a filled button goes under the pointer) and `--br-track-ground` (the segmented
  track and a neutral progress track, one step deeper than the ground, so a white thumb stays visible). They
  are set by the grounds above, never at a call site, and are not shared tokens: the fills stay declared once
  per mode. The table is in `docs/desktop-ui/shared-controls.md` ("Fills before lines").
- A row list that reads apart by air and the hover wash drops its hairlines with `.br-rows-flat` on its shell;
  forced colours draw the divider again, because they repaint the wash as Canvas.
- A dark popover re-points `--background-default` to `--background-medium`, which equals the dark strong step;
  a field there rests on the fill as a quiet well, 1.13:1 below the popover ground, and
  `.dark .biorouter-popover-surface` re-points `--background-fill-strong` to `#1b1b19` so the hover step goes
  one step deeper (1.10:1 off the rest, 1.23:1 under the ground) instead of equalling the ground. Write
  nothing special at the call site: `bg-background-fill` and `hover:bg-background-fill-strong` do the right
  thing inside a popover. The fill is not lifted above the popover ground on purpose: `--text-subtle`
  (placeholders) measures 4.15, 4.44 and 4.50:1 on the smallest useful lift (`#333330`) and 3.74 to 4.06:1 on
  `--background-strong`, below the 4.5:1 a field owes it, and on the well it only gains.
- The off switch track sinks to the same well inside a dark popover (`--control-track-off` re-pointed to
  `#1b1b19` in the same rule). The page's `#33332f` is a raised step on the page, but on the lifted ground it
  measured 1.10:1 and on a highlighted row (the `--overlay-hover` wash a command list opens with) 1.05:1, a
  bare knob. The well is 1.23:1 under the ground and 1.42:1 under the highlighted row, the nearest match to
  light's 1.19:1 in lightness (dL* 8.2 against light's 7.0; `--background-fill` as the track gave 4.3), and
  the knob ring gains (4.31:1). The Effort slider's rail paints the same token.
- Opaque on purpose: a sticky table head, a fade that ends at the fill and anything that sits over content
  need a solid ground, which a translucent ink would not give them.

**Held by `scripts/check-contrast.mjs`** in all six scopes: the fill is at least 1.08:1 and at most 1.35:1 off
the canvas, the default surface and the card; the strong step is at least 1.08:1 off the fill, the sidebar and
`--background-muted`, and at most 1.5:1 on the page; the focus fill is 1.05:1 off the strong step; default and
muted ink clear 4.5:1 on both fills (lowest 5.06:1), `--text-subtle` on the rest fill (lowest 5.06:1); the
control edge and the focus edge clear 3:1 on both fills (lowest 3.10:1 and 4.55:1); the off switch track is a
step off the fill. Inside a dark popover the same pair is measured again in the three dark scopes (the "dark
popover" rows): the fill is 1.08 to 1.35:1 and the hover step at least 1.08:1 off the ground, 1.5:1 at most,
and ink, placeholder and both edges clear their floors on both fills (lowest 4.31:1 for the control edge).
The off switch track is held at 1.08:1 off a popover's resting row and off its highlighted row in all six
scopes (the highlight is composited from the `--overlay-hover` declaration; light's highlighted row, 1.09:1,
is the binding case), and the knob ring at 3:1 on the popover's track. `styles/themeNeutrals.test.ts` keeps both tokens shared, and
`styles/fillTokens.browser.test.ts` measures the forced colours claim in Chromium.

**The accessibility trade-off.** Text inputs and selects lose their resting border. A fill of 1.08 to 1.35:1 is
below the 3:1 a boundary owes when it is the only cue, so it is not the only cue: a field is identified by its
fill, its visible label or placeholder (`--text-subtle` clears 4.5:1 on the fill) and its position, and the
focused field by the focus edge (D-15, 3:1 or more on both fills). Checkboxes, radios and the switch keep their
3:1 edges, because their boundary is the control. Under forced colours the fill is repainted as Canvas and the
transparent border is drawn in the system ink, so every filled box keeps its outline there (measured: the
border reads `rgb(0, 0, 0)` and the box keeps its size). A box that drops its border with `border-0` has no
edge in forced colours, which is why the rule above says keep it.
