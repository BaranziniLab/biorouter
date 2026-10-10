import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import {
  CHAT_COLUMN_CHROME,
  CHAT_FULL_MEASURE_WIDTH,
  CHAT_MEASURE,
  READABLE_CHAT_WIDTH,
  SUMMARY_RAIL_MAX_WIDTH,
  SUMMARY_RAIL_MIN_WIDTH,
} from '../components/Layout/yieldLadder';

/**
 * RUNG 0, pinned at the source.
 *
 * The decision lives in `yieldLadder.summaryRailFit` and is unit-tested on both
 * sides of every seam. What this file guards is the half no component test can
 * see: jsdom never loads a stylesheet, never lays out a grid and never resolves
 * a custom property, so a render test of the split box passes whether the rail's
 * grid agrees with the ladder or not. The real layout is measured in
 * `components/summary/summaryRailGeometry.browser.test.ts`.
 */
const MAIN_CSS = readFileSync(join(__dirname, 'main.css'), 'utf8');
const RAIL_CSS = readFileSync(join(__dirname, '../components/summary/summaryRail.css'), 'utf8');
const BASE_CHAT = readFileSync(join(__dirname, '../components/BaseChat.tsx'), 'utf8');
const RAIL_TSX = readFileSync(join(__dirname, '../components/summary/ChatSummaryRail.tsx'), 'utf8');

/** The stylesheet with comments blanked (same length), so offsets survive. */
const RAIL_CODE = RAIL_CSS.replace(/\/\*[\s\S]*?\*\//g, (comment) =>
  comment.replace(/[^\n]/g, ' ')
);

