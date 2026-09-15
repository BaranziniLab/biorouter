# Theme system architecture

> **What this is.** The architecture of BioRouter's theme system: where a theme actually lives,
> what is generated from what, the token contract, what varies per family and what does not,
> and what a fourth family costs.
> **Status:** Current. Implemented 2026-07-18; §5 is the shipped architecture, and §1–§4 are
> kept as the diagnosis that motivated it. **§8 (2026-08-08) is the current rule for what a family
> may vary — read it before changing any colour.** §7 names what is still open.
> **Audience:** developers adding a theme family, or touching any of the generated regions in
> `main.css`, `themes.generated.ts` and `index.html`.

Three families ship — Parchment, Alma Mater and Roche Limit — the team expects more, and themes
stay baked into the app rather than being user-installable. Sections are numbered and cited by
number from the per-family token references, so the numbering is a stable reference scheme.

**A family is its ink and its accent.** Every background, grey, border, elevation and scrim is one
shared set that all three wear identically, in both modes. That is §8, and it postdates most of
what follows — where an earlier section reasons from three families having three neutral ramps, it
is describing the system as it was measured, not as it runs.

> §1–§4 below are the diagnosis that motivated the work and are kept as the record of what was
> measured. **§5 is the shipped architecture.** The staged plan it originally proposed was compressed:
> the guard work, the token contract and the generator all landed together, because extracting the
> definitions turned out to be the only safe way to prove the generator faithful.

---

## 1 · Where a theme lived before the re-architecture

**Nowhere central.** A theme is not a configuration object, a file, or a plugin. It is a *pattern of
edits* spread across nine files, and it is 100% compile-time — there is no loading, no registry, no
theme artifact. The split is by **rendering technology**, not by concern:

| Layer | Lives in | Why it can't just be CSS |
|---|---|---|
| Semantic UI tokens | `main.css` — `:root[data-theme='x']` + `.dark[data-theme='x']` | — |
| Tailwind utility mapping | `main.css` — `@theme inline` | — |
| Syntax colours | `codeTheme.ts` (TS objects) | `react-syntax-highlighter` takes a JS style object |
| Terminal ANSI-16 | `InAppTerminalDock.tsx` (TS objects) | xterm paints to canvas; cannot read `var()` |
| Boot splash | `index.html` (literal hexes) | paints before the app |
| Family registry | `ThemeContext.tsx` | — |
| Family registry *again* | `index.html` pre-hydration script | can't import TS |
| Label + swatch | `ThemeFamilySelector.tsx` | — |
| Contrast scopes | `scripts/check-contrast.mjs` | — |

### The real cost of a 4th family

| Measure | Count |
|---|---|
| Files touched | **9** |
| Discrete edit sites | **23** |
| Hand-authored colour values | **~220** |
| CSS declarations per family | **147** (87 light + 60 dark) |

Every family **redeclares every token**. The set difference between Alma's and Roche's light blocks
is empty in both directions; so is the difference between a family's own light and dark blocks. Only
17 non-colour tokens (radii, motion, z-index) are inherited.

The cascade rests on specificity `(0,2,0)` beating the bare `:root`/`.dark` `(0,1,0)` — **and on
source order**, because `:root[data-theme='x']` and `.dark[data-theme='x']` have *identical*
specificity. If someone writes the dark block above the light one, dark mode renders light tokens
**and the contrast guard still passes.**

---

## 2 · What is already good (do not break it)

Three things are better than they look and should be the model for everything else:

- **`boot-splash.test.ts` derives its expectations from the source.** It regex-extracts
  `THEME_FAMILIES` from `ThemeContext.tsx` and `FAMILIES` from `index.html` and asserts they match,
  then requires every family to have light + dark splash rules. **Two of the three "duplicated
  family lists" are therefore already CI-guarded** — only `ThemeFamilySelector.tsx` can silently
  drift. *(This corrects an earlier claim that all three were unguarded.)*
