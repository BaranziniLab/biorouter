import { describe, expect, it } from 'vitest';
import {
  CODE_BG,
  codePalettes,
  codePalettesAlma,
  codePalettesRoche,
  codeThemeDark,
  codeThemeLight,
  codeThemesByFamily,
  GUTTER_INK_MIX,
  withFadedGutter,
} from './codeTheme';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from './themes.generated';

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

  // react-syntax-highlighter merges the theme's `comment` entry (italic) over a
  // call site's lineNumberStyle, and strips from the DOM every class a theme
  // KEY names — each half of a dotted key included. So the gutter entry must
  // sit on the last class of the three, and no key may be named after a class
  // something else selects on: `linenumber` (the gutter hook), `table` (the
  // Prism/Tailwind collision rule in main.css) or `token`.
  it('keeps the line-number gutter upright without stripping a hook class', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      for (const mode of ['light', 'dark'] as const) {
        const theme = codeThemesByFamily[family][mode];
        expect(theme['react-syntax-highlighter-line-number']?.fontStyle).toBe('normal');
        expect(theme['react-syntax-highlighter-line-number']?.fontWeight).toBe(400);
        const classes = new Set(Object.keys(theme).flatMap((key) => key.split('.')));
        for (const hook of ['linenumber', 'table', 'token']) {
          expect(classes.has(hook), `${family}.${mode} keys "${hook}"`).toBe(false);
        }
      }
    }
  });

  // A config file is mostly keys and literals. YAML aliases its keys to
  // `atrule` and its true/false/null to `important`, both keyword-coloured, so
  // every key and literal rendered as the keyword hue at weight 600 — a bold
  // coral wall. The pair keys win over the aliases.
  it('does not paint YAML and JSON keys or literals in the keyword colour', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      for (const mode of ['light', 'dark'] as const) {
        const theme = codeThemesByFamily[family][mode];
        const { keyword, func, number } = GENERATED_THEMES[family][mode].syntax;
        const where = `${family}.${mode}`;
        expect(theme['key.atrule'].color, where).toBe(func);
        expect(theme['key.atrule'].fontWeight, where).toBe(400);
        for (const literal of ['boolean.important', 'null.important', 'null.keyword']) {
          expect(theme[literal].color, `${where} ${literal}`).toBe(number);
          expect(theme[literal].color, `${where} ${literal}`).not.toBe(keyword);
          expect(theme[literal].fontWeight, `${where} ${literal}`).toBe(400);
        }
        expect(theme.property.color, where).toBe(func);
        expect(theme['attr-name'].color, where).toBe(func);
      }
    }
  });

  // Hue carries a keyword's role; 600 made a SQL file a column of bold words.
  // A class name stays 600, because at 500 it thinned until it read as ink.
  it('sets keywords at 500 and class names at 600', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      for (const mode of ['light', 'dark'] as const) {
        const theme = codeThemesByFamily[family][mode];
        const { type } = GENERATED_THEMES[family][mode].syntax;
        expect(theme.keyword.fontWeight).toBe(500);
        expect(theme['keyword.module'].fontWeight).toBe(500);
        expect(theme.important.fontWeight).toBe(500);
        expect(theme['class-name']).toEqual({ color: type, fontWeight: 600 });
        // `float`/`int` in the function hue read exactly like the call beside them.
        expect(theme.builtin.color).toBe(type);
        expect(theme['decorator.annotation'].color).toBe(type);
      }
    }
  });

  it('gives each log severity its own hue, and keeps log prose in ink', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      for (const mode of ['light', 'dark'] as const) {
        const theme = codeThemesByFamily[family][mode];
        const palette = GENERATED_THEMES[family][mode].syntax;
        const levels = ['level.error', 'level.warning', 'level.info', 'level.debug'].map(
          (key) => theme[key].color
        );
        expect(new Set(levels).size, `${family}.${mode} level hues`).toBe(4);
        expect(theme['level.error']).toEqual({ color: palette.deleted, fontWeight: 600 });
        expect(theme['level.warning']).toEqual({ color: palette.number, fontWeight: 600 });
        expect(theme['level.info']).toEqual({ color: palette.func, fontWeight: 400 });
        // Timestamps recede; a Nextflow task hash is not number speckle; a
        // `Completed at:` label is not painted like a JSON key.
        expect(theme['date.number'].color).toBe(palette.comment);
        expect(theme['task-hash'].color).toBe(palette.operator);
        expect(theme['property.log-label'].color).toBe(palette.plain);
      }
    }
  });

  it('fades a gutter by mixing its ink, leaving every other entry alone', () => {
    const faded = withFadedGutter(codeThemeLight, '55%');
    expect(faded['react-syntax-highlighter-line-number']).toEqual({
      color: `color-mix(in srgb, ${codePalettes.light.comment} 55%, transparent)`,
      fontStyle: 'normal',
      fontWeight: 400,
    });
    expect(faded['react-syntax-highlighter-line-number']).not.toHaveProperty('opacity');
    expect(faded.keyword).toBe(codeThemeLight.keyword);
    // The shared theme object is not mutated for chat and notebooks.
    expect(codeThemeLight['react-syntax-highlighter-line-number'].color).toBe(
      codePalettes.light.comment
    );
  });

  // The panel's line numbers are the comment ink mixed toward transparent and
  // composited on the paper. At 55% that came to about 2.3:1 in every family —
  // numbers you had to hunt for. A gutter should recede, not disappear.
  it('keeps the faded gutter at 3:1 on the paper in every family and mode', () => {
    const amount = parseFloat(GUTTER_INK_MIX) / 100;
    expect(GUTTER_INK_MIX).toBe(`${amount * 100}%`);
    const channels = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        const { syntax, surface } = GENERATED_THEMES[family][mode];
        const [ink, paper] = [channels(syntax.comment), channels(surface.background)];
        const composite = `#${ink
          .map((v, i) => Math.round(v * amount + paper[i] * (1 - amount)))
          .map((v) => v.toString(16).padStart(2, '0'))
          .join('')}`;
        expect(
          contrast(composite, surface.background),
          `${family}.${mode} gutter ${composite} on ${surface.background}`
        ).toBeGreaterThanOrEqual(3);
      }
    }
  });

  // Every family must be registered for BOTH modes: the consumer indexes
  // codeThemesByFamily[family][mode] with no fallback, so a missing entry is a
  // runtime undefined rather than a type error at the call site.
  it('registers every theme family in both modes', () => {
    for (const family of ['parchment', 'alma-mater', 'roche-limit'] as const) {
      expect(codeThemesByFamily[family]?.light, `${family}.light`).toBeDefined();
      expect(codeThemesByFamily[family]?.dark, `${family}.dark`).toBeDefined();
    }
  });
});
