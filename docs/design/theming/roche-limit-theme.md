# Roche Limit theme tokens

> **What this is.** The colour-token reference for Roche Limit, BioRouter's third theme family:
> the palette it draws from, the accent ramp, a complete token-by-token light and dark mapping,
> and the verified WCAG contrast ratios behind every value.
> **Status:** Current. Approved and implemented on 2026-07-18; §9 records what was actually
> built and wins wherever it disagrees with the design argument in §1–§8. **On 2026-08-08 this
> family's neutrals became the shared set for all three families** — see the banner below.
> **Audience:** developers adding a component that reads colour tokens, and anyone proposing a
> new theme family.

> ## ⚠ These neutrals are now shared by every family (2026-08-08)
>
> Roche Limit's backgrounds, greys, borders, focus surfaces, elevation and scrim were adopted
> **wholesale** as BioRouter's one neutral scaffolding. Parchment and Alma Mater moved onto them;
> this family's definition file was not changed.
>
> That has a consequence worth stating plainly: **the surface tables below (§4.1, §4.3, §4.6,
> §4.7, §4.9, §5.1, §5.3, §5.6, §5.7) are no longer a description of one theme.** Editing any
> value in them now moves all three families, and requires a matching edit in
> `parchment.theme.mjs`, `alma-mater.theme.mjs` and the hand-written `:root` / `.dark` blocks in
> `main.css`. What is still Roche Limit's alone is its **ink** (§4.2, §5.2, the syntax and
> terminal palettes) and its **orange accent** (§4.4, §5.4).
>
> Why this family: it was the one whose `--background-canvas` and `--background-muted` were a real
> step apart in both modes (the other two collapsed them to a single value in three of six scopes,
> making anything that used `bg-background-muted` to lift off the page invisible), it had a
> consistent surface ladder, and its warm-neutral cast sits under a warm accent and a cool one
> equally. Full reasoning:
> [Theme system architecture §8](theme-system-architecture.md#8--shared-neutrals--one-scaffolding-three-inks).

Sections are numbered and cited by number from other documents and from source — `codeTheme.ts`
points at §4.10 and §5.8 — so the numbering is a stable reference scheme, not decoration.

**Owner:** Baranzini Lab, UCSF.
**Companion mockup:** [`roche-limit-theme-studio.html`](roche-limit-theme-studio.html) — renders this palette on real BioRouter chrome, in **light + dark**, with live toggles for the three previewable open questions (chrome warmth, grey temperature, cell rail).

> This document began as **step 1** — naming the palette, mapping every token, proving the contrast —
> and was written before any code changed. **That step is done: the theme shipped on 2026-07-18.**
> §1–§8 keep the original design argument in its original voice; **§9 records what was actually
> built**, and where the two disagree §9 wins. §10 preserves the pre-implementation plan for
> reference.
>
> **Where the values live now.** The theme was later folded into the generated theme system: the
> single authored source is [`ui/desktop/themes/roche-limit.theme.mjs`](../../../ui/desktop/themes/roche-limit.theme.mjs),
> and every consumer is emitted from it by `npm run themes`. See **§9.1** and
> [`docs/design/theming/theme-system-architecture.md`](theme-system-architecture.md). Any hex in this
> document is documentation *about* that file, never the source of it.

---

## 1 · The theme families

A **third** theme family, selected independently of light/dark:

| Family | What it is | Soul |
|---|---|---|
| **Parchment** | The current default — warm paper-and-bone neutrals, one terracotta accent. | Warm, editorial, low-chroma. |
| **Alma Mater** | The UCSF brand palette — navy foundation, eggplant accent. | Cool, institutional, navy-anchored. |
| **Roche Limit** | **JupyterLab's structure** — white page, recessed grey panels, one bright orange. | Clinical, flat, instrument-like. |

**Roche Limit is a re-colouring, not a re-layout.** Same structure, same type scale, same fonts,
same flat-surface discipline. Only colour tokens change.

### The name

The **Roche limit** is the closest distance a satellite can orbit a larger body before tidal forces
tear it apart. Approach is survivable; contact is not. The theme approaches Jupyter as closely as a
theme can — taking its white page, its recessed cells, its flat hairlines, its focus-by-lightness
trick — and deliberately holds position just outside the point where evocation becomes cloning.

**What that rules out:** no `In [1]:` prompt gutter, no Jupyter logo orange, no Material Design
grey ramp. Those are inside the limit.

### Constraints honoured (your four asks)

1. **White / slightly off-white as the main colour.** `--background-canvas: #FFFFFF` (and
   `--background-app` to match) — the page is pure white, matching both your brief and Jupyter's own
   `--jp-layout-color0`. `--background-canvas` is the token the main panel actually paints;
   `--background-app` is the window ground behind everything.
2. **Grey for the intermediate panels.** `--background-muted: #F4F4F2`, code ground `#F5F5F3` — a
   ΔL\* of ~4.6 below the page. Recessed, clearly separated, calm.
3. **Bright orange for icons and accents.** `#EE6C1A` on fills; a deeper `#D95B08` on the rails,
   dots, sidebar icons and light-mode terminal cursor that need to hold up on grey grounds (§4.4, §6).
4. **Real light *and* real dark**, cohesive by construction — same hue relationships, same semantic
   roles, polarity flipped.

---

## 2 · The design thesis

> **Roche Limit = Jupyter's surface structure + one bright orange where Jupyter uses Material blue.**

Three ideas do the work:

- **The page is white and the panels are recessed.** Jupyter's signature is that the *code* surface
  is darker than the *page* — the opposite of the "cards float above the background" convention.
  Light mode keeps this exactly. Dark mode flips polarity (canvas `#131312`, card `#1B1B19`), which
  is what Jupyter itself does (`#111` → `#212121`).
- **Focus is signalled by lightness returning to page level.** JupyterLab sets
  `--jp-cell-editor-active-background: var(--jp-layout-color0)` in *both* themes: a focused editor
  rises to the canvas colour. This costs zero accent budget and is the single most replicable idea
  in their system. Roche Limit applies it to the composer and to `--background-focus`.
- **Orange is identity and attention — never selection.** Jupyter reserves orange for the logo and
  kernel state; everything interactive is blue. We invert which hue is the accent (per your brief)
  but keep the *restraint*: hover, selection and focus stay neutral surface shifts, exactly as
  BioRouter's design rule D-15 already mandates.

---

## 3 · Where the palette comes from

Every Jupyter value below was read byte-exact from `jupyterlab@main`
`packages/theme-{light,dark}-extension/style/variables.css`.

| Role | JupyterLab ships | Roche Limit ships | Why the distance |
|---|---|---|---|
| page | `#FFFFFF` | `#FFFFFF` | **Kept exactly.** |
| panel / cell | `#EEEEEE` / `#F5F5F5` | `#F4F4F2` / `#F5F5F3` | Same lightness, warmed ~2 units of blue. Jupyter's greys are Material Design 2016 — take the structure, leave the era. |
| **accent** | `#2196F3` (brand)<br>`#F37726` (logo only) | **`#EE6C1A`** | **The key divergence.** Jupyter's UI accent is Material blue. We make orange the accent per your brief, and shift off `#F37726` for trademark distance *and* because `#EE6C1A` clears 3:1 on white where Jupyter's orange does not (3.09 vs 2.81). |
| accent as text | — (never used) | `#AE4700` | Bright orange cannot legally carry text on white. Links use ramp step 11. |
| ink | `rgba(0,0,0,.87)` | `#1F1E1C` | Warm near-black, opaque. Jupyter's ramp bottoms out at `rgba(0,0,0,.38)` which fails AA — their team flags it as unfinished. Not inherited. |
| dark canvas | `#111111` / `#212121` | `#131312` / `#1B1B19` | Same polarity flip, warmed so bright orange doesn't halate on a cold ground. |
| cell focus | `active-background = layout-color0` | composer focused = `--background-app` | **Taken wholesale.** |
| collapser | 8px bar, brand when active | 2px `--accent-bar` rail | Concept survives, notebook dimension does not. See **Q4**. |
| prompt gutter | `In [1]:` `#307FC1` / `Out[1]:` `#BF5B3D` | none | BioRouter is a chat/research IDE, not an IPython REPL. Colab dropped it and lost nothing. |
| radius | 2px (cells 0) | 4 / 8 / 12 / 16px | Inherited from `@theme`, outside family scope. See **Q5**. |

---

## 4 · Light palette — `:root[data-theme='roche-limit']`

### 4.1 Surfaces

| Token | Value | Rationale |
|---|---|---|
| `--background-app` | `#FFFFFF` | The page. |
| `--background-default` | `#FFFFFF` | |
| `--background-card` | `#FFFFFF` | Cards separate by hairline, not by fill. |
| `--background-muted` | `#F4F4F2` | **The grey panel.** ΔL\* 4.6 from white. |
| `--background-code` | `#F5F5F3` | The one code ground. A hair lighter than `muted` — Jupyter's `#F5F5F5` inset, warmed. All syntax stops are measured on this. |
| `--background-medium` | `#ECECE9` | Hover fill. |
| `--background-strong` | `#DCDCD8` | Strongest neutral fill. |
| `--background-inverse` | `#1F1E1C` | Tooltips. Warm near-black, not `#000`. |

### 4.2 Text

| Token | Value | On white |
|---|---|---|
| `--text-default` | `#1F1E1C` | 16.66:1 |
| `--text-muted` | `#5C5A55` | 6.89:1 |
| `--text-subtle` | `#69675F` | 5.67:1 |
| `--text-inverse` | `#FFFFFF` | — |

> `--text-subtle` was corrected from `#6B6963` during verification — see §6.

### 4.3 Borders

`--border-subtle: #E4E4E0` · `--border-strong: #D2D2CD` · `--border-input: #C9C9C3` · `--border-default: var(--border-subtle)`

### 4.4 The accent ramp — 12 steps, Radix methodology, anchored on `#EE6C1A`

The ramp is the *design* object — the 12-step scale the semantic values were chosen from. It is **not**
a set of shipped tokens, and most of it never became one. The **Ships as** column is the honest
account, read back from `roche-limit.theme.mjs`:

| # | Value | Role | Ships as |
|---|---|---|---|
| 1 | `#FFFCFA` | app background tint | — not implemented |
| 2 | `#FEF6EE` | subtle background | — not implemented |
| 3 | `#FCEBD8` | UI element background | — not implemented |
| 4 | `#FADBBB` | hover background / selection highlight | terminal `selectionBackground` only |
| 5 | `#F8CCA1` | active / selected background | — not implemented |
| 6 | `#F4BB89` | subtle border | — not implemented |
| 7 | `#EBA671` | border | — not implemented |
| 8 | `#E08C52` | strong / hover border | — not implemented |
| **9** | **`#EE6C1A`** | **solid fill (anchor)** | `--color-coral-500`, `--background-accent`, `--border-accent`, `--sidebar-primary`, `--heat-4`, swatch |
| 10 | `#DE6110` | solid fill hover | `--background-accent-hover` |
| 11 | `#AE4700` | low-contrast text — 5.67:1 on white | `--color-coral-600`, `--text-accent` |
| 12 | `#55321F` | high-contrast text | — not implemented |

**Of the 24 documented steps (12 light + 12 dark), exactly four exist as raw colour tokens:**
`--color-coral-500` (light 9 = dark 9), `--color-coral-600` (light 11), `--color-coral-400`
(dark 11 `#F2955A`), and `--color-coral-700` `#8F3A00` — **which appears in neither ramp.** It is a
real shipped token with no entry above; the ramps were never reconciled against it. Light steps
1–3, 5–8 and 12 and dark steps 1–3, 5–8 and 12 are design scaffolding only. Steps 10 in both modes
ship as semantic values rather than raw tokens, and step 4 in both modes survives solely as the
terminal selection fill.

**Semantic accent tokens:**

| Token | Value | Note |
|---|---|---|
| `--background-accent` | `#EE6C1A` | The bright orange, at full chroma. |
| `--background-accent-hover` | `#DE6110` | |
| `--text-on-accent` | `#1F1E1C` | **Dark ink, not white.** White on `#EE6C1A` is 3.09:1 (fail); ink is 5.39:1. |
| `--border-accent` | `#EE6C1A` | |
| `--text-accent` | `#AE4700` | Links and coloured labels. |
| `--accent-bar` | `#D95B08` | Rails, dots, tab underline. **Deeper than the anchor** — it buys roughly +0.7 of ratio on every light ground, though it does not reach 3:1 on all of them. See §6. |

> If a white-text orange button is ever required, the fill must be `#AE4700` (white on it = 5.67:1).

### 4.5 Status — hue-separated from the accent

Warning is pushed to **true yellow** (ΔHue 53°), not amber: Radix amber sits 15° from our anchor and
reads as a branded chip rather than a warning.

| Token group | Value | ΔHue | On white |
|---|---|---|---|
| `--*-danger` | `#C4232B` | 23° | 5.80:1 |
| `--*-success` | `#0F7150` | 116° | 6.00:1 |
| `--*-info` | `#0A69BC` | 156° | 5.59:1 |
| `--*-warning` | `#6E6300` | 53° | 6.09:1 |
| `--text-on-status` | `#FFFFFF` | — | ≥5.18:1 on all four fills |

ΔHue 23° for danger is still tight. **Never let hue carry the semantic load alone** — every status
surface keeps its icon (WCAG 1.4.1 requires it regardless).

### 4.6 Focus — a surface shift, never the accent (D-15)

`--ring: #5C5A55` · `--background-focus: #E0E0DC` · `--border-focus: #6B6963`

> **Corrected during implementation.** The proposed `#E8E8E4` was only **1.04:1**
> from the hover fill `#ECECE9`, below the repo's 1.1 floor — focus and hover would
> have been indistinguishable. `#E0E0DC` gives 1.12:1, matching Alma Mater's 1.14:1.

### 4.7 Sidebar

`--sidebar: #F7F7F5` · `--sidebar-hover: #EFEFEC` · `--sidebar-active: #EAEAE6` · `--sidebar-border: #E7E7E3`
**`--sidebar-icon: #D95B08`** — this is where "bright orange for the icons" lands (3.59:1 on the sidebar; dark mode uses the pure anchor `#EE6C1A` at 5.81:1).
**Revised 2026-10-10:** nav icons now wear the label ink, as in the other two families. **Revised 2026-10-11:** the resting label ink, `--text-muted` (`#5C5A55` light, `#A5A39D` dark), so icons match the muted labels beside them (V1-R2-5); `--sidebar-foreground` read heavier than the labels. The 2026-10 redesign holds the accent for the active rail, the lock badge, buttons, links and the heat ramp (principle 7, vision finding V1-M3), so the orange in ask 3 lives there now.
Aliases (`--sidebar-foreground`, `-primary`, `-primary-foreground`, `-accent`, `-accent-foreground`, `-ring`) are `var()` refs, copied verbatim from the sibling families.

### 4.8 Heatmap

`--heat-0: #EEEEEA` · `--heat-1: #FADCC0` · `--heat-2: #F6BC8C` · `--heat-3: #EE8B45` · `--heat-4: #C4560E`
(monotonic in luminance: 0.853 → 0.753 → 0.575 → 0.371 → 0.184)

### 4.9 Shadows — warm-neutral tint, Jupyter-flat by default

**Now shared by all three families** (2026-08-08). Parchment cast its light shadows in
`rgba(32,25,15,…)` and part of the set in flat black; Alma Mater cast the whole set — and the
scrim — in UCSF navy `rgba(5,32,73,…)`. Elevation is neutral scaffolding and a full-screen scrim
is a background, so both moved onto the values below. `--scrim` in light is the matching
`rgba(31,30,28,0.18)`; dark is `rgba(0,0,0,0.48)`, which all three already agreed on.

```css
--shadow-default:  0px 1px 3px 0px rgba(31,30,28,0.07), 0px 0px 1px 0px rgba(31,30,28,0.13);
--shadow-composer: 0px 2px 6px -1px rgba(31,30,28,0.09), 0px 1px 2px 0px rgba(31,30,28,0.05);
--shadow-popover:  0px 8px 24px 0px rgba(31,30,28,0.11), 0px 0px 1px 0px rgba(31,30,28,0.16);
--shadow-modal:    0px 22px 60px -18px rgba(31,30,28,0.22), 0px 8px 24px -18px rgba(31,30,28,0.16), 0px 0px 0px 1px rgba(31,30,28,0.05);
--shadow-modal-chrome-bottom: none;
--shadow-modal-chrome-top: none;
```

### 4.10 Code syntax — measured on `#F5F5F3`

Jupyter's IPython/Pygments hues, darkened to clear AA.

| Key | Value | Ratio | Jupyter origin |
|---|---|---|---|
| `plain` | `#1F1E1C` | 15.26 | `--jp-content-font-color1` |
| `comment` | `#3F6E6E` | 5.25 | `#408080` teal, darkened |
| `keyword` | `#0A7A32` | 5.01 | `#008000` green |
| `string` | `#B02121` | 6.23 | `#BA2121` brick |
| `number` | `#0F6E38` | 5.82 | `#080` |
| `func` | `#1849B8` | 7.17 | `#00F` def-blue |
| `type` | `#0F6E38` | 5.82 | `#008000` builtin |
| `operator` | `#7024B0` | 7.49 | `#7800C2` |
| `deleted` | `#C4232B` | 5.31 | = `--text-danger` |
| `inserted` | `#12805C` | 4.51 | green |

### 4.11 Terminal ANSI (light) — measured on `--background-muted` `#F4F4F2`

**Corrected to the shipped values.** The terminal dock does *not* paint the code ground: Roche's
`terminalGround` is `--background-muted` in both modes, declared explicitly in
`roche-limit.theme.mjs`, and `background`/`cursorAccent` are **derived** from it rather than
authored. This table originally quoted the code ground `#F5F5F3` and three pre-regrounding stops;
§9 has admitted the re-grounding since it shipped, but the table was never updated. It is now.

`background #F4F4F2` · `foreground #1F1E1C` · `cursor #D95B08` · `cursorAccent #F4F4F2` · `selectionBackground #FADBBB`
`black #1F1E1C` · `red #C4232B` · `green #0F7150` · `yellow #6E6300` · `blue #0A69BC` · `magenta #7024B0` · `cyan #3F6E6E` · `white #69675F`
`brightBlack #5C5A55` · `brightRed #A8161E` · `brightGreen #0A5E42` · `brightYellow #5A5100` · `brightBlue #08579C` · `brightMagenta #5C1D91` · `brightCyan #2F5A5A` · `brightWhite #1F1E1C`

| Was documented | Ships | Why |
|---|---|---|
| `background #F5F5F3` | **`#F4F4F2`** | derived from `--background-muted`, not `--background-code` |
| `cursorAccent #F5F5F3` | **`#F4F4F2`** | same derivation (the glyph under the cursor is the ground) |
| `cursor #EE6C1A` | **`#D95B08`** | the cursor is a rail-class mark, so it uses `--accent-bar`, not the anchor — 3.50:1 on the ground |
| `white #84827C` | **`#69675F`** | `#84827C` is only 3.49:1 on `#F4F4F2`; `#69675F` reaches 5.15:1 |

Every non-dim slot clears AA on `#F4F4F2` (lowest: `blue #0A69BC` at 5.07:1).

---

## 5 · Dark palette — `.dark[data-theme='roche-limit']`

Same hue relationships, same semantic roles. Polarity flips: panels are **elevated** (lighter than
the canvas), per Jupyter's own `#111 → #212121`.

### 5.1 Surfaces

| Token | Value | Rationale |
|---|---|---|
| `--background-canvas` | `#131312` | What the main panel paints — the conversation, the hub, every top-level view. |
| `--background-app` | `#131312` | Warm-neutral window ground. Not pure black — halation around bright orange, and no room below. |
| `--background-default` / `--background-card` | `#1B1B19` | A card is a surface, not a hole. |
| `--background-muted` | `#232320` | |
| `--background-code` | `#1B1B19` | Matches `default`, as both shipping families do. All syntax stops measured here. |
| `--background-medium` | `#2C2C29` | Hover fill. |
| `--background-strong` | `#3A3A36` | |
| `--background-inverse` | `#EDEDEA` | |

### 5.2 Text

| Token | Value | On `#131312` | On `#1B1B19` |
|---|---|---|---|
| `--text-default` | `#EDEDEA` | 15.85 | 14.71 |
| `--text-muted` | `#A5A39D` | 7.37 | 6.84 |
| `--text-subtle` | `#9C9A93` | 6.60 | 6.13 |
| `--text-inverse` | `#131312` | — | — |

> `--text-subtle` was corrected from `#98968F` during verification — see §6.

### 5.3 Borders

`--border-subtle: #302F2C` · `--border-strong: #3E3D39` · `--border-input: #4A4945` · `--border-default: var(--border-subtle)`

### 5.4 Accent ramp — dark

| # | Value | Role |
|---|---|---|
| 1 | `#16110E` | app background tint |
| 2 | `#1C1510` | subtle background |
| 3 | `#331E0C` | UI element background |
| 4 | `#452201` | hover background / selection highlight |
| 5 | `#552A02` | active / selected background |
| 6 | `#64380F` | subtle border |
| 7 | `#7D471F` | border |
| 8 | `#A05B2B` | strong / hover border |
| **9** | **`#EE6C1A`** | **solid fill — identical to light, so brand fills never shift** |
| 10 | `#F27F30` | solid fill hover |
| 11 | `#F2955A` | low-contrast text — 7.58:1 on card |
| 12 | `#FBDFC2` | high-contrast text |

| Token | Value | Note |
|---|---|---|
| `--background-accent` | `#EE6C1A` | Same anchor both modes. |
| `--background-accent-hover` | `#F27F30` | |
| `--text-on-accent` | `#131312` | 6.02:1 |
| `--border-accent` | `#EE6C1A` | |
| `--text-accent` | `#F2955A` | 7.58:1 on card. |
| `--accent-bar` | `#EE6C1A` | **The pure anchor works in dark** — 6.02:1 on canvas, 5.10:1 even on the muted panel. |

### 5.5 Status

| Token group | Value | On `#131312` | ΔHue |
|---|---|---|---|
| `--*-danger` | `#FF9592` | 8.82 | 23° |
| `--*-success` | `#3DD68C` | 9.91 | 116° |
| `--*-info` | `#70B8FF` | 8.84 | 156° |
| `--*-warning` | `#F2E06B` | 13.85 | 53° |
| `--text-on-status` | `#131312` | ≥8.8:1 on all four fills | |

Note the deliberate light/dark stop split, exactly like both shipping families: warning is dark
olive-yellow in light (white ink) and bright yellow in dark (dark ink).

### 5.6 Focus

`--ring: #A5A39D` · `--background-focus: #35342F` · `--border-focus: #9C9A93`

> **Corrected during implementation**, same failure mirrored: the proposed
> `#33322E` was 1.09:1 from hover `#2C2C29`. `#35342F` gives 1.12:1.

### 5.7 Sidebar, heat, shadows

`--sidebar: #171716` (darker than the card, like Alma dark) · `--sidebar-hover: #232320` · `--sidebar-active: #2E2E2A` · `--sidebar-border: #2A2A27`

Heat: `--heat-0: #32312E` · `--heat-1: #5C3410` · `--heat-2: #7A4413` · `--heat-3: #B45F18` · `--heat-4: #EE6C1A`

> **Changed 2026-10-10 (Codex simplicity redesign).** Dark `--heat-0` was `#1E1D1B` and `--heat-1`
> was `#4A2A0E`. An idle day at `#1E1D1B` measured 1.02:1 on the dark card `#1B1B19` (1.10:1 on the canvas
> `#131312`), so a quiet week disappeared; `#32312E` measures 1.33:1 (`check-contrast.mjs` holds it at 1.3 in dark).
> `--heat-1` moved one step so the first active step stays above the lifted zero (1.21:1).
> Level 0 is shared by all three families, so the same `#32312E` sits in Parchment and Alma Mater.

```css
--shadow-default:  0px 1px 3px 0px rgba(0,0,0,0.25), 0px 0px 1px 0px rgba(0,0,0,0.35);
--shadow-composer: 0px 2px 10px -1px rgba(0,0,0,0.45), 0px 1px 3px 0px rgba(0,0,0,0.3);
--shadow-popover:  0px 8px 24px 0px rgba(0,0,0,0.4), 0px 0px 1px 0px rgba(0,0,0,0.5);
--shadow-modal:    0px 22px 64px -16px rgba(0,0,0,0.62), 0px 8px 26px -18px rgba(0,0,0,0.5), 0px 0px 0px 1px rgba(255,255,255,0.055);
--shadow-modal-chrome-bottom: none;
--shadow-modal-chrome-top: none;
```

### 5.8 Code syntax — measured on `#1B1B19`

| Key | Value | Ratio | Jupyter origin |
|---|---|---|---|
| `plain` | `#EDEDEA` | 14.71 | `rgba(255,255,255,1)` |
| `comment` | `#7FA3A3` | 6.30 | `#408080` lifted (Jupyter ships it unchanged in dark at ~2.8:1 — **do not copy that**) |
| `keyword` | `#6FCB78` | 8.62 | `#4CAF50` |
| `string` | `#FF8F8F` | 7.87 | `#FF7070` |
| `number` | `#84D089` | 9.34 | `#66BB6A` |
| `func` | `#7FBEF7` | 8.72 | `#1E88E5` lifted (3.4:1 as shipped) |
| `type` | `#84D089` | 9.34 | `#43A047` builtin |
| `operator` | `#D9A0FF` | 8.56 | `#D48FFF` |
| `deleted` | `#FF9592` | 8.19 | = `--text-danger` |
| `inserted` | `#3DD68C` | 9.20 | = `--text-success` |

### 5.9 Terminal ANSI (dark) — measured on `--background-muted` `#232320`

**Corrected to the shipped values,** for the same reason as §4.11: `terminalGround` is
`--background-muted` in dark too, not the code ground `#1B1B19`.

`background #232320` · `foreground #EDEDEA` · `cursor #EE6C1A` · `cursorAccent #232320` · `selectionBackground #452201`
`black #3A3A36` · `red #FF9592` · `green #3DD68C` · `yellow #F2E06B` · `blue #70B8FF` · `magenta #D9A0FF` · `cyan #7FA3A3` · `white #A5A39D`
`brightBlack #9C9A93` · `brightRed #FFB3B0` · `brightGreen #6FE5A6` · `brightYellow #F7EC9A` · `brightBlue #9CCDFF` · `brightMagenta #E6BCFF` · `brightCyan #A0C0C0` · `brightWhite #FFFFFF`

| Was documented | Ships | Why |
|---|---|---|
| `background #1B1B19` | **`#232320`** | derived from `--background-muted` |
| `cursorAccent #1B1B19` | **`#232320`** | same derivation |
| `black #131312` | **`#3A3A36`** | on an *elevated* ground the dim slot must lift with it — `#131312` sat 1.18:1 below the new ground, i.e. a hole punched in the panel. `#3A3A36` reads as recessed (1.38:1) rather than absent. Held to a floor of 1.0, not 3.0: it is a ground-by-convention, not text. |
| `brightBlack #5C5A55` | **`#9C9A93`** | the real fix. `brightBlack` carries dimmed/comment output and is held to 3:1 — `#5C5A55` is **2.29:1** on `#232320`, a fail. `#9C9A93` is 5.60:1. |

Every non-dim slot clears AA on `#232320` (lowest: `cursor #EE6C1A` at 5.10:1, `cyan #7FA3A3` at 5.76:1).

---

## 6 · Accessibility ledger

`npm run lint:check` runs `ui/desktop/scripts/check-contrast.mjs` in CI. A family that fails it fails
the build. The palette was verified with an independent WCAG 2.x implementation, self-tested against
the reference value `#767676` on white = 4.54:1.

**Result: 228 assertions, all pass** — every text token against every ground it can land on, every
ink-on-fill pair, every non-text boundary at 3:1, and all 20 syntax stops. (An earlier draft of this
section said 164; the real count at ship was 228, as §9 records.)

Verification **corrected three values** that the first synthesis had rounded past:

| Fix | Was | Now | Why |
|---|---|---|---|
| light `--text-subtle` | `#6B6963` | **`#69675F`** | 4.47:1 on `--background-focus` — a fail reported as a pass. Now 4.61:1 on the darkest ground it can reach. |
| dark `--text-subtle` | `#98968F` | **`#9C9A93`** | Same failure mirrored: 4.34:1 on the dark focus surface. Now 4.56:1. |
| light `--accent-bar` | `#EE6C1A` | **`#D95B08`** | The bright anchor is 3.09:1 on white but only **2.81:1 on the grey panel** — rails and status dots lose most of their separation the moment they sit on a card. `#D95B08` recovers ~0.7 of ratio across the board (3.50:1 on that same panel). `--background-accent` stays `#EE6C1A`, so fills keep their brightness. Dark keeps the pure anchor. |

> **The 3:1 claim this table used to make was false.** It read "`#D95B08` clears 3:1 on *every*
> light ground (min 3.14)." Recomputed against all ten light grounds it can land on, the true
> minimum is **2.80:1**, and two grounds fail:
>
> | Ground | `#D95B08` on it |
> |---|---|
> | `--background-app` / `-default` / `-card` `#FFFFFF` | 3.85 |
> | `--sidebar` `#F7F7F5` | 3.59 |
> | `--background-code` `#F5F5F3` | 3.53 |
> | `--background-muted` `#F4F4F2` | 3.50 |
> | `--sidebar-hover` `#EFEFEC` | 3.34 |
> | `--heat-0` `#EEEEEA` | 3.31 |
> | `--background-medium` `#ECECE9` | 3.26 |
> | `--sidebar-active` `#EAEAE6` | 3.19 |
> | **`--background-focus` `#E0E0DC`** | **2.91 ✗** |
> | **`--background-strong` `#DCDCD8`** | **2.80 ✗** |
>
> **No theme holds this rule, and none ever has.** On its own `--sidebar-active`, Parchment's rail
> `#CF6D47` measures **2.53:1** and Alma Mater's `#16A0AC` measures **2.23:1** — both worse than
> Roche's 3.19 on the equivalent row. The design does not treat the rail as a standalone affordance:
> it is decorative reinforcement of a background change the active row *already* makes, so it is
> never the sole cue and **SC 1.4.11 does not bite.** `check-contrast.mjs` therefore deliberately
> does **not** assert `--accent-bar`, and carries a comment explaining why — asserting it would fail
> all three families on day one. Revisit only if the rail ever becomes the only cue.

**Two things carried forward deliberately:**

- **Ink on orange is dark, never white.** White on `#EE6C1A` is **3.09:1** — a fail at any size.
  (Earlier drafts of this line, and of §4.4, printed 2.81:1. That is **Jupyter's** `#F37726` on
  white, cross-contaminated from the §3 comparison, where both figures are stated correctly. The
  conclusion is unchanged: 3.09 fails 4.5:1 just as 2.81 does.)
- **`--border-input` at `#C9C9C3` is 1.66:1 on white,** below the 3:1 non-text threshold. This is
  inherited parity — Parchment and Alma Mater both do the same, and it matches Jupyter's own
  `#E0E0E0`. Fixing it is a theme-wide change, not a Roche Limit change. Flagged, not silently
  shipped. See **Q2**.

---

## 7 · Open questions — and what was actually decided

Written before implementation, when three of these were live toggles in the companion mockup. The
**Outcome** column records what shipped; the recommendations are left in their original voice.

| # | Question | Recommendation | Outcome |
|---|---|---|---|
| **Q1** | **How orange should the chrome be?** *Instrument* keeps every hover/active/focus surface neutral; *Warm* tints sidebar-active and focus toward the orange ramp. | **Instrument** — the Jupyter-faithful reading, and it keeps orange meaning something. Note the warm option has a hard ceiling: ramp step 4 (`#FADBBB`) as a focus surface **fails AA at 4.29:1**, so the toggle uses steps 2/3. | ✅ **Taken.** Every hover/active/focus surface ships neutral. |
| **Q4** | **Do tool-call cards get the cell rail?** A 2px `--accent-bar` rail on the active card is the most notebook-legible move available. | **On** — but it needs a component change, not just tokens, which breaks the "pure re-colouring" contract Alma Mater set. **Your call on scope.** | ❌ **Not taken.** No component change shipped; tool-call cards have no cell rail. The recommendation stands as a proposal, and the "pure re-colouring" contract held. Whoever picks this up should note §6: `--accent-bar` does not clear 3:1 on every ground, so a rail must not be the card's only active cue. |
| **Q6** | **Warm-neutral or pure grey?** Warm-neutral holds every grey under OKLCH chroma ~0.006. | **Warm-neutral** — it prevents orange-on-cool-grey reading as hazard signage. Both variants verified AA. | ✅ **Taken.** |
| **Q3** | **Adopt Jupyter's font stack?** | **No.** Fonts live in `@theme inline`, the declared single source of truth; Alma Mater's precedent is "same layout, same fonts". Jupyter's stack is near-identical to BioRouter's anyway. | ✅ **Held.** Fonts unchanged. |
| **Q5** | **Should a family be allowed to change radii?** Jupyter reads as technical largely because of its 2px corners; ours are 4–16px and live outside family scope. | **Not now.** Moving `--radius-*` from `@theme` into `:root` is a real architecture change touching all four existing blocks. | ⏸ **Deferred.** Radii remain outside family scope. |
| **Q2** | **Fix `--border-input` contrast globally?** | **A separate theme-wide fix**, not a Roche Limit special case. | ⏸ **Deferred.** `--border-input: #C9C9C3` ships as specified, still 1.66:1 on white, still theme-wide. |

---

## 8 · Exportability

You asked that these adjustments be settings you can export. **The honest answer: no export exists
today, for any setting.**

- Theme state lives only in renderer `localStorage` — `theme`, `use_system_theme`, `theme_family`.
- It never reaches the backend, and never lands in a file you can copy.
- `config.yaml` has no appearance keys. The Electron `settings.json` has six keys, none of them
  appearance.
- This is already true of Parchment and Alma Mater. Roche Limit doesn't introduce the gap — it makes
  it visible.

**Tier 1 — ships with the theme (~1 day, no backend work).** An "Export / Import appearance" pair in
`AppSettingsSection.tsx`, writing a versioned blob through the save-dialog IPC that `WorkflowsView`
already uses:

```json
{ "schema": "biorouter.appearance/1",
  "themeFamily": "roche-limit",
  "theme": "dark",
  "useSystemTheme": false }
```

Import needs `showOpenDialog` + `readFile` on the preload bridge — **only the save side is confirmed
wired**; verify before planning. On import, validate `schema`, validate `themeFamily` against the
shared registry (Tier 3), then call the existing `setThemeFamily` / `setTheme` setters so
localStorage, the DOM and the IPC broadcast all update through one code path.

**Tier 2 — the real fix (separate plan).** Round-trip appearance through `/config/upsert` +
`/config/read` into `~/.config/biorouter/config.yaml`. That file is already what users back up and
move between machines, so "exportable" becomes free. The cost is a **hybrid** read path:
`index.html` reads localStorage *synchronously* to prevent the theme flash, so config becomes the
source of truth and localStorage becomes a sync cache.

**Tier 3 — do this regardless.** The family id is currently a hardcoded string literal in five
places plus a key in three lookup tables. Introduce a registry:

```ts
export const THEME_FAMILIES = ['parchment', 'alma-mater', 'roche-limit'] as const;
export type ThemeFamily = (typeof THEME_FAMILIES)[number];
```

`index.html` cannot import TS and stays a deliberate duplicate — comment both sides, as
`ThemeContext.defaults.test.ts` already does for `loadThemePreference`.

---

## 9 · Implementation — as built

### 9.1 · Where the theme lives now — one file, everything else generated

**This supersedes the file-by-file account below.** Roche Limit shipped by hand-editing ten sites;
that experience — three families each redeclaring every token across nine files — is what motivated
the re-architecture recorded in
[`docs/design/theming/theme-system-architecture.md`](theme-system-architecture.md). **A theme is now a single
authored file:**

```text
ui/desktop/themes/roche-limit.theme.mjs     ← the only file anyone edits
npm run themes                               ← regenerates every consumer
npm run check:themes                         ← --check mode; part of npm run lint:check
```

Everything downstream is emitted from it and marked as generated — do not hand-edit any of it:

| Generated artifact | What it carries |
|---|---|
| `src/styles/main.css` (`THEMES:GENERATED:FAMILIES` block) | the `:root[data-theme='roche-limit']` + `.dark[…]` token blocks |
| `src/styles/themes.generated.ts` | syntax palette, terminal palette, code ground, brand-mark colours, the family id list |
| `src/contexts/ThemeContext.tsx`, `index.html`, `ThemeFamilySelector.tsx`, `codeTheme.ts`, `InAppTerminalDock.tsx`, `BioRouterMark.tsx` | family registry, boot-splash CSS, picker label + swatch, consumer palettes |

Three changes in that re-architecture are visible in this document:

- **`--scrim` is a token.** The per-family hardcoded overlay `rgba()` is gone; see the note at the
  end of §10. Roche light `rgba(31, 30, 28, 0.18)`, dark `rgba(0, 0, 0, 0.48)`.
- **`--sidebar-icon` is a token.** Already noted below as the late discovery of the original ship;
  it is now a declared part of the contract rather than a token found by running CI. Roche light
  `#D95B08` (3.59:1 on `--sidebar`, 3.19:1 on the darkest row it can sit on), dark `#EE6C1A`
  (5.81:1 / 4.41:1). Unlike `--accent-bar`, this one **is** asserted at 3:1 against all three
  sidebar rows, because a nav icon really can be the only cue.
- **`terminalGround` is declared, not assumed.** Roche's terminal paints `--background-muted` in
  **both** modes, and `roche-limit.theme.mjs` says so explicitly. The generator *derives*
  `terminal.background` and `terminal.cursorAccent` from that token instead of letting them be
  authored, which is why §4.11 and §5.9 had drifted: the families genuinely disagree here
  (Parchment dark grounds on `--background-code`), so nothing could be safely unified.

### 9.2 · The original ship, 2026-07-18

Shipped 2026-07-18. The plan said eight files; **ten sites** were needed. The two the
plan missed were both found by looking at the running app rather than the token layer:

| Missed site | Why it mattered |
|---|---|
| **`index.html` boot-splash CSS** | The pre-React splash has per-family blocks (`html[data-theme='alma-mater'] #br-boot`). Without a Roche block the splash paints Parchment cream for the whole backend-startup window, then jumps to white. Added light + dark blocks. |
| **`src/components/icons/BioRouterWordmark.tsx`** | Hardcodes `NAVY`/`CORAL`/`TEAL`. It is **mode**-aware but deliberately **not family**-aware — Parchment and Alma Mater share one wordmark — so this was left alone. See the open item below. |

Two more corrections came out of running the real CI gate rather than a private script:

- **`--sidebar-icon` exists** (60 semantic tokens, not the 59 this spec first claimed).
  It is the token that actually carries "orange icons". *(`--scrim` was tokenised
  afterwards, so the count is **61** today — see §9.1 and §10.)*
- **Both `--background-focus` values were wrong** against the repo's 1.1 focus/hover
  floor. Fixed in §4.6 / §5.6.

A third finding is worth recording because it nearly caused an over-correction:
**Alma Mater ships `text-subtle` on `--background-focus` at 3.92:1**, so the repo
deliberately does *not* assert that pair. An earlier pass tried to force it to 4.5:1,
which would have required moving the hover fill and flattening the muted/medium ramp.
The house rule is: **`text-default` must clear AA on a focus surface; `text-subtle`
need not.**

### Files touched

| # | File | Change |
|---|---|---|
| 1 | `src/styles/main.css` | `:root[data-theme='roche-limit']` + `.dark[data-theme='roche-limit']` after the Alma blocks. 87 declarations light (60 semantic + 27 raw remaps), 60 dark — verified token-for-token against the Alma blocks. *(Today: **89 light** = 62 semantic + 27 raw, **62 dark**, after `--background-canvas`; and the block is generated, not hand-written.)* |
| 2 | `src/contexts/ThemeContext.tsx` | Replaced both hardcoded `=== 'alma-mater'` checks with a `THEME_FAMILIES` registry + `isThemeFamily()` guard, so the next family is one edit. |
| 3 | `index.html` | Pre-hydration script derives from a duplicated `FAMILIES` list (commented as a deliberate duplicate); **plus** the two boot-splash blocks. |
| 4 | `src/components/BioRouterSidebar/ThemeFamilySelector.tsx` | Third entry; `grid-cols-2` → `grid-cols-3`. |
| 5 | `src/styles/codeTheme.ts` | `ROCHE_LIGHT`/`ROCHE_DARK`, registered in **`codeThemesByFamily`**; `CODE_BG_ROCHE`, `codePalettesRoche`. |
| 6 | `src/components/InAppTerminalDock.tsx` | `ROCHE_TERMINAL_THEMES` (20 xterm slots × 2), grounded on `--background-muted` like its siblings — which shifted eight ANSI stops off this spec's original §4.11 / §5.9 values, re-verified on the real ground. **§4.11 and §5.9 now carry the shipped values**, with the diff and the reason for each. |
| 7 | `src/styles/codeTheme.test.ts` | AA assertions for both Roche palettes, a guard that no family is missing from `codeThemesByFamily`, and a pin that the two sub-AA JupyterLab stops are never reintroduced. |
| 8 | `src/components/BioRouterSidebar/ThemeFamilySelector.test.tsx` | **New.** Derives from `THEME_FAMILIES`, so a family added without a button — or without bumping the grid — fails. |
| 9 | `scripts/check-contrast.mjs` | `ROCHE_L`/`ROCHE_D` with the Alma cascade merge. **140 → 228 assertions.** |

### Verification performed

- `node scripts/check-contrast.mjs` — **228/228 pass**.
- `npm run typecheck` — clean. `eslint --max-warnings 0` — clean.
- Theme unit tests — 34 pass (codeTheme 13, ThemeContext 7, terminal 8, selector 6).
- **Real `main.css` rendered in a browser**, computed values read back for all three
  families × both modes — no cross-mode leakage.
- **Real Electron app** driven to `data-theme="roche-limit"` in light and dark;
  pre-hydration script confirmed setting the family before first paint.
- Full suite: 15 failures, **proven pre-existing** by re-running them on a clean
  worktree at HEAD with only the theme files applied (40/40 pass there). They come
  from unrelated in-flight work on the branch, not from this theme.

### 9.3 · Adopted as the shared neutral set, 2026-08-08

**Nothing in `roche-limit.theme.mjs` changed.** What changed is its scope: Parchment and Alma
Mater were moved onto these neutrals, so this family's surface values are now BioRouter's, and
the guards treat them that way.

- The other two families' neutral ramps were deleted; `--color-neutral-50` … `-950` in the base
  `@theme` block is this family's ramp, and both siblings restate it verbatim.
- `check-contrast.mjs` gained a per-scope assertion that `--background-muted` is a real step off
  `--background-canvas` — the invariant this family satisfied and the other two did not. 330
  assertions now, up from 324.
- Parchment's terminal `yellow` and `cyan` had to be darkened by ~2.5%: they were tuned against
  its cream dock ground `#faf8f3` and measured 4.35/4.37:1 on this family's `#f4f4f2`. Roche
  Limit itself needed no retuning, since it was already measured on these grounds.
- `terminalGround` is now `--background-muted` in all three families. Parchment dark used to
  point at `--background-code`; its ANSI palette was re-verified against the muted ground rather
  than assumed compatible with it.

### Known follow-up

The wordmark's `CORAL = '#b85a32'` (Parchment terracotta) now sits beside Roche's
`#EE6C1A` — two oranges about 20° apart, a near-miss rather than a clash. Making the
wordmark family-aware is a **brand** decision, not a theme one, so it was not taken
unilaterally. One-line fix if wanted: derive `CORAL` from `--accent-bar`.

The neutral unification did **not** touch this: the wordmark and the app-icon plate are fixed
brand artwork (`BioRouterMark.tsx`'s `PLATE = '#faf8f3'`, and the `#faf8f3` grounds in
[the logo and wordmark spec](../branding/logo-and-wordmark-spec.md)) and are deliberately not
themed. That cream is brand, not a theme neutral, and it is correct that it no longer matches
any token.

---

## 10 · Original plan (kept for reference)

`grep -rn "alma-mater" ui/desktop/src ui/desktop/index.html ui/desktop/scripts` —
**every hit is a site Roche Limit also needs.**

| # | File | Change |
|---|---|---|
| 1 | `src/styles/main.css` | Add `:root[data-theme='roche-limit']` then `.dark[data-theme='roche-limit']`, inserted after L506. Dark block **must** come second — the two tie at specificity (0,2,0) and dark wins only by source order. Re-declare **all 59** semantic tokens in both. |
| 2 | `src/contexts/ThemeContext.tsx` | L10 widen the `ThemeFamily` union. L77 `loadThemeFamily()` is a hardcoded `=== 'alma-mater'` check. L174 the IPC convergence guard is a second hardcoded allow-list — without it, a family broadcast from window A is silently dropped by window B. |
| 3 | `index.html` (L14–19) | The pre-hydration script duplicates `loadThemeFamily` in plain JS. Must accept `roche-limit` or the app flashes Parchment on every launch. |
| 4 | `src/components/BioRouterSidebar/ThemeFamilySelector.tsx` | Append `{ id: 'roche-limit', label: 'Roche Limit', swatch: '#EE6C1A' }`. **L33 hardcodes `grid-cols-2`** — bump to `grid-cols-3`. |
| 5 | `src/styles/codeTheme.ts` | Add the light/dark `SyntaxPalette` pair and register in **`codeThemesByFamily`** — ⚠️ *not* the flat `codeThemes` map at L166, which is mode-keyed and Parchment-only. |
| 6 | `src/components/InAppTerminalDock.tsx` | Add `ROCHE_TERMINAL_THEMES` (20 xterm slots × 2 modes) to `TERMINAL_THEMES_BY_FAMILY`. Indexed with no fallback — a missing key is a runtime `undefined`. |
| 7 | `src/styles/codeTheme.test.ts` | Extend the ≥4.5:1 `it.each` block over the new palettes. |
| 8 | `scripts/check-contrast.mjs` | Add the `roche-limit` blocks with the same cascade merge the Alma blocks use. **Without this, CI has no guard on the new family.** |

Plus one optional, family-scoped, non-token change: the modal/diagnostics scrim at `main.css`
L1277–1279 is a hardcoded `rgba()` outside the token layer; light Roche Limit should add its own
`rgba(31, 30, 28, 0.22)` rule.

> ✅ **Done — and done better than proposed.** Rather than adding a fourth per-family component
> rule, the scrim was **tokenised**: `--scrim` is now a real semantic token every family declares,
> and the overlay rules are a plain `background: var(--scrim)`. Roche light ships
> **`rgba(31, 30, 28, 0.18)`** — the warm near-black of `--background-inverse` at the same 0.18 alpha
> the other light families use, not the 0.22 proposed here. Roche dark ships `rgba(0, 0, 0, 0.48)`.
> This also fixed a latent bug the plan did not anticipate: because the old rule was hardcoded
> Parchment brown, **Roche Limit silently wore Parchment's scrim** until the token landed.

Electron main and `preload.ts` need **no change** — `themeFamily` is an opaque string on the IPC
payload.

**Verification gate:** `cd ui/desktop && npm run lint:check` (typecheck + eslint + contrast) and
`npm run test:run`.

## Related documentation

- [Theming](README.md) — the folder index, and the other theme families' token references.
- [Theme system architecture](theme-system-architecture.md) — how a theme is defined once and generated into every consumer; the authority over any hex quoted here.
- [Alma Mater theme tokens](alma-mater-theme-tokens.md) — the sibling token reference for the UCSF-brand family.
- [Biorouter Design System](../../../design.md) — the parent design system and the register of the numbered `D-NN` decisions cited above.
