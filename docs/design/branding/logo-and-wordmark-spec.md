# BioRouter logo and wordmark specification

> **What this is.** The normative specification for two BioRouter brand assets: a two-colour
> `BioRouter` wordmark with a split navy/coral rule, and a `BR` monogram derived from it for the
> square app icon. It fixes geometry, colour tokens and lockups, and records the licence blocker
> that ruled out the originally chosen typeface.
> **Status:** Current. Geometry, colour and lockups were approved in the specimen on 2026-07-17
> and still stand. The typeface section is superseded: the face was still open when this was
> written, and [UI overhaul execution status](../ui-overhaul/execution-status.md) records that
> Inter (SIL Open Font License) was chosen and shipped on 2026-07-18. Treat that document as
> authoritative on the typeface and on the values re-tuned in Inter.
> **Audience:** designers and developers regenerating or reviewing BioRouter brand assets.

The BioRouter identity was previously a router glyph. This spec retires it in favour of a
typographic mark. Two terms recur below. **D-06** is a numbered decision in the BioRouter design
system (decisions run D-01…D-37); it is the rule that the app fetches no webfont and renders in the
native font stack only. **The client** is the design's approver — the stakeholder who reviewed the
interactive specimen and signed off on each value.

> **Note.** The specimen was an interactive HTML page served locally during the design session. It
> is not checked into this repository, so the readouts it produced are quoted here rather than
> reproducible from it. The interactive branding studios that live beside this spec are
> [the wordmark studio](logo-wordmark-studio.html) and [the icon centring studio](logo-icon-studio.html).

---

## Summary

Retire the router glyph. The BioRouter identity becomes **typographic**: a two-colour
wordmark set in the app's own font, plus a **BR monogram** derived from it for the
square app icon.

| Asset | Form | Where it lives |
|---|---|---|
| **Wordmark** — beige ground | `BioRouter` + split rule | headers, landing site, docs, print |
| **Wordmark** — transparent | same, alpha background | in-app |
| **BR monogram** — beige ground | `BR` + split rule, square | dock, favicon, `.icns` / `.ico`, CLI |

The monogram is not a compromise on "just the text" — it *is* just the text, cropped to
the two letters that survive at 16px. It carries the same navy→orange rule as the
wordmark, which is what binds the two together.

---

## Decisions and why

### Typeface: a bold outlined grotesque

