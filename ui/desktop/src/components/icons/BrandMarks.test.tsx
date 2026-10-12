import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from '../../styles/themes.generated';

const here = dirname(fileURLToPath(import.meta.url));
const read = (p: string) => readFileSync(join(here, p), 'utf-8');

/**
 * The two brand components take DELIBERATELY OPPOSITE approaches to theming,
 * and both directions have been broken before:
 *
 *   BioRouterMark      follows the family. It used to hold fixed constants
 *                      while the boot splash was family-aware, so Roche Limit
 *                      flashed an orange monogram and then hydrated a Parchment
 *                      coral/navy one.
 *   BioRouterWordmark  does NOT follow the family. It is the brand lockup and
 *                      stays in the brand's colours whichever palette the app
 *                      wears — the same way a logo does on a coloured page.
 *
 * Without these tests the pair reads as an inconsistency, and someone
 * "fixes" whichever one they noticed second. That is exactly how the mark and
 * the splash drifted apart the first time.
 */
describe('brand marks and theming', () => {
  describe('BioRouterMark — follows the family', () => {
    const src = read('BioRouterMark.tsx');

    it('reads its inks from the generated theme data, not from constants', () => {
      expect(src).toContain('GENERATED_THEMES');
      expect(src).toMatch(/mark\.navy/);
      expect(src).toMatch(/mark\.coral/);
      // The old fixed constants must not creep back.
      expect(src).not.toMatch(/^const NAVY = /m);
      expect(src).not.toMatch(/^const CORAL = /m);
    });

    it('has a distinct ink set for every family, so the splash cannot disagree', () => {
      // The boot splash paints --br-navy/--br-coral from the SAME definition
      // field. If a family were missing one, the splash would silently inherit
      // the base family's — which is precisely the regression this guards.
      for (const id of THEME_FAMILY_IDS) {
        for (const mode of ['light', 'dark'] as const) {
          const { navy, coral } = GENERATED_THEMES[id][mode].mark;
          expect(navy, `${id}.${mode} mark.navy`).toMatch(/^#[0-9a-f]{6}$/);
          expect(coral, `${id}.${mode} mark.coral`).toMatch(/^#[0-9a-f]{6}$/);
        }
      }
    });

    it('lifts the primary ink on dark, so the mark never vanishes', () => {
      // Every dark ink must differ from its light counterpart. A regression once
      // left every dark splash carrying the LIGHT navy, rendering the mark's
      // structural half at 1.02:1 across all three families with nothing failing.
      for (const id of THEME_FAMILY_IDS) {
        const light = GENERATED_THEMES[id].light.mark.navy;
        const dark = GENERATED_THEMES[id].dark.mark.navy;
        expect(dark, `${id}: dark mark.navy must not reuse the light ink`).not.toBe(light);
      }
    });
  });

  describe('BioRouterWordmark — deliberately does NOT follow the family', () => {
    const src = read('BioRouterWordmark.tsx');

    it('keeps its inks as fixed brand constants', () => {
      expect(src).toMatch(/^const NAVY = '#052049';$/m);
      expect(src).toMatch(/^const CORAL = '#b85a32';$/m);
      expect(src).toMatch(/^const TEAL = '#18a3ac';$/m);
    });

    it('never reads the theme family', () => {
      // This is the assertion that stops a well-meaning "consistency" fix.
      // Matched against IMPORTS, not any mention: the component's own comment
      // names GENERATED_THEMES to explain why it deliberately does not use it.
      const imports = src
        .split('\n')
        .filter((l) => l.trimStart().startsWith('import'))
        .join('\n');
      expect(imports).not.toContain('themes.generated');
      expect(imports).not.toContain('useThemeFamily');
    });

    it('takes one ink under forced colours, on every drawn part', () => {
      // Chromium does not force an SVG `fill` given as a colour, so under
      // `forced-colors: active` the navy "Bio" kept its ink on the person's
      // canvas (about 1.3:1 on black). Each drawn part carries the class and
      // main.css gives it CanvasText inside the media query. The paint itself
      // is measured in `BioRouterWordmark.browser.test.tsx`; this pins the
      // pair at the source for a run with no Chromium.
      expect(src).toMatch(/const INK_CLASS = 'br-wordmark-ink';/);
      expect(src.match(/className=\{INK_CLASS\}/g)).toHaveLength(4);
      const css = read('../../styles/main.css');
      expect(css).toMatch(
        /@media \(forced-colors: active\) \{\s*\.br-wordmark-ink \{\s*fill: CanvasText;\s*\}\s*\}/
      );
    });

    it('still flips for light/dark, which is legibility rather than theming', () => {
      // Navy is unreadable on a dark ground, so the primary ink lifts to the
      // brand teal. That axis is kept; the family axis is not.
      expect(src).toMatch(/const navy = dark \? TEAL : NAVY;/);
    });
  });
});
