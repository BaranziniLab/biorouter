import { readdirSync, readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import * as themeTokens from '../../scripts/lib/theme-tokens.mjs';

/**
 * The Knowledge surfaces under fills before lines (owner message 13, R-27; design.md 9.11).
 *
 * The Sources drop target, the graph's filter chips, the paste box, the digest model field and the
 * tier confirmation's inner box are subtle fills with a transparent 1px edge and no resting
 * outline; a recessed ground (a pane header, the graph canvas) rests a filled control on the strong
 * step and hovers one fill deeper. jsdom applies no stylesheet, so each claim is read from the
 * authored CSS at its source, and the ones that are measurements are read from the stylesheet's own
 * tokens in all six scopes, the way `check-contrast.mjs` reads them. `knowledgeFill.browser.test.tsx`
 * measures the same surfaces in Chromium.
 */

type Scope = { decls: Record<string, string>; theme: Record<string, string> };
const tokens = themeTokens as unknown as {
  buildScopes(css: string): Record<string, Scope>;
  resolveHex(name: string, scope: Scope): string | null;
  contrast(a: string, b: string): number;
  blend(fillHex: string, alpha: number, groundHex: string): string;
};

const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const scopes = tokens.buildScopes(CSS);
const stripComments = (source: string) => source.replace(/\/\*[\s\S]*?\*\//g, '');

/** The declarations of the first rule whose selector list is exactly `selector` (comments dropped). */
function rule(selector: string): Record<string, string> {
  const source = stripComments(CSS);
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\s+/g, '\\s+');
  const match = source.match(new RegExp(`(?:^|\\})\\s*${escaped}\\s*\\{([^}]*)\\}`));
  expect(match, `main.css has a ${selector} rule`).toBeTruthy();
  return Object.fromEntries(
    match![1]
      .split(';')
      .map((decl) => decl.trim())
      .filter(Boolean)
      .map((decl) => {
        const at = decl.indexOf(':');
        return [decl.slice(0, at).trim(), decl.slice(at + 1).trim()];
      })
  );
}

describe('the Sources drop target is a fill, not an outline', () => {
  it('rests on the fill with a transparent edge, and steps to the strong fill under the pointer', () => {
    const zone = rule('.br-knowledge-dropzone');
    expect(zone.border).toBe('1px solid transparent');
    expect(zone['background-color']).toBe('var(--background-fill)');
    expect(rule('.br-knowledge-dropzone:hover')['background-color']).toBe(
      'var(--background-fill-strong)'
    );
  });

  it('firms the edge only while a file is held over it, with an inset line, not a border', () => {
    const drag = rule(".br-knowledge-dropzone[data-dragging='true']");
    expect(drag['background-color']).toBe('var(--background-fill-strong)');
    expect(drag['box-shadow']).toBe('inset 0 0 0 1px var(--border-strong)');
    expect(drag['border-color']).toBeUndefined();
  });

  it('draws its glyph plate one ground back, with no edge of its own', () => {
    const plate = rule('.br-knowledge-drop-plate');
    expect(plate['background-color']).toBe('var(--background-default)');
    expect(plate.border).toBeUndefined();
  });
});

describe('the graph filters are the filled control', () => {
  it('rests on the control ground with a transparent 1px edge, and hovers on the strong fill', () => {
    const facet = rule('.br-facet');
    expect(facet.border).toBe('1px solid transparent');
    expect(facet['background-color']).toBe('var(--br-control-ground, var(--background-fill))');
    expect(rule('.br-facet:hover')['background-color']).toBe('var(--background-fill-strong)');
  });

  it('is engaged by the selected wash over the strong fill, which hover firms', () => {
    const engaged = rule(".br-facet[data-engaged='true']");
    expect(engaged['background-color']).toBe('var(--background-fill-strong)');
    expect(engaged['background-image']).toBe(
      'linear-gradient(var(--overlay-selected), var(--overlay-selected))'
    );
    expect(rule(".br-facet[data-engaged='true']:hover")['background-image']).toBe(
      'linear-gradient(var(--overlay-selected-hover), var(--overlay-selected-hover))'
    );
  });
});

describe('a recessed ground rests a filled control on the strong step', () => {
  it('is the shared ground, which sets the control, hover and track grounds', () => {
    const ground = rule('.br-ground-recessed');
    expect(ground['--br-control-ground']).toBe('var(--background-fill-strong)');
    expect(ground['--br-control-hover']).toBe('var(--background-strong)');
    expect(ground['--br-track-ground']).toBe('var(--background-fill-strong)');
  });

  it('is carried by the pane header and the graph canvas, with no rule of Knowledge’s own', () => {
    const root = join(__dirname, '../components/knowledge');
    for (const file of ['KnowledgePaneHeader.tsx', 'graph/KnowledgeGraphPanel.tsx']) {
      const source = readFileSync(join(root, file), 'utf8');
      expect(source, file).toMatch(/className="[^"]*\bbr-ground-recessed\b/);
    }
    // The class Knowledge used to write for itself, and the stop-gap keyed on the empty state's
    // tile, are gone: the primitives read the control ground themselves now.
    expect(CSS).not.toContain('br-knowledge-recessed');
    for (const path of walk(root)) {
      expect(readFileSync(path, 'utf8'), path).not.toContain('br-knowledge-recessed');
    }
  });

  const cases = Object.entries(scopes);
  it('is measured in all six scopes', () => {
    expect(cases).toHaveLength(6);
  });

  it.each(cases)('%s: rest and hover are each a step off the ground they sit on', (name, scope) => {
    const hex = (token: string) => {
      const value = tokens.resolveHex(token, scope);
      expect(value, `${name}: ${token} is a hex`).toBeTruthy();
      return value!;
    };
    const strongFill = hex('--background-fill-strong');
    // The rest fill is the muted value there (1.00:1), which is why the strong step is the rest.
    expect(tokens.contrast(hex('--background-fill'), hex('--background-muted'))).toBeLessThan(1.02);
    for (const ground of ['--sidebar', '--background-muted']) {
      expect(
        tokens.contrast(strongFill, hex(ground)),
        `${name}: rest vs ${ground}`
      ).toBeGreaterThanOrEqual(1.08);
    }
    // The hover has a step to take: the strong background is off the strong fill.
    expect(tokens.contrast(hex('--background-strong'), strongFill)).toBeGreaterThanOrEqual(1.08);
    // The tier badge's hover (a Public badge rests on the strong fill) is that same step.
    expect(tokens.contrast(hex('--background-strong'), strongFill)).toBeGreaterThan(
      tokens.contrast(hex('--background-medium'), strongFill)
    );
  });
});

