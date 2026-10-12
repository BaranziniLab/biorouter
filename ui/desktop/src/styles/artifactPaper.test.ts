import { readFileSync } from 'node:fs';
import postcss from 'postcss';
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

  it('gives the code view an overflow hint', () => {
    const body = ruleBody(".br-paper-scroll[data-view='code']");
    expect(body).toMatch(/no-repeat local,/);
    expect(body).toMatch(
      /var\(--paper-hint-ink\), transparent\) 100% 0 \/ 14px 100% no-repeat scroll/
    );
    // A table draws its own fade; the scroller's background hint is the code view's.
    expect(flat).not.toMatch(/\.br-paper-table-scroll \{[^}]*no-repeat local/);
    expect(flat).not.toContain('br-paper-fill-hint');
    expect(flat).not.toContain('scroll-state(stuck');
  });

  // V2-09: a CSV scrolls sideways in a narrow panel, and nothing said so. Every
  // row's filler cell sticks to the scroller's right edge and paints 40px of the
  // row's own ground over the text, while `data-overflow` says more lies to the
  // right (DelimitedTable). A mask on the scroller would fade its vertical
  // scrollbar too, which is why the fade is not one.
  it('fades a scrolling table out at its right edge, on each row’s own ground', () => {
    expect(ruleBody('.br-paper-table')).toContain('--paper-fade-width: 40px;');
    // Every row, the header included, rests on the table's one fill (R-27), so
    // there is one fade ground and no second one for the header.
    expect(ruleBody('.br-paper-table')).toContain('--paper-fade-ground: var(--background-fill);');
    expect(flat).not.toContain('--paper-fade-head-ground');
    const filler = ruleBody('.br-paper-table .br-paper-fill');
    expect(filler).toContain('position: sticky;');
    expect(filler).toContain('right: 0;');
    const fade = ruleBody('.br-paper-table .br-paper-fill::after');
    expect(fade).toContain('width: var(--paper-fade-width);');
    expect(fade).toContain('pointer-events: none;');
    expect(fade).toContain('display: none;');
    expect(fade).toMatch(/linear-gradient\(to left, var\(--paper-fade-ground\), transparent\)/);
    // A hovered row fades into its own ground: the strong step the row takes.
    expect(ruleBody('.br-paper-table tbody tr:hover .br-paper-fill::after')).toContain(
      'var(--background-fill-strong)'
    );
    // Drawn only while more lies to the right, and never over forced colours.
    expect(
      ruleBody(".br-paper-table-scroll[data-overflow='true'] .br-paper-fill::after")
    ).toContain('display: block;');
    expect(flat).toMatch(
      /@media \(forced-colors: active\) \{ \.br-paper-table \.br-paper-fill::after \{ display: none !important;/
    );
  });

  it('keeps fenced code in the well with a right-edge fade while it scrolls', () => {
    expect(ruleBody('.br-md-code')).toContain('background-color: var(--background-well);');
    // The fade is as wide as what is still past the edge, at most 40px
    // (`--br-overflow-fade`, WS-TRANSCRIPT final check 2026-10-12).
    expect(css).toMatch(
      /\.br-md-code-body\[data-overflow='true'\],\s*\.br-md-table-scroll\[data-overflow='true'\]\s*\{\s*mask-image: linear-gradient\(\s*to right,\s*black calc\(100% - var\(--br-overflow-fade, 40px\)\),\s*transparent\s*\);/
    );
  });

  // A narrow CSV used to stretch its columns across the whole column.
  it('packs table columns and lets a filler cell take the slack', () => {
    expect(ruleBody('.br-paper-table.br-md-table td')).toContain('width: 1px;');
    expect(ruleBody('.br-paper-table.br-md-table th')).toContain('width: 1px;');
    expect(ruleBody('.br-paper-table.br-md-table .br-paper-fill')).toContain('width: auto;');
    expect(ruleBody('.br-paper-table')).toContain('width: max-content;');
    expect(ruleBody('.br-paper-table')).toContain(
      'min-width: min(var(--paper-column), calc(100cqw - var(--paper-lead) - var(--paper-gutter)));'
    );
  });

  // One table look (spec section 0): the CSV wears the shared recipe, and only
  // undoes the corner clip so its header can stick.
  it('puts a CSV on the shared table recipe with a sticky header', () => {
    const source = readFileSync('src/components/artifacts/DelimitedTable.tsx', 'utf-8');
    expect(source).toContain('className="br-md-table br-paper-table"');
    expect(ruleBody('.br-paper-table.br-md-table')).toContain('overflow: visible;');
    const head = ruleBody('.br-paper-table.br-md-table th');
    expect(head).toContain('position: sticky;');
    expect(head).toContain('box-shadow: inset 0 -1px 0 var(--border-subtle);');
    // The sticky header paints the table's own fill, so rows passing under it
    // never show through (R-27: no second ground, which would read as a hover).
    expect(head).toContain('background-color: var(--background-fill);');
    // The frame sits on the column edge the bar's name and the prose share…
    expect(ruleBody('.br-paper-table-frame')).toContain('padding-left: var(--paper-inset);');
    // …and leaves it only for a table that fits by doing so (V2-09: a 1300px
    // table in a 600px panel sat at the 10px lead, 18px left of the bar's name).
    const yields = ruleBody('.br-paper-table-frame[data-yield]');
    expect(yields).toContain('min(var(--paper-inset),');
    expect(yields).toContain('var(--paper-lead)');
    expect(flat).not.toContain('.br-paper-table-note');
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
    const header = ruleBody('.br-md-table thead :where(th, td), .br-md-table th[data-head]');
    expect(header, 'table header rule').not.toBeNull();
    // The header rests on a ground token: the page's muted ground, or (R-27, the
    // filled table) the table's own fill, never a literal.
    expect(header).toMatch(/background-color: var\(--background-(muted|fill)\);/);
    expect(header).toContain('color: var(--text-muted);');
    // The point of the exercise: no hex may come back.
    expect(header).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    // And no `.dark` override may reintroduce one: the tokens already flip, so
    // a dark-mode rule here is a literal waiting to be written.
    expect(css).not.toMatch(/\.dark \.br-md-table/);
    expect(ruleBody('.br-md-table thead :where(strong, em, a)')).toContain('color: inherit;');
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

/**
 * The panel's own stylesheet (`components/artifacts/preview-panel.css`, spec 3.8):
 * two grounds for every kind, the 32px bar that yields its meta before its
 * controls, and the fades that keep any part of the panel from popping.
 */
describe('the preview panel stylesheet', () => {
  const panelCss = readFileSync('src/components/artifacts/preview-panel.css', 'utf-8');
  const panelFlat = panelCss.replace(/\s+/g, ' ');
  function panelRule(selector: string): string | null {
    const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    // Anchored after a rule or a comment, so `.br-preview-bar` cannot match the
    // tail of `.br-preview-panel > .br-preview-bar`.
    const match = new RegExp(`(?:^|[}/])\\s*${escaped} \\{([^}]*)\\}`).exec(panelFlat);
    return match ? match[1] : null;
  }

  it('puts every kind on paper, under a band-ground strip', () => {
    expect(panelRule('.br-preview-panel')).toContain('background: var(--background-default);');
    expect(panelRule('.br-preview-body')).toContain('background: var(--background-default);');
  });

  it('draws the bar at the tab height with one hairline', () => {
    const bar = panelRule('.br-preview-bar');
    expect(bar).toContain('height: var(--tab-height);');
    expect(bar).toContain('border-bottom: 1px solid var(--border-subtle);');
  });

  it('lets the meta give way first and never shrinks the controls', () => {
    expect(panelRule('.br-preview-bar__meta')).toContain('flex: 0 100000 auto;');
    expect(panelRule('.br-preview-bar__actions')).toContain('flex: none;');
  });

  // An animation on the hook's marker, not a transition: the panel is inserted
  // and opened before its first style pass, so a transition never ran and the
  // strip appeared in one frame (measured in the harness's panel mode).
  it('fades the strip and the bar with the body, never moving them', () => {
    const fade = panelRule(
      ".br-preview-panel[data-preview-motion='open'] > .br-tabstrip, .br-preview-panel[data-preview-motion='open'] > .br-preview-bar"
    );
    expect(fade).toContain('animation: br-preview-fade-in var(--dur-med) var(--ease-out) both;');
    const motion = readFileSync('src/components/artifacts/usePreviewMotion.ts', 'utf-8');
    expect(motion).toContain('panel.dataset.previewMotion = change;');
    expect(
      panelRule(
        ".br-preview-panel[data-panel-open='false'] > .br-tabstrip, .br-preview-panel[data-panel-open='false'] > .br-preview-bar"
      )
    ).toContain('opacity: 0;');
  });

  it('holds a frame transparent until it has loaded', () => {
    expect(panelRule('.br-preview-frame')).toContain('opacity: 0;');
    expect(panelRule(".br-preview-frame[data-loaded='true']")).toContain('opacity: 1;');
  });

  it('animates only opacity and transform, on the tokens', () => {
    const code = panelCss.replace(/\/\*[\s\S]*?\*\//g, '');
    for (const [, property] of code.replace(/\s+/g, ' ').matchAll(/transition: ([a-z-]+) /g)) {
      expect(['opacity', 'background-color', 'border-left-color']).toContain(property);
    }
    // The one literal: the skeleton waits 200ms before it shows (spec 3.8), a
    // delay rather than a motion tier.
    expect(code.replace(/animation-delay: 200ms;/g, '')).not.toMatch(/\d+ms/);
    expect(code).not.toMatch(/cubic-bezier/);
  });
});

describe('the document front matter and the fold chevron', () => {
  it('sets the front-matter title on the title role, at 400', () => {
    const title = ruleBody('.br-paper-title');
    expect(title).toContain('font-size: var(--text-title);');
    expect(title).toContain('font-weight: var(--text-title--font-weight);');
    expect(title).not.toMatch(/700|28px/);
    expect(ruleBody('.br-paper-subtitle')).toContain('font-size: var(--text-body);');
    expect(ruleBody('.br-paper-byline')).toContain('font-size: var(--text-supporting);');
  });

  it('turns the fold chevron outside the motion-free rung-2 block', () => {
    const resizing = css.indexOf('body.biorouter-window-resizing .biorouter-sidebar-inset-depth');
    const turn = css.indexOf('.br-preview-fold-toggle > svg {');
    expect(resizing).toBeGreaterThan(0);
    expect(turn).toBeGreaterThan(resizing);
    expect(ruleBody('.br-preview-fold-toggle > svg')).toContain(
      'transition: rotate var(--dur-fast) var(--ease-out);'
    );
  });
});

/**
 * FILLS BEFORE LINES in the panel (R-27, owner message 13, design.md 9.11). A
 * box or a control in the preview is a subtle fill with no resting outline; a
 * line stays only where it is structural. The second half is a ratchet: every
 * rule in the panel's own families that draws an edge in a border token is named
 * here, so a new outline cannot arrive without someone writing down why it is
 * structure.
 */
describe('fills before lines in the preview panel', () => {
  const panelCss = readFileSync('src/components/artifacts/preview-panel.css', 'utf-8');
  const paperStart = css.indexOf('/* ── Preview paper');
  // The paper block runs to the focus-surface comment that follows the notebook.
  const paperEnd = css.indexOf("P5 / D-15's focus surface");
  const paperCss = css.slice(paperStart, paperEnd);

  /** Selectors of every rule that draws an edge in a border token. */
  function edgeSelectors(source: string): string[] {
    const code = source.replace(/\/\*[\s\S]*?\*\//g, '');
    const out: string[] = [];
    for (const [, selector, body] of code.matchAll(/([^{}]+)\{([^{}]*)\}/g)) {
      const draws = body
        .split(';')
        .some(
          (declaration) =>
            /^\s*(border(-[a-z-]+)?|box-shadow|outline)\s*:/.test(declaration) &&
            /var\(--border-/.test(declaration)
        );
      if (draws) out.push(selector.replace(/\s+/g, ' ').trim());
    }
    return out;
  }

  it('draws edges only where they are structure, and says which', () => {
    expect(edgeSelectors(panelCss).sort()).toEqual(
      [
        // The pane's divider from the conversation.
        '.br-preview-panel',
        // The 32px bar's bottom edge: the band the file's name sits in.
        '.br-preview-bar',
        // A directory's rail divider, and its header band's edge (aligned with the bar's).
        '.br-preview-rail',
        '.br-preview-rail__filter',
        // A rendered page's own edge on the white paper (PDF, Word, a slide): the
        // document's boundary, drawn outside the page, not a box around UI.
        '.br-preview-page',
        '.artifact-docx-preview .docx-wrapper > section.docx',
        ".artifact-pptx-preview > div > div[style*='box-shadow']",
      ].sort()
    );
    expect(edgeSelectors(paperCss).sort()).toEqual(
      [
        // The title block's divider from the body: a rule in a document.
        '.br-paper-frontmatter',
        // A table's rule under its header: inside the filled table, structure.
        '.br-paper-table.br-md-table th',
      ].sort()
    );
  });

  it('fills the CSV table, with a transparent border, instead of framing it', () => {
    const table = ruleBody('.br-paper-table.br-md-table');
    expect(table).toContain('border-color: transparent;');
    expect(table).toContain('background-color: var(--background-fill);');
    // The fill stops at the padding box, so the transparent border never rings it.
    expect(table).toContain('background-clip: padding-box;');
    expect(ruleBody('.br-paper-table tbody tr:hover td')).toContain(
      'background-color: var(--background-fill-strong);'
    );
  });

  it('gives the notebook one radius for its source, figure and error boxes', () => {
    for (const selector of ['.br-paper-well', '.br-paper-nb-error', '.br-paper-nb-figure']) {
      expect(ruleBody(selector), selector).toContain('border-radius: var(--radius-container);');
    }
    // A figure is a tile of the fill; the ring that used to be its edge is gone.
    const figure = ruleBody('.br-paper-nb-figure');
    expect(figure).toContain('background-color: var(--background-fill);');
    expect(ruleBody('.br-paper-nb-figure > img')).not.toContain('box-shadow');
  });

  it('rests the sheet picker on the fill and steps it to the strong fill', () => {
    const flatPanel = panelCss.replace(/\s+/g, ' ');
    const rule = (selector: string) =>
      new RegExp(
        `(?:^|[}/])\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')} \\{([^}]*)\\}`
      ).exec(flatPanel)?.[1] ?? '';
    const picker = rule('.br-preview-select');
    expect(picker).toContain('background: var(--background-fill);');
    expect(picker).toContain('border: 1px solid transparent;');
    expect(rule(".br-preview-select:hover, .br-preview-select[data-state='open']")).toContain(
      'background: var(--background-fill-strong);'
    );
  });

  it('draws the directory filter and the address field as the app’s filled field', () => {
    const viewer = readFileSync('src/components/artifacts/ArtifactViewer.tsx', 'utf-8');
    expect(viewer).toContain('className="br-field br-preview-rail__input border"');
    const page = readFileSync('src/components/artifacts/WebPagePreview.tsx', 'utf-8');
    expect(page).toMatch(/className="br-field h-6 w-full min-w-0 truncate rounded-element border /);
    // The old bare-input recipe, a ground written by hand, is gone from both.
    expect(page).not.toContain('bg-background-muted px-2');
    expect(viewer).not.toContain('bg-transparent text-secondary text-text-default outline-none');
  });
});

/**
 * The stylesheet PARSES the way these tests read it. Every other assertion in
 * this file reads rule bodies out of the source with a regular expression, and a
 * regular expression happily finds a declaration that a real parser would never
 * see: commit 0807c69db staged the CSV frame's hunks out of order, so main.css
 * carried `padding-left: …; }` and a selector INSIDE a comment, lost the
 * `[data-yield]` rule and the `.br-paper-table` rule, and every test here still
 * passed. A parse is the check that cannot be fooled that way.
 */
describe('the preview families parse', () => {
  const root = postcss.parse(css);
  const selectors = new Set<string>();
  root.walkRules((rule) => {
    selectors.add(rule.selector.replace(/\s+/g, ' '));
  });

  it('has every rule the panel relies on as a rule, not as text in a comment', () => {
    for (const selector of [
      '.br-paper-table-frame',
      '.br-paper-table-frame[data-yield]',
      '.br-paper-table',
      '.br-paper-table.br-md-table',
      '.br-paper-table.br-md-table th',
      '.br-paper-table .br-paper-fill::after',
      '.br-paper-nb',
      '.br-paper-nb > .br-paper-nb-cell:first-child',
      '.br-paper-well',
      '.br-paper-nb-figure',
      '.br-paper-nb-figure > img',
    ]) {
      expect(selectors.has(selector), selector).toBe(true);
    }
  });

  it('keeps declarations out of comments', () => {
    const paperStart = css.indexOf('/* ── Preview paper');
    const paperEnd = css.indexOf("P5 / D-15's focus surface");
    let seen = 0;
    root.walkComments((comment) => {
      const at = comment.source?.start?.offset ?? -1;
      if (at < paperStart || at > paperEnd) return;
      seen += 1;
      // A line that is only a closing brace, or opens a block, is CSS that was
      // swallowed by a comment.
      expect(comment.text, comment.text.slice(0, 60)).not.toMatch(/^\s*\}\s*$|\{\s*$/m);
    });
    expect(seen).toBeGreaterThan(20);
  });
});
