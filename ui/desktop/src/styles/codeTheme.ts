/**
 * The Biorouter syntax palette (design.md §5.1, decision D-10).
 *
 * Derived from the warm neutral ramp rather than imported from a stock theme, so
 * code sits on the same ground as the rest of the app instead of reading as a
 * pasted-in foreign object. Every foreground below clears WCAG AA (4.5:1)
 * against its stated background; the ratios are asserted in codeTheme.test.ts.
 *
 * Syntax colours are DEFINED in themes/<id>.theme.mjs and reach this file via
 * themes.generated.ts. This module owns the Prism token MAPPING, not the values.
 * Both the chat markdown renderer and the artifact preview import from here.
 */
import type { CSSProperties } from 'react';
import { GENERATED_THEMES, THEME_FAMILY_IDS } from './themes.generated';
import type { ThemeFamilyId } from './themes.generated';

type PrismTheme = Record<string, CSSProperties>;

/**
 * Ground each palette is measured against — DERIVED from --background-code by
 * the theme generator, not typed twice. This value used to be hand-copied here
 * and in main.css and InAppTerminalDock, and drifted: a palette was once
 * verified against a surface the app never painted, rendering `comment` at
 * 4.15:1 with every check green.
 */
export const CODE_BG = {
  light: GENERATED_THEMES.parchment.light.codeGround,
  dark: GENERATED_THEMES.parchment.dark.codeGround,
} as const;

/** Shared with the xterm terminal so a pasted command and its output match. */
export const CODE_FONT_FAMILY = 'var(--font-mono)';
export const CODE_FONT_SIZE = '13px';
export const CODE_LINE_HEIGHT = '20px';

/**
 * Alma Mater (UCSF) syntax palette — recoloured to UCSF hue families, measured
 * against the Alma code grounds (light #f2f3f4, dark #08213f). `type` is the
 * accent so code ties to the brand — now the teal C column, which is why `type`
 * and `func` swapped families: the accent teal took `type`, and the eggplant it
 * displaced from the chrome moved to `func`, where its hue distance from teal
 * keeps the two roles legibly apart. (The accent's own C2 #14828c is only
 * 4.11:1 on the code ground and could not be used here; C1 #0e5258 is 7.99:1.)
 * Every stop clears WCAG AA; ratios asserted in codeTheme.test.ts.
 * See docs/design/theming/alma-mater-theme-tokens.md §5g.
 */
/**
 * Roche Limit syntax palette — JupyterLab's own IPython/Pygments hues, darkened
 * (light) and lifted (dark) until every stop clears WCAG AA on the Roche code
 * grounds (light #f5f5f3, dark #1b1b19). Ratios asserted in codeTheme.test.ts.
 *
 * Two stops deliberately do NOT copy Jupyter: their `comment` (#408080) ships
 * unchanged in dark at ~2.8:1, and their dark `func` (#1e88e5) at ~3.4:1 —
 * both fail AA. See docs/design/theming/roche-limit-theme.md §4.10 / §5.8.
 */
type SyntaxPalette = {
  plain: string;
  comment: string;
  keyword: string;
  string: string;
  number: string;
  func: string;
  type: string;
  operator: string;
  deleted: string;
  inserted: string;
};

