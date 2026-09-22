import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The select trigger is the `<Input>` chrome class for class, including the 32px
 * md rung — and for a long time it was not, because react-select outranked it.
 *
 * ⚠ **This is asserted at the SOURCE, and it has to be.** jsdom has no layout
 * engine: a test that renders a `<Select>` and reads `height` sees `0px` whether
 * the control is 32px or 38px, so a component test asserting the `h-8` class
 * passes happily while the real control renders six pixels too tall. That is
 * exactly what happened. The only thing assertable here is the declaration.
 *
 * Measured in the running app (Chrome, Parchment light, the two selects in the
 * model dialog): BEFORE `height 38 / min-height 38px` against `class="… h-8…"`;
 * AFTER `height 32 / min-height 0px`.
 *
 * ⚠ **Why the override is necessary — do not "simplify" it away.** react-select's
 * `control` style function emits `minHeight: spacing.controlHeight` (38) OUTSIDE
 * its `unstyled ? {} : {…}` branch, so `unstyled` does not remove it. Emotion
 * injects that rule unlayered while Tailwind's `h-8` lives in `@layer utilities`,
 * and min-height beats height regardless of who wins the cascade. Standing the
 * floor down is the only way the class on the control means anything.
 */
const SELECT = readFileSync(join(__dirname, 'Select.tsx'), 'utf8');

describe('the shared Select', () => {
  it('stands react-selects 38px min-height floor down', () => {
    // Whitespace-tolerant: prettier may reflow the arrow body.
    expect(SELECT).toMatch(/control:\s*\(base\)\s*=>\s*\(\{\s*\.\.\.base,\s*minHeight:\s*0\s*\}\)/);
  });

  /**
   * The override belongs in `styles` (Emotion), not `classNames` (Tailwind): a
   * `min-h-0` utility sits in `@layer utilities` and would lose to react-select's
   * unlayered Emotion rule no matter where it appeared in the class string.
   */
  it('places the override in the styles API, not the classNames API', () => {
    const stylesBlock = SELECT.match(/styles=\{\{([\s\S]*?)\n {6}\}\}/)?.[1];
    expect(stylesBlock).toBeTruthy();
    expect(stylesBlock).toContain('minHeight: 0');
  });

  /** The rung itself still lives on the class, as the one source of truth. */
  it('keeps the 32px rung on the control class', () => {
    expect(SELECT).toMatch(/'flex h-control-md w-full items-center/);
  });
});