- **`ThemeFamilySelector.test.tsx`** is likewise registry-driven.
- **`@theme inline` is load-bearing and correct.** It emits `.bg-sidebar { background-color:
  var(--sidebar) }` and deliberately does **not** emit `--color-sidebar` into the cascade (verified:
  0 occurrences in compiled output). Utilities are late-bound to the semantic token. Reverting to
  plain `@theme` would still work today by accident and would silently break scoped theming, with no
  test failing.

---

## 3 · What is actually broken

### Three live inconsistencies, already shipping at N=3

1. **Roche Limit wears Parchment's scrim.** The modal/diagnostics overlays are hardcoded
   `rgba(32,25,15,0.18)` (warm brown) outside the token layer. Only Alma-light gets a retint
   (`main.css:1533`). No test.
2. **Roche Limit's brand mark contradicts its own splash.** `index.html` rebrands the BR monogram
   for Roche (`--br-coral: #ee6c1a`), but `BioRouterMark.tsx` holds `const NAVY/CORAL/TEAL` as fixed
   module constants and never reads the family. The splash paints orange, React hydrates coral/navy.
3. **The three families disagree about which token the terminal paints** — verified by resolving the
   cascade:

   | Family / mode | Terminal bg | `--background-muted` | `--background-code` | Actually equals |
   |---|---|---|---|---|
   | parchment dark | `#16120c` | `#282217` | `#16120c` | **code** |
   | alma dark | `#0d2a50` | `#0d2a50` | `#08213f` | muted |
   | roche dark | `#232320` | `#232320` | `#1b1b19` | muted |

   `InAppTerminalDock.tsx:75` states *"The ground is --background-muted"* — false for Parchment.
   **This is the trap a naive generator walks into**: any codegen that emits
   `terminal.background = ref('--background-code')` silently re-grounds two terminals under ANSI
   palettes tuned for a different surface. That is the same defect class as the 4.15:1 bug.

   > **Resolved by §8.** All three families now point `terminalGround` at `--background-muted` in
   > both modes, and the values behind that token are identical across families. The trap was
   > real and the reasoning still holds — the fix was not to assume the grounds agreed but to
   > *make* them agree and then re-measure Parchment's dark ANSI palette against the one it
   > moved to. Two of its stops needed retuning; see §8.

### The largest untested surface

**126 hand-tuned terminal hexes across three families, with documented per-stop ratios and not one
assertion.** `InAppTerminalDock.test.tsx` contains zero colour tests.

Roughly half the token vocabulary is never contrast-asserted at all, including `--accent-bar` —
whose own code comment argues at length that it must clear 3:1 on every light ground.

### The guard's own blind spots

- It matches selectors by **exact string equality**. A block written with double quotes or an extra
  space yields `{}`, the scope silently becomes pure Parchment, and ~40 assertions pass while
  measuring the wrong theme.
- It models the cascade **by construction** (`Object.assign({}, LIGHT, DARK, X_L, X_D)`), so it
  cannot see the source-order hazard above.
- The 4.15:1 incident was **a missing assertion, not a cascade bug** — `--background-code` simply had
  no check. Worth stating plainly, because it changes what the fix is.

---

## 4 · Empirical findings (tested, not assumed)

Four experiments in a real browser against the compiled stylesheet:

| Question | Result |
|---|---|
| Can a theme the Tailwind build never saw be added at **runtime**? | **Yes.** Injecting `:root[data-theme='kelp-forest']{--sidebar:…}` at runtime recoloured `bg-sidebar`, `text-sidebar-icon` and `bg-background-accent` correctly. |
| Does a **partial** theme break the UI? | **No.** Omitted tokens fall back to Parchment's `:root` defaults, not to black. A 5-token theme still yields a usable app. |
| Do **dark variants** work at runtime? | **Yes**, via `.dark[data-theme='x']`. |
| Do tokens resolve in a plain `<style>` block? | **Yes** — `var(--background-muted, magenta)` computed `#f2f3f4`. |

