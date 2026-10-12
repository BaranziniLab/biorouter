import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import * as themeTokens from '../../../scripts/lib/theme-tokens.mjs';

/**
 * The usage surfaces under fills before lines (owner message 13, R-27; design.md 9.11).
 *
 * The Home usage card is a subtle fill with no resting outline; the segmented range control inside
 * it rests one ground deeper (a fill never sits on a fill); the Settings report's two tables take
 * the same filled panel the chat's tables take. jsdom applies no stylesheet, so each claim is read
 * from the authored CSS at its source, and the one that is a measurement (the heatmap's empty day
 * on the new fill) is read from the stylesheet's own tokens in all six scopes, the way
 * `check-contrast.mjs` reads them. `usageCardGeometry.browser.test.tsx` measures the same card in
 * Chromium.
 */

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
  contrast(a: string, b: string): number;
};

const usageCss = readFileSync(join(__dirname, 'usage.css'), 'utf8');
const mainCss = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
const homeCard = readFileSync(join(__dirname, '../sessions/HomeUsageCard.tsx'), 'utf8');
const scopes = tokens.buildScopes(mainCss);

/** The declarations of the rule whose selector is exactly `selector` (comments are dropped). */
function rule(selector: string): Record<string, string> {
  const source = usageCss.replace(/\/\*[\s\S]*?\*\//g, '');
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = source.match(new RegExp(`(?:^|\\})\\s*${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `usage.css has a ${selector} rule`).toBeTruthy();
  return Object.fromEntries(
    match![1]
      .split(';')
      .map((decl) => decl.trim())
      .filter(Boolean)
      .map((decl) => {
        const at = decl.indexOf(':');
        return [decl.slice(0, at).trim(), decl.slice(at + 1).trim()];
      })
  );
}

describe('the Home usage card is a fill, not an outline', () => {
  const card = rule('.br-usage-card');

  it('rests on the fill with a transparent edge and the container radius', () => {
    expect(card['background-color']).toBe('var(--background-fill)');
    // The 1px edge stays, transparent: nothing moves, and forced colours draw it (9.11).
    expect(card.border).toBe('1px solid transparent');
    expect(card['border-radius']).toBe('var(--radius-container)');
    expect(Object.values(card).join(' ')).not.toMatch(/--border-|--background-card/);
  });

  it('keeps the hairline under the head, the tabs’ baseline, and no other line', () => {
    const head = rule('.br-usage-card-head');
    expect(head['border-block-end']).toBe('1px solid var(--border-subtle)');
    // One structural rule is every line the card draws: nothing else in the file is a border on
    // a card, a tile or a row.
    const borders = usageCss
      .replace(/\/\*[\s\S]*?\*\//g, '')
      .match(/border(?:-block-end|-block-start|-top|-bottom)?:\s*[^;]+;/g);
    expect(borders).toEqual([
      'border: 1px solid transparent;',
      'border-block-end: 1px solid var(--border-subtle);',
    ]);
  });

  it('rests the range control one ground deeper through the shared fill surface', () => {
    // A fill never sits on a fill: the segmented track must not be the card's own ground. That is
    // said once, by `.br-fill-surface` in the primitives block, and the card only opts in. Both
    // sections carry the class (the loaded card and its loading twin), so the track does not
    // change colour when the real card replaces the skeleton.
    const sections = homeCard.match(/<section[^>]*className="br-usage-card[^"]*"/g) ?? [];
    expect(sections).toHaveLength(2);
    for (const section of sections) expect(section).toMatch(/\bbr-fill-surface\b/);

    // No second statement of it here: a hand-written track rule would drift from the primitive.
    expect(usageCss.replace(/\/\*[\s\S]*?\*\//g, '')).not.toMatch(/\.br-segmented/);

    // And the primitive does what the card used to say by hand: its children's track is the
    // strong step, and the track reads the variable.
    const surface = mainCss.match(/\.br-fill-surface > \*,[^{]*\{([^}]*)\}/);
    expect(surface, 'main.css has the .br-fill-surface rule').toBeTruthy();
    expect(surface![1]).toMatch(/--br-track-ground:\s*var\(--background-fill-strong\)/);
    expect(mainCss).toMatch(
      /\n\.br-segmented \{[^}]*background:\s*var\(--br-track-ground, var\(--background-fill\)\)/
    );
  });

  it('gives the Settings report’s tables the filled panel and no frame', () => {
    const frame = rule('.br-usage-table-frame');
    expect(frame['background-color']).toBe('var(--background-fill)');
    expect(frame['border-radius']).toBe('var(--radius-element)');
    expect(frame.border).toBeUndefined();
  });
});

describe('an empty day reads on the card’s fill', () => {
  // `--heat-0` is a step off WHITE (1.14:1) and 1.06:1 on the fill in light. Light re-points it to
  // the page's own white inside the card, a punched-out cell; dark keeps the token, which is
  // already a step lighter than the dark fill.
  it('is re-pointed in light only', () => {
    const light = rule(':root:not(.dark) .br-usage-card');
    expect(light['--heat-0']).toBe('var(--background-default)');
    expect(usageCss).not.toMatch(/(?<!not\()\.dark[^{]*\.br-usage-card[^{]*\{[^}]*--heat-0/);
  });

  const cases = Object.entries(scopes);
  it('is measured in all six scopes', () => {
    expect(cases).toHaveLength(6);
  });

  it.each(cases)(
    '%s: the empty day, and the ramp above it, are steps off the fill',
    (name, scope) => {
      const dark = name.endsWith(':dark');
      const hex = (token: string) => {
        const value = tokens.resolveHex(token, scope);
        expect(value, `${name}: ${token} is a hex`).toBeTruthy();
        return value!;
      };
      const fill = hex('--background-fill');
      const empty = dark ? hex('--heat-0') : hex('--background-default');

      // The step the eye has to find with no line around it: 1.08 is the fill's own floor
      // (check-contrast), and dark is held to 1.2 as the heat ramp's dark floor is.
      expect(tokens.contrast(empty, fill)).toBeGreaterThanOrEqual(dark ? 1.2 : 1.08);
      // The first step stays a step above an empty day, and every step stays off the card.
      expect(tokens.contrast(hex('--heat-1'), empty)).toBeGreaterThanOrEqual(dark ? 1.2 : 1.1);
      for (const level of [1, 2, 3, 4]) {
        expect(tokens.contrast(hex(`--heat-${level}`), fill)).toBeGreaterThanOrEqual(1.15);
      }
    }
  );
});
