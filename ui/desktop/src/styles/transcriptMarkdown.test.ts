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
 *   with 8px 12px cells.
 */

// vitest runs with `ui/desktop` as the root.
const css = readFileSync('src/styles/main.css', 'utf-8');

/** The body of the first rule whose selector list is exactly `selector`. */
function ruleBody(selector: string): string | null {
  const escaped = selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&').replace(/\s+/g, '\\s+');
  const match = new RegExp(`(?:^|\\n)${escaped}\\s*\\{([^}]*)\\}`).exec(css);
  return match ? match[1] : null;
}

function block(name: string): string {
  const begin = css.indexOf(`/* @ws WS-TRANSCRIPT begin: ${name} */`);
  const end = css.indexOf(`/* @ws WS-TRANSCRIPT end: ${name} */`);
  expect(begin, `begin marker for ${name}`).toBeGreaterThan(-1);
  expect(end, `end marker for ${name}`).toBeGreaterThan(begin);
  return css.slice(begin, end);
}

describe('markdown type', () => {
  it('sets chat prose on the 14/21 reading line', () => {
    const body = ruleBody('.biorouter-markdown.prose');
    expect(body).toContain('font-size: 14px;');
    expect(body).toContain('line-height: 21px;');
  });

  it('uses 17/24 for chat h1 and h2 and 14/21 below them, all at 600', () => {
    expect(ruleBody('.biorouter-markdown.prose :where(h1, h2, h3, h4, h5, h6)')).toContain(
      'font-weight: 600;'
    );
    const top = ruleBody('.biorouter-markdown.prose :where(h1, h2)');
    expect(top).toContain('font-size: 17px;');
    expect(top).toContain('line-height: 24px;');
    const rest = ruleBody('.biorouter-markdown.prose :where(h3, h4, h5, h6)');
    expect(rest).toContain('font-size: 14px;');
    expect(rest).toContain('line-height: 21px;');
  });

  it("makes a document's h1 the 24/32 title at regular weight", () => {
    const h1 = ruleBody(".biorouter-markdown.prose[data-variant='document'] h1");
    expect(h1).toContain('font-size: 24px;');
    expect(h1).toContain('line-height: 32px;');
    expect(h1).toContain('font-weight: 400;');
  });

  it('uses only the five sizes of the scale', () => {
    const sizes = new Set(
      [...block('markdown').matchAll(/font-size:\s*([^;]+);/g)].map((m) => m[1].trim())
    );
    for (const size of sizes) {
      expect(['12px', '13px', '14px', '17px', '24px', 'var(--text-code)']).toContain(size);
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
});

describe("Crew's code block and table, promoted", () => {
  it('puts fenced code in the well, framed, with a head row over a hairline', () => {
    const code = ruleBody('.br-md-code');
    expect(code).toContain('background-color: var(--background-well);');
    expect(code).toContain('border: 1px solid var(--border-subtle);');
    expect(code).toContain('border-radius: var(--radius-container);');
    const head = ruleBody('.br-md-code-head');
    expect(head).toContain('border-bottom: 1px solid var(--border-subtle);');
    expect(head).not.toContain('background');
    const body = ruleBody('.br-md-code-body');
    expect(body).toContain('padding: 10px 12px;');
    expect(body).toContain('font-size: var(--text-code);');
  });

  it('fades a box that scrolls at its right edge, and not while focused', () => {
    expect(
      ruleBody(".br-md-code-body[data-overflow='true'],\n.br-md-table-scroll[data-overflow='true']")
    ).toContain('mask-image: linear-gradient(to right, black calc(100% - 40px), transparent);');
    expect(
      ruleBody(
        ".br-md-code-body[data-overflow='true']:focus-visible,\n.br-md-table-scroll[data-overflow='true']:focus-visible"
      )
    ).toContain('mask-image: none;');
  });

  it('frames a table at radius 8 with top rules only and 8px 12px cells', () => {
    const table = ruleBody('.br-md-table');
    expect(table).toContain('border-collapse: separate;');
    expect(table).toContain('border-radius: var(--radius-element);');
    expect(table).toContain('font-size: var(--text-secondary);');
    expect(table).toContain('font-variant-numeric: tabular-nums;');
    const cell = ruleBody('.br-md-table :where(th, td)');
    expect(cell).toContain('padding: 8px 12px;');
    expect(cell).toContain('border-block-start: 1px solid var(--border-subtle);');
    const head = ruleBody('.br-md-table thead :where(th, td),\n.br-md-table th[data-head]');
    expect(head).toContain('background-color: var(--background-muted);');
    expect(head).toContain('color: var(--text-muted);');
    expect(head).toContain('font-size: var(--text-supporting);');
    expect(head).toContain('font-weight: 600;');
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
