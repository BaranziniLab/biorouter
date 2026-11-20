import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { CHAT_MEASURE } from '../Layout/yieldLadder';
import {
  HOME_USAGE_MIN_GAP,
  HOME_USAGE_MIN_WIDTH,
  homeUsageFit,
  type HomeUsageMeasure,
} from './homeUsageFit';

/**
 * The decision is tested here; the geometry it is fed, and that a card that shows leaves the gap
 * above the composer at every size, are measured in Chromium in `homeLayout.browser.test.tsx`.
 */

/** Home at 1048x600, measured in the running app: 498px above the composer, greeting 88px. */
const ROOM: HomeUsageMeasure = {
  columnWidth: 760,
  roomHeight: 498,
  restHeight: 88 + HOME_USAGE_MIN_GAP,
  fullHeight: 349,
  compactHeight: 180,
};

describe('homeUsageFit', () => {
  it('keeps the card whole before anything is measured', () => {
    expect(homeUsageFit(null)).toBe('full');
    for (const bad of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(homeUsageFit({ ...ROOM, columnWidth: bad })).toBe('full');
      expect(homeUsageFit({ ...ROOM, roomHeight: bad })).toBe('full');
      expect(homeUsageFit({ ...ROOM, fullHeight: bad })).toBe('full');
    }
  });

  it('shows the card only at its whole measure', () => {
    expect(HOME_USAGE_MIN_WIDTH).toBe(CHAT_MEASURE);
    expect(homeUsageFit({ ...ROOM, columnWidth: 760 })).toBe('full');
    // A fractional column on the boundary is the measure, not a miss.
    expect(homeUsageFit({ ...ROOM, columnWidth: 759.6 })).toBe('full');
    expect(homeUsageFit({ ...ROOM, columnWidth: 759 })).toBe('hidden');
    // The window's floor: 721 less Home's 2 x 24px gutters.
    expect(homeUsageFit({ ...ROOM, columnWidth: 673 })).toBe('hidden');
    // However tall the window: width alone hides it.
    expect(homeUsageFit({ ...ROOM, columnWidth: 712, roomHeight: 4000 })).toBe('hidden');
  });

  it('keeps the card whole while it and the greeting fit with the gap', () => {
    expect(homeUsageFit(ROOM)).toBe('full');
    const exact = ROOM.restHeight + ROOM.fullHeight;
    expect(homeUsageFit({ ...ROOM, roomHeight: exact })).toBe('full');
    expect(homeUsageFit({ ...ROOM, roomHeight: exact - 0.4 })).toBe('full');
  });

  it('drops the heat map before the card would touch the composer', () => {
    const exact = ROOM.restHeight + ROOM.fullHeight;
    expect(homeUsageFit({ ...ROOM, roomHeight: exact - 1 })).toBe('compact');
    // A greeting on two lines (one more 32px line) at the shortest window.
    expect(homeUsageFit({ ...ROOM, restHeight: ROOM.restHeight + 32 + 30 })).toBe('compact');
  });

  it('steps aside when even the tiles would not leave the gap', () => {
    const compactExact = ROOM.restHeight + ROOM.compactHeight;
    expect(homeUsageFit({ ...ROOM, roomHeight: compactExact })).toBe('compact');
    expect(homeUsageFit({ ...ROOM, roomHeight: compactExact - 1 })).toBe('hidden');
    // A compact height that was never measured is not a fit.
    expect(homeUsageFit({ ...ROOM, roomHeight: compactExact - 1, compactHeight: 0 })).toBe(
      'hidden'
    );
  });

  it('reads a negative or missing rest as none, never as extra room', () => {
    expect(homeUsageFit({ ...ROOM, restHeight: -50, roomHeight: ROOM.fullHeight })).toBe('full');
    expect(homeUsageFit({ ...ROOM, restHeight: Number.NaN, roomHeight: ROOM.fullHeight })).toBe(
      'full'
    );
  });

  /**
   * No loop: the answer is a function of the room, the greeting and the card's natural heights,
   * none of which the answer changes (a fold clips the card; it does not unmount or restyle it).
   * Feeding an answer's own consequence back in, the column shrinking by what was folded, must
   * not be one of the inputs, and is not.
   */
  it('has no input the answer can change', () => {
    const answer = homeUsageFit({ ...ROOM, roomHeight: 400 });
    expect(answer).toBe('compact');
    expect(homeUsageFit({ ...ROOM, roomHeight: 400 })).toBe(answer);
  });
});

describe('the fold the answer draws (main.css, usage.css)', () => {
  const styles = join(__dirname, '../../styles');
  const main = readFileSync(join(styles, 'main.css'), 'utf8');
  const usage = readFileSync(join(__dirname, '../usage/usage.css'), 'utf8');
  const rule = (css: string, selector: string) => {
    const at = css.indexOf(`${selector} {`);
    if (at < 0) throw new Error(`${selector} not found`);
    return css.slice(at, css.indexOf('}', at));
  };

  it('leaves the column’s bottom padding as the gap above the composer', () => {
    expect(rule(main, '.biorouter-home-column')).toMatch(
      new RegExp(`padding-block-end:\\s*${HOME_USAGE_MIN_GAP}px`)
    );
  });

  it('clips a folded card and a folded heat map instead of unmounting them', () => {
    const selector =
      ".biorouter-home-usage[data-fit='hidden'],\n.biorouter-home-usage[data-fit='compact'] .br-usage-card-extra";
    const body = rule(main, selector);
    expect(body).toMatch(/block-size:\s*0;/);
    expect(body).toMatch(/overflow:\s*clip;/);
    expect(body).toMatch(/visibility:\s*hidden;/);
    expect(body).not.toMatch(/display:\s*none/);
  });

  it('keeps the heat map’s top margin inside the box the fit measures', () => {
    expect(rule(usage, '.br-usage-card-extra-body')).toMatch(/display:\s*flow-root;/);
  });
});