function build(p: SyntaxPalette, tint: string): PrismTheme {
  const base: CSSProperties = {
    color: p.plain,
    background: 'transparent',
    fontFamily: CODE_FONT_FAMILY,
    fontSize: CODE_FONT_SIZE,
    lineHeight: CODE_LINE_HEIGHT,
    // Prism's stock themes ship a text-shadow that smears on a warm ground.
    textShadow: 'none',
    tabSize: 2,
  };

  return {
    'code[class*="language-"]': base,
    'pre[class*="language-"]': { ...base, margin: 0, padding: 0, overflow: 'auto' },

    comment: { color: p.comment, fontStyle: 'italic' },
    prolog: { color: p.comment },
    doctype: { color: p.comment },
    cdata: { color: p.comment },

    punctuation: { color: p.operator },
    operator: { color: p.operator },
    entity: { color: p.operator },
    url: { color: p.func },

    property: { color: p.plain },
    tag: { color: p.keyword },
    'attr-name': { color: p.number },
    'attr-value': { color: p.string },
    selector: { color: p.type },
    atrule: { color: p.keyword },

    boolean: { color: p.number },
    number: { color: p.number },
    constant: { color: p.number },
    symbol: { color: p.number },

    string: { color: p.string },
    char: { color: p.string },
    regex: { color: p.string },

    keyword: { color: p.keyword, fontWeight: 600 },
    'keyword.module': { color: p.keyword, fontWeight: 600 },
    builtin: { color: p.keyword },
    important: { color: p.keyword, fontWeight: 600 },

    function: { color: p.func },
    'class-name': { color: p.type, fontWeight: 600 },
    namespace: { color: p.type },

    variable: { color: p.plain },

    // ⚠ The line-number gutter. react-syntax-highlighter gives every number span
    // the classes `comment linenumber react-syntax-highlighter-line-number` and
    // merges the matching entries of THIS object over the caller's
    // `lineNumberStyle`, in that class order — so the `comment` entry above
    // (italic, comment ink) used to win over any `fontStyle: 'normal'` a call
    // site passed, and every gutter in the app leaned. This key is the LAST of
    // the three, so it is the one that sticks.
    //
    // Keyed on the long name, not `linenumber`, on purpose: in inline-style mode
    // the library strips every class that names a stylesheet key from the DOM,
    // and `.linenumber` is the hook tests and CSS select the gutter by.
    'react-syntax-highlighter-line-number': {
      color: p.comment,
      fontStyle: 'normal',
      fontWeight: 400,
    },

    // The `log` grammar (a `.log`, or a `.txt` the preview recognises as one).
    // Every level used to share the one keyword colour, so an ERROR read like an
    // INFO; severity now reads by hue. Timestamps step back to the comment ink
    // rather than painting the whole left column in the number colour. Pair keys
    // (`level.error`) because the grammar emits `level error important` and the
    // pair is merged after the singles, so it beats `important`.
    'level.error': { color: p.deleted, fontWeight: 600 },
    'level.warning': { color: p.number, fontWeight: 600 },
    'level.info': { color: p.func, fontWeight: 400 },
    'level.debug': { color: p.comment, fontWeight: 400 },
    'level.trace': { color: p.comment, fontStyle: 'normal' },
    'date.number': { color: p.comment },
    'time.number': { color: p.comment },

    // Diff rows tint the whole line, not just the glyphs.
    deleted: {
      color: p.deleted,
      background: `color-mix(in srgb, ${p.deleted} ${tint}, transparent)`,
    },
    inserted: {
      color: p.inserted,
      background: `color-mix(in srgb, ${p.inserted} ${tint}, transparent)`,
    },
  };
}

export const codeThemeLight = build(GENERATED_THEMES.parchment.light.syntax, '9%');
export const codeThemeDark = build(GENERATED_THEMES.parchment.dark.syntax, '10%');
export const codeThemeAlmaLight = build(GENERATED_THEMES['alma-mater'].light.syntax, '9%');
export const codeThemeAlmaDark = build(GENERATED_THEMES['alma-mater'].dark.syntax, '10%');
export const codeThemeRocheLight = build(GENERATED_THEMES['roche-limit'].light.syntax, '9%');
export const codeThemeRocheDark = build(GENERATED_THEMES['roche-limit'].dark.syntax, '10%');

/** Parchment themes, keyed by resolved mode (kept for back-compat). */
export const codeThemes = { light: codeThemeLight, dark: codeThemeDark } as const;

/**
 * Syntax themes keyed by theme family, then resolved mode. Consumers select
 * with `codeThemesByFamily[useThemeFamily()][useResolvedTheme()]` so code
 * matches whichever theme (Parchment / Alma Mater / Roche Limit) is active.
 */
export const codeThemesByFamily = Object.fromEntries(
  THEME_FAMILY_IDS.map((id) => [
    id,
    {
      light: build(GENERATED_THEMES[id].light.syntax, '9%'),
      dark: build(GENERATED_THEMES[id].dark.syntax, '10%'),
    },
  ])
) as Record<ThemeFamilyId, { light: PrismTheme; dark: PrismTheme }>;

export const CODE_BG_ALMA = {
  light: GENERATED_THEMES['alma-mater'].light.codeGround,
  dark: GENERATED_THEMES['alma-mater'].dark.codeGround,
} as const;

export const CODE_BG_ROCHE = {
  light: GENERATED_THEMES['roche-limit'].light.codeGround,
  dark: GENERATED_THEMES['roche-limit'].dark.codeGround,
} as const;

/** Palette values, exported so tests can assert the contrast ratios. */
export const codePalettes = {
  light: GENERATED_THEMES.parchment.light.syntax,
  dark: GENERATED_THEMES.parchment.dark.syntax,
} as const;

/** Alma Mater palettes + their grounds, exported for the contrast test. */
export const codePalettesAlma = {
  light: { palette: GENERATED_THEMES['alma-mater'].light.syntax, bg: CODE_BG_ALMA.light },
  dark: { palette: GENERATED_THEMES['alma-mater'].dark.syntax, bg: CODE_BG_ALMA.dark },
} as const;

/** Roche Limit palettes + their grounds, exported for the contrast test. */
export const codePalettesRoche = {
  light: { palette: GENERATED_THEMES['roche-limit'].light.syntax, bg: CODE_BG_ROCHE.light },
  dark: { palette: GENERATED_THEMES['roche-limit'].dark.syntax, bg: CODE_BG_ROCHE.dark },
} as const;
