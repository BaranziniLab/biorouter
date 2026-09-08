import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The note clamp, asserted at the SOURCE — and it has to be.
 *
 * `components/ui/note.tsx` folds a notice longer than eight lines behind a fade
 * with a "Show more" control, because past that a notice has stopped being a
 * notice. The mechanism is a `max-height` and a `::after` gradient, and jsdom
 * has neither a layout engine nor resolved custom properties: a render test can
 * mount a 900-line note, read `clientHeight`, and get `0` whether the rule
 * exists or not. The only thing assertable here is the declaration.
 *
 * ⚠ **Why the rule is authored CSS and not a Tailwind arbitrary value — do not
 * "simplify" it back.** The renderer runs with `watch: { ignored: ['**'] }`
 * under `BIOROUTER_NO_HMR`, which is the same signal Tailwind's scanner uses to
 * notice new class strings, so a *newly written* utility can silently fail to
 * generate (`styles/composerFocus.test.ts` records three spellings that were
 * each measured failing in the running app). A clamp that fails to generate
 * does not degrade gracefully — it prints the whole document, which is the
 * defect the clamp exists to fix.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const NOTE = readFileSync(join(__dirname, '../components/ui/note.tsx'), 'utf8');

describe('the note clamp', () => {
  it('declares `--note-max-height` beside the other geometry tokens', () => {
    expect(CSS).toMatch(/--note-max-height:\s*148px;/);
  });

  it('derives the ceiling from the type role rather than picking a number', () => {
    // Eight lines of `--text-supporting` plus the note's own 2 × 10px padding.
    // If either moves, the comment is what says the 148 has to move with it.
    expect(CSS).toMatch(/--text-supporting--line-height:\s*16px;/);
  });

  it('is an authored rule that reads the token', () => {
    const rule = CSS.match(/\.biorouter-note-clamp\s*\{([^}]*)\}/)?.[1];
    expect(rule, 'expected an authored `.biorouter-note-clamp` rule').toBeTruthy();
    expect(rule).toContain('var(--note-max-height)');
    expect(rule).toContain('overflow: hidden');
  });

  /**
   * The fade is what makes a clamp read as "there is more" rather than as text
   * that was cut off mid-sentence.
   */
  it('paints a fade in the note’s own ground, not a hardcoded colour', () => {
    const fade = CSS.match(/\.biorouter-note-clamp::after\s*\{([^}]*)\}/)?.[1];
    expect(fade, 'expected a `::after` fade').toBeTruthy();
    expect(fade).toContain('--note-fade-ground');
    expect(fade).not.toMatch(/#[0-9a-f]{3,8}\b/i);
    expect(fade).not.toMatch(/\brgba?\(/);
  });

  /**
   * A wash cannot terminate a gradient — fading toward `--wash-warning` paints a
   * second wash over the note's own. `--wash-solid-*` is the same mix against
   * the page ground, which is the note's actual composite.
   */
  it('has an opaque companion for every wash', () => {
    for (const tone of ['danger', 'success', 'warning', 'info']) {
      expect(CSS).toContain(`--wash-${tone}:`);
      expect(CSS).toContain(`--wash-solid-${tone}:`);
    }
  });

  it('has its hook in the Note primitive, so the pair cannot drift apart', () => {
    expect(NOTE).toContain('biorouter-note-clamp');
    expect(NOTE).toContain('--note-fade-ground');
  });
});
