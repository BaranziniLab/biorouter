// ui/desktop/src/styles/knowledgeLadder.test.ts
import { describe, expect, it } from 'vitest';
import { readFileSync } from 'node:fs';
import { join } from 'node:path';

/**
 * The Knowledge pane's container-query ladder, asserted AT THE SOURCE.
 *
 * ⚠ **jsdom cannot catch what this guards.** It has no layout engine and does
 * not evaluate `@container` at all, so a component test renders every step
 * simultaneously and measures nothing — the two real overflow seams this ladder
 * has had were both found by sweeping pane widths in a browser and were both
 * invisible to the suite. This file therefore does what `styles/measures.test.ts`
 * and `styles/toastLayer.test.ts` do: assert the declaration itself.
 *
 * ⚠ **A `@container` condition cannot read a custom property.** The thresholds
 * exist twice on purpose — once as a documented token, once as the literal the
 * query actually obeys — and drift between the two is silent, because the token
 * is inert. Pinning them together is the whole point of the first test.
 */

const CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const STRIP = readFileSync(
  join(__dirname, '../components/knowledge/graph/GraphFacetStrip.tsx'),
  'utf8'
);

/** Every step of R-08's ladder: the token that documents it, and its query. */
const LADDER: { token: string; px: number; query: RegExp }[] = [
  {
    token: '--knowledge-pane-two-col',
    px: 860,
    query: /@container br-knowledge-pane \(min-width: 860px\)/,
  },
  {
    token: '--knowledge-pane-legend-card',
    px: 940,
    query: /@container br-knowledge-pane \(min-width: 940px\)/,
  },
  {
    token: '--knowledge-pane-full-filters',
    px: 1140,
    query: /@container br-knowledge-pane \(min-width: 1140px\)/,
  },
  {
    token: '--knowledge-pane-three-col',
    px: 1400,
    query: /@container br-knowledge-pane \(min-width: 1400px\)/,
  },
];