That last one **contradicts a comment in `index.html:86-92`**, which asserts that `@theme inline`
"compiles those tokens away — they do not exist as runtime custom properties" and instructs future
readers not to "simplify this back to tokens." The magenta observation behind it was real, but the
cause is almost certainly **stylesheet timing at boot** (Vite injects CSS via JS in dev), not
compilation. The tokens demonstrably exist. This matters: that incorrect belief is what forces every
new family to hand-copy splash hexes into `index.html`.

---

## 5 · The shipped architecture

**One definition per family; everything else generated. Compile-time only — themes are baked into
the app and are not user-installable, by decision.**

```text
ui/desktop/themes/<id>.theme.mjs      the ONE file you write
npm run themes                        emits everything below
npm run themes -- --check             CI gate: fails if generated output is stale
```

### What is generated

| Artifact | What lands there |
|---|---|
| `src/styles/main.css` | the `:root[data-theme=X]` / `.dark[data-theme=X]` token blocks, inside a marker region |
| `src/styles/themes.generated.ts` | syntax palettes, terminal ANSI palettes, brand-mark inks, family manifest, `THEME_FAMILY_IDS` |
| `index.html` | the pre-hydration family list and the per-family boot-splash CSS |

Regions, not whole files: `main.css` and `index.html` carry hand-written reasoning that is not
derivable from a palette, so the generator owns only a delimited span. Parchment's `:root`/`.dark`
blocks stay hand-written — it is the base layer and also carries the 17 structural tokens no theme
may vary.

### What is derived, never authored

These are exactly the values that used to be typed in two-to-four places and drift:

| Derived | From |
|---|---|
| `terminal.background`, `terminal.cursorAccent` | the family's own `terminalGround` token |
| code ground (`CODE_BG*`) | `--background-code` |
| boot-splash `--br-bg` | `--background-muted` |
| picker label + swatch, family list | the definition's `label` / `swatch` / `id` |

`terminalGround` is **per family on purpose**. Parchment dark paints `--background-code`; Alma Mater
and Roche Limit paint `--background-muted`. Assuming they agreed would silently re-ground two
terminals under ANSI palettes tuned for a different surface.

### `--background-canvas` is not `--background-app`

Two page-ish tokens, deliberately:

| Token | Paints | Who reads it |
|---|---|---|
| `--background-app` | the **window** ground, behind everything | `body` |
| `--background-canvas` | the **main panel** — conversation, hub, every top-level view | `MainPanelLayout`, `Hub`, `BaseChat`, `App` root |

The original reason for two tokens was that the families disagreed about the
ladder: Parchment's canvas carried a warm tint while its window ground was pure
white, and Parchment dark and Alma Mater dark **inverted** it, putting the canvas
*above* the cards. §8 ended that disagreement — one ladder, canvas darkest, cards
a step up — so that is no longer why the tokens are separate.

**They stay separate because they mean different things**, and the app reads them
in different places: `body` paints one, `MainPanelLayout` / `Hub` / `BaseChat`
paint the other. Collapsing them would be an irreversible loss of that
distinction for a saving of one line. In the shared set they happen to hold the
same value (`#ffffff` light, `#131312` dark) — which is a fact about today's
palette, not a licence to alias one to the other.

Two bugs this shape has caught, both worth keeping in mind:

- The main panel once painted `--background-muted`, so the whole canvas read grey
  and the sidebar/canvas two-tone collapsed. `--background-canvas` is in
  `TEXT_GROUNDS` in `check-contrast.mjs`, so body text is measured against the
  surface it actually lands on.
- `--background-canvas` and `--background-muted` were **byte-identical** in three
  of the six family/mode scopes (parchment light and dark, alma-mater dark), so
  anything that used `bg-background-muted` to lift itself off the page was
  invisible there — which is exactly what happened to the composer's chips. Roche
  Limit was the one family that kept a real step, and adopting its neutrals fixed
  all three at once. `check-contrast.mjs` now asserts the step in every scope
  ("background-muted is a step off the canvas"), with a floor set to catch a
  collapse to zero rather than to pin today's 1.10:1 / 1.18:1.

### The contract

