# Theming

This folder holds BioRouter's theme families — the named colour environments the desktop app
can be dressed in, selected independently of light/dark mode. It covers the colour tokens each
family overrides, the palettes they draw from, the accent decisions behind them, and the
verified WCAG contrast ratios for every value. Three families ship — Parchment, Alma Mater and
Roche Limit — and both non-default families are specified here in full, alongside the
architecture that generates every one of them from a single file per family.

Come here when you need the authoritative value for a colour token, when you are adding a
component that reads colour tokens and need to know what it will look like in every family, in
light and dark, or when you are proposing a new theme family. This folder is colour only — it
deliberately does not cover layout, typography or motion. For the parent design system and its
numbered decisions see [`design.md`](../../../design.md) at the repo root; for the app-wide
structural redesign work see [`ui-overhaul/`](../ui-overhaul/README.md); for the logo, wordmark
and brand assets see [`branding/`](../branding/README.md).

> **The one rule to know before reading anything here (2026-08-08).** A family varies in exactly
> two things: its **ink** and its **accent**. Backgrounds, greys, borders, focus surfaces,
> elevation and the scrim are **one shared set**, byte-identical across all three families in
> both modes — Roche Limit's values, adopted wholesale. Both token references below still contain
> tables of surface hexes; those tables now describe the shared set, not that family's own, and
> editing one moves all three families. If a family's ink fails on a shared ground, **retune the
> ink** — reintroducing a per-family grey is the failure mode the rule exists to prevent. The
> reasoning is
> [Theme system architecture §8](theme-system-architecture.md#8--shared-neutrals--one-scaffolding-three-inks).

## Documents

| Document | What it covers |
|---|---|
| [Theme system architecture](theme-system-architecture.md) | How the theme system is put together: where a theme actually lives, the single authored file per family, what is generated from it and what is derived and must never be authored, the token contract, **what varies per family and what is shared (§8)**, what guards it, and the measured cost of a fourth family. Read this before either token reference — it is the authority over any hex quoted in them. |
| [Alma Mater theme tokens](alma-mater-theme-tokens.md) | The colour-token reference for Alma Mater, BioRouter's UCSF-brand theme family: the UCSF palette it draws from, the accent decision, a complete token-by-token light/dark mapping, and the verified WCAG contrast ratios behind every value. Approved with the UCSF Teal accent, originally implemented 2026-07-10 and reconciled against the shipped code on 2026-07-18. **Revised 2026-08-08:** its cool blue-grey neutrals and navy dark surfaces were replaced by the shared set; navy ink and teal accent are untouched. |
| [Baseten theme](baseten-theme.md) | Baseten's brand applied as the fourth family and the default: green accent on the shared neutrals, Hanken Grotesk (Beausite Classic when installed) and Geist Mono, 2px corners. What the family owns, what `main.css` authors, and its known limits. |
| [Roche Limit theme tokens](roche-limit-theme.md) | The colour-token reference for Roche Limit, the JupyterLab-inspired family — white page, recessed grey panels, one bright orange doing all the signalling. Approved and implemented on 2026-07-18; §9 records what was actually built. **Its neutrals are now the shared set for all three families** (§9.3), so its surface tables are BioRouter-wide rather than family-specific. |

## Interactive studios

These are self-contained HTML pages that render themes on real BioRouter chrome. **They need a
browser to be useful** — open them locally; they are not readable as source.

| File | What it is |
|---|---|
| `theme-system-explorer.html` | "One interface. Four colour environments." — how the token layers and theme axes fit together across both shipped families, showing Parchment and Alma Mater in light and dark modes. |
| `alma-mater-theme-studio.html` | The Alma Mater studio: renders the token set on real BioRouter chrome in light and dark, and carries the accent picker used to compare options while the accent was being chosen. |
| `alma-mater-light-theme-studio.html` | A deeper exploration of Alma Mater's light variant (UCSF Teal). |
| `roche-limit-theme-studio.html` | The **Roche Limit** studio — a white page, recessed grey panels, and one bright orange doing all the signalling. Family id `roche-limit`, light + dark, 59 × 2 tokens, 164/164 contrast pairs at AA. The page still carries the *awaiting approval* framing it was written with; the family was approved and shipped on 2026-07-18, and [its token reference](roche-limit-theme.md) is the current authority. |

## Related documentation

- [Biorouter Design System](../../../design.md) — the parent design system that these themes
  re-colour, and the register of the numbered `D-NN` decisions the token reference cites.
- [UI overhaul execution status](../ui-overhaul/execution-status.md) — records the sweep that
  verified light + dark × Parchment + Alma Mater through the real persistence path.
- [BioRouter logo and wordmark specification](../branding/logo-and-wordmark-spec.md) — the brand
  assets that sit alongside these palettes, including their own colour tokens.
- [UI cohesion redesign](../ui-overhaul/ui-cohesion-redesign.md) — the structural counterpart:
  the markdown layer, preview panel, terminal and floating surfaces these tokens are applied to.
- [Design](../README.md) — the parent folder index, and the boot-splash studio that paints the
  brand mark before any of these tokens exist as runtime custom properties.
