import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import {
  TOAST_CLEAR_BELOW_GAP,
  TOAST_CLEAR_BESIDE_GAP,
  TOAST_CLEAR_MIN_BOTTOM,
  TOAST_CLEAR_WIDTH,
  TOAST_CLEAR_X,
  TOAST_CLEAR_Y,
  TOAST_OBSTACLES,
  TOAST_PLACED_ATTRIBUTE,
  TOAST_ASIDE,
  TOAST_ASIDE_ATTRIBUTE,
  TOAST_ASIDE_SIBLING_GAP,
  toastClearance,
  toastAside,
  watchToastClearance,
  type ClearanceBox,
} from './toastClearance';

/**
 * Vision V2-R7-4: a toast in the top-right corner covered find in chat's controls and the docked
 * Chat summary's first rows. The geometry is the vision pass's own, measured at 1300 and 1440
 * wide: the toast at 1004,44 to 1284,109, the find bar at 844,52 to 1284,94, the docked rail's
 * card from x 1160 at 1440.
 */
const box = (left: number, top: number, right: number, bottom: number): ClearanceBox => ({
  left,
  top,
  right,
  bottom,
});

/** The toast stack at rest in a 1300px window: 280 wide, 16 in from the right, at 44. */
const STACK_1300 = box(1004, 44, 1284, 109);
const FIND_1300 = box(844, 52, 1284, 94);
/** At 1440: the stack, the docked rail's card, and find open in the conversation beside it. */
const STACK_1440 = box(1144, 44, 1424, 109);
/** A card whose to-do list fills the rail: it runs to 12px above the 900px window's bottom. */
const RAIL_CARD_1440 = box(1160, 56, 1428, 888);
/**
 * The card as the final app check measured it (screens 03, 04, 12 to 14): no to-do list yet, so it
 * ends under its Diagnostics row, at 250.
 */
const SHORT_RAIL_CARD_1440 = box(1160, 56, 1428, 250);
const FIND_BESIDE_RAIL_1440 = box(704, 52, 1144, 94);
/** The window's height in the fixtures above. */
const BOTTOM = 900;