`scripts/lib/theme-contract.mjs` is the written-down answer to "what must a theme define": 62
semantic tokens × 2 modes, 27 raw-palette remaps, 10 syntax stops, 19 terminal stops, 3 splash
values. A definition missing any of them **cannot be emitted** — the generator validates first, then
contrast-checks the result, and refuses to write on failure.

That refusal is not theoretical: it caught five Parchment light terminal stops sitting below the AA
floor their own comment claimed they cleared.

### What guards it

- **`check-contrast.mjs` discovers families** by sweeping the stylesheet for `[data-theme='…']`.
  A new family is audited with **zero** edits to the guard. It also asserts light-before-dark block
  order, which is load-bearing and was previously unchecked.
- **`npm run themes -- --check`** is wired into `lint:check`, so stale generated output fails CI.
- **Per-slot terminal floors** (`TERMINAL_FLOORS`) with a recorded reason for every relaxation.

### Cost of a 4th family — measured, not estimated

Verified by actually adding a throwaway family: **one file**, no other edits. The contrast guard
picked it up unprompted (228 → 304 assertions), the type system flagged the two maps that still
needed deriving, and the splash test caught it wearing another family's ground because the demo
copied its surfaces verbatim.

| | Before | After |
|---|---|---|
| Files touched | 9 | **1** |
| Edit sites | 23 | **1** |
| Hand-authored values | ~220 | ~200 (in one place, validated) |
| Hardcoded family lists | 3 | **0** |
| Guard edits | 5 | **0** |

### Migration was proven, not asserted

A one-shot extractor pulled the shipping values into definitions; resolved-token output was then
diffed against a pre-change baseline. **All 104 tokens per family identical; Parchment 77/77
untouched.** The only differences were the two intended ones.

That extractor has since been **deleted**, deliberately. It read the hand-written values out of
`main.css` / `codeTheme.ts` / `InAppTerminalDock.tsx` — which no longer hold them, because those
files are now generated or read from the generated module. Re-running it would have emptied all
three definitions and exited 0. The migration is recorded in commit `74a8fe01`; recovering the tool
means recovering it from there, with fresh eyes on what it reads.

---

## 6 · Decisions taken

1. **Runtime / user-installable themes: rejected.** Technically viable (§4 proves the mechanism
   works), but themes stay baked in by decision. The obvious implementation is also a trap: injecting
   into `@layer user-theme` loses to the existing tokens at every value, because `main.css`'s token
   blocks are unlayered and unlayered beats every layer.
2. **Terminal ground: codified, not unified.** Each family declares which token its terminal paints.
   *Superseded by §8:* it is now codified **and** unified — all three declare `--background-muted`.
   The declaration stays in the contract, because the point of codifying it was that the choice be
   written down and measured rather than assumed, and that is still true when the answer agrees.
3. **Shadows stay raw strings**, outside the contrast set. *Amended by §8:* still raw strings, but
   no longer per family — elevation is neutral scaffolding and all three share one set.
4. **Bright ANSI slots hold 3:1, base slots hold 4.5.** On a light ground "bright" (conventionally
   *lighter*) and AA are mutually exclusive; forcing 4.5 would collapse `brightCyan` into `cyan`.
5. **`--accent-bar` is deliberately NOT asserted.** On the rail's own ground (`--sidebar-active`)
   Parchment measures 2.53, Alma Mater 2.23 and Roche 3.19; on `--background-strong` all three fail
   (2.18 / 2.10 / 2.80). So two of three families have never met 3:1 and the rule has never been
   enforced — asserting it would fail the default theme on day one, not catch a regression. The rail
   reinforces a background change the active row already makes, so it is not the sole cue. Roche's
   doc claimed a guarantee nothing meets; the doc was corrected rather than the themes.

## 7 · Still open

- The `index.html` comment claiming tokens "do not exist as runtime custom properties" is wrong
  (§4). The splash grounds are now generated from `--background-muted`, so the duplication is gone,
  but the comment's reasoning should be corrected.