describe('an engaged filter stays legible and a step off a resting one', () => {
  it('measures the wash the stylesheet declares (14%, and 19% under the pointer)', () => {
    const percent = (name: string) =>
      Number(
        CSS.match(
          new RegExp(`${name}:\\s*color-mix\\(in oklab, var\\(--text-default\\) (\\d+)%`)
        )?.[1]
      );
    expect(percent('--overlay-selected')).toBe(14);
    expect(percent('--overlay-selected-hover')).toBe(19);
  });

  it.each(Object.entries(scopes))(
    '%s: ink on the selected wash, and the wash is a step',
    (name, scope) => {
      const hex = (token: string) => {
        const value = tokens.resolveHex(token, scope);
        expect(value, `${name}: ${token} is a hex`).toBeTruthy();
        return value!;
      };
      const ink = hex('--text-default');
      const rest = hex('--background-fill');
      const strong = hex('--background-fill-strong');
      // `--overlay-selected` is the default ink at 14% (hover 19%), composited over the strong fill.
      const engaged = tokens.blend(ink, 0.14, strong);
      const engagedHover = tokens.blend(ink, 0.19, strong);
      expect(tokens.contrast(ink, engagedHover)).toBeGreaterThanOrEqual(4.5);
      expect(tokens.contrast(engaged, rest)).toBeGreaterThanOrEqual(1.25);
    }
  );
});

/**
 * A chip on a washed row (vision round 2, V3-R2-5). The neutral Badge rests on the strong fill, and
 * a hovered list row is the page ink at 5%, so under the pointer the change log's kind chip and the
 * manager's Built-in, format, Private and Primary chips lost their boxes. R-27 usage rule 3 (a fill
 * never sits on a fill) says where they go: the page ground.
 */
