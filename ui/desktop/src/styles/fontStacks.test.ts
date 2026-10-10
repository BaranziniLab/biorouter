import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { BRAND_SANS } from '../components/icons/brandFont';

/**
 * The font stacks that are written out in more than one place.
 *
 * Nearly every face in the app is late-bound through a token, so it cannot
 * drift. Two cannot be, and each is a documented exception rather than an
 * oversight:
 *
 *  - the terminal's, because **xterm measures glyph widths itself** and so
 *    resolves no `var()`. Its stack has to be a literal string in TypeScript.
 *  - the brand mark's, because it is drawn into an SVG `fontFamily` attribute.
 *    That one is now a single exported constant, so the assertion below is
 *    about it not silently becoming two again.
 *
 * A comment saying "keep this identical to X" is not a guard, and until this
 * file existed that comment was the only thing holding the terminal stack to
 * `--font-mono`. The consequence of drift is quiet and specific: a command
 * copied out of a chat code block would render in a different face from the
 * terminal it is pasted into, at the same nominal size, with different glyph
 * widths — which reads as a rendering bug in the terminal rather than as two
 * stacks disagreeing.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const TERMINAL_DOCK = readFileSync(join(__dirname, '../components/InAppTerminalDock.tsx'), 'utf8');
const LANDING_CSS = readFileSync(join(__dirname, '../../../../landing/site.css'), 'utf8');
const DOCUMENT_PREVIEW = readFileSync(
  join(__dirname, '../components/artifacts/DocumentPreview.tsx'),
  'utf8'
);

/** A font stack as a comparable list: quoting and whitespace are not identity. */
function families(stack: string): string[] {
  return stack
    .replace(/\s+/g, ' ')
    .split(',')
    .map((family) => family.trim().replace(/^['"]|['"]$/g, ''))
    .filter(Boolean);
}

function cssToken(name: string): string {
  const match = CSS.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!match) throw new Error(`--${name} is not declared in main.css`);
  return match[1];
}

describe('font stacks written out more than once', () => {
  it('sets the terminal in exactly the app monospace face', () => {
    const declared = TERMINAL_DOCK.match(/const TERMINAL_FONT =\s*\n?\s*'([^']+)'/);
    if (!declared) throw new Error('TERMINAL_FONT is no longer a single-quoted literal');

    // Family for family, in order. A stack that merely SHARES its first family
    // is not the same stack: the fallbacks decide what a machine without SF
    // Mono actually renders, which is most of them.
    expect(families(declared[1])).toEqual(families(cssToken('font-mono')));
  });

  it('keeps one brand face rather than a copy per mark', () => {
    const marks = ['BioRouterMark.tsx', 'BioRouterWordmark.tsx'].map((file) =>
      readFileSync(join(__dirname, '../components/icons', file), 'utf8')
    );
    for (const source of marks) {
      expect(source).toContain("from './brandFont'");
      expect(source).not.toMatch(/const SANS =\s*\n?\s*'Inter/);
    }
    expect(families(BRAND_SANS)[0]).toBe('Inter');
  });
});

/**
 * The interface face (redesign 2026-10, spec 2.1, owner decision of 2026-10-09):
 * the app wears the WEBSITE's Arial-first stack, so a screenshot of the app and
 * the page that sells it are set in one face. The two stylesheets are in
 * different packages and nothing else ties them together, so a change to one
 * stack would otherwise go unnoticed until someone compared screenshots.
 *
 * Arial draws 400 and 700 only, which is why the type scale has two visible
 * weights (`typeScale.test.ts`). Inter is the brand face, used for the wordmark
 * and the BR mark alone.
 */
describe('the interface face is the website face', () => {
  const body = families(cssToken('font-body'));

  it('sets the body stack family for family as landing/site.css --font', () => {
    const landing = LANDING_CSS.match(/[\s;{]--font:\s*([^;]+);/);
    if (!landing) throw new Error('--font is not declared in landing/site.css');
    expect(body).toEqual(families(landing[1]));
  });

  it('leads with Arial and never names Inter, which is the mark only', () => {
    expect(body[0]).toBe('Arial');
    expect(body).not.toContain('Inter');
    expect(body.at(-1)).toBe('sans-serif');
  });

  it('keeps --font-heading and --font-sans as aliases of the body role', () => {
    expect(cssToken('font-heading').trim()).toBe('var(--font-body)');
    expect(cssToken('font-sans').trim()).toBe('var(--font-body)');
  });

  /**
   * A sandboxed document cannot read a custom property, so the preview's iframe
   * carries the stack as a resolved string. It may drop a family the body names
   * (a fallback it never needs) but it may not add one, reorder them, or start
   * with anything but Arial: that is how a third UI face got into the app.
   */
  it('sets the document preview iframe in the body stack, in order', () => {
    const stacks = [...DOCUMENT_PREVIEW.matchAll(/font-family:\s*([^;]+);/g)]
      .map((m) => families(m[1]))
      .filter((stack) => !stack.some((family) => /mono/i.test(family)));
    expect(stacks.length).toBeGreaterThan(0);
    for (const stack of stacks) {
      expect(stack[0]).toBe('Arial');
      let from = 0;
      for (const family of stack) {
        const at = body.indexOf(family, from);
        expect(at, `${family} is not in the body stack, in order`).toBeGreaterThanOrEqual(0);
        from = at + 1;
      }
    }
  });
});
