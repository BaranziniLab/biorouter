import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * `.biorouter-settings-row`'s hover wash, asserted at the SOURCE.
 *
 * ⚠ **The bug this guards is invisible to every render test, and the reason is
 * precedence rather than pixels.** `.biorouter-settings-row:hover` is
 * UNLAYERED — it sits outside `@layer utilities` — so it beats any Tailwind
 * utility on the same element whatever the specificity says. While it used the
 * `background` SHORTHAND it therefore reset `background-image` to `none` on
 * every row it touched, and `tint-selected`'s wash IS a background-image. The
 * consequence: pointing at a selected row erased its selection. That is the
 * same inversion `main.css` records above `.tint-selected.tint-interactive`
 * ("hovering a selected row visibly un-selects it"), arriving by a second
 * route, and it is why the composed pair could not be used on a settings row at
 * all.
 *
 * jsdom applies no stylesheet cascade and computes no layout, so a test that
 * mounts a row and hovers it reads the same value in both directions. The
 * declaration is the only thing assertable, so that is what is asserted.
 *
 * No settings row keeps a state-dependent wash in this PR (V1: the switch, the
 * radio or the checkbox states the state, never the row's fill). This rule
 * removes the trap that would make the *sanctioned* exception — a row where
 * selection genuinely is the only indicator — impossible to implement later.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');

function hoverRule(): string {
  const match = CSS.match(
    /\.biorouter-settings-row:not\(\.tint-selected\):hover,\s*\.biorouter-settings-row:not\(\.tint-selected\):focus-within\s*\{([^}]*)\}/
  );
  expect(match, 'expected the settings-row hover rule to exclude a tinted row').toBeTruthy();
  return match![1];
}

function baseRule(): string {
  const match = CSS.match(/\n\.biorouter-settings-row\s*\{([^}]*)\}/);
  expect(match, 'expected a `.biorouter-settings-row` base rule').toBeTruthy();
  return match![1];
}

describe('the settings row hover wash', () => {
  it('sets `background-color`, never the `background` shorthand', () => {
    const rule = hoverRule();
    expect(rule).toContain('background-color:');
    expect(rule).not.toMatch(/(^|[\s;])background\s*:/);
  });

  it('still paints the same 38% neutral wash', () => {
    expect(hoverRule()).toContain('color-mix(in srgb, var(--background-medium) 38%, transparent)');
  });

  /**
   * Chrome does not interpolate `background-image`, so the tint is carried by
   * the registered `--tint-ink` (see the note above `tint-interactive`). A row
   * that declares its own `transition` REPLACES the utility's fallback, so
   * omitting the property here gives a selection that snaps beside a hover that
   * eases.
   */
  it('names `--tint-ink` in the row’s own transition list', () => {
    expect(baseRule()).toContain('--tint-ink');
  });

  /**
   * The trailing hairline is suppressed by `:last-child` and by nothing else —
   * which only works while every row is a DIRECT child of its list. A section
   * that wraps its rows (a per-item box, a `space-y-*` group, a disclosure that
   * brings the panel it expands) breaks it in one of two directions: one row per
   * wrapper makes EVERY row `:last-child` and suppresses all the hairlines, and
   * several rows in one wrapper hides the wrapper's last hairline mid-list. Both
   * had shipped on the Chat tab. The Chat sections therefore contribute
   * fragments of rows rather than boxes, and the disclosure renders its two rows
   * as siblings.
   */
  it('suppresses the trailing hairline by `:last-child` alone', () => {
    expect(CSS).toMatch(
      /\.biorouter-settings-row:last-child\s*\{\s*border-bottom-color:\s*transparent;\s*\}/
    );
  });
});
