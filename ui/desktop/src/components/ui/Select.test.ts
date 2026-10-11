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
    expect(SELECT).toMatch(
      /control:\s*\(base, state\)\s*=>\s*\(\{\s*\.\.\.base,\s*minHeight:\s*0,/
    );
  });

  /**
   * R-27 (design.md 9.11): the trigger is Input's filled field. It rests on the control ground
   * (`--background-fill`, or the page ground inside a filled box) with a TRANSPARENT edge (forced
   * colours still paint it), steps to `--background-fill-strong` under the pointer unless focused
   * or disabled, and firms to `--border-focus` while focused. Emotion writes all of it unlayered,
   * so no utility can put a resting line back.
   */
  it('rests filled with no outline and firms an edge on focus', () => {
    const stylesBlock = SELECT.match(/styles=\{\{([\s\S]*?)\n {6}\}\}/)?.[1] ?? '';
    expect(stylesBlock).toMatch(
      /borderColor:\s*state\.isFocused\s*\?\s*'var\(--border-focus\)'\s*:\s*'transparent'/
    );
    expect(stylesBlock).toMatch(
      /backgroundColor:\s*'var\(--br-control-ground, var\(--background-fill\)\)'/
    );
    expect(stylesBlock).toMatch(
      /state\.isFocused \|\| state\.isDisabled\s*\?\s*\{\}\s*:\s*\{ '&:hover': \{ backgroundColor: 'var\(--background-fill-strong\)' \} \}/
    );
    const control = SELECT.match(/control: \(\{ isDisabled \}\) =>([\s\S]*?)\.join/)?.[1];
    expect(control).toMatch(/'flex h-control-md w-full items-center rounded-element border px-2'/);
    // No resting line, no inset-ring whisper, no ground utility to fight the styles above.
    expect(control).not.toMatch(/inset-ring|bg-background-/);
    expect(SELECT).not.toMatch(/'border-border-emphasized'|var\(--border-control\)/);
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

  /**
   * Spec 2.6 / principle 9: every menu opens the same way. The react-select menu has no Radix
   * `data-state`, so it carries `.br-select-menu`, an authored keyframe on the same values
   * (opacity, scale .97, 4px toward the side it hangs from, `--dur-fast-max`, `--ease-out`).
   */
  it('opens the menu with the shared entrance, from the side it hangs from', () => {
    expect(SELECT).toMatch(/menu:\s*\(\{ placement \}\)/);
    expect(SELECT).toContain('br-select-menu');
    expect(SELECT).toContain("placement === 'top'");
    expect(SELECT).toContain('br-select-menu--top');
    const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
    const rule = css.match(/\n\.br-select-menu \{([^}]*)\}/)?.[1] ?? '';
    expect(rule).toMatch(/animation:\s*br-select-menu-in var\(--dur-fast-max\) var\(--ease-out\)/);
    const keyframes = css.match(/@keyframes br-select-menu-in \{([\s\S]*?)\n\}/)?.[1] ?? '';
    expect(keyframes).toMatch(/opacity:\s*0/);
    expect(keyframes).toMatch(/scale\(0\.97\)/);
    expect(css).toMatch(/\.br-select-menu--top \{[^}]*--br-select-menu-from:\s*4px/);
  });

  /** The rung itself still lives on the class, as the one source of truth. */
  it('keeps the 32px rung on the control class', () => {
    expect(SELECT).toMatch(/'flex h-control-md w-full items-center/);
  });

  // V4-11: Emotion's unlayered `fontSize: inherit` on every option beats the row's own
  // `text-secondary`, so the option takes the MENU's size. The menu carries the menu row's role,
  // so a select's options are 13px like every DropdownMenu row beside them, not 14px.
  it('sizes its menu as every menu row: text-secondary, not text-body', () => {
    const menu = SELECT.match(/menu: \(\{ placement \}\) =>\s*`([^`]*)`/);
    expect(menu, 'the menu classNames entry').toBeTruthy();
    expect(menu![1]).toMatch(/\btext-secondary\b/);
    expect(menu![1]).not.toMatch(/\btext-body\b/);
    expect(SELECT).toMatch(/noOptionsMessage: \(\) => '[^']*\btext-secondary\b/);
  });

  it('marks the chosen option with the check and the wash, never with weight', () => {
    const selected = SELECT.match(/if \(isSelected\) \{[\s\S]*?return `([^`]*)`/);
    expect(selected).toBeTruthy();
    expect(selected![1]).toMatch(/tint-selected/);
    expect(selected![1]).not.toMatch(/font-(medium|semibold|bold)/);
  });
});
