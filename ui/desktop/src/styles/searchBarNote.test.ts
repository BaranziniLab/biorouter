import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The search bar's open/close reveal is a `max-height` transition under
 * `overflow: hidden`, so the ceiling is also a CLIP: a bar taller than
 * `--search-bar-height` is cut off, not scrolled.
 *
 * When the bar is telling the user that the term is too short to search, it is
 * two rows rather than one, and the one-row ceiling cut that sentence through
 * its middle — measured in the running app on Settings → Extensions before this
 * rule existed. `SearchBar` adds `search-bar-has-note` for exactly that state
 * (asserted in `SearchBar.test.tsx`, which can read the class but not the
 * layout) and this is the stylesheet's half: the class has to buy real room.
 *
 * ⚠ **Asserted at the source, and it has to be.** jsdom has no layout engine and
 * never loads this file, so a component test that reads `maxHeight` sees the
 * empty string whether the rule exists or not.
 */
const CSS = readFileSync(join(__dirname, 'search.css'), 'utf8');

/** The `--search-bar-height` a selector sets, in px. */
function ceiling(selector: RegExp): number {
  const rule = CSS.match(new RegExp(`${selector.source}[^}]*\\}`));
  expect(rule, `no rule matching ${selector}`).not.toBeNull();
  const value = rule![0].match(/--search-bar-height:\s*(\d+)px/);
  expect(value, `no --search-bar-height in ${rule![0]}`).not.toBeNull();
  return Number(value![1]);
}

describe('the search bar reveal ceiling', () => {
  it('gives the two-row bar more room than the one-row bar', () => {
    const oneRow = ceiling(/\.search-bar-enter,\s*\n?\s*\.search-bar-exit\s*\{/);
    const withNote = ceiling(
      /\.search-bar-enter\.search-bar-has-note,\s*\n?\s*\.search-bar-exit\.search-bar-has-note\s*\{/
    );

    // The note row measured 33px in the app (py-2 + a line + its hairline), and
    // it wraps to two lines in a narrow window.
    expect(withNote).toBeGreaterThanOrEqual(oneRow + 33);
  });

  it('still collapses to nothing on the way out', () => {
    // The exit animates to 0 whatever the ceiling is; a `has-note` bar that
    // raised the floor instead of the ceiling would never close.
    expect(CSS).toMatch(/\.search-bar-exit\s*\{\s*max-height:\s*0;/);
  });
});
