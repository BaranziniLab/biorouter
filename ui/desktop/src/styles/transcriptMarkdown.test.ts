import { readFileSync } from 'node:fs';
import { describe, expect, it } from 'vitest';

/**
 * The transcript's markdown stylesheet (`main.css`, the WS-TRANSCRIPT
 * "markdown" and "md recipes" blocks). jsdom applies no stylesheet and runs no
 * Tailwind, so a component test cannot see a size, a margin or a fill; these
 * assertions read the source the browser will.
 *
 * What they hold in place (implementation spec 2.1 and 3.6, section 0):
 * - five sizes: chat prose 14/21, chat h1 and h2 17/24, h3 to h6 14/21, a
 *   document's h1 the 24/32 title at 400;
 * - no accent bar beside a heading, no tinted quote;
 * - Crew's code block and table as shared `br-md-*` recipes, on the well and
 *   with 8px 12px cells;
 * - fills before lines (owner message 13, R-27): the block and the table are a
 *   solid fill with a transparent 1px border, and only the table's rules
 *   between rows are drawn.
 */

// vitest runs with `ui/desktop` as the root.
const css = readFileSync('src/styles/main.css', 'utf-8');

/** The body of the first rule whose selector list is exactly `selector`. */
function ruleBody(selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\s+/g, '\\s+');
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css);
  return match ? match[1] : null;
}

/** A `:root` token's value, e.g. `--text-prose` is `14px`. */
function token(name: string): string {
  const match = new RegExp(`\\n\\s*${name}:\\s*([^;]+);`).exec(css);
  expect(match, `token ${name}`).not.toBeNull();
  return match![1].trim();
}

function block(name: string): string {
  const begin = css.indexOf(`/* @ws WS-TRANSCRIPT begin: ${name} */`);
  const end = css.indexOf(`/* @ws WS-TRANSCRIPT end: ${name} */`);
  expect(begin, `begin marker for ${name}`).toBeGreaterThan(-1);
  expect(end, `end marker for ${name}`).toBeGreaterThan(begin);
  return css.slice(begin, end);
}

describe('markdown type', () => {
  it('sets chat prose on the 14/21 reading line, from the prose role', () => {
    const body = ruleBody('.biorouter-markdown.prose');
    expect(body).toContain('font-size: var(--text-prose);');
    expect(body).toContain('line-height: var(--text-prose--line-height);');
    expect(token('--text-prose')).toBe('14px');
    expect(token('--text-prose--line-height')).toBe('21px');
  });

  it('uses 17/24 for chat h1 and h2 and 14/21 below them, all at 600', () => {
    expect(ruleBody('.biorouter-markdown.prose :where(h1, h2, h3, h4, h5, h6)')).toContain(
      'font-weight: 600;'
    );
    const top = ruleBody('.biorouter-markdown.prose :where(h1, h2)');
    expect(top).toContain('font-size: var(--text-subheading);');
    expect(top).toContain('line-height: var(--text-subheading--line-height);');
    expect(token('--text-subheading')).toBe('17px');
    expect(token('--text-subheading--line-height')).toBe('24px');
    const rest = ruleBody('.biorouter-markdown.prose :where(h3, h4, h5, h6)');
    expect(rest).toContain('font-size: var(--text-prose);');
    expect(rest).toContain('line-height: var(--text-prose--line-height);');
  });

  it("makes a document's h1 the 24/32 title at regular weight", () => {
    const h1 = ruleBody(".biorouter-markdown.prose[data-variant='document'] h1");
    expect(h1).toContain('font-size: var(--text-title);');
    expect(h1).toContain('line-height: var(--text-title--line-height);');
    expect(h1).toContain('font-weight: var(--text-title--font-weight);');
    expect(token('--text-title')).toBe('24px');
    expect(token('--text-title--line-height')).toBe('32px');
    expect(token('--text-title--font-weight')).toBe('400');
  });

  it('takes every size from a role token, never a literal', () => {
    // A literal here is a sixth size waiting to happen; the tokens are the
    // scale (spec 2.1), so changing it is one edit in `:root`.
    const sizes = [...block('markdown').matchAll(/font-size:\s*([^;]+);/g)].map((m) => m[1].trim());
    expect(sizes.length).toBeGreaterThan(4);
    for (const size of sizes) {
      expect(size).toMatch(/^var\(--text-(?:prose|subheading|title|code)\)$/);
    }
  });

  it('carries no accent bar on headings and no tinted quote', () => {
    const markdown = block('markdown');
    expect(markdown).not.toMatch(/::before/);
    expect(markdown).not.toContain('--accent-bar');
    expect(markdown).not.toMatch(/letter-spacing:\s*-0\.015em/);
    const quote = ruleBody('.biorouter-markdown.prose blockquote');
    expect(quote).toContain('border-left: 2px solid var(--border-strong);');
    expect(quote).toContain('color: var(--text-muted);');
    expect(quote).not.toContain('background');
  });

  it('keeps a table cell a plain cell: no bold first column', () => {
    expect(css).not.toMatch(/td:first-child\s*\{[^}]*font-weight:\s*600/);
  });

  // Vision V2-06: a task item drops its bullet and hangs the rail's 14px glyph
  // in the gutter (14px glyph + 6px gap = 20px), so its text keeps the edge of
  // the lists around it; the glyph is muted ink, never the accent.
  it('hangs a task glyph in the bullet gutter, muted, at 14px', () => {
    const markdown = block('markdown');
    expect(markdown).toContain('.biorouter-markdown.prose li.task-list-item');
    expect(ruleBody('.biorouter-markdown.prose li.task-list-item')).toContain('list-style: none;');
    const glyph = ruleBody('.biorouter-markdown .br-md-task');
    expect(glyph).toContain('width: 14px;');
    expect(glyph).toContain('margin-right: 6px;');
    expect(glyph).toContain('margin-left: -20px;');
    expect(glyph).toContain('color: var(--text-muted);');
    expect(glyph).toContain('height: var(--text-prose--line-height);');
    expect(ruleBody('.biorouter-markdown .br-md-task > svg')).toContain('width: 14px;');
  });
});

