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
  it('sets the control ground, and moves the hover one fill deeper', () => {
    expect(rule('.br-knowledge-recessed')['--br-control-ground']).toBe(
      'var(--background-fill-strong)'
    );
    const hover = rule(
      '.br-knowledge-recessed .br-fill-hover:hover,\n.br-knowledge-recessed .br-fill-hover:active'
    );
    expect(hover['background-color']).toBe('var(--background-strong)');
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
