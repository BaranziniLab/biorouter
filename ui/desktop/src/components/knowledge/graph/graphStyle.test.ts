import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CANVAS_FONT_FALLBACK,
  CANVAS_INK_FALLBACK,
  CANVAS_MONO_FALLBACK,
  resolveComputed,
  resolveGraphTheme,
  withAlpha,
} from './graphStyle';
import { GRAPH_PALETTE } from '../../../styles/graphPalette';

function el(apply: (style: CSSStyleDeclaration) => void): HTMLDivElement {
  const node = document.createElement('div');
  apply(node.style);
  document.body.appendChild(node);
  return node;
}

/**
 * These used to be two describes over two functions — `resolveCanvasFontFamily`
 * and `resolveCanvasInk` — and the pair is exactly what §5.11 collapsed into one
 * `resolveComputed`. The reason is worth keeping in the test file, because the
 * old shape is what hid the bug: both resolvers were covered by a case named
 * "never hands a canvas an unresolved custom property", and only one of them
 * actually rejected `var(`. The font case passed because a computed
 * `font-family` happens never to be a `var()`, not because the code prevented
 * it. One function means one guard means one test that means what it says.
 */
describe('resolveComputed', () => {
  it('reads the value off the element rather than naming one', () => {
    // The graph paints its labels with `ctx.font`, which is parsed against the
    // canvas and NOT against the cascade — so `var(--font-body)` in that string
    // is invalid and the whole assignment is silently dropped. The bug this
    // replaces was the other way of getting a family into that string: a
    // hardcoded `ui-sans-serif` literal, which kept the graph on the OS face
    // after the app moved to Figtree.
    const node = el((s) => {
      s.fontFamily = 'Figtree, ui-sans-serif, sans-serif';
    });
    expect(resolveComputed(node, (s) => s.fontFamily, CANVAS_FONT_FALLBACK)).toContain('Figtree');
    node.remove();
  });

  it('never hands a canvas an unresolved custom property', () => {
    // `--text-default` is itself `var(--color-neutral-100)` in the dark blocks,
    // so reading the custom property by name — the tempting shortcut — yields a
    // reference the canvas silently drops, leaving the previous fill in place.
    const node = el((s) => {
      s.color = 'var(--text-default)';
    });
    const out = resolveComputed(node, (s) => s.color, CANVAS_INK_FALLBACK);
    expect(out).not.toContain('var(');
    expect(out).toBe(CANVAS_INK_FALLBACK);
    node.remove();
  });

  it('falls back rather than emitting an empty value', () => {
    // An empty string in `ctx.font` or `ctx.fillStyle` is a no-op that leaves
    // the canvas default in place, so "" must never be returned.
    expect(resolveComputed(null, (s) => s.fontFamily, CANVAS_FONT_FALLBACK)).toBe(
      CANVAS_FONT_FALLBACK
    );
    expect(resolveComputed(undefined, (s) => s.color, CANVAS_INK_FALLBACK)).toBe(
      CANVAS_INK_FALLBACK
    );
    expect(
      resolveComputed(document.createElement('div'), (s) => s.fontFamily, CANVAS_FONT_FALLBACK)
    ).not.toBe('');
  });
});

/**
 * The same trap, one property over, and the reason the ink is resolved at all:
 * `ctx.fillStyle = '#1f242c'` was a hardcoded near-black drawn onto a near-black
 * canvas in every dark theme, so the graph's labels vanished. jsdom has no
 * layout engine and no themes, so nothing here can prove the graph *looks*
 * right — what these pin is that every field is READ from an element, and that a
 * value the canvas cannot parse never reaches it.
 */
