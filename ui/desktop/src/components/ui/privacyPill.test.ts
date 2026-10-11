import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The privacy pills keep their box on a washed list row (WS-SKILLS-1). They rest on
 * `--background-muted`, the rest fill's own value, which a row's hover wash composites to; on a
 * washed `.biorouter-list-row` they step to the strong fill (main.css, the rows block). jsdom runs
 * no cascade, so this reads the sources; `groundsAndRows.browser.test.tsx` paints it in Chromium.
 */
const SRC = join(__dirname, '..', '..');
const read = (...p: string[]) => readFileSync(join(SRC, ...p), 'utf8');
const MAIN = read('styles', 'main.css').replace(/\/\*[\s\S]*?\*\//g, ' ');
const RULE = /\n([^{}\n]*\.br-privacy-pill)\s*\{([^}]*)\}/.exec(MAIN);

describe('the privacy pills on a washed row', () => {
  it('step to the strong fill on a washed list row, by one rule', () => {
    expect(RULE, 'main.css has a .br-privacy-pill rule').toBeTruthy();
    expect(RULE![1]).toMatch(
      /^\.biorouter-list-row:is\(:hover, :focus-visible, :has\(:focus-visible\)\) \.br-privacy-pill$/
    );
    expect(RULE![2]).toMatch(/background-color:\s*var\(--background-fill-strong\);/);
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

  it('use inks the contrast guard holds to 4.5:1 on the strong fill', () => {
    const guard = read('..', 'scripts', 'check-contrast.mjs');
    expect(guard).toMatch(
      /for \(const fill of \['--background-fill', '--background-fill-strong'\]\) \{[\s\S]{0,200}'--text-default', fill, 4\.5[\s\S]{0,200}'--text-muted', fill, 4\.5/
    );
  });
});
