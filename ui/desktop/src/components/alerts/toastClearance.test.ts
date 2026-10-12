import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';

import {
  TOAST_CLEAR_BELOW_GAP,
  TOAST_CLEAR_BESIDE_GAP,
  TOAST_CLEAR_X,
  TOAST_CLEAR_Y,
  TOAST_OBSTACLES,
  TOAST_PLACED_ATTRIBUTE,
  toastClearance,
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
const RAIL_CARD_1440 = box(1160, 56, 1428, 888);
const FIND_BESIDE_RAIL_1440 = box(704, 52, 1144, 94);

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

  it('steps 16px left of the docked rail, at the same height (the finding at 1440)', () => {
    const { x, y } = toastClearance(STACK_1440, [{ side: 'beside', box: RAIL_CARD_1440 }]);
    expect(y).toBe(0);
    expect(STACK_1440.right - x).toBe(RAIL_CARD_1440.left - TOAST_CLEAR_BESIDE_GAP);
    expect(TOAST_CLEAR_BESIDE_GAP).toBe(16);
  });

  it('clears both when find is open beside the docked rail', () => {
    const { x, y } = toastClearance(STACK_1440, [
      { side: 'below', box: FIND_BESIDE_RAIL_1440 },
      { side: 'beside', box: RAIL_CARD_1440 },
    ]);
    expect(STACK_1440.right - x).toBe(RAIL_CARD_1440.left - 16);
    // The conversation ends where the rail's card starts, and find sits 16px in from that edge:
    // the two right edges line up.
    expect(STACK_1440.right - x).toBe(FIND_BESIDE_RAIL_1440.right);
    expect(STACK_1440.top + y).toBe(FIND_BESIDE_RAIL_1440.bottom + 8);
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

  it('only ever moves down and left, so it can never rise into the drag band', () => {
    const cases = [
      [FIND_1300, RAIL_CARD_1440, FIND_BESIDE_RAIL_1440],
      [box(0, 0, 2000, 30)],
      [box(1200, 0, 1300, 50), box(900, 40, 1300, 100)],
    ];
    for (const surfaces of cases) {
      for (const side of ['below', 'beside'] as const) {
        const { x, y } = toastClearance(
          STACK_1300,
          surfaces.map((b) => ({ side, box: b }))
        );
        expect(x).toBeGreaterThanOrEqual(0);
        expect(y).toBeGreaterThanOrEqual(0);
      }
    }
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

  it('lists find in chat below, the rail and the popover beside', () => {
    expect(TOAST_OBSTACLES.map((o) => [o.selector, o.side])).toEqual([
      ['.br-find-overlay', 'below'],
      ['.br-summary-card', 'beside'],
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