describe('resolveGraphTheme', () => {
  it('reads the ink off the container rather than naming one', () => {
    // A light ink — i.e. what a dark theme resolves to. The old literal could
    // only ever return the dark one.
    const container = el((s) => {
      s.color = 'rgb(244, 240, 230)';
    });
    expect(resolveGraphTheme(container, {}, 'dark').ink).toBe('rgb(244, 240, 230)');
    container.remove();
  });

  it('resolves every field from its own probe, not from the container', () => {
    const container = el((s) => {
      s.color = 'rgb(31, 36, 44)';
      s.fontFamily = 'Figtree, sans-serif';
    });
    const mono = el((s) => {
      s.fontFamily = 'Menlo, monospace';
    });
    const danger = el((s) => {
      s.color = 'rgb(179, 38, 30)';
    });
    const muted = el((s) => {
      s.color = 'rgb(120, 120, 120)';
    });
    const border = el((s) => {
      s.borderTopColor = 'rgb(200, 200, 200)';
      s.borderTopStyle = 'solid';
      s.borderTopWidth = '1px';
    });

    const theme = resolveGraphTheme(container, { mono, danger, muted, border }, 'light');

    expect(theme.fontFamily).toContain('Figtree');
    expect(theme.monoFamily).toContain('Menlo');
    expect(theme.ink).toBe('rgb(31, 36, 44)');
    expect(theme.danger).toBe('rgb(179, 38, 30)');
    expect(theme.muted).toBe('rgb(120, 120, 120)');
    expect(theme.border).toBe('rgb(200, 200, 200)');
    expect(theme.mode).toBe('light');

    [container, mono, danger, muted, border].forEach((n) => n.remove());
  });

  it('falls back per field, and never to an empty string', () => {
    const theme = resolveGraphTheme(null, {}, 'light');
    expect(theme.fontFamily).toBe(CANVAS_FONT_FALLBACK);
    expect(theme.monoFamily).toBe(CANVAS_MONO_FALLBACK);
    expect(theme.ink).toBe(CANVAS_INK_FALLBACK);
    expect(theme.muted).toBe(CANVAS_INK_FALLBACK);
    expect(theme.border).toBe(CANVAS_INK_FALLBACK);
  });

  it('returns a null danger rather than substituting one family’s red', () => {
    // The status hues stay per family — Parchment `#b3261e`, Alma Mater
    // `#c40d3e`, Roche Limit `#c4232b` — so a shared fallback constant would
    // paint one family's red inside the other two, on the negative-edge stroke
    // and the strike-through, where a wrong red is a wrong MEANING. The painters
    // skip the treatment instead.
    expect(resolveGraphTheme(null, {}, 'light').danger).toBeNull();

    // And a probe that is present but resolves to a custom property is the same
    // state: `--text-danger` is itself a `var()` in some blocks, and a canvas
    // cannot parse one. It must become `null`, not the unresolvable string.
    const unresolved = el((s) => {
      s.color = 'var(--text-danger)';
    });
    expect(resolveGraphTheme(null, { danger: unresolved }, 'dark').danger).toBeNull();
    unresolved.remove();
  });

  it('takes the ground per mode from the palette, never a single light hex', () => {
    // A single light value for a dual-mode quantity is how the boot mark once
    // came to measure 1.02:1 on every dark splash.
    expect(resolveGraphTheme(null, {}, 'light').ground).toBe(GRAPH_PALETTE.light.ground);
    expect(resolveGraphTheme(null, {}, 'dark').ground).toBe(GRAPH_PALETTE.dark.ground);
  });

  it('treats a transparent container background as no ground at all', () => {
    // `getComputedStyle(el).backgroundColor` returns `rgba(0, 0, 0, 0)` when the
    // colour lives in a `background-image`, which is one of the four reasons
    // §4.5 deletes the three-layer wash this container used to paint. A
    // transparent ground is not a ground — the palette's is used instead.
    const container = el((s) => {
      s.backgroundColor = 'rgba(0, 0, 0, 0)';
    });
    expect(resolveGraphTheme(container, {}, 'dark').ground).toBe(GRAPH_PALETTE.dark.ground);
    container.remove();
  });
});

describe('withAlpha', () => {
  it('re-alphas a resolved rgb(), which is what getComputedStyle returns', () => {
    expect(withAlpha('rgb(244, 240, 230)', 0.5)).toBe('rgba(244, 240, 230, 0.5)');
    expect(withAlpha('rgba(42, 37, 32, 0.9)', 0.5)).toBe('rgba(42, 37, 32, 0.5)');
  });

  it('handles the hex fallback constant', () => {
    expect(withAlpha('#1f242c', 0.5)).toBe('rgba(31, 36, 44, 0.5)');
    expect(withAlpha('#fff', 0.25)).toBe('rgba(255, 255, 255, 0.25)');
  });

  it('handles a palette hex, which is where every node fill comes from', () => {
    expect(withAlpha(GRAPH_PALETTE.light.types.Gene, 1)).toBe('rgba(160, 178, 255, 1)');
  });

  it('returns an unrecognised colour unchanged rather than mangling it', () => {
    // An invalid `strokeStyle` is a silent no-op that leaves the PREVIOUS
    // stroke in place — the node ring would then inherit a neighbour's colour.
    expect(withAlpha('canvastext', 0.5)).toBe('canvastext');
  });
});

/**
 * Spec 2.1: one UI face (the website's Arial-first stack) and one mono stack. The
 * canvas resolves both off the cascade; its fallbacks are what it draws when it
 * cannot (jsdom, a detached node), and a third stack there is how the graph used
 * to label nodes in a face nothing else in the app used.
 */
describe('the canvas font fallbacks', () => {
  const css = readFileSync(join(__dirname, '../../../styles/main.css'), 'utf8');
  const stack = (token: string) =>
    css
      .match(new RegExp(`${token}:\\s*([^;]+);`))?.[1]
      .replace(/\s+/g, ' ')
      .trim();

  it('is the body stack, verbatim', () => {
    expect(CANVAS_FONT_FALLBACK).toBe(stack('--font-body'));
  });

  it('is the mono stack, verbatim', () => {
    expect(CANVAS_MONO_FALLBACK).toBe(stack('--font-mono'));
  });
});