describe('toastClearance: where the stack goes', () => {
  it('stays in the corner when nothing is there', () => {
    expect(toastClearance(STACK_1300, [])).toEqual({ x: 0, y: 0 });
  });

  it('steps 8px below an open find bar, keeping its right edge (the finding at 1300)', () => {
    const { x, y } = toastClearance(STACK_1300, [{ side: 'below', box: FIND_1300 }]);
    expect(x).toBe(0);
    expect(STACK_1300.top + y).toBe(FIND_1300.bottom + TOAST_CLEAR_BELOW_GAP);
    expect(TOAST_CLEAR_BELOW_GAP).toBe(8);
  });

  it('steps 16px left of a surface it stands beside, at the same height', () => {
    const { x, y } = toastClearance(STACK_1440, [{ side: 'beside', box: RAIL_CARD_1440 }]);
    expect(y).toBe(0);
    expect(STACK_1440.right - x).toBe(RAIL_CARD_1440.left - TOAST_CLEAR_BESIDE_GAP);
    expect(TOAST_CLEAR_BESIDE_GAP).toBe(16);
  });

  /**
   * Final app check, "Toasts do not cover transcript content": stepped left of the docked rail,
   * the sticky "1 of 2 extensions loaded" stood at x 865 to 1143, over the person's own message
   * for minutes. Beside the rail IS the conversation. So the stack joins the rail's column
   * instead: as wide as the card, its right edge on the card's, 8px under it.
   */
  it('joins the docked rail: under the card, as wide as it, never over the conversation', () => {
    const { x, y, width } = toastClearance(
      STACK_1440,
      [{ side: 'column', box: SHORT_RAIL_CARD_1440 }],
      { bottom: BOTTOM }
    );
    expect(width).toBe(SHORT_RAIL_CARD_1440.right - SHORT_RAIL_CARD_1440.left);
    expect(STACK_1440.right - x).toBe(SHORT_RAIL_CARD_1440.right);
    expect(STACK_1440.right - x - width!).toBe(SHORT_RAIL_CARD_1440.left);
    expect(STACK_1440.top + y).toBe(SHORT_RAIL_CARD_1440.bottom + TOAST_CLEAR_BELOW_GAP);
  });

  it('over a card that fills the rail it stays at the rail top, still in the column', () => {
    // 8px under a card ending at 888 is off a 900px window: the stack keeps its corner height
    // and covers the card's header, which is the rail's, not the person's conversation.
    const { x, y, width } = toastClearance(STACK_1440, [{ side: 'column', box: RAIL_CARD_1440 }], {
      bottom: BOTTOM,
    });
    expect(y).toBe(0);
    expect(width).toBe(RAIL_CARD_1440.right - RAIL_CARD_1440.left);
    expect(STACK_1440.right - x).toBe(RAIL_CARD_1440.right);
  });

  it('goes under the card only when the whole stack fits above the window bottom inset', () => {
    const height = STACK_1440.bottom - STACK_1440.top;
    const fits = BOTTOM - TOAST_CLEAR_MIN_BOTTOM - TOAST_CLEAR_BELOW_GAP - height;
    const at = (bottom: number) =>
      toastClearance(STACK_1440, [{ side: 'column', box: box(1160, 56, 1428, bottom) }], {
        bottom: BOTTOM,
      }).y;
    expect(at(fits)).toBe(fits + TOAST_CLEAR_BELOW_GAP - STACK_1440.top);
    expect(at(fits + 1)).toBe(0);
    expect(TOAST_CLEAR_MIN_BOTTOM).toBe(16);
  });

  it('with find open beside the docked rail, the stack stays in the column and clears both', () => {
    for (const card of [SHORT_RAIL_CARD_1440, RAIL_CARD_1440]) {
      const { x, y, width } = toastClearance(
        STACK_1440,
        [
          { side: 'below', box: FIND_BESIDE_RAIL_1440 },
          { side: 'column', box: card },
        ],
        { bottom: BOTTOM }
      );
      const at = box(STACK_1440.right - x - width!, STACK_1440.top + y, STACK_1440.right - x, 0);
      at.bottom = at.top + (STACK_1440.bottom - STACK_1440.top);
      // The conversation ends 16px left of the card, where find's right edge sits.
      expect(at.left).toBeGreaterThanOrEqual(FIND_BESIDE_RAIL_1440.right);
      expect(at.left).toBe(card.left);
      expect(
        at.top >= card.bottom || y === 0,
        'under the card, or at the rail top over a full one'
      ).toBe(true);
    }
  });

  it('a rail that is not in the way (beside a side preview) moves nothing', () => {
    const railBesidePreview = box(700, 56, 968, 250);
    expect(
      toastClearance(STACK_1440, [{ side: 'column', box: railBesidePreview }], { bottom: BOTTOM })
    ).toEqual({ x: 0, y: 0 });
  });

  it('steps left of the summary popover, then below a find bar it now meets', () => {
    const popover = box(1004, 48, 1284, 892);
    const { x, y } = toastClearance(STACK_1300, [
      { side: 'beside', box: popover },
      { side: 'below', box: FIND_1300 },
    ]);
    expect(STACK_1300.right - x).toBe(popover.left - 16);
    expect(STACK_1300.top + y).toBe(FIND_1300.bottom + 8);
  });

  it('moves nothing for a surface that is not in the way', () => {
    // Find open in the left pane of a split: the bar ends far left of the stack.
    const leftPane = box(300, 52, 634, 94);
    // A surface lower down than a one-toast stack reaches.
    const lower = box(1004, 300, 1284, 340);
    expect(
      toastClearance(STACK_1300, [
        { side: 'below', box: leftPane },
        { side: 'below', box: lower },
        { side: 'beside', box: lower },
      ])
    ).toEqual({ x: 0, y: 0 });
  });

  it('reaches a lower surface once the stack has grown down to it', () => {
    const tall = box(1004, 44, 1284, 320);
    const lower = box(1004, 300, 1284, 340);
    expect(toastClearance(tall, [{ side: 'below', box: lower }]).y).toBe(340 + 8 - 44);
  });

  it('ignores a surface with no area (display none, or not laid out yet)', () => {
    expect(toastClearance(STACK_1300, [{ side: 'beside', box: box(1100, 50, 1100, 50) }])).toEqual({
      x: 0,
      y: 0,
    });
  });

  it('does not step left off the window: it stays, and a popover stands above it anyway', () => {
    // A 300px window: a 280 stack cannot sit left of a popover that starts at 20.
    const narrow = box(4, 44, 284, 109);
    expect(toastClearance(narrow, [{ side: 'beside', box: box(20, 48, 292, 600) }])).toEqual({
      x: 0,
      y: 0,
    });
  });

  it('never rises, so it can never enter the drag band', () => {
    const cases = [
      [FIND_1300, RAIL_CARD_1440, FIND_BESIDE_RAIL_1440],
      [box(0, 0, 2000, 30)],
      [box(1200, 0, 1300, 50), box(900, 40, 1300, 100)],
    ];
    for (const surfaces of cases) {
      for (const side of ['below', 'beside', 'column'] as const) {
        const { x, y } = toastClearance(
          STACK_1300,
          surfaces.map((b) => ({ side, box: b })),
          { bottom: BOTTOM }
        );
        expect(y).toBeGreaterThanOrEqual(0);
        // Left, or (joining a column) right only as far as that surface's own right edge.
        const limit = Math.max(
          0,
          ...surfaces.map((b) => (side === 'column' ? b.right - STACK_1300.right : 0))
        );
        expect(-x).toBeLessThanOrEqual(limit);
      }
    }
  });
});

