// ui/desktop/src/components/knowledge/graph/graphStyle.ts
import { GRAPH_PALETTE } from '../../../styles/graphPalette';
import type { GraphMode } from '../../../styles/graphPalette';

/**
 * ONE resolver, and that is a requirement rather than a style note (ui-spec §5.11).
 *
 * A canvas cannot parse `var(--…)`: `ctx.font` and `ctx.fillStyle` are parsed
 * against the canvas element, not the cascade, so the assignment is silently
 * DROPPED and the previous value stays. Reading the custom property by name does
 * not help either — `getPropertyValue('--text-default')` returns the *declared*
 * value, which in the dark blocks is itself `var(--color-neutral-100)`, another
 * reference the canvas cannot resolve. **Only the used value is safe.**
 *
 * ⚠ **The two guards below are the whole point, and this file has already been
 * burned by writing them twice.** Before this rewrite there were two resolvers:
 * `resolveCanvasInk` rejected `var(`, `resolveCanvasFontFamily` did not — and
 * BOTH were covered by a test named "never hands a canvas an unresolved custom
 * property". One of them was not actually guarded; the case passed because a
 * computed `fontFamily` happens never to be a `var()`, not because the code
 * prevented it. Two resolvers had already diverged, and §5.11 adds five more
 * fields. So there is exactly one function, and every field goes through it.
 */
export function resolveComputed(
  el: Element | null | undefined,
  read: (style: CSSStyleDeclaration) => string,
  fallback: string
): string {
  if (!el || typeof window === 'undefined' || typeof window.getComputedStyle !== 'function') {
    return fallback;
  }
  const value = read(window.getComputedStyle(el));
  return value && value.trim().length > 0 && !value.includes('var(') ? value : fallback;
}

/**
 * The family a `ctx.font` shorthand must name so the graph's labels are drawn in
 * the app's face.
 *
 * The fix is to RESOLVE the family rather than to hardcode a second literal: the
 * graph container inherits `font-family: var(--font-sans)` from `body`, so its
 * computed style already holds the resolved stack. Hardcoding `'Figtree', …`
 * here would work today and silently rot the first time the token moves.
 *
 * `fallback` covers jsdom, where `getComputedStyle` reports no family at all, and
 * it is the app's body stack verbatim (`--font-body`, the website's Arial-first
 * `--font`; spec 2.1), so even the fallback draws the one UI face rather than a
 * third stack. `graphStyle.test.ts` pins both fallbacks to `main.css`.
 */
export const CANVAS_FONT_FALLBACK =
  "Arial, 'Helvetica Neue', Helvetica, ui-sans-serif, -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif";

/** The mono stack, resolved off a `font-mono` probe. Edge labels are machine tokens. */
export const CANVAS_MONO_FALLBACK =
  "ui-monospace, 'SF Mono', SFMono-Regular, 'Cascadia Mono', Menlo, Consolas, 'Liberation Mono', monospace";

/**
 * The ink every canvas glyph, outline, edge and grid dot is drawn in.
 *
 * Same trap as the family, and it bit the same file: the label ink was a
 * hardcoded `#1f242c` — a near-black that is correct in a light theme and
 * INVISIBLE in every dark one, where the canvas ground is near-black too. The
 * panel became a set of unlabelled coloured dots.
 */
export const CANVAS_INK_FALLBACK = '#1f242c';

/**
 * ⚠ **There is deliberately NO danger fallback constant, and adding one is a
 * regression.**
 *
 * The obvious candidate — a single light hex — is one FAMILY's value. Parchment
 * light `--text-danger` is `#b3261e`, Alma Mater's is `#c40d3e`, Roche Limit's is
 * `#c4232b`, and the three dark values are a different family again. The
 * theme-system architecture is explicit that the status hues stay per family, so
 * a shared constant here would paint one family's red inside the other two —
 * on §5.7's negative-edge stroke and §5.8's strike-through, which are exactly
 * the two places a wrong red is a wrong MEANING and not merely a wrong tint.
 *
 * `resolveGraphTheme` therefore returns `danger: null` when the probe does not
 * resolve, and the painters skip the treatment rather than substituting. jsdom
 * resolves nothing, so a fallback would make the test pass and ship the bug.
 */
export interface GraphTheme {
  fontFamily: string;
  monoFamily: string;
  ink: string;
  ground: string;
  /** `null` when the probe did not resolve — see the note above. Never defaulted. */
  danger: string | null;
  muted: string;
  border: string;
  mode: GraphMode;
}

