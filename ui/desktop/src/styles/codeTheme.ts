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
// Side effect: the CSV/TSV raw grammars, R/Python call tokens and the log
// refinements the mapping below keys on. Imported here so every highlighter
// that reads this palette also gets the grammars it was written for.
import './prismGrammars';
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
export const CODE_FONT_SIZE = 'calc(13px * var(--app-font-scale, 1))';
export const CODE_LINE_HEIGHT = 'calc(20px * var(--app-font-scale, 1))';

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

  // ⚠ A stylesheet key is not only a colour: react-syntax-highlighter REMOVES
  // from the rendered span every class it finds a key for — each half of a
  // dotted key included (create-element.js, `allStylesheetSelectors`). A key
  // named `linenumber`, `table` or `token` would strip the very class main.css
  // and the tests select on. Style those in CSS.
  //
  // Dotted keys (`key.atrule`) match a token carrying both classes and are
  // merged AFTER the single-class keys (`createStyleObject`), which is the only
  // way to beat an alias Prism appends later in the class list.
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
    url: { color: p.func, textDecoration: 'underline', textUnderlineOffset: '2px' },

    // KEYS take the function hue at body weight: JSON/TOML/CSS keys and a raw
    // CSV header (`property`), YAML keys (`key`, aliased `atrule`), markup
    // attribute names. A config file is mostly keys; in ink they read as
    // undifferentiated text, and in the keyword colour (YAML's `atrule`) at 600
    // every key and literal was a bold coral wall.
    property: { color: p.func },
    // …except the `log` grammar's prose labels (`Module rseqc:`, `Notes:`),
    // which prismGrammars.ts aliases so a log message is not painted blue.
    'property.log-label': { color: p.plain },
    'key.atrule': { color: p.func, fontWeight: 400 },
    tag: { color: p.keyword },
    'attr-name': { color: p.func },
    'attr-value': { color: p.string },
    selector: { color: p.type },
    atrule: { color: p.keyword },

    boolean: { color: p.number },
    // YAML aliases its literals to `important` (keyword, 500) and JSON its
    // `null` to `keyword`: a value is a value, so they take the number hue.
    'boolean.important': { color: p.number, fontWeight: 400 },
    'null.important': { color: p.number, fontWeight: 400 },
    'null.keyword': { color: p.number, fontWeight: 400 },
    number: { color: p.number },
    constant: { color: p.number },
    symbol: { color: p.number },
    // Shell `$VAR`, SQL `@var`: a value you did not write inline.
    variable: { color: p.number },

    string: { color: p.string },
    char: { color: p.string },
    regex: { color: p.string },

    // Weight 500, not 600. At 13px mono a semibold keyword out-weighs the
    // identifiers it governs and a SQL file read as a column of bold words;
    // hue carries the role, and weight only has to separate it from ink.
    keyword: { color: p.keyword, fontWeight: 500 },
    'keyword.module': { color: p.keyword, fontWeight: 500 },
    important: { color: p.keyword, fontWeight: 500 },
    // The type hue, not the function hue: `float`/`int`/`set` in the function
    // hue read exactly like the `getLogger(` call beside them.
    builtin: { color: p.type },

    function: { color: p.func },
    // Stays 600: at 500 a class name thinned until it read as ink.
    'class-name': { color: p.type, fontWeight: 600 },
    namespace: { color: p.type },
    // Python decorators arrive as `decorator annotation punctuation`, so the
    // single-class `punctuation` greyed them out.
    'decorator.annotation': { color: p.type },

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
    // INFO; severity now reads by hue, and only the two that need a reader's eye
    // are heavy. Timestamps step back to the comment ink rather than painting
    // the whole left column in the number colour, and a Nextflow task hash
    // (`4f/a1c2e9`) takes operator ink instead of number-coloured speckle. Pair
    // keys (`level.error`) because the grammar emits `level error important`
    // and the pair is merged after the singles, so it beats `important`.
    'level.error': { color: p.deleted, fontWeight: 600 },
    'level.warning': { color: p.number, fontWeight: 600 },
    'level.info': { color: p.func, fontWeight: 400 },
    'level.debug': { color: p.comment, fontWeight: 400 },
    'level.trace': { color: p.comment, fontStyle: 'normal' },
    'date.number': { color: p.comment },
    'time.number': { color: p.comment },
    'task-hash': { color: p.operator },

    // Markdown's raw view: headings and emphasis read as structure.
    'title.important': { color: p.keyword, fontWeight: 600 },
    bold: { fontWeight: 600 },
    italic: { fontStyle: 'italic' },

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

/**
 * How much of the comment ink the artifact panel's line numbers keep. A gutter
 * should recede behind the code, not vanish: at 55% the numbers composited to
 * about 2.3:1 on the paper in every family and were hard to find. 70% is the
 * least that holds 3:1 on the paper in all three families and both modes, which
 * codeTheme.test.ts asserts.
 */
export const GUTTER_INK_MIX = '70%';

/**
 * The same theme with its line-number gutter ink faded to `amount` of the
 * comment ink by MIXING, never with `opacity`.
 *
 * The artifact panel's gutter is sticky and paints an opaque paper ground so a
 * long line scrolled under it stays hidden; an `opacity` on the number span
 * fades that ground along with the digits, and the code showed through.
 */
export function withFadedGutter(
  theme: Record<string, CSSProperties>,
  amount: string
): Record<string, CSSProperties> {
  const gutter = theme['react-syntax-highlighter-line-number'] ?? {};
  return {
    ...theme,
    'react-syntax-highlighter-line-number': {
      ...gutter,
      color: `color-mix(in srgb, ${gutter.color} ${amount}, transparent)`,
    },
  };
}

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