/**
 * Final smoke (2026-10-12), the Extensions page at 1440: the sticky "1 of 2 extensions loaded"
 * report at 1144,44 to 1424,146 stood over the load-failure notice's actions, which end at the
 * note's 12px inner edge (1208): Copy error ran 1086 to 1176, the dismiss mark after it. The
 * notice's name, "CDWAgent failed to load", ends at 697 on the same row.
 */
const REPORT_1440 = box(1144, 44, 1424, 146);
const NOTICE_ACTIONS_1440 = box(975, 66, 1208, 94);
const NOTICE_NAME_1440 = box(544, 70, 697, 90);

describe('toastAside: how far a group of page actions steps aside', () => {
  it('moves out from under the stack and 16px clear of it (the finding at 1440)', () => {
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, NOTICE_NAME_1440)).toBe(
      NOTICE_ACTIONS_1440.right - (REPORT_1440.left - TOAST_CLEAR_BESIDE_GAP)
    );
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, NOTICE_NAME_1440)).toBe(80);
  });

  it('is zero with no stack, or one with no area', () => {
    expect(toastAside(null, NOTICE_ACTIONS_1440)).toBe(0);
    expect(toastAside(box(1144, 44, 1144, 44), NOTICE_ACTIONS_1440)).toBe(0);
  });

  it('is zero for actions above or below the stack, however far right they reach', () => {
    expect(toastAside(REPORT_1440, box(975, 150, 1208, 178))).toBe(0);
    expect(toastAside(box(1144, 102, 1424, 204), NOTICE_ACTIONS_1440)).toBe(0);
  });

  it('keeps the gap for actions that end just short of the stack, and leaves them alone past it', () => {
    expect(toastAside(REPORT_1440, box(903, 66, 1136, 94))).toBe(8);
    expect(toastAside(REPORT_1440, box(895, 66, 1128, 94))).toBe(0);
  });

  it('stops 8px short of the name before them on their row, even with the stack still there', () => {
    const longName = box(544, 70, 920, 90);
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, longName)).toBe(
      NOTICE_ACTIONS_1440.left - (longName.right + TOAST_ASIDE_SIBLING_GAP)
    );
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, longName)).toBe(47);
    // Already touching: nowhere to go, and never right.
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, box(544, 70, 980, 90))).toBe(0);
  });

  it('is not held back by an element on another row', () => {
    expect(toastAside(REPORT_1440, NOTICE_ACTIONS_1440, box(544, 96, 1200, 128))).toBe(80);
  });
});

