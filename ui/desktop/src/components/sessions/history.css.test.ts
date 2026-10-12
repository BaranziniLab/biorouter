import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * Source guards for the History rules that are load-bearing and that jsdom can
 * neither run nor measure (it applies no stylesheet). The geometry is measured in
 * Chromium by `historyRow.browser.test.ts`; these pin the two things a browser
 * test would not notice going missing.
 */

const HISTORY_CSS = readFileSync(resolve(__dirname, 'history.css'), 'utf8');
const MAIN_CSS = readFileSync(resolve(__dirname, '../../styles/main.css'), 'utf8');

describe('Share on a saved transcript while it is unavailable', () => {
  /**
   * The button stays focusable (`aria-disabled`) so its tooltip can say why, and
   * `buttonVariants` styles only the `disabled` attribute, so without this rule an
   * unavailable Share looks exactly like one that works.
   */
  it('looks unavailable, and shows its spinner at full strength while it runs', () => {
    const idle = HISTORY_CSS.match(/\.br-transcript-action\[aria-disabled='true'\]\s*\{([^}]*)\}/);
    expect(idle?.[1]).toMatch(/opacity:\s*0\.5/);
    expect(idle?.[1]).toMatch(/cursor:\s*not-allowed/);
    const busy = HISTORY_CSS.match(/\.br-transcript-action\[aria-busy='true'\]\s*\{([^}]*)\}/);
    expect(busy?.[1]).toMatch(/opacity:\s*1/);
  });
});

describe('a row whose menu is open (V1-R2-6)', () => {
  const rule = HISTORY_CSS.match(
    /\.br-history-row:is\(\[data-state='open'\],\s*\[data-menu-open\]\)\s*\{([^}]*)\}/
  );

  /**
   * A modal menu removes `:hover` from the row it opened from. The right-click menu marks the
   * row with Radix's `data-state="open"`, the `⋯` menu with `data-menu-open` (its trigger sits in
   * a Tooltip whose own `data-state` would overwrite the menu's), and both must paint the same
   * wash as the shared row's `:hover`, or the row being acted on is unmarked.
   */
  it('keeps the shared hover wash, from either menu', () => {
    expect(rule).not.toBeNull();
    expect(rule![1]).toMatch(/background-color:\s*var\(--overlay-hover\)/);
    const shared = MAIN_CSS.match(/\.biorouter-list-row:hover,[^{]*\{([^}]*)\}/);
    expect(shared?.[1]).toMatch(/background-color:\s*var\(--overlay-hover\)/);
  });

  it('steps a control inside the row back one ground, as a hovered row does', () => {
    expect(rule![1]).toMatch(/--br-control-ground:\s*var\(--background-default\)/);
    expect(rule![1]).toMatch(/--br-control-hover:\s*var\(--background-fill-strong\)/);
  });
});

describe("a row's keyboard focus (V1-R3-4)", () => {
  const row = HISTORY_CSS.match(
    /\.br-history-row:has\(\.br-history-row-open:focus-visible\)\s*\{([^}]*)\}/
  );
  const control = HISTORY_CSS.match(/\.br-history-row-open:focus-visible\s*\{([^}]*)\}/);

  /**
   * Spec 2.5: a row takes `--background-focus` and an inset 2px `--border-focus` edge. The open
   * control is the row's text block with no padding, so the base rule's ring on it clipped the
   * title; the ring moves to the row and the control paints nothing. Keyed on `:focus-visible`
   * through `:has()`, never `:focus-within`, so a click leaves no residue. Measured in Chromium
   * by `historyRow.browser.test.ts`; this pins the two rules a jsdom test cannot see.
   */
  it('draws the focus ground and the inset edge on the row', () => {
    expect(row).not.toBeNull();
    expect(row![1]).toMatch(/background-color:\s*var\(--background-focus\)/);
    expect(row![1]).toMatch(/box-shadow:\s*inset 0 0 0 2px var\(--border-focus\)/);
    const rules = HISTORY_CSS.replace(/\/\*[\s\S]*?\*\//g, '');
    expect(rules).not.toMatch(/\.br-history-row[^{]*:focus-within/);
  });

  it('draws nothing on the open control itself', () => {
    expect(control).not.toBeNull();
    expect(control![1]).toMatch(/background-color:\s*transparent/);
    expect(control![1]).toMatch(/box-shadow:\s*none/);
  });
});

describe('the loading skeleton', () => {
  const block = MAIN_CSS.match(
    /@ws WS-HISTORY begin: history loading[\s\S]*?@ws WS-HISTORY end: history loading/
  );

  /** Principle 9: only opacity, transform and clip-path animate. */
  it('animates opacity and nothing else', () => {
    expect(block).not.toBeNull();
    const keyframes = block![0].match(/@keyframes biorouter-history-loading\s*\{([\s\S]*?)\n\}/);
    const properties = [...(keyframes?.[1] ?? '').matchAll(/([a-z-]+)\s*:/g)].map(
      (match) => match[1]
    );
    expect(properties.length).toBeGreaterThan(0);
    expect(new Set(properties)).toEqual(new Set(['opacity']));
  });

  it('loops on a multiple of --dur-slow with the app easing, and rests under reduced motion', () => {
    expect(block![0]).toMatch(
      /animation:\s*biorouter-history-loading\s+calc\(var\(--dur-slow\)\s*\*\s*\d+\)\s+var\(--ease-out\)/
    );
    expect(block![0]).toMatch(
      /prefers-reduced-motion:\s*reduce\)\s*\{\s*\.biorouter-history-loading-cell\s*\{\s*animation:\s*none/
    );
  });
});
