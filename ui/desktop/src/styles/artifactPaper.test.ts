import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';
import { PAPER_GUTTER_EM } from '../components/artifacts/artifactUtils';
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

/** `fill` at `alpha` composited over `ground`, as a hex. */
function blend(fill: string, alpha: number, ground: string): string {
  const ch = (hex: string) => [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
  const [f, g] = [ch(fill), ch(ground)];
  return `#${f
    .map((v, i) => Math.round(v * alpha + g[i] * (1 - alpha)))
    .map((v) => v.toString(16).padStart(2, '0'))
    .join('')}`;
}

/** The body of the first rule whose selector is exactly `selector`. */
function ruleBody(selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  const match = new RegExp(`(?:^|[}\\s])${escaped} \\{([^}]*)\\}`).exec(flat);
  return match ? match[1] : null;
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
    expect(flat).toMatch(/\.br-preview-measure \{[^}]*max-width: var\(--measure-chat\);/);
    expect(flat).toContain(
      '--paper-inset: max(var(--paper-gutter), calc((100cqw - var(--measure-chat)) / 2));'
    );
    expect(flat).toContain(
      '--paper-column: min(var(--measure-chat), calc(100cqw - 2 * var(--paper-gutter)));'
    );
    expect(css).toMatch(/--measure-chat: 760px;/);
  });

  // The code view's gutter is set in JS (lineNumberStyle) and hung in CSS.
  // If the two disagree, code text stops landing on the column edge.
  it('hangs the code gutter by exactly the width CodeBlock gives it', () => {
    const em = parseFloat(PAPER_GUTTER_EM);
    expect(PAPER_GUTTER_EM).toBe(`${em}em`);
    expect(flat).toContain(`--paper-code-gutter: calc(${em} * 13px);`);
  });

  // The paper block is authored CSS that must sit right after the Prism/Tailwind
  // `.table` collision rule — and never at EOF, where the panel-geometry track
  // adds its own rules — while that rule keeps working.
  it('sits after the token display rule, which still keeps tokens inline', () => {
    const tokenRule = css.indexOf("code [class~='token'] {\n  display: inline;\n}");
    const paper = css.indexOf('/* ── Preview paper');
    const selection = css.indexOf('/* ── Selection: Biorouter orange');
    expect(tokenRule).toBeGreaterThan(-1);
    expect(selection).toBeGreaterThan(tokenRule);
    expect(paper).toBeGreaterThan(selection);
    expect(css.slice(tokenRule, selection)).not.toMatch(/\n[.:a-z][^\n]*\{/);
    // …and not at the end of the file.
    expect(css.trimEnd().slice(-400)).not.toContain('.br-paper');
  });

  it('gives the code view and table an overflow hint', () => {
    const body = ruleBody(".br-paper-scroll[data-view='code'], .br-paper-table-scroll");
    expect(body).toMatch(/no-repeat local,/);
    expect(body).toMatch(
      /var\(--paper-hint-ink\), transparent\) 100% 0 \/ 14px 100% no-repeat scroll/
    );
  });

  it('keeps fenced code on the syntax palette ground with an overflow hint', () => {
    const body = ruleBody('.biorouter-markdown .biorouter-md-code-body');
    expect(body).toContain('var(--background-code)');
    expect(body).toMatch(/no-repeat local,/);
    expect(body).toMatch(/no-repeat scroll/);
  });

  // A narrow CSV used to stretch its columns across the whole column.
  it('packs table columns and lets a filler cell take the slack', () => {
    expect(ruleBody('.br-paper-table td')).toContain('width: 1px;');
    expect(ruleBody('.br-paper-table th')).toContain('width: 1px;');
    expect(ruleBody('.br-paper-table .br-paper-fill')).toContain('width: auto;');
    expect(ruleBody('.br-paper-table')).toContain('width: max-content;');
    expect(ruleBody('.br-paper-table')).toContain(
      'min-width: min( calc(var(--paper-column) + var(--paper-rownum-width)), calc(100cqw - var(--paper-lead) - var(--paper-gutter)) );'
    );
  });

  // The gutter stays put while a long line scrolls under it; it must stick to
  // the paper scroller and be opaque, so no glyph shows through.
  it('keeps the code gutter sticky and opaque', () => {
    const gutter = ruleBody('.br-paper-code .linenumber');
    expect(gutter).toContain('position: sticky;');
    expect(gutter).toContain('left: 0;');
    expect(gutter).toContain('background-color: var(--background-default);');
    expect(ruleBody(".br-paper-code [aria-current='location'] .linenumber")).toContain(
      'background-color: var(--background-medium);'
    );
  });

  // ⚠ This assertion used to READ THE COLOURS OUT OF THE RULE, with
  // `/color: (#[0-9a-f]{6})/`, and compute their ratio. That only works while
  // the colours are literals — so the test passed, and would have gone RED the
  // moment someone did the right thing and used a token. A contrast test that
  // requires hardcoded colour is a test that enforces the bug.
  //
  // The rule states tokens now. Contrast is owned by the checker that resolves
  // them per family: `check-contrast.mjs` asserts `--text-default` on
  // `--background-medium` at 4.5:1 in all six scopes (three families x light and
  // dark). What is left for this file is the part that checker cannot see —
  // whether this rule uses those tokens at all.
  it('paints the shared markdown table header from tokens, never literals', () => {
    const header = ruleBody('.biorouter-markdown.prose thead th');
    const band = ruleBody('.biorouter-markdown.prose thead');
    expect(band, 'thead rule').not.toBeNull();
    expect(header, 'thead th rule').not.toBeNull();
    expect(band).toContain('background: var(--background-medium);');
    expect(header).toContain('color: var(--text-default);');
    // The point of the exercise: no hex may come back, in either rule.
    expect(band).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(header).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    // And no `.dark` override may reintroduce one — the tokens already flip, so
    // a dark-mode rule here is a literal waiting to be written.
    expect(ruleBody('.dark .biorouter-markdown.prose thead')).toBeNull();
    expect(ruleBody('.dark .biorouter-markdown.prose thead th')).toBeNull();
    expect(
      ruleBody(
        '.biorouter-markdown.prose thead th :where(strong, em, a, button:not(.biorouter-inline-code), span:not(.biorouter-inline-code))'
      )
    ).toContain('color: inherit;');
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

  // 24% in BOTH modes. Dark at 30% composited to a muddy brown slab over the
  // #1b1b19 page and pulled coral keywords down near 3.9:1.
  it('uses a 24% tint in light and in dark', () => {
    expect(flat).toContain(':root { --selection-hue: #cf6d47; --selection-alpha: 24%; }');
    expect(flat).toContain('.dark { --selection-hue: #e8895f; --selection-alpha: 24%; }');
    expect(css.match(/--selection-alpha:/g)).toHaveLength(2);
  });

  // The generator refuses a palette that fails this; the same rule, seen from
  // the renderer's side.
  it('leaves every syntax stop readable under the tint', () => {
    const hue = { light: '#cf6d47', dark: '#e8895f' } as const;
    for (const family of THEME_FAMILY_IDS) {
      for (const mode of ['light', 'dark'] as const) {
        const { syntax, wellGround, surface } = GENERATED_THEMES[family][mode];
        for (const ground of [surface.background, wellGround]) {
          const tinted = blend(hue[mode], 0.24, ground);
          for (const [stop, hex] of Object.entries(syntax)) {
            expect(
              contrast(hex, tinted),
              `${family}.${mode} ${stop} ${hex} under selection on ${ground}`
            ).toBeGreaterThanOrEqual(3);
          }
        }
      }
    }
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
