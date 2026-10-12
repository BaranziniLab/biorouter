# Knowledge UI redesign

> **What this is.** The design-only proposal for rebuilding the Knowledge section's interface —
> the shell, the graph canvas, the legend, the filters and how they degrade, the sources rail, the
> seven pop-up surfaces, and a responsive ladder built on container queries. It amends
> [`../okf-migration/ui-spec.md`](../okf-migration/ui-spec.md), the binding spec the current UI was
> built from, in eight named places.
> **Status:** **Implemented** on `design/knowledge-ui-redesign`. §10 of the specification records
> the six places where measurement changed the design during the build; those corrections are
> authoritative over the records above them.
> **Audience:** Contributors working on the Knowledge subsystem and on the desktop design system.

> **Superseded in part (2026-10).** The Codex simplicity redesign (section 3.12 of
> [its implementation spec](../../design/codex-simplicity-redesign/implementation-spec.md)) merged
> the title and subject bands into one 44px `PageHeader` band, moved the tier control's trigger
> into that band as a badge-button (its confirmation is unchanged), flattened the Sources rail and
> graph into flush columns, made the facet strip 44px, and retired the height step. The container
> ladder (860/940/1140/1400) is unchanged.

Read the specification first, then open the studio beside it — the studio draws every surface the
specification describes, at BioRouter's real token values, in both light and dark, so a disagreement
about a decision can be settled by looking at it rather than by imagining it.

## Documents

| Document | What it covers |
|---|---|
| [Redesign specification](redesign-spec.md) | The ten decision records (`R-01`…`R-10`), the measurements each rests on, the file-by-file change map, the verification plan, the eight amendments to the OKF migration UI spec, and — in §10 — the six places where measurement changed the design during the build. |
| [Redesign studio](knowledge-redesign-studio.html) | The rendered companion; open it in a browser, no server needed. It renders **one page markup at four real pane sizes** (760, 946, 1040, 1626 px) laid out by the very container queries the spec calls for; a **live pop-up demo** where six of the seven surfaces open on click; a live SVG of the all-circle canvas; the old and new palettes side by side; and before/after pairs for the filter bar, the legend, the sources rail and the manager. Has its own light/dark toggle. |

## The three findings worth knowing first

- **The section responds to the wrong thing.** Its one breakpoint, `md:` at 930px, is a *viewport*
  media query — but what changes size is the *pane*. `minWidth: 1048` is derived in `main.ts` as the
  sidebar's 288px default plus a 760px column, so at the app's own minimum window with the sidebar
  open the pane is **760px** while the viewport is 1048: the breakpoint fires, lays out two columns, and the
  300px Sources rail leaves 444px for a filter strip that needs 757. **The section is at its most
  broken at the smallest size the app allows.** `R-08` replaces the ladder with `@container` queries
  on the pane, with a floor that fits at 760.

- **The node colours read dark because they are solved to WCAG *text* contrast rungs** — 3.5, 4.5,
  5.8, 7.3, and a Provenance ladder to 12.0:1 — applied to a *mark*. Seven of the 28 fills sit at
  ≥ 7:1. BioOKF instead puts a near-black hairline around every circle and lets the fill be light.
  The fix is two numbers: ring alpha 0.50 → 0.85, and a lightness band in place of the rungs.
  Within-family separation is *verified* to survive, at ΔE00 4.80 against a guard floor of 3.0.

- **All-circle nodes cost a measured accessibility channel, and the replacement is a better one.**
  The seven silhouettes carried node family precisely because cross-family colour distance under
  simulated dichromacy bottoms out at ΔE00 **0.00**. `R-04` delivers the circles and rebuilds what it
  can (hollow provenance marks at a thinned 1.7px ring, always-on haloed labels, an interactive
  legend). It originally kept the silhouettes behind an opt-in preference and said the design should
  not ship without it; the operator withdrew that hedge and the channel was removed outright.
  What closes the gap instead is §5.12's keyboard model, now built: the canvas is one tab stop with
  arrow-key traversal and a live region that speaks `Multiple sclerosis, Disease, Clinical` on every
  focus change. That covers strictly more people — a silhouette served a sighted viewer with
  dichromacy and nobody else, and only while the mark was large enough to resolve. The residual cost
  stands and is not argued away: ΔE00 0.00 means `Phenotype` and `Food` are one colour under
  tritanopia, told apart by label, hover, legend and inspector — all deliberate acts, where a
  silhouette was passive.

## Two reversals, kept on the record

The shape language went wrong twice before it went right, and both corrections are recorded in `R-02`
rather than quietly edited out:

1. Revision 1 made the filter controls **full pills**, to separate them from buttons — by introducing
   a roundness the app uses nowhere else.
2. Revision 2 over-corrected to a **4px radius**, which made a 32px control read squarer than
   everything around it, and drew the toggle as a squared rectangle.

The answer is neither. Every control sits on the app's own ladder — the switch is `ui/switch.tsx`'s
40 × 24 `rounded-full` track with a thumb that grows 16 → 20px, the filter is `--radius-element`
(8px) like every button and input, chips and swatches are `--radius-inner` (4px), and cards are
`--radius-container` (12px). Filters are differentiated by **edge weight and ground**, not by shape.

## Related documentation

- [Knowledge section — binding UI specification](../okf-migration/ui-spec.md) — what this amends.
- [OKF migration](../okf-migration/README.md) — the `DR-n` decision records upstream of this work.
- [Knowledge base](../README.md) — the subsystem index.
- [`design.md`](../../../design.md) — the desktop design system this proposal is bound by.
- [BioOKF](https://github.com/Broccolito/BioOKF) — the external reference the graph treatment draws
  on: its palette, its all-circle marks, its haloed labels and its grouped legend.
