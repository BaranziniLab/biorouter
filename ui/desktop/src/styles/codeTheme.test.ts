import { describe, expect, it } from 'vitest';
import {
  CODE_BG,
  codePalettes,
  codePalettesAlma,
  codePalettesRoche,
  codeThemeDark,
  codeThemeLight,
  codeThemesByFamily,
} from './codeTheme';

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

describe('code theme', () => {
  it.each(['light', 'dark'] as const)('%s palette clears WCAG AA on its own ground', (theme) => {
    const bg = CODE_BG[theme];
    for (const [token, hex] of Object.entries(codePalettes[theme])) {
      expect(
        contrast(hex, bg),
        `${theme} "${token}" (${hex}) on ${bg} is ${contrast(hex, bg).toFixed(2)}:1`
      ).toBeGreaterThanOrEqual(4.5);
    }
  });

  it.each([
    ['light', codeThemeLight],
    ['dark', codeThemeDark],
  ] as const)('%s theme drives every colour through the mono font', (_theme, style) => {
    // A stock Prism theme would set a proportional font here. P6: code is monospace.
    expect(style['code[class*="language-"]'].fontFamily).toBe('var(--font-mono)');
    expect(style['pre[class*="language-"]'].fontFamily).toBe('var(--font-mono)');
  });

  it.each([
    ['light', codeThemeLight],
    ['dark', codeThemeDark],
  ] as const)('%s theme carries no text-shadow', (_theme, style) => {
    // Prism's defaults smear against a warm ground.
    expect(style['code[class*="language-"]'].textShadow).toBe('none');
  });

  it('uses a different hue per theme rather than sharing one', () => {
    expect(codePalettes.light.keyword).not.toBe(codePalettes.dark.keyword);
    expect(codePalettes.light.plain).not.toBe(codePalettes.dark.plain);
  });

  it.each(['light', 'dark'] as const)(
    'Alma Mater %s palette clears WCAG AA on its own ground',
    (theme) => {
      const { palette, bg } = codePalettesAlma[theme];
      for (const [token, hex] of Object.entries(palette)) {
        expect(
          contrast(hex, bg),
          `alma ${theme} "${token}" (${hex}) on ${bg} is ${contrast(hex, bg).toFixed(2)}:1`
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  );

  it.each(['light', 'dark'] as const)(
    'Roche Limit %s palette clears WCAG AA on its own ground',
    (theme) => {
      const { palette, bg } = codePalettesRoche[theme];
      for (const [token, hex] of Object.entries(palette)) {
        expect(
          contrast(hex, bg),
          `roche ${theme} "${token}" (${hex}) on ${bg} is ${contrast(hex, bg).toFixed(2)}:1`
        ).toBeGreaterThanOrEqual(4.5);
      }
    }
  );

  // Roche Limit deliberately does NOT inherit two JupyterLab stops that ship
  // below AA: their `comment` (#408080) is ~2.8:1 on the dark ground, and their
  // dark `func` (#1e88e5) is ~3.4:1. Pin that so a future "be more faithful to
  // Jupyter" edit cannot quietly reintroduce them.
  it('does not inherit JupyterLab stops that fail AA', () => {
    expect(codePalettesRoche.dark.palette.comment).not.toBe('#408080');
    expect(codePalettesRoche.dark.palette.func).not.toBe('#1e88e5');
  });

  // Every family must be registered for BOTH modes: the consumer indexes
  // codeThemesByFamily[family][mode] with no fallback, so a missing entry is a
  // runtime undefined rather than a type error at the call site.
  // react-syntax-highlighter merges the theme's `comment` entry (italic) over a
  // call site's lineNumberStyle, and strips from the DOM every class a theme
  // KEY names. So the gutter entry must sit on the last class of the three and
  // must not be keyed `linenumber`, which is the class the gutter is found by.
  it('keeps the line-number gutter upright without stripping its class', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      for (const mode of ['light', 'dark'] as const) {
        const theme = codeThemesByFamily[family][mode];
        expect(theme['react-syntax-highlighter-line-number']?.fontStyle).toBe('normal');
        expect(theme).not.toHaveProperty('linenumber');
      }
    }
  });

  it('gives log severity its own hue instead of one keyword colour', () => {
    const theme = codeThemeLight;
    expect(theme['level.error'].color).toBe(codePalettes.light.deleted);
    expect(theme['level.warning'].color).not.toBe(theme['level.info'].color);
  });

  it('registers every theme family in both modes', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      expect(codeThemesByFamily[family]?.light, `${family}.light`).toBeDefined();
      expect(codeThemesByFamily[family]?.dark, `${family}.dark`).toBeDefined();
    }
  });
});
