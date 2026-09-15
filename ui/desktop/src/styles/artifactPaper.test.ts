import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from './themes.generated';

/**
 * The artifact panel's "paper" (main.css, "Preview paper"). jsdom applies no
 * stylesheet, so a component test cannot see any of this; these assertions
 * read the sources the browser will.
 */

// vitest runs with `ui/desktop` as the root.
const css = readFileSync('src/styles/main.css', 'utf-8');
const flat = css.replace(/\s+/g, ' ');

function luminance(hex: string): number {
  const channels = [1, 3, 5]
    .map((i) => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map((v) => (v <= 0.03928 ? v / 12.92 : Math.pow((v + 0.055) / 1.055, 2.4)));
  return 0.2126 * channels[0] + 0.7152 * channels[1] + 0.0722 * channels[2];
}

function contrast(fg: string, bg: string): number {
  const [a, b] = [luminance(fg), luminance(bg)];
  return (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);
}

describe('artifact panel paper', () => {
  // The operator's ask: a markdown file and a chart open in the panel should
  // look like the same family of surface. The chart's ground is set by the
  // Auto Visualiser runtime, in another crate; this pins the two together so a
  // change on either side fails here rather than in someone's eye.
  it('paints the same ground an Auto Visualiser chart paints, in every family', () => {
    const common = readFileSync(
      '../../crates/biorouter-mcp/src/autovisualiser/templates/_common.js',
      'utf-8'
    );
    const match = /bg\s*:\s*dark\s*\?\s*'(#[0-9a-f]{6})'\s*:\s*'(#[0-9a-f]{6})'/i.exec(common);
    expect(match, 'chart ground declaration in _common.js').not.toBeNull();
    const [, chartDark, chartLight] = match!;
    for (const family of THEME_FAMILY_IDS) {
      expect(GENERATED_THEMES[family].light.surface.background, family).toBe(chartLight);
      expect(GENERATED_THEMES[family].dark.surface.background, family).toBe(chartDark);
    }
    expect(flat).toMatch(/\.br-paper \{[^}]*background-color: var\(--background-default\);/);
  });

  it('sets the well one visible step off the paper in both modes', () => {
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        const { wellGround, surface } = GENERATED_THEMES[family][mode];
        expect(contrast(wellGround, surface.background), `${family}.${mode}`).toBeGreaterThan(1.05);
      }
    }
  });

  // The generator refuses to emit a palette that fails these; this is the
  // same rule seen from the renderer's side, so a hand-edited generated file
  // cannot slip past it.
  it('holds every syntax stop to AA on the paper and in the well', () => {
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        const { syntax, wellGround, surface } = GENERATED_THEMES[family][mode];
        for (const [stop, hex] of Object.entries(syntax)) {
          for (const ground of [wellGround, surface.background]) {
            expect(
              contrast(hex, ground),
              `${family}.${mode} ${stop} ${hex} on ${ground}`
            ).toBeGreaterThanOrEqual(4.5);
          }
        }
      }
    }
  });

  // A measure that is a class name in one place and a number in another drifts.
  it('caps the paper column at the chat measure', () => {
    expect(flat).toMatch(/\.br-paper-measure \{[^}]*max-width: var\(--measure-chat\);/);
    expect(flat).toContain(
      '--paper-inset: max(var(--paper-gutter), calc((100cqw - var(--measure-chat)) / 2));'
    );
  });
});

describe('selection', () => {
  it('is Biorouter orange and leaves the ink alone', () => {
    const rule = /::selection \{([^}]*)\}/.exec(flat);
    expect(rule, '::selection rule').not.toBeNull();
    expect(rule![1]).toContain('var(--selection-hue)');
    // Setting `color` would flatten every syntax token to one ink.
    expect(rule![1]).not.toMatch(/(^|[^-])color:/);
    expect(flat).toMatch(/:root \{ --selection-hue: #cf6d47;/);
    expect(flat).toMatch(/\.dark \{ --selection-hue: #e8895f;/);
  });

  // Alma Mater re-points the coral scale to teal. A selection built on a
  // family token turned teal there; the hue must never be re-declared per family.
  it('is not re-declared by any theme family', () => {
    const start = css.indexOf('/* THEMES:GENERATED:FAMILIES:START */');
    const end = css.indexOf('/* THEMES:GENERATED:FAMILIES:END */');
    expect(start).toBeGreaterThan(-1);
    expect(css.slice(start, end)).not.toContain('--selection');
  });
});