/**
 * The four 0×0 probe elements §5.11's table resolves the non-inherited fields from.
 *
 * Every field is OPTIONAL, and that is the same argument `CanvasThemeProbeRefs`
 * makes: a probe ref is `null` on the first render of every caller, so "this
 * probe is not here" is a state the resolver already has to handle, and it
 * handles it by falling back per field. A required field would only move the
 * failure from a fallback to a crash.
 */
export interface GraphThemeProbes {
  mono?: Element | null;
  danger?: Element | null;
  muted?: Element | null;
  border?: Element | null;
}

/**
 * Every field in one pass, so a caller cannot resolve six and forget the seventh.
 *
 * `ground` falls back to `GRAPH_PALETTE[mode].ground` — the value the theme
 * generator resolves and emits, and the exact surface all 28 fills and 7 ring
 * hues were solved against. No hex is restated and the fallback is per-mode,
 * which is the whole point: a single light value for a dual-mode quantity is how
 * the boot mark once came to measure 1.02:1 on every dark splash.
 */
export function resolveGraphTheme(
  container: Element | null | undefined,
  probes: GraphThemeProbes,
  mode: GraphMode
): GraphTheme {
  const ink = resolveComputed(container, (s) => s.color, CANVAS_INK_FALLBACK);
  return {
    fontFamily: resolveComputed(container, (s) => s.fontFamily, CANVAS_FONT_FALLBACK),
    monoFamily: resolveComputed(probes.mono, (s) => s.fontFamily, CANVAS_MONO_FALLBACK),
    ink,
    // ⚠ Resolving this AT ALL requires the container to have no
    // `background-image`: `getComputedStyle(el).backgroundColor` returns
    // `rgba(0, 0, 0, 0)` when the colour lives in a gradient. That is one of the
    // four reasons §4.5 deletes the three-layer wash this container used to paint.
    ground: resolveComputed(
      container,
      (s) => {
        const bg = s.backgroundColor;
        // A transparent ground is not a ground. Fall through to the palette's.
        return !bg || bg === 'transparent' || /,\s*0\s*\)$/.test(bg) ? '' : bg;
      },
      GRAPH_PALETTE[mode].ground
    ),
    danger: resolveComputed(probes.danger, (s) => s.color, '') || null,
    muted: resolveComputed(probes.muted, (s) => s.color, ink),
    border: resolveComputed(probes.border, (s) => s.borderTopColor, ink),
    mode,
  };
}

/**
 * The node outline alpha, when nothing in the density ladder is fading it.
 *
 * ⚠ **This constant was DEAD and its value was a lie.** It read `0.5` while the
 * painter hardcoded `0.92` in two places, so the number a reader would have
 * trusted was never the number on screen. It is now the single source and set
 * to the value that actually ships.
 *
 * ⚠ **The ring is what frees the fill, so this is load-bearing** (R-05).
 * Composited over the light ground, ink at 0.92 measures **10.88:1** — that
 * ring, not the fill, is what satisfies WCAG 1.4.11 for the graphical object's
 * boundary. It is precisely because the boundary carries the contrast that the
 * 28 fills could move off text-contrast rungs onto a light lightness band.
 * Lowering this without re-solving the fills would take the legibility away
 * from both channels at once.
 */
export const NODE_RING_ALPHA = 0.92;

/**
 * `color` with `alpha` substituted, for a canvas.
 *
 * `getComputedStyle().color` is always a resolved `rgb()`/`rgba()` in a real
 * engine, so the parse below is the whole job; the hex branch covers the
 * fallback constants and the palette's own hexes. Anything unrecognised is
 * returned UNCHANGED rather than mangled into an invalid colour, which a canvas
 * would silently ignore — leaving the previous fill in place.
 */
export function withAlpha(color: string, alpha: number): string {
  const rgb = /^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/i.exec(color);
  if (rgb) {
    return `rgba(${rgb[1]}, ${rgb[2]}, ${rgb[3]}, ${alpha})`;
  }
  const hex = /^#([0-9a-f]{3}|[0-9a-f]{6})$/i.exec(color.trim());
  if (hex) {
    const h = hex[1];
    const full =
      h.length === 3
        ? h
            .split('')
            .map((c) => c + c)
            .join('')
        : h;
    const n = parseInt(full, 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
  }
  return color;
}
