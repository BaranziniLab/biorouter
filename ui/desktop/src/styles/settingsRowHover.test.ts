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

/** The ONE rule the content row and the settings row share (spec 2.6). */
function baseRule(): string {
  const match = CSS.match(/\n\.biorouter-list-row,\s*\.biorouter-settings-row\s*\{([^}]*)\}/);
  expect(match, 'expected one rule shared by the list row and the settings row').toBeTruthy();
  return match![1];
}

function ruleFor(selector: RegExp): string {
  const match = CSS.match(new RegExp(`\\n${selector.source}\\s*\\{([^}]*)\\}`));
  expect(match, `expected a rule for ${selector.source}`).toBeTruthy();
  return match![1];
}

describe('the settings row hover wash', () => {
  it('sets `background-color`, never the `background` shorthand', () => {
    const rule = hoverRule();
    expect(rule).toContain('background-color:');
    expect(rule).not.toMatch(/(^|[\s;])background\s*:/);
  });

  it('paints the one row hover, --overlay-hover, the same as a list row (no 42% / 38% fork)', () => {
    expect(hoverRule()).toMatch(/background-color:\s*var\(--overlay-hover\);/);
    expect(ruleFor(/\.biorouter-list-row:hover,\s*\.biorouter-list-row:focus-within/)).toMatch(
      /background-color:\s*var\(--overlay-hover\);/
    );
    expect(CSS).not.toMatch(/var\(--background-medium\) (42|38)%/);
  });

  it('eases the hover over --dur-fast-min', () => {
    expect(baseRule()).toMatch(/background-color var\(--dur-fast-min\) var\(--ease-out\)/);
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
      /\.biorouter-list-row:last-child::after,\s*\.biorouter-settings-row:last-child::after\s*\{\s*content:\s*none;\s*\}/
    );
  });

  /**
   * A bottom BORDER on a rounded row curls up at both ends (plainly in forced colours, which
   * paint every border in the text colour). The divider is a straight line under the row, inset
   * by the corner radius, and the border stays only as 1px of transparent room so no row moves
   * (Crew's `dialogs/dialogs.css` recipe, QA Q4-25).
   */
  it('draws a straight hairline inset by the row radius, not a curling border', () => {
    expect(baseRule()).toMatch(/border-bottom:\s*1px solid transparent;/);
    const line = ruleFor(/\.biorouter-list-row::after,\s*\.biorouter-settings-row::after/);
    expect(line).toMatch(/inset-inline:\s*var\(--radius-md\);/);
    expect(line).toMatch(/bottom:\s*-1px;/);
    expect(line).toMatch(/border-top:\s*1px solid var\(--border-subtle\);/);
    expect(
      ruleFor(
        /\.biorouter-list-row:focus-within::after,\s*\.biorouter-settings-row:focus-within::after/
      )
    ).toMatch(/border-top-color:\s*var\(--border-focus\);/);
  });
});
