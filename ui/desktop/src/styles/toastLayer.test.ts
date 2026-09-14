import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { TOAST_SURFACE_CLASS_NAME } from '../components/alerts/NotificationSurface';

/**
 * Where notifications appear, and in what order they pile up.
 *
 * Both facts were reported as bugs and both are invisible to every other suite
 * in this repo, for the same reason `measures.test.ts` exists: jsdom has no
 * layout engine and never runs Tailwind, so a toast rendered in a test lands at
 * the same (non-)coordinates whatever the stylesheet says. The only assertable
 * artefacts are the declaration in main.css and the props on the container, so
 * those are what is asserted, at the source.
 *
 * The two rules, in the user's words:
 *   1. a notification always appears in the UPPER-RIGHT CORNER;
 *   2. a second, newer notification is pushed DIRECTLY BELOW the first, so the
 *      oldest sits nearest the corner and the stack grows downward.
 *
 * ⚠ Rule 2 CONTRADICTS docs/design/astryx-adoption/astryx-ui-adoption-design.md
 * §3.7 ("newest nearest the corner"). The user's instruction is what shipped;
 * the doc is what needs reconciling. Anyone arriving here from §3.7 to "fix"
 * `newestOnTop` should change the doc instead.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const APP = readFileSync(join(__dirname, '../App.tsx'), 'utf8');

function declaration(name: string): string {
  const match = CSS.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1].trim();
}

/**
 * Resolve a token whose value is a `calc()` over pixel literals and other
 * pixel-valued tokens. Enough for the inset ladder and nothing more — the point
 * is to assert the NUMBER the corner ends up at, not to reimplement CSS.
 */
function resolvePx(name: string): number {
  const raw = declaration(name).replace(
    /var\(\s*--([a-z-]+)\s*\)/g,
    (_, ref: string) => `${resolvePx(ref)}px`
  );
  const arithmetic = raw
    .replace(/^calc\(/, '')
    .replace(/\)$/, '')
    .replace(/px\b/g, '');
  // Digits, whitespace and the four operators, and nothing else — the guard is
  // what makes evaluating the string safe, so it is deliberately total rather
  // than a "looks numeric" heuristic.
  if (!/^[\d\s.+\-*/]+$/.test(arithmetic)) {
    throw new Error(`--${name} is not a pixel expression: ${raw}`);
  }
  return Number(new Function(`return ${arithmetic};`)());
}

describe('the toast layer sits in the upper-right corner', () => {
  /**
   * The regression, exactly. `--toast-inset-top` was 144px — derived from the
   * tallest page header so a toast could never cover one — which docked the
   * layer in the middle of the content pane instead of the corner. The
   * extension-load toast floating halfway down the chat is what was reported.
   *
   * 64px is a ceiling, not a target: it is the titlebar band with room for one
   * gap and a little slack, and anything past it has stopped being a corner.
   */
  it('starts within the top-corner band, not below the page header', () => {
    expect(resolvePx('toast-inset-top')).toBeLessThanOrEqual(64);
  });

  /**
   * The floor. Electron folds `-webkit-app-region: drag` rects in DOM order,
   * and `.titlebar-drag-region` lives inside App's main tree — LATER than the
   * toast container that sits above it — so a drag rect overlapping the toast
   * eats clicks on its × and its "View details" no matter what `--z-toast`
   * says (issue #74). A toast in that band looks present and is dead.
   */
  it('clears the titlebar drag band it is measured from', () => {
    expect(resolvePx('toast-inset-top')).toBeGreaterThanOrEqual(resolvePx('titlebar-drag-height'));
    expect(declaration('toast-inset-top')).toContain('var(--titlebar-drag-height)');
  });

  /**
   * One number, one place. The drag band and the inset measured from it were
   * two independent literals; asserting the token here is what stops a change
   * to the titlebar's height from silently moving the toast layer into it.
   */
  it('reads the titlebar height from the same token the drag region does', () => {
    expect(CSS).toMatch(/\.titlebar-drag-region\s*\{[^}]*height:\s*var\(--titlebar-drag-height\)/);
  });

  it('docks the container top-right and takes its inset from the token', () => {
    expect(APP).toContain('position="top-right"');
    expect(APP).toContain("top: 'var(--toast-inset-top)'");
  });
});

describe('a newer notification lands directly below the older one', () => {
  /**
   * Spelled out rather than left to react-toastify, whose default happens to
   * agree today: without the prop the behaviour is correct BY ACCIDENT, and the
   * next reader following the design doc's "newest nearest the corner" would
   * add `newestOnTop` and undo a decision nothing was defending.
   */
  it('pins newestOnTop to false on the container', () => {
    expect(APP).toMatch(/newestOnTop=\{false\}/);
  });

  /**
   * §3.7's 12px gap between stacked toasts. It lives on the card, not the
   * container, because the container is a plain flex column — and it survives
   * react-toastify's own 16px `margin-bottom` only because `toastClassName` is
   * passed as a FUNCTION, which replaces the library's `Toastify__toast` class
   * list outright instead of appending to it. Drop the function form and the
   * vendor's unlayered rule wins over this Tailwind utility.
   */
  it('keeps the 12px stack gap on the toast card', () => {
    expect(TOAST_SURFACE_CLASS_NAME.split(/\s+/)).toContain('mb-3');
    expect(APP).toContain('toastClassName={() => TOAST_SURFACE_CLASS_NAME}');
  });
});

/**
 * Defect D3b (2026-09-13). Radix's modal sets `pointer-events: none` on <body>
 * while a dialog is open, and the toast layer inherited it: a toast drawn above
 * the dialog could not be clicked, and the press fell through to the backdrop
 * and closed the dialog. jsdom applies no stylesheet, so this is asserted at the
 * source, like everything else here; `dialog.test.tsx` covers the half that
 * decides whether a press on a toast dismisses the dialog.
 */
describe('a toast stays clickable over a modal dialog', () => {
  it('gives every toast card pointer events back', () => {
    expect(CSS).toMatch(/\.Toastify__toast-container\s*>\s*\*\s*\{[^}]*pointer-events:\s*auto\s*;/);
  });
});