describe("Crew's code block and table, promoted", () => {
  it('puts fenced code on the well as a fill, with no frame and no rule under its head', () => {
    const code = ruleBody('.br-md-code');
    expect(code).toContain('background-color: var(--background-well);');
    // Transparent, never removed: nothing moves, and forced colours draw it.
    expect(code).toContain('border: 1px solid transparent;');
    expect(code).toContain('border-radius: var(--radius-container);');
    const head = ruleBody('.br-md-code-head');
    expect(head).toContain('border-bottom: 1px solid transparent;');
    expect(head).not.toContain('background');
    const body = ruleBody('.br-md-code-body');
    expect(body).toContain('padding: 10px 12px;');
    expect(body).toContain('font-size: var(--text-code);');
  });

  it('fades a box that scrolls at its right edge, and not while focused', () => {
    // One fade for both boxes; a table no longer closes its cut with a hairline.
    expect(block('md recipes').replace(/\/\*[\s\S]*?\*\//g, '')).not.toMatch(/border-right/);
    expect(ruleBody('.br-md-table-scroll')).toContain('border-radius: var(--radius-element);');
    // The fade is as wide as what is still out of view, at most 40px
    // (`--br-overflow-fade`, written by `useOverflowsSideways`). A fixed 40px
    // dimmed a right-aligned last column to nothing when only 10px lay past
    // the edge (final check, 2026-10-12).
    // Whitespace is Prettier's to choose, so it is folded before comparing.
    const fade = ruleBody(
      ".br-md-code-body[data-overflow='true'],\n.br-md-table-scroll[data-overflow='true']"
    )!
      .replace(/\(\s+/g, '(')
      .replace(/\s+\)/g, ')')
      .replace(/\s+/g, ' ');
    expect(fade).toContain(
      'mask-image: linear-gradient(to right, black calc(100% - var(--br-overflow-fade, 40px)), transparent);'
    );
    expect(fade).not.toContain('calc(100% - 40px)');
    expect(
      ruleBody(
        ".br-md-code-body[data-overflow='true']:focus-visible,\n.br-md-table-scroll[data-overflow='true']:focus-visible"
      )
    ).toContain('mask-image: none;');
  });

  it('fills a table at radius 8 with top rules only and 8px 12px cells', () => {
    const table = ruleBody('.br-md-table');
    expect(table).toContain('border-collapse: separate;');
    expect(table).toContain('border: 1px solid transparent;');
    expect(table).toContain('background-color: var(--background-fill);');
    expect(table).toContain('border-radius: var(--radius-element);');
    expect(table).toContain('font-size: var(--text-secondary);');
    expect(table).toContain('font-variant-numeric: tabular-nums;');
    const cell = ruleBody('.br-md-table :where(th, td)');
    expect(cell).toContain('padding: 8px 12px;');
    expect(cell).toContain('border-block-start: 1px solid var(--border-subtle);');
    // The header is the table's own fill, never a band on it (a fill on a
    // fill), and paints it so a sticky header stays opaque.
    const head = ruleBody('.br-md-table thead :where(th, td),\n.br-md-table th[data-head]');
    expect(head).toContain('background-color: var(--background-fill);');
    expect(head).toContain('color: var(--text-muted);');
    expect(head).toContain('font-size: var(--text-supporting);');
    expect(head).toContain('font-weight: 600;');
  });

  it('drops the right-edge fade under forced colours, as Crew does', () => {
    // A mask is alpha: where the system repaints the rest, the fade only hides ink.
    const recipes = block('md recipes');
    const forced = /@media \(forced-colors: active\) \{([\s\S]*?)\n\}/.exec(recipes);
    expect(forced, 'a forced-colors block in the md recipes').not.toBeNull();
    expect(forced![1]).toMatch(
      /\.br-md-code-body\[data-overflow='true'\],\s*\.br-md-table-scroll\[data-overflow='true'\]\s*\{\s*mask-image: none;/
    );
  });

  it('names no crew-* class outside Crew', () => {
    // Comments may cite Crew's source; selectors may not borrow it (principle 11).
    const rules = block('md recipes').replace(/\/\*[\s\S]*?\*\//g, '');
    expect(rules).not.toMatch(/\.crew-/);
    expect(rules).toContain('.br-md-table');
  });
});

describe('the transcript rhythm', () => {
  it('reserves no padding under an assistant reply before a user turn', () => {
    // The `.assistant:has(+ .user)` padding was a third of the 84px gap
    // between turns; the list's `mt-6` owns that gap now (spec 3.6).
    expect(css).not.toContain('.assistant:has(+ .user)');
  });
});

/**
 * The promoted recipes stay Crew's (spec section 0, decision 6.15, critic item
 * 39): one code block and one table look in the app. Read from both sheets and
 * compared declaration by declaration, so a change to Crew's recipe that is not
 * carried over (or the reverse) fails here. The deliberate differences are
 * named rather than skipped: the table cell's 8px 12px against Crew's off-token
 * 11px 16px (Crew defect D11), the role line height, and fills before lines
 * (owner message 13, R-27), which converts these recipes and not Crew's: the
 * frames and the code head's rule are transparent, the table carries the fill,
 * and the table's scroll box rounds its focus edge instead of closing its cut
 * with a hairline.
 */
describe('parity with Crew’s code block and table', () => {
  const crew = readFileSync('src/components/crew/timeline/timeline.css', 'utf-8');

  /** Declarations of the first rule whose selector list is exactly `selector`. */
  function declarations(sheet: string, selector: string): string[] {
    const want = selector.replace(/\s+/g, ' ');
    const rules = sheet.replace(/\/\*[\s\S]*?\*\//g, '').split('}');
    const rule = rules
      .map((chunk) => chunk.split('{'))
      .find(([head]) => head !== undefined && head.trim().replace(/\s+/g, ' ') === want);
    expect(rule, `rule ${selector}`).toBeDefined();
    return rule![1]
      .split(';')
      .map((line) => line.trim().replace(/\s+/g, ' '))
      .filter(Boolean)
      .sort();
  }

  /** What ours declares that Crew's does not, and the reverse. */
  function difference(ours: string, theirs: string) {
    const a = declarations(css, ours);
    const b = declarations(crew, theirs);
    return { added: a.filter((d) => !b.includes(d)), dropped: b.filter((d) => !a.includes(d)) };
  }

  const same: Array<[string, string]> = [
    ['.br-md-code-lang', '.crew-md-code-lang'],
    ['.br-md-code-body', '.crew-md-code-body'],
  ];

  it.each(same)('%s declares exactly what %s does', (ours, theirs) => {
    expect(declarations(css, ours)).toEqual(declarations(crew, theirs));
  });

  it('frames the code block as Crew does, but with a transparent edge (R-27)', () => {
    expect(difference('.br-md-code', '.crew-md-code')).toEqual({
      added: ['border: 1px solid transparent'],
      dropped: ['border: 1px solid var(--border-subtle)'],
    });
    expect(difference('.br-md-code-head', '.crew-md-code-head')).toEqual({
      added: ['border-bottom: 1px solid transparent'],
      dropped: ['border-bottom: 1px solid var(--border-subtle)'],
    });
  });

  it('fills the table where Crew frames it, adding the role line height', () => {
    expect(difference('.br-md-table', '.crew-md-table')).toEqual({
      added: [
        'background-color: var(--background-fill)',
        'border: 1px solid transparent',
        'line-height: var(--text-secondary--line-height)',
      ],
      dropped: ['border: 1px solid var(--border-subtle)'],
    });
  });

  it('scrolls the table as Crew does, with a rounded focus edge and no closing hairline', () => {
    expect(difference('.br-md-table-scroll', '.crew-md-table-scroll')).toEqual({
      added: ['border-radius: var(--radius-element)'],
      dropped: [],
    });
    // Crew closes an overflowing table's cut with a hairline; ours fades the
    // fill instead, through the fade the code body shares.
    expect(declarations(crew, ".crew-md-table-scroll[data-overflow='true']")).toContain(
      'border-right: 1px solid var(--border-subtle)'
    );
  });

  it('differs from Crew’s cell only in the on-token padding', () => {
    expect(declarations(crew, '.crew-md-cell')).toContain('padding: 11px 16px');
    expect(declarations(css, '.br-md-table :where(th, td)')).toContain('padding: 8px 12px');
  });
});