/** Strip `//` and block comments from TS/TSX so prose cannot satisfy a match. */
function codeWithoutComments(source: string): string {
  return source.replace(/\/\*[\s\S]*?\*\//g, '').replace(/^\s*\/\/.*$/gm, '');
}

function geometryBlock(): string {
  const start = RAIL_CSS.indexOf('/* RAIL GEOMETRY BEGIN */');
  const end = RAIL_CSS.indexOf('/* RAIL GEOMETRY END */');
  expect(start, 'the geometry block markers moved').toBeGreaterThan(0);
  expect(end).toBeGreaterThan(start);
  return RAIL_CODE.slice(start, end);
}

/** Every rule in the rail stylesheet whose whitespace-collapsed selector equals `selector`. */
function rulesFor(selector: string): { index: number; body: string }[] {
  const wanted = selector.replace(/\s+/g, ' ').trim();
  const found: { index: number; body: string }[] = [];
  const pattern = /([^{}]+)\{([^{}]*)\}/g;
  for (let match = pattern.exec(RAIL_CODE); match; match = pattern.exec(RAIL_CODE)) {
    if (match[1].replace(/\s+/g, ' ').trim() === wanted) {
      found.push({ index: match.index, body: match[2] });
    }
  }
  return found;
}

/** The one top-level (unconditioned) rule for `selector`. */
function onlyRule(selector: string): { index: number; body: string } {
  const found = rulesFor(selector).filter(({ index }) => depthAt(index) === 0);
  if (found.length !== 1) {
    throw new Error(`expected exactly one rule for ${selector}, got ${found.length}`);
  }
  return found[0];
}

function property(body: string, name: string): string | null {
  const match = body.match(new RegExp(`(?:^|;|\\s)${name}:\\s*([^;]+);`));
  return match ? match[1].replace(/\s+/g, ' ').trim() : null;
}

/** Brace depth at an offset: 0 is a top-level, unlayered rule. */
function depthAt(index: number): number {
  let depth = 0;
  for (let i = 0; i < index; i += 1) {
    if (RAIL_CODE[i] === '{') depth += 1;
    else if (RAIL_CODE[i] === '}') depth -= 1;
  }
  return depth;
}

const RAIL_ONLY = '[data-preview-split][data-summary-rail]:not([data-preview-layout])';
const SIDE_AND_RAIL = "[data-preview-split][data-preview-layout='side'][data-summary-rail]";
const RAIL_CELL = "[data-preview-split][data-summary-rail] > [data-preview-area='rail']";

describe('rung 0: the summary rail’s grid agrees with the ladder', () => {
  it('derives the 816px floor from the chat measure and the transcript’s own chrome', () => {
    const measure = MAIN_CSS.match(/--measure-chat:\s*([^;]+);/);
    expect(measure?.[1].trim()).toBe(`${CHAT_MEASURE}px`);
    expect(CHAT_FULL_MEASURE_WIDTH).toBe(CHAT_MEASURE + CHAT_COLUMN_CHROME);
    // 56 = the ScrollArea root's `px-1` (2 × 4) + its viewport's `paddingX={6}`
    // (2 × 24). If either moves in BaseChat, the 56 is wrong and so is every
    // seam rung 0 computes from it.
    expect(CHAT_COLUMN_CHROME).toBe(2 * 4 + 2 * 6 * 4);
    const code = codeWithoutComments(BASE_CHAT);
    expect(code).toContain('paddingX={6}');
    expect(code).toContain("'px-1',");
  });

  it('places the rail beside the transcript and the composer, under the header', () => {
    const rail = onlyRule(RAIL_ONLY);
    expect(property(rail.body, 'display')).toBe('grid');
    // `clip`, never `hidden`, for the reason rung 2 gives: a hidden box is a
    // scroll container that a descendant's scrollIntoView can shift sideways.
    expect(property(rail.body, 'overflow')).toBe('clip');
    expect(property(rail.body, 'grid-template-columns')).toBe(
      'minmax(0, 1fr) var(--summary-rail-width)'
    );
    expect(property(rail.body, 'grid-template-rows')).toBe('auto auto minmax(0, 1fr) auto');
    expect(property(rail.body, 'grid-template-areas')).toBe(
      "'header header' 'subheader subheader' 'transcript rail' 'composer rail'"
    );
  });

  it('keys the rail-only template on the absence of a preview layout', () => {
    // Rung 2's stack template has the same specificity; without the `:not()` a
    // stray `data-summary-rail` could override it on source order.
    for (const { body } of rulesFor(RAIL_ONLY)) expect(body).toContain('grid-template');
    const selectors = [...geometryBlock().matchAll(/([^{}]+)\{/g)]
      .flatMap((match) => match[1].split(','))
      .map((selector) => selector.replace(/\s+/g, ' ').trim())
      .filter((selector) => selector.includes('[data-summary-rail]'));
    expect(selectors.length).toBeGreaterThan(0);
    for (const selector of selectors) {
      const guarded =
        selector.includes(':not([data-preview-layout])') ||
        selector.includes("[data-preview-layout='side']") ||
        selector === RAIL_CELL;
      expect(guarded, selector).toBe(true);
    }
  });

  it('flattens the column and the body instead of re-parenting them', () => {
    const flattened = onlyRule(
      `${RAIL_ONLY} > [data-preview-area='column'], ${RAIL_ONLY} > [data-preview-area='column'] > [data-preview-area='body']`
    );
    expect(property(flattened.body, 'display')).toBe('contents');
    for (const area of ['header', 'subheader', 'transcript', 'composer']) {
      const placed = onlyRule(`${RAIL_ONLY} [data-preview-area='${area}']`);
      expect(property(placed.body, 'grid-area'), area).toBe(area);
    }
  });

  it('seats the rail between the conversation and a side preview on an ultrawide pane', () => {
    const both = onlyRule(SIDE_AND_RAIL);
    expect(property(both.body, 'grid-template-columns')).toBe(
      `minmax(${READABLE_CHAT_WIDTH}px, 1fr) var(--summary-rail-width) var(--preview-panel-width)`
    );
    expect(property(both.body, 'grid-template-areas')).toBe(
      "'header header preview' 'subheader subheader preview' 'transcript rail preview' 'composer rail preview'"
    );
  });

  it('pads the rail cell 12px except on the side the transcript’s gutter already holds', () => {
    const cell = onlyRule(RAIL_CELL);
    expect(property(cell.body, 'grid-area')).toBe('rail');
    expect(property(cell.body, 'padding')).toBe('12px 12px 12px 0');
    expect(property(cell.body, 'min-width')).toBe('0');
    expect(SUMMARY_RAIL_MIN_WIDTH).toBe(240);
    expect(SUMMARY_RAIL_MAX_WIDTH).toBe(280);
  });

  it('is unlayered, so it beats the utilities on the same elements', () => {
    for (const selector of [RAIL_ONLY, SIDE_AND_RAIL, RAIL_CELL]) {
      // Every rule for the selector, not just the top-level one `onlyRule` picks.
      const found = rulesFor(selector);
      expect(found, selector).toHaveLength(1);
      expect(depthAt(found[0].index), selector).toBe(0);
    }
    expect(RAIL_CODE).not.toMatch(/@layer/);
  });

  /**
   * The seam is decided in JS, once. A container or media condition here would
   * be a second seam free to drift from 1056; a transition here would tween the
   * transcript's width, which re-lays out a transcript that is not virtualised
   * on every frame (Codex's pinned-summary jitter).
   */
  it('has no condition, transition or animation in its geometry', () => {
    const block = geometryBlock();
    expect(block).not.toMatch(/@container|@media|transition|animation/);
  });
});

describe('rung 0: the card’s motion stays off the grid', () => {
  it('enters once, from the card, and never replays after a resize', () => {
    const enter = onlyRule(".br-summary-card[data-state='open']:not([data-entered])");
    expect(property(enter.body, 'animation')).toBe(
      'br-summary-rail-in var(--dur-fast-max) var(--ease-out) 60ms both'
    );
    const exit = onlyRule(".br-summary-card[data-state='closed']");
    expect(property(exit.body, 'animation')).toBe(
      'br-summary-rail-out var(--dur-fast) var(--ease-out) forwards'
    );
  });

  it('animates only transform and opacity', () => {
    const keyframes = RAIL_CODE.match(/@keyframes br-summary-rail-[\w-]+\s*\{[\s\S]*?\}\s*\}/g);
    expect(keyframes?.length).toBe(4);
    for (const frame of keyframes ?? []) {
      const props = [...frame.matchAll(/([a-z-]+)\s*:/g)].map((m) => m[1]);
      for (const prop of props) expect(['opacity', 'transform']).toContain(prop);
    }
  });

  it('stops while the window or the sidebar is resized, and only fades under reduced motion', () => {
    const resize = onlyRule(
      "body.biorouter-window-resizing [data-preview-area='rail'] > .br-summary-card, body.biorouter-sidebar-resizing [data-preview-area='rail'] > .br-summary-card"
    );
    expect(property(resize.body, 'animation')).toBe('none !important');
    const reduced = RAIL_CODE.slice(RAIL_CODE.indexOf('@media (prefers-reduced-motion: reduce)'));
    expect(reduced).toContain('br-summary-rail-fade-in');
    expect(reduced).toContain('br-summary-rail-fade-out');
  });
});

describe('rung 0: BaseChat mounts one rail, statically', () => {
  it('marks the rail cell exactly once and renders one ChatSummaryRail', () => {
    const code = codeWithoutComments(BASE_CHAT);
    expect(code.match(/data-preview-area="rail"/g) ?? []).toHaveLength(1);
    expect(code.match(/<ChatSummaryRail\b/g) ?? []).toHaveLength(1);
    // Both spreads stay verbatim (measures.test.ts pins the first), and their
    // styles are merged after them so neither replaces the other.
    expect(code).toContain('{...artifactPanel.splitPaneProps}');
    expect(code).toContain('{...summaryRail.splitPaneProps}');
    expect(code).toContain(
      'style={{ ...artifactPanel.splitPaneProps.style, ...summaryRail.splitPaneProps.style }}'
    );
  });

  it('loads the rail’s stylesheet before the first chat renders', () => {
    // A lazily imported stylesheet would leave the split box unstyled until the
    // module arrived.
    expect(codeWithoutComments(BASE_CHAT)).toMatch(
      /^import \{ ChatSummaryRail \} from '\.\/summary\/ChatSummaryRail';$/m
    );
    expect(codeWithoutComments(BASE_CHAT)).not.toMatch(
      /lazy\(\s*\(\)\s*=>\s*import\([^)]*ChatSummaryRail/
    );
    expect(codeWithoutComments(RAIL_TSX)).toMatch(/^import '\.\/summaryRail\.css';$/m);
  });
});
