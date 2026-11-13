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
    expect(code).toContain("const contentClassName = 'px-1';");
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
    // The subagent line sits on the transcript's track, so its box is centred
    // on the column it belongs to; the rail starts beside it, under the header
    // (vision V2-R2-3).
    expect(property(rail.body, 'grid-template-areas')).toBe(
      "'header header' 'subheader rail' 'transcript rail' 'composer rail'"
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
      "'header header preview' 'subheader rail preview' 'transcript rail preview' 'composer rail preview'"
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

  it('snaps the header button’s pressed fill when geometry flips it (V2-R2-6)', () => {
    // Top level and unlayered, so it beats the Button's layered transition utility.
    expect(property(onlyRule('[data-summary-mode][data-still]').body, 'transition')).toBe('none');
    // The button stamps it, and BaseChat tells it which changes were the person's.
    const button = readFileSync(
      join(__dirname, '../components/summary/SummaryHeaderButton.tsx'),
      'utf8'
    );
    expect(codeWithoutComments(button)).toContain("data-still={stillAt !== 0 ? '' : undefined}");
    expect(codeWithoutComments(BASE_CHAT)).toContain('railStill={summaryRail.still}');
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

describe('the summary’s type: two sizes only, 13 and 12', () => {
  const CONTENT_CSS = readFileSync(
    join(__dirname, '../components/summary/chatSummary.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');
  const SUMMARY_TSX = codeWithoutComments(
    readFileSync(join(__dirname, '../components/ChatSummary.tsx'), 'utf8')
  );

  it('declares only the secondary (13) and supporting (12) roles', () => {
    const sizes = [...CONTENT_CSS.matchAll(/font-size:\s*([^;]+);/g)].map((match) =>
      match[1].trim()
    );
    expect(sizes.length).toBeGreaterThan(0);
    for (const size of sizes) {
      expect(['var(--text-secondary)', 'var(--text-supporting)'], size).toContain(size);
    }
  });

  it('sets a size on every Button it draws, because a Button is 14/500 at every rung', () => {
    // The footer rows and the retry each carry a class that overrides it; a new
    // Button without one would put a third size in the card.
    const buttons = SUMMARY_TSX.match(/<Button\b[\s\S]*?>/g) ?? [];
    expect(buttons.length).toBe(3);
    for (const button of buttons) {
      expect(button).toMatch(/className="br-summary__(action|retry)"/);
    }
    expect(CONTENT_CSS).toMatch(/\.br-summary \.br-summary__retry \{[^}]*font-size/);
    expect(CONTENT_CSS).toMatch(/\.br-summary \.br-summary__action \{[^}]*font-size/);
  });
});

describe('the summary is filled, not outlined, and parted into sections by hairlines (R-27, owner direction 2026-11-12)', () => {
  const CONTENT_CODE = readFileSync(
    join(__dirname, '../components/summary/chatSummary.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');
  const BUTTON_TSX = codeWithoutComments(
    readFileSync(join(__dirname, '../components/summary/SummaryHeaderButton.tsx'), 'utf8')
  );

  /** The body of the one top-level rule for `selector` in the content stylesheet. */
  function contentRule(selector: string): string {
    const wanted = selector.replace(/\s+/g, ' ').trim();
    const pattern = /([^{}]+)\{([^{}]*)\}/g;
    const found: string[] = [];
    for (let match = pattern.exec(CONTENT_CODE); match; match = pattern.exec(CONTENT_CODE)) {
      if (match[1].replace(/\s+/g, ' ').trim() === wanted) found.push(match[2]);
    }
    if (found.length !== 1)
      throw new Error(`expected one rule for ${selector}, got ${found.length}`);
    return found[0];
  }

  it('fills the rail card and keeps its 1px border, transparent', () => {
    const card = onlyRule('.br-summary-card').body;
    expect(property(card, 'background')).toBe('var(--background-fill)');
    // Transparent, never removed: nothing moves, and forced colours still draw it.
    expect(property(card, 'border')).toBe('1px solid transparent');
    expect(property(card, 'border-radius')).toBe('var(--radius-container)');
    // The card is a filled box, so a filled control inside steps back to the page.
    expect(codeWithoutComments(RAIL_TSX)).toMatch(/className="br-summary-card br-fill-surface"/);
  });

  /**
   * Sections, never boxes (owner direction, 2026-11-12). The divider between two sections is a
   * structural line, the kind design.md 9.11 ("Fills before lines") keeps, and it is the ONE line
   * the content draws: no section, list or row names a fill or a radius of its own, so nothing in
   * the card is a box inside the box. The footer is parted by space.
   */
  it('parts its sections with one hairline across the card, and draws no other line or box', () => {
    const borders = [
      ...CONTENT_CODE.matchAll(/border(?:-top|-bottom|-left|-right)?\s*:\s*[^;]+;/g),
    ];
    expect(borders.map(([declaration]) => declaration)).toEqual([
      'border-top: 1px solid var(--border-subtle);',
    ]);
    const divider = contentRule('.br-summary__body > * + .br-summary__section');
    expect(property(divider, 'border-top')).toBe('1px solid var(--border-subtle)');
    // Across the card's whole width: it reaches into the body's 12px gutter and gives the
    // gutter back as padding, so nothing inside the section moves sideways.
    expect(property(divider, 'margin')).toBe('8px -12px 0');
    expect(property(divider, 'padding')).toBe('8px 12px 0');
    // The old per-section gaps are gone: the divider is the one rule that parts them.
    expect(CONTENT_CODE).not.toMatch(/\.br-summary__details\s*\{/);
    expect(CONTENT_CODE).not.toMatch(/data-after-todos/);
    // No fill and no radius anywhere in the content but the popover's own surface.
    const filled = [...CONTENT_CODE.matchAll(/([^{}]+)\{([^{}]*)\}/g)].filter(([, , body]) =>
      /(^|[;\s])(background(-color)?|border-radius):/.test(body)
    );
    expect(filled.map(([, selector]) => selector.replace(/\s+/g, ' ').trim())).toEqual([
      '.br-summary .br-summary__progress',
    ]);
    expect(property(contentRule('.br-summary__footer'), 'padding')).toBe('4px');
  });

  it('grounds the progress track one step off the card’s fill, where the primitive’s would vanish', () => {
    // The primitive's track is --background-muted, which IS --background-fill.
    expect(property(onlyRule('.br-summary-card').body, '--br-summary-track')).toBe(
      'var(--background-fill-strong)'
    );
    expect(property(contentRule('.br-summary .br-summary__progress'), 'background-color')).toBe(
      'var(--br-summary-track, var(--background-muted))'
    );
  });

  it('fades a scroll edge only where rows are hidden, from the body’s own scroll position', () => {
    for (const name of ['--br-summary-fade-top', '--br-summary-fade-bottom']) {
      expect(CONTENT_CODE).toMatch(
        new RegExp(`@property ${name} \\{[^}]*syntax: '<length>';[^}]*initial-value: 0px;`)
      );
    }
    const supports = CONTENT_CODE.slice(CONTENT_CODE.indexOf('@supports (animation-timeline'));
    expect(supports).toMatch(/animation-timeline: scroll\(self\), scroll\(self\);/);
    expect(supports.replace(/\s+/g, ' ')).toContain(
      'animation-range: 0 16px, calc(100% - 16px) 100%;'
    );
    expect(supports).toMatch(/mask-image: linear-gradient\(/);
    // A focused body draws its focus edge whole.
    expect(supports.replace(/\s+/g, ' ')).toMatch(
      /\.br-summary__body:focus-visible \{ -webkit-mask-image: none; mask-image: none; \}/
    );
    // The running step is kept clear of the fades when scrolled into view.
    expect(CONTENT_CODE).toMatch(/\n\.br-summary__body \{[^}]*scroll-padding-block: 16px;/);
  });

  it('presses the header button on the band’s fill step, and rings the dot in the ground under it', () => {
    expect(BUTTON_TSX).toContain("expanded && 'bg-background-fill-strong text-text-default'");
    expect(BUTTON_TSX).not.toContain('bg-background-medium');
    expect(codeWithoutComments(BASE_CHAT)).toContain(
      "isTerminalDockOpen && 'bg-background-fill-strong text-text-default'"
    );
    expect(property(onlyRule('.br-summary-dot').body, 'box-shadow')).toBe(
      '0 0 0 2px var(--sidebar)'
    );
    expect(property(onlyRule("[aria-expanded='true'] > .br-summary-dot").body, 'box-shadow')).toBe(
      '0 0 0 2px var(--background-fill-strong)'
    );
  });
});

/**
 * The subagents and phases (`summary/SummarySubagents.tsx`) sit inside the same
 * card, so the card's rules bind their stylesheet too. Their real geometry (28px
 * rows, a 30-row list scrolling at 240px with no sideways overflow) is measured
 * in `components/summary/summaryAgents.browser.test.tsx`.
 */
describe('the subagents keep the summary’s rules', () => {
  const AGENTS_CSS = readFileSync(
    join(__dirname, '../components/summary/summaryAgents.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');
  const GRID_CSS = readFileSync(
    join(__dirname, '../components/summary/summaryAgentGrid.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');
  const AGENTS_TSX = codeWithoutComments(
    readFileSync(join(__dirname, '../components/summary/SummarySubagents.tsx'), 'utf8')
  );
  const DECISIONS_CSS = readFileSync(
    join(__dirname, '../components/summary/summaryDecisions.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');
  const SUMMARY_CSS = readFileSync(
    join(__dirname, '../components/summary/chatSummary.css'),
    'utf8'
  ).replace(/\/\*[\s\S]*?\*\//g, '');

  /** The body of the one top-level rule whose selector list is exactly `selector`. */
  function agentsRule(selector: string): string {
    const wanted = selector.replace(/\s+/g, ' ').trim();
    const found: string[] = [];
    const pattern = /([^{}]+)\{([^{}]*)\}/g;
    for (let match = pattern.exec(AGENTS_CSS); match; match = pattern.exec(AGENTS_CSS)) {
      if (match[1].replace(/\s+/g, ' ').trim() === wanted) found.push(match[2]);
    }
    if (found.length !== 1)
      throw new Error(`expected one rule for ${selector}, got ${found.length}`);
    return found[0];
  }

  it('declares only the secondary (13) and supporting (12) sizes', () => {
    const sizes = [...AGENTS_CSS.matchAll(/font-size:\s*([^;]+);/g)].map((match) =>
      match[1].trim()
    );
    expect(sizes.length).toBeGreaterThan(0);
    for (const size of sizes) {
      expect(['var(--text-secondary)', 'var(--text-supporting)'], size).toContain(size);
    }
  });

  it('draws one line, the inset hairline between two phases; every other border is a reset', () => {
    const borders = [
      ...AGENTS_CSS.matchAll(/border(?:-top|-bottom|-left|-right)?\s*:\s*([^;]+);/g),
    ];
    const lines = borders.filter(([, value]) => !['0', 'none'].includes(value.trim()));
    expect(lines.map(([declaration]) => declaration)).toEqual([
      'border-top: 1px solid var(--border-subtle);',
    ]);
    expect(property(agentsRule('.br-agents__phase + .br-agents__phase'), 'border-top')).toBe(
      '1px solid var(--border-subtle)'
    );
    expect(AGENTS_CSS).not.toMatch(/outline:(?!\s*none\s*;)[^;]+;/);
  });

  it('gives every row the 28px of the card’s other rows', () => {
    const rows = agentsRule('.br-agents__row, .br-agents__phase-toggle, .br-agents__more');
    expect(property(rows, 'min-height')).toBe('28px');
    expect(property(rows, 'font-size')).toBe('var(--text-secondary)');
  });

  it('keeps no section spacing of its own: the content’s one divider parts every section', () => {
    expect(AGENTS_CSS).not.toMatch(/br-agents__section/);
  });

  /**
   * The phase cards were white boxes on the grey rail (before/phases-20.png), a box in the
   * card's box, which the owner named for removal on 2026-11-12. A phase is now a chunk on
   * the card's ground: its line is its heading, and a hairline inset to the content (the
   * section divider runs the card's full width) parts it from the phase above.
   */
  it('draws a phase as a chunk on the card’s own ground, never a box', () => {
    const phase = agentsRule('.br-agents__phase');
    for (const name of ['background', 'background-color', 'border-radius', 'padding']) {
      expect(property(phase, name), name).toBeNull();
    }
    const next = agentsRule('.br-agents__phase + .br-agents__phase');
    expect(property(next, 'border-top')).toBe('1px solid var(--border-subtle)');
    expect(property(next, 'margin-top')).toBe('4px');
    expect(property(next, 'padding-top')).toBe('4px');
    // Nothing that holds a phase, its grid or its rows names a fill or a radius: a row's hover,
    // focus and kept fills are the only fills, and they belong to the row.
    const containers = [
      ...AGENTS_CSS.matchAll(/([^{}]+)\{([^{}]*)\}/g),
      ...GRID_CSS.matchAll(/([^{}]+)\{([^{}]*)\}/g),
    ]
      .map(([, selector, body]) => ({ selector: selector.replace(/\s+/g, ' ').trim(), body }))
      .filter(({ selector }) =>
        /\.br-agents(__phases?|__group-body|__list)?(?![\w-])|\.br-agent-grid(__cells|__item)?(?![\w-])/.test(
          selector
        )
      )
      .filter(({ selector }) => !/__(row|toggle|more|cell(?!s)|retry|chevron)/.test(selector));
    expect(containers.length).toBeGreaterThan(0);
    for (const { selector, body } of containers) {
      expect(body, selector).not.toMatch(/(^|[;\s])(background(-color)?|border-radius):/);
    }
  });

  it('labels a section in the app’s one caps style, at the supporting size', () => {
    const label = agentsRule('.br-summary__toggle-label');
    expect(property(label, 'text-transform')).toBe('uppercase');
    expect(property(label, 'letter-spacing')).toBe('var(--text-caps--letter-spacing, 0.04em)');
  });

  it('restates the focus fill wherever it paints a row, and stops the pulse under reduced motion', () => {
    const focus = agentsRule(
      '.br-agents__row:focus-visible, .br-agents__phase-toggle:focus-visible, .br-agents__more:focus-visible, .br-agents__retry:focus-visible'
    );
    expect(property(focus, 'background-color')).toBe('var(--background-focus)');
    expect(property(focus, 'box-shadow')).toBe('inset 0 0 0 2px var(--border-focus)');
    // A running mark breathes (opacity only), and never under reduced motion.
    expect(GRID_CSS).toMatch(
      /\.br-agent-mark\[data-pulse\] \.br-agent-mark__core \{\s*animation: br-agent-mark-pulse/
    );
    const reduced = GRID_CSS.slice(GRID_CSS.indexOf('@media (prefers-reduced-motion: reduce)'));
    expect(reduced).toMatch(
      /\.br-agent-mark\[data-pulse\] \.br-agent-mark__core \{\s*animation: none;/
    );
    const frames = GRID_CSS.match(/@keyframes br-agent-mark-pulse \{[\s\S]*?\}\s*\}/)?.[0] ?? '';
    expect([...frames.matchAll(/([a-z-]+)\s*:/g)].map((m) => m[1])).toEqual(['opacity']);
  });

  /**
   * Owner direction, 2026-11-12: red is rare (design.md 9.13). A failed mark is the outline with
   * a small cross in the muted ink, and a mark waiting for the person (or stalled) the half fill
   * in the default ink, so neither reaches for a danger or warning hue; the word says which.
   */
  it('inks every status with an existing token, never red, and draws it in forced colours', () => {
    for (const [kind, ink] of [
      ['running', 'var(--text-info)'],
      ['waiting', 'var(--text-default)'],
      ['failed', 'var(--text-muted)'],
    ]) {
      const rule = GRID_CSS.match(
        new RegExp(`\\.br-agent-mark\\[data-kind='${kind}'\\] \\{([^}]*)\\}`)
      );
      expect(property(rule?.[1] ?? '', 'color'), kind).toBe(ink);
    }
    expect(GRID_CSS).toMatch(/\.br-agent-mark\[data-kind='done'\][^{]*\{[^}]*--text-success/);
    // The failed mark is the ring and its small cross, both stroked at the ring's width.
    expect(GRID_CSS).toMatch(
      /\.br-agent-mark__ring,\s*\.br-agent-mark__cross \{[^}]*stroke: currentColor;/
    );
    // The grid declares no type of its own.
    expect(GRID_CSS).not.toMatch(/font-size:/);
    const forced = GRID_CSS.slice(GRID_CSS.indexOf('@media (forced-colors: active)'));
    expect(forced).toMatch(/\.br-agent-mark \{\s*color: CanvasText;/);
  });

  it('names no danger or warning ink anywhere in the summary', () => {
    for (const [name, css] of [
      ['chatSummary.css', SUMMARY_CSS],
      ['summaryAgents.css', AGENTS_CSS],
      ['summaryAgentGrid.css', GRID_CSS],
      ['summaryDecisions.css', DECISIONS_CSS],
    ]) {
      expect(css, name).not.toMatch(/--(?:text|background|border|wash)-(?:danger|warning)/);
    }
    // A gate that wants a look reads solid, and one that waits for the person takes the accent.
    expect(
      property(
        DECISIONS_CSS.match(/\.br-agents__gate\[data-attention\] \{([^}]*)\}/)?.[1] ?? '',
        'color'
      )
    ).toBe('var(--text-default)');
    expect(
      property(
        DECISIONS_CSS.match(/\.br-agents__gate\[data-needs-person\] \{([^}]*)\}/)?.[1] ?? '',
        'color'
      )
    ).toBe('var(--text-accent)');
    // A failed refresh is a sentence in the default ink beside its Retry.
    expect(property(SUMMARY_CSS.match(/\.br-summary__error \{([^}]*)\}/)?.[1] ?? '', 'color')).toBe(
      'var(--text-default)'
    );
  });

  /**
   * Owner direction, 2026-11-12: text reads solid. A name, a phase title and a value keep the
   * default ink at every status; secondary text is the muted token itself, never the subtler
   * step, a mix or an opacity fade. The ui-readability lane strengthens `--text-muted` at the
   * token layer (design.md 9.14), so the rail takes that change with no edit here.
   */
  it('keeps names and phase titles in the default ink, and secondary text on the muted token', () => {
    // No rule fades a name or a title.
    expect(AGENTS_CSS).not.toMatch(/__name[^{]*\{[^}]*color: var\(--text-muted\)/);
    expect(AGENTS_CSS).not.toMatch(/__phase-title[^{]*\{[^}]*color/);
    expect(AGENTS_CSS).not.toMatch(/\[data-status='[a-z_]+'\] \.br-agents__(?:name|phase-title)/);
    for (const selector of [
      '.br-agents__meta, .br-agents__trailing, .br-agents__phase-counts',
      '.br-agents__more',
      '.br-agents__key',
    ]) {
      expect(property(agentsRule(selector), 'color'), selector).toBe('var(--text-muted)');
    }
    // Every text ink in the rail is a token: the default, the muted or the accent (a running
    // to-do's glyph). Never the subtle step, a mix or a literal.
    for (const [name, css] of [
      ['chatSummary.css', SUMMARY_CSS],
      ['summaryAgents.css', AGENTS_CSS],
      ['summaryDecisions.css', DECISIONS_CSS],
    ]) {
      const inks = [...css.matchAll(/(?:^|[\s;{])color:\s*([^;]+);/g)].map((m) => m[1].trim());
      expect(inks.length, name).toBeGreaterThan(0);
      for (const ink of inks) {
        expect(
          ['var(--text-default)', 'var(--text-muted)', 'var(--text-accent)', 'inherit'],
          `${name}: ${ink}`
        ).toContain(ink);
      }
      expect(css, name).not.toMatch(/--text-subtle/);
    }
    // Text never fades by opacity (only the quiet "Add gate" plus does, in summaryDecisions.css).
    expect(AGENTS_CSS).not.toMatch(/opacity:/);
  });

  it('keeps a phase title to one line, cut with an ellipsis', () => {
    const title = agentsRule('.br-agents__phase-title');
    expect(property(title, 'white-space')).toBe('nowrap');
    expect(property(title, 'text-overflow')).toBe('ellipsis');
    expect(property(title, 'overflow')).toBe('hidden');
    expect(title).not.toMatch(/line-clamp/);
  });

  it('draws its rows as plain buttons, so the card’s three Buttons stay three', () => {
    expect(AGENTS_TSX).not.toMatch(/<Button\b/);
    expect(AGENTS_TSX).toMatch(/^import '\.\/summaryAgents\.css';$/m);
    expect(AGENTS_CSS).not.toMatch(/@layer/);
  });
});