describe('a chip on a washed row steps back to the page ground', () => {
  const WASHED = ':is(:hover, :focus-visible, :has(:focus-visible))';
  const SELECTOR = `.biorouter-list-row${WASHED} :is(.br-wash-chip, .br-wash-chips > span)`;

  it('names the chip, in the same three states the shared rows wash in', () => {
    expect(rule(SELECTOR)['background-color']).toBe('var(--background-default)');
    // The shared rows block washes (and steps its controls back) in exactly these states; this
    // rule must not wash earlier or later than the row does.
    expect(stripComments(CSS)).toContain(`.biorouter-list-row${WASHED},`);
  });

  it('is unlayered, so it beats the Badge utility and the privacy pill rule', () => {
    const at = CSS.indexOf('@ws WS-KNOWLEDGE begin: washed-row chips');
    expect(at).toBeGreaterThan(0);
    // Not inside an @layer: the only `@layer` blocks before it close before it, which is what
    // keeps `bg-background-fill-strong` (a utilities-layer class) under this rule.
    const before = stripComments(CSS.slice(0, at));
    let depth = 0;
    for (const ch of before) {
      if (ch === '{') depth += 1;
      if (ch === '}') depth -= 1;
    }
    expect(depth, 'the rule sits at the top level of the stylesheet').toBe(0);
    // And it comes after the privacy pill's washed-row rule, which has the same specificity.
    expect(at).toBeGreaterThan(
      CSS.indexOf(
        '.biorouter-list-row:is(:hover, :focus-visible, :has(:focus-visible)) .br-privacy-pill'
      )
    );
  });

  it('is carried by the kind chip, the Primary badge and the manager chip line', () => {
    const root = join(__dirname, '../components/knowledge');
    expect(readFileSync(join(root, 'changelog/ChangeKindChip.tsx'), 'utf8')).toContain(
      "'br-wash-chip'"
    );
    const manager = readFileSync(join(root, 'KBSelector/KBManagerDialog.tsx'), 'utf8');
    expect(manager).toMatch(/<Badge className="br-wash-chip">/);
    expect(manager).toMatch(/className="br-wash-chips flex items-center gap-2"/);
  });

  it.each(Object.entries(scopes))(
    '%s: the page ground is a more visible box than the strong fill on the wash',
    (name, scope) => {
      const hex = (token: string) => {
        const value = tokens.resolveHex(token, scope);
        expect(value, `${name}: ${token} is a hex`).toBeTruthy();
        return value!;
      };
      const page = hex('--background-default');
      // `--overlay-hover` is the default ink at 5% over the row's ground (the page, in a dialog or
      // a drawer).
      const wash = tokens.blend(hex('--text-default'), 0.05, page);
      const strong = tokens.contrast(hex('--background-fill-strong'), wash);
      const chip = tokens.contrast(page, wash);
      expect(chip, `${name}: page ground vs wash`).toBeGreaterThan(strong);
      // As visible a box as a resting fill is on the page, the step every fill in the app is
      // drawn at (1.09:1 in light), so a chip under the pointer is no fainter than at rest.
      expect(chip, `${name}: the same step as a resting fill on the page`).toBeGreaterThanOrEqual(
        tokens.contrast(hex('--background-fill'), page) - 0.01
      );
      // The chip's ink is read on the page ground, as it is everywhere else.
      expect(tokens.contrast(hex('--text-muted'), page)).toBeGreaterThanOrEqual(4.5);
    }
  );
});

/**
 * Every edge the Knowledge components draw in a colour, by file. A line stays only where it is
 * structural (a band's or strip's bottom edge, the footer rule over a scrolling body, a pane
 * divider, a menu's separator) or the hairline of a floating surface (the node and edge cards,
 * the popovers); an in-page box or control is a fill with a TRANSPARENT edge, which this does not
 * count. A new outline fails here, and the fix is a fill, not a longer list.
 */
const EDGES: Record<string, string[]> = {
  'KnowledgePaneHeader.tsx': ['border-border-subtle'], // the 44px band's bottom edge
  'IngestPanel/IngestPanel.tsx': ['border-border-subtle'], // the footer rule over a scrolling body
  'IngestPanel/IngestModelPicker.tsx': ['border-border-subtle'], // a rule inside the popover
  'KBSelector/KBManagerDialog.tsx': ['border-border-subtle'], // the pinned header over the list
  'KBSelector/KBSelectorMenu.tsx': ['border-border-subtle'], // a rule inside the popover
  'changelog/ChangeLogDrawer.tsx': ['border-border-subtle'], // the pinned filter row over the list
  'lint/LintDrawer.tsx': ['border-border-subtle'], // the pinned counts row over the list
  'graph/GraphFacetStrip.tsx': ['border-border-subtle'], // the 44px strip's bottom edge
  'graph/GraphLegend.tsx': ['border-border-subtle', 'border-text-muted'], // the rail's divider; a dashed mark
  'graph/NodePreview.tsx': ['border-border-subtle'], // a floating card
  'graph/EdgePreview.tsx': ['border-border-subtle'], // a floating card
  'graph/ForceGraphCanvas.tsx': ['border-border-subtle'], // a hidden probe that reads the token
};

function walk(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return walk(path);
    return /\.tsx$/.test(entry.name) && !/\.test\./.test(entry.name) ? [path] : [];
  });
}

describe('the Knowledge components draw no outline of their own', () => {
  const root = join(__dirname, '../components/knowledge');
  const edgeRe = /(?<![\w-])(?:hover:|focus:|focus-within:)?border-(?:border|text|accent)-[a-z]+/g;

  it('draws a coloured edge only where it is structural or floating', () => {
    const found: Record<string, string[]> = {};
    for (const path of walk(root)) {
      // Class strings, not the comments that explain them.
      const source = readFileSync(path, 'utf8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*\/\/.*$/gm, '');
      const edges = [...new Set(source.match(edgeRe) ?? [])].sort();
      if (edges.length > 0) found[path.slice(root.length + 1)] = edges;
    }
    const want = Object.fromEntries(Object.entries(EDGES).map(([k, v]) => [k, [...v].sort()]));
    expect(found).toEqual(want);
  });

  it('keeps no hover outline and no inset ring on a control', () => {
    for (const path of walk(root)) {
      const source = readFileSync(path, 'utf8')
        .replace(/\/\*[\s\S]*?\*\//g, '')
        .replace(/^\s*\/\/.*$/gm, '');
      expect(source, path).not.toMatch(/hover:inset-ring|hover:border-|bg-background-medium/);
    }
  });
});
