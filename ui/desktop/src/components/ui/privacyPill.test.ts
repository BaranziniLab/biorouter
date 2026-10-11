import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The privacy pills keep their box on a washed list row (WS-SKILLS-1). They rest on
 * `--background-muted`, the rest fill's own value, which a row's hover wash composites to; on a
 * washed `.biorouter-list-row` they take the neutral Badge's ground beside them, the page ground,
 * so one row is one ground (main.css, the rows block; WS-KNOWLEDGE-3). jsdom runs no cascade, so
 * this reads the sources; `groundsAndRows.browser.test.tsx` paints it in Chromium.
 */
const SRC = join(__dirname, '..', '..');
const read = (...p: string[]) => readFileSync(join(SRC, ...p), 'utf8');
const MAIN = read('styles', 'main.css').replace(/\/\*[\s\S]*?\*\//g, ' ');
const RULE = /\n([^{}\n]*\.br-privacy-pill)\s*\{([^}]*)\}/.exec(MAIN);

describe('the privacy pills on a washed row', () => {
  it('take the badge ground on a washed list row, by one rule', () => {
    expect(RULE, 'main.css has a .br-privacy-pill rule').toBeTruthy();
    expect(RULE![1]).toMatch(
      /^\.biorouter-list-row:is\(:hover, :focus-visible, :has\(:focus-visible\)\) \.br-privacy-pill$/
    );
    expect(RULE![2]).toMatch(/background-color:\s*var\(--br-badge-ground\);/);
  });

  it('read the ground the washed row names for its badges, which is the page ground', () => {
    const washed =
      /\.biorouter-list-row:is\(:hover, :focus-visible, :has\(:focus-visible\)\),\s*\.biorouter-settings-row:is\(:hover, :focus-visible, :has\(:focus-visible\)\)\s*\{([^}]*)\}/.exec(
        MAIN
      );
    expect(washed, 'the shared washed-row rule').toBeTruthy();
    expect(washed![1]).toMatch(/--br-badge-ground:\s*var\(--background-default\);/);
    expect(MAIN).toMatch(
      /\.br-badge-neutral\s*\{\s*background-color:\s*var\(--br-badge-ground, var\(--background-fill-strong\)\);\s*\}/
    );
  });

  it('leave a settings row alone, where Crew draws its one-piece privacy chip', () => {
    // Crew's workspace settings put "🔒 Private · UCSF" in a `.biorouter-settings-row`; the chip's
    // own box paints the muted token under the pill AND the institution, so stepping the pill
    // there would split the chip in two.
    const pillRules = MAIN.split('}').filter((block) => block.includes('.br-privacy-pill'));
    expect(pillRules.length).toBe(1);
    expect(pillRules[0]).not.toMatch(/settings-row/);
  });

  it('carry the hook and keep the muted rest in both badges', () => {
    expect(read('components', 'ui', 'PrivacyBadge.tsx')).toMatch(
      /'br-privacy-pill bg-background-muted'/
    );
    expect(read('components', 'ui', 'AffiliationBadge.tsx')).toMatch(
      /'br-privacy-pill bg-background-muted text-text-default'/
    );
  });

  it('use the two inks, which only gain on the page ground over the muted rest', () => {
    // Public is `--text-muted`, Private `--text-default`; both are held to 4.5:1 on the fills and
    // on `--background-medium` by the contrast guard, and the page ground is the far end of the
    // ladder from the ink in both modes.
    const badge = read('components', 'ui', 'PrivacyBadge.tsx');
    expect(badge).toMatch(/ink: 'text-text-default'/);
    expect(badge).toMatch(/ink: 'text-text-muted'/);
    const guard = read('..', 'scripts', 'check-contrast.mjs');
    expect(guard).toMatch(
      /for \(const fill of \['--background-fill', '--background-fill-strong'\]\) \{[\s\S]{0,200}'--text-default', fill, 4\.5[\s\S]{0,200}'--text-muted', fill, 4\.5/
    );
  });
});