- `--sidebar-icon` on a navy sidebar, and the scoped `<div data-theme>` live preview for settings,
  remain unbuilt.

---

## 8 · Shared neutrals — one scaffolding, three inks

**The rule.** A theme family varies in exactly two things: its **ink** (the text and syntax
colours) and its **accent** (one hue, plus the affordances derived from it). Everything else —
backgrounds, greys, borders, the focus ring and fill, elevation, the scrim, the empty heatmap
step, the splash ground — is **one shared set**, byte-identical across all three families in both
light and dark.

### Why

At N=3 the families had three separate neutral ramps and, of the 35 background / border / sidebar
keys in the light block, **agreed on four**. Parchment ran a warm cream ramp (`#faf8f3` /
`#f4f0e6` / `#d4cab6`), Alma Mater a cool blue-grey one (`#f2f3f4` / `#e1e3e5` / `#d1d3d3`), Roche
Limit a warm neutral one. Three consequences, all of which had already shipped:

- **Three ramps is three times the surface for the same bug.** The canvas/muted collapse above was
  present in three of six scopes and absent in the fourth family that happened to pick different
  numbers. Nothing structural distinguished the correct case from the broken ones.
- **Every component that reasons about a surface had to be right three times.** A shared ramp
  turns "does this read on the sidebar?" from a question with three answers into one with one.
- **The families were not actually distinguished by their greys.** What a user recognises is the
  ink and the accent — warm brown and dark orange, UCSF navy and teal, near-black and orange. The
  greys carried the variation without carrying the identity.

### The reference set

Roche Limit's neutrals were adopted **wholesale, not averaged**: they were the set with a real
canvas/muted step in both modes, a consistent surface ladder, and a warm-neutral cast that sits
under a warm accent and a cool one equally. Its definition file is therefore unchanged by this
work; `parchment.theme.mjs`, `alma-mater.theme.mjs` and the hand-written `:root` / `.dark` blocks
in `main.css` moved onto it.

### What each family still owns

| Owned by the family | Shared by all families |
|---|---|
| `text-default`, `text-muted`, `text-subtle`, `text-inverse` | `background-app`, `-canvas`, `-default`, `-card`, `-muted`, `-code`, `-well`, `-medium`, `-strong`, `-inverse` |
| `sidebar-foreground`, `sidebar-accent-foreground` | `border-subtle`, `-strong`, `-input`, `-default` |
| the 10 syntax stops, the 19 terminal stops | `sidebar`, `-hover`, `-active`, `-accent`, `-border` |
| `background-accent`, `-accent-hover`, `border-accent`, `text-accent`, `text-on-accent`, `accent-bar`, `sidebar-icon`, `swatch` | `ring`, `sidebar-ring`, `background-focus`, `border-focus` |
| `heat-1` … `heat-4` (the accent ramp), `mark.navy`, `mark.coral` | `heat-0` (the empty-day fill), `mark.track` |
| the status hues — see below | `scrim`, the four `shadow-*`, `shadow-raised`, the whole `--color-neutral-*` ramp |

Two entries in that table are worth their reasoning:

- **`background-inverse` is shared**, even though each family used to set it to its own
  `text-default`. It is the tooltip fill — a *surface* — and surfaces are shared. Each family still
  places its own `text-inverse` on it.
- **The status hues stay per family.** `background-danger` / `success` / `info` / `warning` and
  their `text-` and `border-` twins are neither neutral scaffolding nor the family accent, and the
  rule above does not reach them. Two reasons to leave them: the user's axes are grounds (shared)
  versus ink and accent (varied), and status is a third thing that was not in scope; and Alma
  Mater's are UCSF institutional brand values, not a palette choice. They are re-verified against
  the shared grounds by the contrast guard like everything else.

### Repairing a family that no longer reads

When a family's ink fails on a shared ground, **retune the ink**. That is the axis the family owns,
and it is why the axes are split this way. Reintroducing a per-family background to rescue a text
colour is the failure mode this section exists to prevent — it recreates all three problems above
to fix one ratio.

