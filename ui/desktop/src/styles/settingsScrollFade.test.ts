import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * The Settings scroller's top scroll-edge fade.
 *
 * **The defect it fixes.** Settings' `TabsList` carries `border-b-0` on purpose
 * (§4.2 — its own hairline landed a few pixels under the header's and read as a
 * double rule unique to Settings), so the tab strip's bottom edge and the scroll
 * viewport's top edge are the SAME y with nothing between them. Measured in the
 * running app at 1440x900, App tab, `scrollTop` 663: the last 1px of the
 * "Configuration guide" `Button variant="link"` survives the clip and lands
 * beside the active tab's underline — same y, same accent colour, a second and
 * thinner tab indicator. The window is about 3px wide (660 shows 4px of the
 * link, 666 shows none), which is why it reads as a rendering glitch rather
 * than as content.
 *
 * ⚠ **Asserted at the SOURCE, and it has to be.** jsdom has no layout engine,
 * never runs Tailwind, and computes no `mask-image`: a component test can mount
 * Settings, set `scrollTop`, read the viewport's `maskImage`, and see `''`
 * whether the rule exists or not. `styles/composerFocus.test.ts` and
 * `styles/measures.test.ts` make the same argument for the same reason.
 *
 * ⚠ **Authored CSS, never a Tailwind arbitrary variant.** A
 * `data-[scrolled=true]:[mask-image:...]` at the call site is a newly written
 * class string, and a newly written class can silently fail to generate — three
 * spellings of the composer's focus edge were each measured in the running app
 * with the class on the element and no matching rule anywhere in the
 * stylesheet. That is why the rule below lives in `main.css` and the call site
 * carries only its name.
 */
const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const SETTINGS_VIEW = readFileSync(
  join(__dirname, '../components/settings/SettingsView.tsx'),
  'utf8'
);
const SCROLL_AREA = readFileSync(join(__dirname, '../components/ui/scroll-area.tsx'), 'utf8');

/** The `[data-scrolled='true']` rule body, or `undefined` if the rule is gone. */
function scrolledRule(): string | undefined {
  return CSS.match(
    /\.biorouter-scroll-fade-top\[data-scrolled='true'\]\s*>\s*\[data-radix-scroll-area-viewport\]\s*\{([^}]*)\}/
  )?.[1];
}

describe('the settings scroll-edge fade', () => {
  it('is declared as authored CSS, not left to a Tailwind variant', () => {
    expect(scrolledRule()).toBeTruthy();
  });

  /**
   * A mask rather than an overlay gradient: an overlay has to be painted in the
   * ground's colour, and the ground differs per theme family and per mode, so
   * an overlay is three families x two modes of colour to keep in step. A mask
   * removes alpha and names no colour at all.
   */
  it('fades by masking alpha, so it needs no per-theme colour', () => {
    const rule = scrolledRule();
    expect(rule).toContain('mask-image');
    expect(rule).toContain('linear-gradient(to bottom, transparent');
    // Prefixed alongside the standard property, not instead of it.
    expect(rule).toContain('-webkit-mask-image');
  });

  /**
   * The height is a token, not a literal, because the fade and the
   * `scroll-padding-top` that keeps keyboard focus out of it must be the same
   * number — and two places writing `10px` is how they stop being.
   */
  it('takes its height from one token, shared with the scroll padding', () => {
    expect(CSS).toMatch(/--scroll-fade-top:\s*10px;/);
    expect(scrolledRule()).toContain('var(--scroll-fade-top)');
    const resting = CSS.match(
      /\.biorouter-scroll-fade-top\s*>\s*\[data-radix-scroll-area-viewport\]\s*\{([^}]*)\}/
    )?.[1];
    expect(resting).toContain('scroll-padding-top: var(--scroll-fade-top)');
  });

  /**
   * ⚠ Only while scrolled. A page nobody has moved has nothing hidden above its
   * first row, and dimming that row is a regression rather than a fix — so the
   * mask hangs off `data-scrolled`, and the unconditional rule beside it must
   * carry the scroll padding and nothing else.
   */
  it('fades only the clipped edge, never an unscrolled page', () => {
    const resting = CSS.match(
      /\.biorouter-scroll-fade-top\s*>\s*\[data-radix-scroll-area-viewport\]\s*\{([^}]*)\}/
    )?.[1];
    expect(resting).toBeTruthy();
    expect(resting).not.toContain('mask-image');
  });

  /**
   * The rule masks the VIEWPORT, not the `ScrollArea` root: the root also holds
   * the scrollbar, which would fade with the content.
   */
  it('masks the viewport, so the scrollbar is left alone', () => {
    expect(CSS).not.toMatch(/\.biorouter-scroll-fade-top\[data-scrolled='true'\]\s*\{/);
  });

  /**
   * The rule matches nothing without its hook, and the hook is on the one
   * scroller the defect appears in — so the two are asserted together or they
   * drift apart silently. The same pairing `composerFocus.test.ts` makes.
   */
  it('has its hook on the settings scroller', () => {
    expect(SETTINGS_VIEW).toMatch(
      /<ScrollArea className="biorouter-scroll-fade-top flex-1" paddingX=\{1\}>/
    );
  });

  /**
   * `data-scrolled` is the selector's other half and it lives in a component
   * this rule does not own. It had NO consumers before this fade, which is
   * exactly the state in which an attribute gets deleted as dead.
   */
  it('keeps the data-scrolled attribute the selector depends on', () => {
    expect(SCROLL_AREA).toContain('data-scrolled={isScrolled}');
    expect(SCROLL_AREA).toContain('setIsScrolled(scrollTop > 0)');
  });
});
