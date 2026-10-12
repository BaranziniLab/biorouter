import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The composer's WORKING edge: while a turn runs, the card's own 1px border
 * carries a travelling segment of the brand accent.
 *
 * It replaced a row above the composer holding a breathing dot, which was
 * byte-identical to `TurnActivityIndicator`'s, sat 8px out of line with it
 * (each anchored to a different grid), and cost a ~34px layout shift on Send.
 *
 * ⚠ **Asserted at the SOURCE, like its sibling `composerFocus.test.ts`, and for
 * the same reason.** jsdom has no layout engine, never runs Tailwind, does not
 * evaluate `:has()`, does not resolve `color-mix()`, and does not run
 * `@property`-registered animations. A component test can mount the composer,
 * set the working state, read `borderColor` and see the resting value in every
 * case — passing identically whether any of this exists or not. The only thing
 * assertable here is the declaration.
 *
 * Verified in a real browser when it landed (Chrome, compiled tokens):
 *  - `[data-working]` present   -> `::after` animates `br-composer-working`
 *  - focused + working          -> base border resolves to a 30% accent mix
 *  - focused + idle             -> base border resolves to the full accent
 *  - `prefers-reduced-motion`   -> `animation-name: none`, ring stays dashed
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const CHAT_INPUT = readFileSync(join(__dirname, '../components/ChatInput.tsx'), 'utf8');
const COMPOSER_CARD = readFileSync(
  join(__dirname, '../components/composer/composerCard.ts'),
  'utf8'
);
const BASE_CHAT = readFileSync(join(__dirname, '../components/BaseChat.tsx'), 'utf8');

/** The `::after` rule body, which carries the sweep itself. */
const sweepRule = () =>
  CSS.match(/\.biorouter-composer-card\[data-working='true'\]::after\s*\{([^}]*)\}/)?.[1];

/** The focused-and-working rule body, which dims the base border. */
const focusedWorkingRule = () =>
  CSS.match(
    /\.biorouter-composer-card\[data-working='true'\]:has\(\s*textarea:focus\s*\)\s*\{([^}]*)\}/
  )?.[1];