This happened once during the migration, and it is the worked example: Parchment's terminal
`yellow` (`#9b6818`) and `cyan` (`#107e89`) were tuned against its own cream dock ground `#faf8f3`,
and on the shared `#f4f4f2` they measured 4.35:1 and 4.37:1 — just under AA. They were darkened by
about 2.5% to `#976517` and `#107a85` (4.55 and 4.60). The generator refuses to emit on a contrast
failure, so this was caught at build time rather than by eye.

### The preview paper, its well, and a selection no family owns

The artifact panel's text previews (markdown, CSV/TSV, code, logs, notebooks) paint
`--background-default` — `#ffffff` / `#1b1b19` in every family, which is byte-for-byte the ground
an Auto Visualiser chart paints (`bg` in `autovisualiser/templates/_common.js`, pinned by
`artifactPaper.test.ts`). Fenced blocks, inline code and notebook source sit in a new shared neutral,
**`--background-well`** (`#f5f5f3` light, `#232320` dark: 1.09:1 off the page in both modes). It is
its own token because `--background-code` cannot do the job — in dark it *is* the page. Like every
neutral, it is declared in all three theme files and in `main.css`'s hand-authored `:root` / `.dark`
base block, and the generator emits it per family as `wellGround`.

**Selection is Biorouter orange in every family, and deliberately not a theme token.**
`--selection-hue` (`#cf6d47` light, `#e8895f` dark) and `--selection-alpha` (`24%` both) live in a
bare `:root` / `.dark` pair outside every `[data-theme]` block, as literals — Alma Mater re-points
`--color-coral-*` and the accent tokens to teal, so a selection built on them turned teal there.
`::selection` sets no `color`, so every ink survives under the tint. `artifactPaper.test.ts` fails
if any family block re-declares them. Separate documents (a chart, an `.html` file, a PDF or
spreadsheet page) cannot see the rule; the notebook's sandboxed HTML output restates the tint, and
the terminal keeps its own per-family `selectionBackground`.

Two syntax stops moved to fit the paper, both along their own hue: Parchment dark `comment`
`#8d8266` → `#958a6c` (4.14:1 on the well), and Alma Mater light `keyword` `#0f388a` → `#0b67a8`,
which was navy on the navy ink and separated from an identifier by weight alone.

### What it costs a new family

Strictly less than before. The neutral half of a definition is now copied verbatim from any
existing family and never thought about; what a fourth family actually has to design is its ink,
its accent, and the two palettes derived from them. The contract still requires every token to be
declared — the generator will not infer a shared value — because an explicit restatement is what
lets the guards resolve a family in isolation.

### Guards

- `npm run themes` refuses to emit if any syntax stop, terminal stop or sandboxed-surface pair
  falls below its floor on the **shared** grounds. Syntax stops are held to 4.5:1 on
  `--background-code`, `--background-default` **and** `--background-well`, and to 3:1 under the
  selection tint composited over the paper and the well.
- `check-contrast.mjs` asserts 404 pairs across three families × two modes (measured 2026-09-14;
  re-measure rather than trusting the figure), including the canvas/muted step described in §5,
  body and muted ink on the well, the well's step off the page, and text-default (4.5:1),
  text-muted and text-accent (3:1) under the selection tint over three grounds.
- `boot-splash.test.ts`, `artifactUtils.test.ts` and `NotebookPreview.test.tsx` each used to assert
  that the families' **grounds** were all distinct. That premise is now inverted, and all three were
  rewritten rather than deleted: they assert that the **ink** still differs three ways *and* that
  the ground is shared, which catches both a re-hardcoded preview and a family drifting off the set.

## Related documentation

- [Theming](README.md) — the folder index, and the per-family token references this architecture generates.
- [Alma Mater theme tokens](alma-mater-theme-tokens.md) — the UCSF-brand family's token reference.
- [Roche Limit theme tokens](roche-limit-theme.md) — the JupyterLab-inspired family's token reference.
- [Biorouter Design System](../../../design.md) — the parent design system this architecture serves.