The letters are set in a **bold grotesque and converted to outlined paths**. The specific face was
originally SF Pro; that choice does not survive the licence analysis in
[SF Pro cannot be used for this logo](#sf-pro-cannot-be-used-for-this-logo). The reasoning for
*outlining* below is independent of the face and drives the design either way.

The app's `--font-sans` (`ui/desktop/src/styles/main.css:526`) is a **native stack** —
`ui-sans-serif, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, …` — and the block
is explicit that no webfont is fetched (D-06). That stack resolves to **SF Pro on macOS,
Segoe UI on Windows, Roboto on Linux**.

A logo set in live text would therefore be *a different logo on every OS*, and the PNG
exports would not match the SVG. Outlining the letters to paths removes the font
dependency: the logo renders identically everywhere with no `@font-face`, which also keeps
D-06 intact.

Cost accepted: the text is no longer editable or selectable. Re-generating from source is
the only way to change it.

### Why the square icon is a BR monogram

This decision was made **from measurement, not taste.** The specimen renders each lockup
at true icon sizes and reads the cap height from the rasteriser
(`TextMetrics.actualBoundingBoxAscent`, measured ratio **0.7046em** for SF Pro Bold).

| Icon size | **BR monogram** | Stacked `Bio`/`Router` | One-line `BioRouter` |
|---|---|---|---|
| 128px | **62.0px cap** | 20.8px | 13.7px |
| 64px | **31.0px** | 10.4px | 6.9px |
| 48px | **23.3px** | 7.8px | 5.2px |
| 32px | **15.5px** | 5.2px | 3.4px |
| **16px (favicon)** | **7.8px** | **2.6px** | **1.7px** |

> **Note.** The 0.7046em cap-height ratio is a measurement of SF Pro Bold, so every absolute pixel
> value in this table is typeface-dependent and changes with the substitute face.

At 16px the monogram has **3× the stacked cap height and 4.6× the one-line's**. Six letters
cannot render into 1.7px of cap; two letters into 7.8px can, and the favicon mock confirms
`BR` is genuinely readable. The stacked and one-line squares are retained in the specimen
as the evidence for this decision, not as live options.

### Colour

| Role | Token | Hex |
|---|---|---|
| `Bio`, and the rule's left half | UCSF Navy | `#052049` |
| `Router`, and the rule's right half | `--color-coral-700` | `#a94f2a` |
| Ground (opaque variant) | *(none — fixed brand value)* | `#faf8f3` |

> **The beige ground is a literal, not a token, and that is deliberate.** It used to be documented
> as `--color-neutral-50`, which was `#faf8f3` while Parchment's cream ramp was the app's neutral
> ramp. On **2026-08-08** the three theme families were moved onto one shared neutral set and
> `--color-neutral-50` became `#fafaf9`, so the mapping no longer holds. **Nothing about the mark
> changed**: `BioRouterMark.tsx` carries `const PLATE = '#faf8f3'` with a comment stating that the
> plate is *not* themed, because it renders the OS app-icon variant — fixed brand artwork that
> should not shift with whatever palette the app is wearing. The cream no longer matching any app
> token is the correct outcome, not drift. Do not "fix" it by re-pointing it at a token.

`coral-700` was chosen over `coral-600` (`#b85a32`, the app's `--text-accent`/CTA fill)
after comparing all three stops side by side on the real beige ground.

Contrast on beige, live-computed in the specimen: coral-500 **3.34:1**, coral-600
**4.35:1**, coral-700 **5.15:1**. Note that **WCAG 1.4.3 explicitly exempts logotypes**
("text that is part of a logo or brand name has no minimum contrast requirement"), so
coral-600 would not have been a violation — but coral-700 is the only stop that would
clear AA if this were body text, and it is visibly sturdier at small sizes.

**Inverted (navy ground):** `Bio` and the rule's left half flip to beige `#faf8f3`;
`Router` and the right half stay `coral-700`. Without this the navy half of the rule
disappears into the ground and the underline reads as orange-only-starting-midway.

---

## Wordmark specification

```text
BioRouter
```

| Property | Value |
|---|---|
| Font | SF Pro Display **Bold** (weight 700), outlined — **superseded**, see [SF Pro cannot be used for this logo](#sf-pro-cannot-be-used-for-this-logo) |
| Tracking | **0** |
| `Bio` | `#052049` |
| `Router` | `#a94f2a` |
| Rule — start | left edge of the **`o`** in `Bio` |
| Rule — end | right edge of the **`R`** in `Router` |
| Rule — colour split | at the `Bio`\|`Router` advance boundary |
| Rule — weight | **0.15em** |
| Rule — gap below baseline | **0.21em** (see [Recording the baseline gap](#recording-the-baseline-gap)) |
| Rule — corner radius | **0** (square terminals) |

The rule's colour split lands on the same x as the letters' colour split, so the bar reads
as an echo of the word above it rather than an independent graphic.

### Recording the baseline gap

The client approved this geometry with the specimen's **"Bar gap" reading `.00em`**. That
readout was **wrong**. `layoutBar()` offset the rule from `bioRect.bottom`, which under
`line-height:1` is the *font box* bottom — already **0.209em below the baseline** — not the
baseline. The control's entire 0–0.22 travel therefore spanned **0.209em–0.429em**, and
no setting could produce a tight underline.

The specimen was corrected to measure the **true baseline** (a zero-height inline-block probe),
making the readout honest. The approved value is recorded here as **0.21em** because that is what
the client actually saw and approved — not `0`.

> **Open.** The client's own BR reference image has a gap of roughly **0.09em**, a
> conventional underline distance the broken control could not reach. `0.21em` may have been
> "as tight as the slider allowed" rather than a preference. Re-confirm the gap against the
> now-honest control before outlining.

---

## BR monogram specification

```text
BR
```

| Property | Value |
|---|---|
| `B` | `#052049`, full size |
| `R` | `#a94f2a`, **72%** of `B`'s font size |
| Baseline | shared — `B` and `R` sit on the same baseline |
| Tracking | 0 |
| Rule | spans the **full lockup width**, splits at the `B`\|`R` boundary |
| Rule weight / gap / radius | as the wordmark: 0.15em / 0.21em / 0 |
| Icon ground | `#faf8f3`, corner radius **20%** (matches the current `icon.svg` `rx=205` on 1024) |
| Lockup width | ≈**76%** of the icon (font-size 352 in a 512 box → 704 in 1024) |

`R` at 72% is the specimen default and matches the client's reference (~68–72%). The
specimen exposes an **R size** control (55–100%) if this wants re-tuning.

---

## Deliverables

**SVG** (letters as `<path>`, no font dependency):

1. `wordmark-beige.svg` — opaque `#faf8f3` ground
2. `wordmark-transparent.svg` — alpha ground
3. `wordmark-inverted.svg` — navy ground variant
4. `monogram-beige.svg` — square, 1024×1024, `rx=205`
5. `monogram-transparent.svg`

**PNG / platform icons**, regenerated from the monogram SVG: `icon.png`, `icon@2x.png`,
`icon.icns`, `icon-light.icns`, `icon.ico`, `iconTemplate*.png`, `landing/icon.png`,
`crates/biorouter-cli/static/img/logo_{dark,light}.png`.

---

## Scope: files this touches

Replacing the glyph is **not** a two-file change. `scripts/check-brand-consistency.sh`
hard-asserts the glyph path `M 125 220` across 8 locations and will fail CI on day one.

**Canonical assets**

- `ui/desktop/src/images/icon.svg` ← canonical, `rect fill="#faf8f3" rx="205"`
- `ui/desktop/src/images/glyph.svg` ← canonical mark
- `ui/desktop/src/images/icon-light.svg`

**Copies the script byte-compares against the canonical**

- `landing/icon.svg`, `landing/video/icon.svg`, `landing/video/assets/icon.svg`, `landing/video/reel/icon.svg`
- `crates/biorouter-cli/static/img/logo_dark.png`, `logo_light.png` (compared to `landing/icon.png`)

**Hard-coded glyph assertions to rewrite**

- `scripts/check-brand-consistency.sh` — `grep -q 'M 125 220'` on `glyph.svg` + `icon.svg`,
  and on `docs/agentic-system.html`, `docs/design-system.html`, `docs/theme-system.html`
- `ui/desktop/src/components/icons/BioRouter.tsx` — script asserts it references `glyph.svg`

**Components**

- `ui/desktop/src/components/BioRouterLogo.tsx` and `WelcomeBioRouterLogo.tsx` were removed
  as unused (R-29 of the Codex simplicity redesign). They composed the glyph with a `Rain`
  overlay, nothing rendered them, and the open question about the overlay went with them.
  Wherever the app shows a Biorouter logo it is the text logo,
  `ui/desktop/src/components/icons/BioRouterWordmark.tsx`, or the square mark in
  `icons/BioRouterMark.tsx` where the wordmark is too wide.

> **Note.** The three `docs/*.html` paths asserted by `check-brand-consistency.sh` have since
> moved: `docs/agentic-system.html` is now [`docs/architecture/agentic-system-explorer.html`](../../architecture/agentic-system-explorer.html),
> `docs/design-system.html` is now [`docs/design/design-system-gallery.html`](../design-system-gallery.html),
> and `docs/theme-system.html` is now [`docs/design/theming/theme-system-explorer.html`](../theming/theme-system-explorer.html).

---

## Verification

- `./scripts/check-brand-consistency.sh` must pass after the rewrite.
- Render the favicon at a true 16px and confirm `BR` is legible (not a smudge).
- Confirm the Intel/ARM `.icns` and the `.ico` regenerate and the dock icon is not blank.
- The specimen's own guard: cap heights come from the rasteriser, so the section's
  "measured, not estimated" claim is literally true.

---

## Open risks

### SF Pro cannot be used for this logo

> **Warning.** This is a verified licence blocker, checked against Apple's licence text on
> 2026-07-17, not assumed.

**Apple San Francisco font licence** (the separate agreement referenced by §1.B of the
Design Resources licence, accepted on download from `developer.apple.com/fonts`):

> "The Apple San Francisco font is to be used **solely for creating mock-ups of user
> interfaces** to be used in software products running on Apple's iOS, OS X or tvOS
> operating systems."

> "You **may not use the Apple Font to create, develop, display or otherwise distribute
> any documentation, artwork, website content or any other work product**."

**Apple Design Resources Licence** (`Apple-Design-Resources-License-20230621`), §2.B
*Other Use Restrictions*, verbatim:

> "The grants set forth in this License do not permit you to, and you agree not to,
> install, use or run the Apple Design Resources for the purpose of creating mock-ups of
> user interfaces to be used in software products running on **any non-Apple operating
> system software**. You may not embed the Apple Design Resources in any software programs
> or other products. Except as expressly provided for herein, you may not use the Apple
> Design Resources to create, develop, display or otherwise distribute any documentation,
> artwork, website content or any other work product."

And §2.D *No Reverse Engineering; Limitations*:

> "You may not … **create derivative works** of the Apple Design Resources or any part
> thereof."

**Why every clause bites this specific project:**

| What was planned | Clause it hits |
|---|---|
| A **logo** — not a UI mock-up | "solely for creating mock-ups of user interfaces" |
| Ships on **Windows + Linux** | "running on any non-Apple operating system software" |
| Ships on the public **landing site** | "may not … distribute any … website content" |
| **Outlining the glyphs to paths** | "may not … create derivative works" |

The outlining step proposed above as the portability fix is itself the clearest
violation. **SF Pro is out for the logo.** No legal review is needed to reach that
conclusion — it is unambiguous on the face of the licence.

**Important scope note — this does *not* affect the app's UI.** `--font-sans` resolving to
SF Pro on macOS via `-apple-system` is the OS supplying its own system font to a native
app, which is normal and unaffected. Only *baking SF outlines into a distributed brand
asset* is prohibited.

**The design survives; only the typeface changes.** The brief's intent — "the main font of
the app" — is about *matching the app*, and the app's font is a **stack**, not SF
specifically. On Windows that stack is already Segoe UI, on Linux Roboto; there has never
been one true BioRouter letterform. Substitutes that are SIL OFL (logo-safe) and sit
naturally beside SF Pro:

| Font | Note |
|---|---|
| **Inter** | Deliberately SF-adjacent; the closest match, licence-clean (OFL) |
| **Public Sans** | USWDS; a neutral, institutional grotesque |
| **Source Sans 3** | Adobe, OFL; slightly warmer |
| **IBM Plex Sans** | More character; reads less "system default" |

Everything else in this spec — the colour split, the rule geometry, the monogram, and
every measurement in [Why the square icon is a BR monogram](#why-the-square-icon-is-a-br-monogram) —
carries over unchanged. The next step recorded at the time was to re-render the specimen with these
candidates and pick; [UI overhaul execution status](../ui-overhaul/execution-status.md) records the
outcome of that step.

### `coral-700` is a theme token, not a brand constant

`--color-coral-700` is re-defined by the **Alma Mater** theme family
(`main.css:286`), which recolours Parchment's coral to UCSF Violet/Eggplant. The logo must
use the **literal hex `#a94f2a`**, not `var(--color-coral-700)`, or the wordmark will turn
purple under that theme.

### The unconfirmed gap value

See [Recording the baseline gap](#recording-the-baseline-gap) — re-confirm 0.21em vs ~0.09em
against the fixed control.

---

## Rejected alternatives

| Option | Why rejected |
|---|---|
| Keep the router glyph as the app icon | Client wants the glyph retired entirely. |
| Stacked `Bio`/`Router` square | 2.6px cap at 16px — illegible. Superseded by the monogram. |
| One-line `BioRouter` squeezed into the square | 1.7px cap at 16px — the worst option measured. |
| Live `<text>` in the system stack | A different logo per OS; PNG ≠ SVG on Windows. |
| An embedded webfont | Violates D-06 (no webfont), and would not be the app's font anyway. |

---

## Related documentation

- [UI overhaul execution status](../ui-overhaul/execution-status.md) — records how the typeface
  blocker above was resolved, which values were re-tuned, and how the assets were shipped.
- [Wordmark studio](logo-wordmark-studio.html) — the interactive studio for tuning the wordmark's
  weight, tracking, underline gap and vertical position.
- [Icon centring studio](logo-icon-studio.html) — the interactive studio for the BR mark's size and
  placement inside the square icon.
- [Alma Mater theme tokens](../theming/alma-mater-theme-tokens.md) — the theme family that
  re-defines `--color-coral-700`, which is why this spec pins the literal hex.
