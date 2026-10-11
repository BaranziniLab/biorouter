import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The composer's focus edge: the input's own 1px border turns the brand accent
 * while you are typing in it.
 *
 * ⚠ **This is asserted at the SOURCE, and it has to be.** jsdom has no layout
 * engine, never runs Tailwind, and does not evaluate `:has()` — so a component
 * test can mount the composer, focus the textarea, read `borderColor`, and see
 * the resting value in every case, passing identically whether the rule exists
 * or not. The only thing assertable here is the declaration, so that is what is
 * asserted. Verified in a real browser when it landed: focused border computes
 * to `rgb(184, 90, 50)` = `#b85a32` = `--border-accent` in Parchment light.
 *
 * ⚠ **Why the rule is authored CSS rather than a Tailwind variant at the call
 * site — do not "simplify" it back.** Three spellings were tried on the element
 * (`has-[textarea:focus-visible]:border-border-accent`,
 * `has-[textarea:focus]:inset-shadow-accent`, and
 * `has-[textarea:focus]:border-border-accent`) and only the first ever reached
 * the stylesheet. Each was measured in the running app: the class was on the
 * element, the textarea was a focused descendant, and no matching rule existed.
 * Clearing `node_modules/.vite` did not change it. The renderer runs with
 * `watch: { ignored: ['**'] }` under `BIOROUTER_NO_HMR`, which is the signal
 * Tailwind's scanner uses to notice new class strings — so a *newly written*
 * utility can silently fail to generate. A focus indicator must not depend on
 * class-scanning having worked.
 *
 * ⚠ **`--border-accent`, never `--accent-muted`.** `--accent-muted` is the
 * accent at 8%; 2px of it reads as a thick warm-grey band, not an orange edge,
 * and it was reported that way twice ("a very thick rim", then "not orange, a
 * thick border"). The accent proper is what makes this read as the brand.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const CHAT_INPUT = readFileSync(join(__dirname, '../components/ChatInput.tsx'), 'utf8');
const COMPOSER_CARD = readFileSync(
  join(__dirname, '../components/composer/composerCard.ts'),
  'utf8'
);

describe('the composer focus edge', () => {
  it('is declared as authored CSS, not left to a Tailwind variant', () => {
    // Whitespace-tolerant: prettier may reflow the selector.
    expect(CSS).toMatch(/\.biorouter-composer-card:has\(\s*textarea:focus\s*\)\s*\{/);
  });

  it('paints the full accent, not the 8% muted one', () => {
    const rule = CSS.match(
      /\.biorouter-composer-card:has\(\s*textarea:focus\s*\)\s*\{([^}]*)\}/
    )?.[1];
    expect(rule).toBeTruthy();
    expect(rule).toContain('var(--border-accent)');
    expect(rule).not.toContain('--accent-muted');
  });

  /**
   * `:focus-within` would light the edge when Send or any composer control took
   * focus, which is not "you are typing here".
   */
  it('keys off the textarea, not focus-within', () => {
    expect(CSS).not.toMatch(/\.biorouter-composer-card:focus-within/);
  });

  /**
   * The rule matches nothing without its hook, and the hook lives on the one
   * element that carries the border — so the two are asserted together or the
   * pair can drift apart silently.
   */
  it('has its hook on the composer card in ChatInput', () => {
    // The card's classes live in one module (`composer/composerCard.ts`), which
    // ChatInput's card renders.
    expect(COMPOSER_CARD).toContain("'biorouter-composer-card'");
    expect(CHAT_INPUT).toMatch(/className=\{composerCardClassName\(/);
  });
});
