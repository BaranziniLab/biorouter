import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { TAB_MIN_WIDTH } from '../components/Layout/yieldLadder';

/**
 * Rung 3 of the yield ladder, at the source: **a tab shrinks to a floor, then
 * the strip SCROLLS. It never shrinks the label away.**
 *
 * The defect this guards was reported as "chat tab labels collapse to a single
 * letter": with six chats open at 1440 and the artifact panel showing, the
 * strip read `R.` `R.` `R.` `R.` `B..` — four different conversations wearing
 * the same name. The floor was 88px and a tab spends 73px of that on padding,
 * the leading kind/privacy glyph and the close control, so the title got 15px.
 *
 * ⚠ **jsdom can prove none of this and never could.** It has no layout engine,
 * so a test that mounts `ChatTabStrip` with six tabs and reads
 * `getBoundingClientRect()` sees zeroes; it never runs Tailwind; and the rules
 * here are authored CSS the component does not name. What IS checkable — and is
 * exactly what went wrong — is that the floor, the token and the constant the
 * ladder reasons with all say the same thing, and that the two rules which turn
 * a too-narrow strip into a SCROLLING one rather than a shrinking or wrapping
 * one are still present. The pixel outcome is verified by driving the real app.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');

/** A declaration's value, read off the token block. */
function token(name: string): string {
  const match = CSS.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1].trim();
}

/** One rule's body, comments stripped so prose quoting a property cannot satisfy a rule. */
function ruleBody(selector: string): string {
  const withoutComments = CSS.replace(/\/\*[\s\S]*?\*\//g, '');
  const index = withoutComments.indexOf(`\n${selector} {`);
  if (index === -1) throw new Error(`${selector} has no rule in main.css`);
  const open = withoutComments.indexOf('{', index);
  const close = withoutComments.indexOf('}', open);
  return withoutComments.slice(open + 1, close);
}

describe('the tab shrink floor', () => {
  /**
   * Asserted as an exact string for the same reason `--measure-chat` is: the
   * failure being guarded against is a NARROWING, and every loose matcher
   * (`/\d+px/`, `/^1/`) is satisfied by the 88px being ruled out.
   */
  it('is 136px, not the 15px-of-label 88 it replaced', () => {
    expect(token('tab-min-width')).toBe('136px');
  });

  /**
   * ONE number, in two languages. `yieldLadder.ts` reasons about the floor in
   * TypeScript (`shouldShowTabOverflowMenu` is the rung that fires once tabs
   * stop fitting) while the stylesheet enforces it; a copy that drifted would
   * leave the ladder measuring a strip it no longer describes, and nothing on
   * screen would look wrong until the ▾ menu appeared at the wrong width.
   */
  it('is the same number the ladder reasons with', () => {
    expect(`${TAB_MIN_WIDTH}px`).toBe(token('tab-min-width'));
  });

  /**
   * The floor has to be READ, not restated. `.br-tab` carrying its own literal
   * is how the 88 above outlived the reasoning that produced it.
   */
  it('is read from the token by the tab itself', () => {
    expect(ruleBody('.br-tab')).toMatch(/min-width:\s*var\(--tab-min-width\)/);
  });

  /** A floor above the ceiling would make every tab exactly one width. */
  it('leaves room between the floor and the tab’s maximum width', () => {
    const max = ruleBody('.br-tab').match(/max-width:\s*(\d+)px/);
    if (!max) throw new Error('.br-tab declares no max-width');
    expect(TAB_MIN_WIDTH).toBeLessThan(Number(max[1]));
  });
});

describe('the strip that the floor hands over to', () => {
  /**
   * Raising the floor is only half the fix: a floor with nowhere to overflow to
   * is a clipped strip. These two declarations are what make a too-narrow strip
   * scroll — and `flex-wrap: nowrap` is the one that must never become `wrap`,
   * because a wrapped second row moves every tab under the cursor.
   */
  it('scrolls rather than wrapping or clipping', () => {
    const strip = ruleBody('.br-tabstrip');
    expect(strip).toMatch(/flex-wrap:\s*nowrap/);
    expect(strip).toMatch(/overflow-x:\s*auto/);
  });

  /**
   * The preview panel nests its tablist inside the strip, so the scroll box is
   * a different element there. It was `overflow-hidden` once — its tabs did not
   * scroll at all, they were simply unreachable — and the same floor now
   * applies to it, which makes the scroll it hands over to load-bearing.
   */
  it('applies the same rule to the panel’s nested tablist', () => {
    expect(ruleBody('.br-tabstrip__scroll')).toMatch(/flex-wrap:\s*nowrap/);
  });

  /**
   * The label truncates from the END. `text-overflow: ellipsis` on an
   * `ltr` box keeps the LEADING characters, which is the half of the reported
   * defect that was already right: the fix is that there are now enough of them
   * to read (`Research…`), not that the ellipsis moved.
   */
  it('truncates the label from the end, keeping its opening characters', () => {
    const label = ruleBody('.br-tab__label');
    expect(label).toMatch(/text-overflow:\s*ellipsis/);
    expect(label).toMatch(/white-space:\s*nowrap/);
    expect(label).toMatch(/overflow:\s*hidden/);
    // `min-width: 0` is what lets the flex item shrink to the ellipsis at all;
    // without it the label refuses to shrink and pushes the close control out.
    expect(label).toMatch(/min-width:\s*0/);
  });
});