describe('TOAST_OBSTACLES names surfaces that exist', () => {
  /**
   * The selectors are other owners' class names (find in chat, the summary rail and popover),
   * so a rename there would silently put the toast back over them. Each is held at its source.
   */
  const SOURCES: Record<string, string> = {
    '.br-find-overlay': '../conversation/SearchBar.tsx',
    '.br-summary-card': '../summary/ChatSummaryRail.tsx',
    '.br-summary-popover': '../summary/SummaryHeaderButton.tsx',
  };

  it('lists find in chat below, the docked rail as a column to join, the popover beside', () => {
    expect(TOAST_OBSTACLES.map((o) => [o.selector, o.side])).toEqual([
      ['.br-find-overlay', 'below'],
      ['.br-summary-card', 'column'],
      ['.br-summary-popover', 'beside'],
    ]);
  });

  it('each class is the one its component renders', () => {
    for (const { selector } of TOAST_OBSTACLES) {
      const source = readFileSync(join(__dirname, SOURCES[selector]), 'utf8');
      expect(source, SOURCES[selector]).toMatch(new RegExp(`className="${selector.slice(1)}[" ]`));
    }
  });
});

describe('the page actions that step aside, and the rule that moves them', () => {
  /**
   * The attribute is written in another owner's component and read here and in main.css, so a
   * rename on any side would silently put the toast back over the notice's actions.
   */
  it('the load-failure notice marks its actions, right after its name', () => {
    const source = readFileSync(
      join(__dirname, '../extensions/ExtensionLoadFailureNotice.tsx'),
      'utf8'
    );
    expect(source).toContain(`<div className="br-ext-notice-actions" ${TOAST_ASIDE_ATTRIBUTE}="">`);
    // The name is the element before them: the one the step stops short of.
    expect(source.indexOf('br-ext-notice-name')).toBeLessThan(
      source.indexOf('br-ext-notice-actions')
    );
  });

  it('main.css turns the step into a translate, so nothing else moves', () => {
    const css = readFileSync(join(__dirname, '../../styles/main.css'), 'utf8');
    expect(css).toContain(
      `[${TOAST_ASIDE_ATTRIBUTE}] {\n  translate: calc(-1 * var(${TOAST_ASIDE}, 0px)) 0;\n}`
    );
  });
});

