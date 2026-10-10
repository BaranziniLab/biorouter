import { describe, expect, it } from 'vitest';
import { readFileSync, readdirSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The composer's popups share one rail, so their type must come from one scale.
 *
 * ⚠ **jsdom cannot check this and never will.** It has no layout engine and
 * never runs the Tailwind compiler, so `text-[10px]` and `text-supporting`
 * produce byte-identical DOM apart from the class attribute, and
 * `getComputedStyle` reports the same thing for both. A component test that
 * rendered a popup and read a font size would pass whichever token was there.
 * The only thing that can hold this is an assertion on the source, which is why
 * this file reads the files rather than rendering them (same reasoning as
 * `measures.test.ts`).
 *
 * The drift being prevented is measured, not hypothetical: the row label in
 * these popups rendered at 13px, 14px and 12px in four popups on the same rail,
 * because two of them restated a size the shared menu row already owned.
 */
const DIR = join(__dirname, '..', 'components', 'bottom_menu');

const popups = readdirSync(DIR).filter((f) => f.endsWith('.tsx') && !f.endsWith('.test.tsx'));

/**
 * The source with every comment form removed.
 *
 * ⚠ This used to strip only lines beginning with `//` or `*`, and that is a
 * third instance of the bug the docstring above already records — except this
 * time the prose it matched was not its own. A JSX comment explaining WHY a
 * path is monospace (`{@literal /}* `ui/Tooltip.tsx` pins font-sans … *{@literal /}`)
 * begins with `{`, survived the filter, and failed the rule it was explaining.
 *
 * A guard that forbids a token must not be able to see that token inside an
 * explanation of the rule, or the only way to document the rule is to avoid
 * naming it — which is how a rule stops being understood.
 */
function codeOnly(source: string): string {
  // Two rules, not three. A JSX comment is a `/* … */` inside braces, so the
  // block rule already removes its body and leaves `{}` behind — a separate
  // JSX branch looked reasonable and was dead code, which mutation testing
  // caught by deleting it and watching nothing fail. A guard with a branch
  // that cannot fail is the thing this file exists to prevent.
  return source
    .replace(/\/\*[\s\S]*?\*\//g, '') // /* block */, which covers {/* JSX */}
    .replace(/^\s*\/\/.*$/gm, ''); // // line
}

describe('composer popup typography', () => {
  it('has popups to check', () => {
    expect(popups.length).toBeGreaterThan(3);
  });

  it.each(popups)('%s uses the type scale, not raw pixel sizes', (file) => {
    // Strip comments so a note explaining the rule cannot trip it — this repo
    // has now three times shipped a guard that matched prose.
    const code = codeOnly(readFileSync(join(DIR, file), 'utf8'));

    const raw = code.match(/text-\[\d+px\]/g);
    expect(raw, `${file} pins a raw font size; use a token from main.css:103-170`).toBeNull();
  });

  it.each(popups)('%s does not re-pin font-sans, which the body already sets', (file) => {
    const code = codeOnly(readFileSync(join(DIR, file), 'utf8'));
    expect(code.includes('font-sans')).toBe(false);
  });

  /**
   * Spec 3.7 acceptance: "No mono anywhere in the composer." Mono is for
   * machine strings (principle 3); a count, a cost or a folder name is not one.
   * The one sanctioned use is the working folder's full path, which lives in the
   * folder chip's tooltip, not on the line.
   */
  const MONO_ALLOWED: Record<string, number> = { 'DirSwitcher.tsx': 1 };
  const composerSurfaces = [
    ...popups.map((file) => join(DIR, file)),
    join(DIR, '..', 'ContextWindowIndicator.tsx'),
    join(DIR, '..', 'settings', 'models', 'bottom_bar', 'ModelsBottomBar.tsx'),
  ];

  it.each(composerSurfaces)('%s draws no visible text in mono', (path) => {
    const code = codeOnly(readFileSync(path, 'utf8'));
    const name = path.split(/[\\/]/).pop() ?? path;
    const uses = code.match(/font-mono/g)?.length ?? 0;
    expect(uses, `${name}: mono is for machine strings in tooltips only`).toBeLessThanOrEqual(
      MONO_ALLOWED[name] ?? 0
    );
  });

  it('keeps the pickers stylesheet on the type tokens', () => {
    const css = codeOnly(readFileSync(join(DIR, 'pickers.css'), 'utf8'));
    expect(css, 'a raw px font size').not.toMatch(/font-size:\s*\d/);
    expect(css, 'a font stack').not.toMatch(/font-family/);
    expect(css).toMatch(/font-size: var\(--text-secondary\)/);
  });

  // Guard the guard. Without this, a stripper that quietly stopped working
  // would make BOTH rules above vacuous and every one of them would still be
  // green — which is the more dangerous half of the bug this file keeps hitting.
  it('strips every comment form, so a rule can be explained by name', () => {
    const withProse = [
      '{/* font-sans is pinned by the tooltip, so a path re-pins font-mono */}',
      '/* text-[13px] would be a raw size */',
      '// font-sans again',
      '<div className="font-mono" />',
    ].join('\n');
    const stripped = codeOnly(withProse);
    expect(stripped).not.toMatch(/font-sans/);
    expect(stripped).not.toMatch(/text-\[13px\]/);
    expect(stripped).toContain('font-mono');
  });
});