describe('the composer working edge', () => {
  it('is declared as authored CSS, not left to a Tailwind variant', () => {
    // A newly written arbitrary utility can silently fail to reach the
    // stylesheet under BIOROUTER_NO_HMR. A turn indicator may not depend on
    // class-scanning having worked.
    expect(sweepRule()).toBeTruthy();
  });

  it('registers the angle so it can animate at all', () => {
    // An unregistered custom property is not interpolable; without this the
    // keyframes would jump from 0deg to 360deg with nothing in between.
    expect(CSS).toMatch(/@property\s+--br-working-angle\s*\{/);
    const decl = CSS.match(/@property\s+--br-working-angle\s*\{([^}]*)\}/)?.[1];
    expect(decl).toContain('<angle>');
  });

  it('drives the sweep from named keyframes', () => {
    expect(CSS).toMatch(/@keyframes\s+br-composer-working\s*\{/);
    expect(sweepRule()).toMatch(/animation:\s*br-composer-working/);
  });

  describe('colour follows the theme family', () => {
    /**
     * The whole point of the requirement: Parchment is coral, Alma Mater is
     * teal, Roche Limit is orange, and each differs again in dark. A literal
     * would pin the edge to one family and silently break the other two — and
     * nothing else in the app would fail.
     */
    it('paints --border-accent and nothing else', () => {
      expect(sweepRule()).toContain('var(--border-accent)');
    });

    it('contains no literal PAINT colour anywhere in the working-edge block', () => {
      const block = CSS.slice(
        CSS.indexOf('@property --br-working-angle'),
        CSS.indexOf('.br-tab {')
      );
      expect(block.length).toBeGreaterThan(0);

      // The `#000` inside `mask`/`-webkit-mask` is NOT a theme colour and must
      // not be flagged: a mask reads only the alpha channel, so the hex is an
      // opacity carrier and any opaque value is identical. Strip those
      // declarations, then nothing that PAINTS may carry a literal.
      const painted = block.replace(/-?(webkit-)?mask[^;]*;/g, '');

      expect(painted).not.toMatch(/#[0-9a-fA-F]{3,8}\b/);
      expect(painted).not.toMatch(/\brgba?\(/);
      expect(painted).not.toMatch(/\bhsla?\(/);
      expect(painted).not.toMatch(/\b(orange|coral|teal|white|black)\b/);
    });

    it('never reaches for --accent-muted, which is the accent at 8%', () => {
      // Same trap the focus edge fell into twice: 8% reads as a warm grey band,
      // not as the brand.
      const block = CSS.slice(
        CSS.indexOf('@property --br-working-angle'),
        CSS.indexOf('.br-tab {')
      );
      expect(block).not.toContain('--accent-muted');
    });
  });

  describe('focus and working stay distinguishable', () => {
    /**
     * THE CORE OF THE FEATURE. The composer autofocuses, so `--border-accent`
     * is its RESTING appearance. If the working state left the base border at
     * full accent, a full-accent travelling segment would sit on a full-accent
     * edge — a uniform accent border, which is exactly the focus state. The
     * base must therefore dim so the segment has something to be brighter than.
     */
    it('dims the base border when the composer is BOTH focused and working', () => {
      const rule = focusedWorkingRule();
      expect(rule).toBeTruthy();
      expect(rule).toMatch(/border-color:\s*color-mix\(/);
      expect(rule).toContain('var(--border-accent)');
      // Dimmed, not full — a bare `var(--border-accent)` here is the bug.
      expect(rule).not.toMatch(/border-color:\s*var\(--border-accent\)\s*;/);
    });

    it('leaves the plain focus edge at FULL accent, unchanged', () => {
      // D-15. Focus is still "the edge is accent"; only its evenness is spent.
      const focusRule = CSS.match(
        /\.biorouter-composer-card:has\(\s*textarea:focus\s*\)\s*\{([^}]*)\}/
      )?.[1];
      expect(focusRule).toBeTruthy();
      expect(focusRule).toMatch(/border-color:\s*var\(--border-accent\)/);
    });

    /**
     * The two rules have identical specificity on the `:has()` half, so ONLY
     * source order separates them — the same hazard the light/dark theme blocks
     * carry. If the plain focus rule came second it would win, restore the full
     * accent edge, and swallow the segment.
     */
    it('declares the working rule AFTER the plain focus rule', () => {
      const focusAt = CSS.indexOf('.biorouter-composer-card:has(textarea:focus)');
      const workingAt = CSS.indexOf(
        ".biorouter-composer-card[data-working='true']:has(textarea:focus)"
      );
      expect(focusAt).toBeGreaterThan(-1);
      expect(workingAt).toBeGreaterThan(-1);
      expect(workingAt).toBeGreaterThan(focusAt);
    });

    /**
     * Working but not focused is the resting hairline with the segment riding
     * on it (owner message 17, R-32: the card rests on the page ground with its
     * hairline again). No rule may recolour the base border for the working
     * state alone, or the unfocused working card loses the edge the segment
     * travels along.
     */
    it('keeps the resting hairline under the segment when the field is not focused', () => {
      expect(CSS).not.toMatch(/\.biorouter-composer-card\[data-working='true'\]\s*\{/);
      expect(COMPOSER_CARD).toContain("'bg-background-default border border-border-subtle/60'");
    });
  });

  describe('reduced motion', () => {
    /**
     * The global reset nulls duration and clamps iteration count, which would
     * PARK the segment at one angle — a bright blob on one corner that reads as
     * damage. So the working edge declares its own resting appearance, the way
     * `.br-progress__fill--indeterminate` does.
     */
    const reducedBlock = () => {
      const start = CSS.indexOf(
        '@media (prefers-reduced-motion: reduce)',
        CSS.indexOf('@property --br-working-angle')
      );
      return CSS.slice(start, CSS.indexOf('.br-tab {'));
    };

    it('stops the animation rather than freezing it mid-sweep', () => {
      expect(reducedBlock()).toMatch(/animation:\s*none/);
    });

    it('holds a DASHED ring, so it is still not the solid focus edge', () => {
      // A solid ring would be indistinguishable from focus, which is the one
      // thing this feature may not do.
      expect(reducedBlock()).toContain('repeating-conic-gradient');
    });

    it('keeps the dimmed base under reduced motion too', () => {
      expect(reducedBlock()).toMatch(/border-color:\s*color-mix\(/);
    });
  });

  describe('the hook', () => {
    /**
     * The rule matches nothing without its attribute, and the attribute is
     * inert without the rule — so they are asserted together or the pair can
     * drift apart silently.
     */
    it('is set on the composer card in ChatInput', () => {
      expect(COMPOSER_CARD).toContain("'biorouter-composer-card'");
      expect(CHAT_INPUT).toMatch(/className=\{composerCardClassName\(/);
      expect(CHAT_INPUT).toMatch(/data-working=\{isWorking \? 'true' : undefined\}/);
    });

    it('is absent from the DOM when idle, rather than "false"', () => {
      // `[data-working]` is then the whole test, in CSS and in the e2e specs.
      expect(CHAT_INPUT).not.toMatch(/data-working=\{[^}]*'false'/);
    });

    it('has replaced the duplicated status row above the composer', () => {
      // The row is what duplicated the transcript indicator and could not be
      // aligned with it. If it comes back, the 8px is back.
      expect(BASE_CHAT).not.toContain('renderWorkingStatus');
      expect(BASE_CHAT).not.toContain("from './LoadingBioRouter'");
    });
  });
});