describe('watchToastClearance', () => {
  let stop: (() => void) | null = null;

  afterEach(() => {
    stop?.();
    stop = null;
    document.body.innerHTML = '';
  });

  const frames = (n = 3) =>
    new Promise<void>((done) => {
      const step = (left: number) =>
        left === 0 ? done() : requestAnimationFrame(() => step(left - 1));
      step(n);
    });

  /** jsdom has no layout: give an element the box the browser would. */
  function sized(el: HTMLElement, r: ClearanceBox) {
    el.getBoundingClientRect = () =>
      ({ ...r, x: r.left, y: r.top, width: r.right - r.left, height: r.bottom - r.top }) as DOMRect;
    Object.defineProperty(el, 'offsetWidth', { configurable: true, value: r.right - r.left });
    Object.defineProperty(el, 'offsetHeight', { configurable: true, value: r.bottom - r.top });
    return el;
  }

  function layer() {
    const section = document.createElement('section');
    section.className = 'Toastify';
    document.body.appendChild(section);
    return section;
  }

  function container(section: HTMLElement) {
    const el = sized(document.createElement('div'), STACK_1300);
    el.className = 'Toastify__toast-container Toastify__toast-container--top-right';
    el.style.top = '44px';
    el.style.right = '16px';
    section.appendChild(el);
    return el;
  }

  function findBar() {
    const el = sized(document.createElement('div'), FIND_1300);
    el.className = 'br-find-overlay';
    document.body.appendChild(el);
    return el;
  }

  it('places a new container below an open find bar before it is ever marked placed', async () => {
    Object.defineProperty(document.documentElement, 'clientWidth', {
      configurable: true,
      value: 1300,
    });
    const section = layer();
    findBar();
    stop = watchToastClearance();
    const el = container(section);
    await frames(1);
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('58px');
    expect(el.style.getPropertyValue(TOAST_CLEAR_X)).toBe('0px');
    expect(el.hasAttribute(TOAST_PLACED_ATTRIBUTE)).toBe(false);
    await frames(2);
    expect(el.hasAttribute(TOAST_PLACED_ATTRIBUTE)).toBe(true);
  });

  it('follows find opening and closing while the toast stands', async () => {
    Object.defineProperty(document.documentElement, 'clientWidth', {
      configurable: true,
      value: 1300,
    });
    const section = layer();
    stop = watchToastClearance();
    const el = container(section);
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('0px');
    const bar = findBar();
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('58px');
    bar.remove();
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('0px');
  });

  it('joins a docked rail with the card width, and lets go of the width when the rail closes', async () => {
    Object.defineProperty(document.documentElement, 'clientWidth', {
      configurable: true,
      value: 1440,
    });
    Object.defineProperty(document.documentElement, 'clientHeight', {
      configurable: true,
      value: BOTTOM,
    });
    const section = layer();
    stop = watchToastClearance();
    const el = sized(document.createElement('div'), STACK_1440);
    el.className = 'Toastify__toast-container Toastify__toast-container--top-right';
    el.style.top = '44px';
    el.style.right = '16px';
    section.appendChild(el);
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_WIDTH)).toBe('');

    const card = sized(document.createElement('div'), SHORT_RAIL_CARD_1440);
    card.className = 'br-summary-card';
    document.body.appendChild(card);
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_WIDTH)).toBe('268px');
    expect(el.style.getPropertyValue(TOAST_CLEAR_X)).toBe('-4px');
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('214px');

    card.remove();
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_WIDTH)).toBe('');
    expect(el.style.getPropertyValue(TOAST_CLEAR_X)).toBe('0px');
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('0px');
  });

  /**
   * The final smoke's page at 1440: the notice's name and its actions, and the report standing
   * over them. The actions' box follows their own `translate`, as the browser's does.
   */
  function smokePage() {
    Object.defineProperty(document.documentElement, 'clientWidth', {
      configurable: true,
      value: 1440,
    });
    Object.defineProperty(document.documentElement, 'clientHeight', {
      configurable: true,
      value: BOTTOM,
    });
    const section = layer();
    const row = document.createElement('div');
    row.appendChild(sized(document.createElement('div'), NOTICE_NAME_1440));
    const actions = document.createElement('div');
    actions.getBoundingClientRect = () => {
      const shift = parseFloat(actions.style.getPropertyValue(TOAST_ASIDE)) || 0;
      const r = NOTICE_ACTIONS_1440;
      return {
        left: r.left - shift,
        right: r.right - shift,
        top: r.top,
        bottom: r.bottom,
        x: r.left - shift,
        y: r.top,
        width: r.right - r.left,
        height: r.bottom - r.top,
      } as DOMRect;
    };
    actions.setAttribute(TOAST_ASIDE_ATTRIBUTE, '');
    row.appendChild(actions);
    document.body.appendChild(row);
    const report = () => {
      const el = sized(document.createElement('div'), REPORT_1440);
      el.className = 'Toastify__toast-container Toastify__toast-container--top-right';
      el.style.top = '44px';
      el.style.right = '16px';
      section.appendChild(el);
      return el;
    };
    return { actions, report };
  }

  it('steps a page’s actions aside while a toast stands beside them, and back when it goes', async () => {
    const { actions, report } = smokePage();
    stop = watchToastClearance();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('');
    const el = report();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('80px');
    el.remove();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('');
  });

  it('keeps the actions under a resting pointer when the toast goes, until the pointer leaves', async () => {
    const { actions, report } = smokePage();
    let hovered = false;
    actions.matches = (selector: string) =>
      selector === ':hover' ? hovered : Element.prototype.matches.call(actions, selector);
    stop = watchToastClearance();
    const el = report();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('80px');
    hovered = true;
    el.remove();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('80px');
    hovered = false;
    actions.dispatchEvent(new Event('pointerleave'));
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('');
  });

  it('gives the actions back when it stops watching', async () => {
    const { actions, report } = smokePage();
    stop = watchToastClearance();
    report();
    await frames();
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('80px');
    stop();
    stop = null;
    expect(actions.style.getPropertyValue(TOAST_ASIDE)).toBe('');
  });

  it('stops following the page once the last toast has gone', async () => {
    Object.defineProperty(document.documentElement, 'clientWidth', {
      configurable: true,
      value: 1300,
    });
    const section = layer();
    stop = watchToastClearance();
    const el = container(section);
    await frames();
    el.remove();
    await frames();
    // A find bar opening now measures nothing and writes nothing: there is no stack.
    findBar();
    await frames();
    expect(el.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('0px');
    // A new toast arrives: it is placed afresh, below the bar.
    const next = container(section);
    await frames(1);
    expect(next.style.getPropertyValue(TOAST_CLEAR_Y)).toBe('58px');
    expect(next.hasAttribute(TOAST_PLACED_ATTRIBUTE)).toBe(false);
  });
});