describe('the Knowledge pane ladder', () => {
  it.each(LADDER)(
    '$token declares $px and its query obeys the same number',
    ({ token, px, query }) => {
      expect(CSS).toContain(`${token}: ${px}px;`);
      expect(CSS).toMatch(query);
    }
  );

  /**
   * The height step is retired with the subtitle it hid (spec 3.12): the view's description is
   * the band's InfoTip now, so a short pane has nothing left to yield. A height query coming back
   * would be a step the ladder above does not document.
   */
  it('has no height step', () => {
    expect(CSS).not.toMatch(/@container br-knowledge-pane \(max-height:/);
    expect(CSS).not.toContain('.br-knowledge-subtitle');
  });

  it('has no @container step the ladder above does not account for', () => {
    const found = [
      ...CSS.matchAll(/@container br-knowledge-pane \((?:min-width|max-height): (\d+)px\)/g),
    ].map((m) => Number(m[1]));
    expect([...new Set(found)].sort((a, b) => a - b)).toEqual(
      [...new Set(LADDER.map((s) => s.px))].sort((a, b) => a - b)
    );
  });

  /**
   * ⚠ **`Predicate` must NOT be core.** At a 946px pane the row measured
   * search (200) + Type (76) + Predicate (105) + More (78) + Legend (93) plus
   * gaps = 576px in a 532px column and clipped `Legend` mid-word. The seam is
   * that 940px widens the sources rail — narrowing this column — while the
   * filter row sheds nothing until the full-filters step. Only `Type` holds the
   * always-visible slot.
   */
  it('keeps exactly one facet in the always-visible slot', () => {
    const core = [...STRIP.matchAll(/(\w+)Facet\('br-facet-core'\)/g)].map((m) => m[1]);
    expect(core).toEqual(['type']);
  });

  /**
   * ⚠ **The rail REPLACES the chip; it must not join it.** At the three-column
   * step the legend is a permanent rail, so the `Legend` control has nothing
   * left to open — and a chip that survives beside its own replacement both
   * lies and costs the filter row ~93px it no longer has. The companion
   * artifact has exactly this bug from an inline `display` that outlives the
   * state it was set for, which is why this is pinned in the app: measured
   * across 37 pane widths from 1380 to 1560, the shipped chip carries no inline
   * style at any width and flips to `none` at exactly 1400 as the rail appears.
   */
  it('retires the Legend chip at the step where the rail becomes permanent', () => {
    const threeCol = CSS.slice(
      CSS.indexOf('@container br-knowledge-pane (min-width: 1400px)')
    ).slice(0, 1200);
    expect(threeCol).toMatch(/\.br-facet-legend\s*\{\s*display:\s*none/);
    expect(threeCol).toMatch(/\.br-knowledge-detail\s*\{[^}]*display:\s*flex/);
  });

  /**
   * ⚠ **Source order is the whole mechanism, and it has failed twice.** These
   * defaults and the `@container` blocks that override them have no specificity
   * relationship to fall back on — a default declared AFTER a container block
   * silently wins inside it, and the symptom is a facet appearing at the wrong
   * step rather than any error. Both regressions were found by eye in a browser.
   */
  it('declares the facet defaults before the first container block', () => {
    const firstDefault = CSS.indexOf('.br-facet-core,');
    const firstContainer = CSS.indexOf('@container br-knowledge-pane');
    expect(firstDefault).toBeGreaterThan(-1);
    expect(firstContainer).toBeGreaterThan(-1);
    expect(firstDefault).toBeLessThan(firstContainer);
  });

  it('offers every folded facet inside the More menu', () => {
    const extra = [...STRIP.matchAll(/(\w+)Facet\('br-facet-extra'\)/g)].map((m) => m[1]);
    expect(extra.sort()).toEqual(['predicate', 'source', 'status']);
    // A facet that is `extra` but absent from `More` is unreachable between the
    // two-column step and the full-filters step — a filter that exists and
    // cannot be opened, which is worse than one that is merely hidden.
    const menu = [...STRIP.matchAll(/(\w+)Facet\('', 'knowledge-graph-facet-\w+-in-menu'\)/g)].map(
      (m) => m[1]
    );
    expect(menu.sort()).toEqual(extra.sort());
  });
});

/**
 * The filter strip's two states that jsdom cannot see, asserted at the source like the ladder.
 * Both were found by driving a real render of the pane, not by a test.
 */
describe('the filter strip', () => {
  /**
   * ⚠ **The readout and the Clear label wait for the step where the Legend chip retires.** They
   * used to appear at 1140 beside all four facets AND the Legend: with a filter engaged the row
   * needed ~977px in an 820px column at the common 1152px pane, and "Showing 3 of 16" painted
   * over the Legend. Below the step Clear is the bare icon, whose Tooltip carries the count.
   */
  it('shows the readout only at the step where the Legend chip gives its room back', () => {
    const at = (px: number) => {
      const from = CSS.indexOf(`@container br-knowledge-pane (min-width: ${px}px)`);
      const next = CSS.indexOf('@container br-knowledge-pane', from + 1);
      return CSS.slice(from, next === -1 ? undefined : next);
    };
    expect(at(1140)).not.toContain('.br-knowledge-readout');
    expect(at(1400)).toMatch(/\.br-knowledge-readout\s*\{\s*display:\s*flex/);
  });

  /**
   * ⚠ **An engaged facet is NEUTRAL now (R-27), and that is what keeps it readable under focus.**
   * It was a solid accent fill, which cannot take the grey focus surface (white `--text-on-accent`
   * over `--background-focus`) and needed its own restated focus rule. The strong fill under the
   * selected wash carries the default ink on every ground, so there is no accent and no on-accent
   * ink to protect, and the wash is a background-IMAGE, so the focus ground (a background-colour)
   * still reads under it. A rule that painted a facet accent again, or restated its focus ground,
   * would bring the old failure back.
   */
  it('keeps an engaged facet readable under keyboard focus', () => {
    const from = CSS.indexOf('/* @ws WS-KNOWLEDGE begin: filter facet */');
    const to = CSS.indexOf('/* @ws WS-KNOWLEDGE end: filter facet */');
    expect(from).toBeGreaterThan(-1);
    expect(to).toBeGreaterThan(from);
    const block = CSS.slice(from, to);
    const rule = block.match(/\.br-facet\[data-engaged='true'\]\s*\{([^}]*)\}/);
    expect(rule, 'an engaged facet needs its own rule').not.toBeNull();
    expect(rule![1]).toContain('background-color: var(--background-fill-strong)');
    expect(rule![1]).toContain('background-image: linear-gradient(var(--overlay-selected)');
    // Neutral: no accent ground and no accent ink anywhere in the facet block.
    expect(block.replace(/\/\*[\s\S]*?\*\//g, '')).not.toMatch(
      /--background-accent|--text-on-accent/
    );
    // And no focus rule of its own: the global focus surface reads through the wash.
    expect(block.replace(/\/\*[\s\S]*?\*\//g, '')).not.toContain(':focus-visible');
  });
});
