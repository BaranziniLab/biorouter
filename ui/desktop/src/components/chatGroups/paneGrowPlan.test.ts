import { describe, expect, it } from 'vitest';
import { GroupLayout, leafGroupIds } from './chatGroupsTypes';
import { MAX_GROUPS, setSizesAtPath } from './chatGroupsLayout';
import {
  PANE_GROW_HEIGHT,
  PANE_GROW_WIDTH,
  PANE_KEEP_HEIGHT,
  PANE_KEEP_WIDTH,
  PANE_SEAT_HEIGHT,
  PANE_SEAT_WIDTH,
  PANE_YIELD_HEIGHT,
  PANE_YIELD_WIDTH,
  type PaneResize,
  growPanePlan,
  panePixelBoxes,
  planPaneAxis,
} from './paneGrowPlan';
import {
  COMPACT_CARD_MIN_HEIGHT,
  COMPACT_CARD_MIN_WIDTH,
  PANE_MIN_HEIGHT,
  PANE_MIN_WIDTH,
  paneFit,
} from '../Layout/yieldLadder';

const leaf = (groupId: string): GroupLayout => ({ kind: 'leaf', groupId });
const branch =
  (dir: 'row' | 'col') =>
  (...children: GroupLayout[]): GroupLayout => ({
    kind: 'branch',
    dir,
    children,
    sizes: children.map(() => 1 / children.length),
  });
const row = branch('row');
const col = branch('col');
const sized = (layout: GroupLayout, sizes: number[]): GroupLayout =>
  layout.kind === 'branch' ? { ...layout, sizes } : layout;

function apply(layout: GroupLayout, plan: PaneResize[]): GroupLayout {
  return plan.reduce((next, entry) => setSizesAtPath(next, entry.path, entry.sizes), layout);
}

function widths(layout: GroupLayout, tree: { width: number; height: number }) {
  const boxes = panePixelBoxes(layout, tree);
  return leafGroupIds(layout).map((id) => boxes.get(id)!.width);
}

function heights(layout: GroupLayout, tree: { width: number; height: number }) {
  const boxes = panePixelBoxes(layout, tree);
  return leafGroupIds(layout).map((id) => boxes.get(id)!.height);
}

function expectWithin(actual: number[], expected: number[], tolerance = 1) {
  expect(actual.length).toBe(expected.length);
  actual.forEach((value, index) => {
    expect(
      Math.abs(value - expected[index]),
      `index ${index}: ${actual.map((v) => v.toFixed(1)).join(' / ')}`
    ).toBeLessThanOrEqual(tolerance);
  });
}

function branchesOf(layout: GroupLayout, path: number[] = []): { path: number[]; dir: string }[] {
  if (layout.kind === 'leaf') return [];
  return [
    { path, dir: layout.dir },
    ...layout.children.flatMap((child, index) => branchesOf(child, [...path, index])),
  ];
}

describe('the grow constants', () => {
  it('are built from the ladder', () => {
    expect(PANE_GROW_WIDTH).toBe(496);
    expect(PANE_GROW_HEIGHT).toBe(320);
    expect(PANE_SEAT_WIDTH).toBe(376);
    expect(PANE_SEAT_HEIGHT).toBe(286);
    expect(PANE_KEEP_WIDTH).toBe(362);
    expect(PANE_KEEP_HEIGHT).toBe(272);
    expect(PANE_YIELD_WIDTH).toBe(64);
    expect(PANE_YIELD_HEIGHT).toBe(88);
  });
});

describe('panePixelBoxes', () => {
  it('gives a lone leaf the whole tree', () => {
    expect(panePixelBoxes(leaf('a'), { width: 800, height: 600 }).get('a')).toEqual({
      width: 800,
      height: 600,
    });
  });

  it('divides a row after its 1px splitters', () => {
    const boxes = panePixelBoxes(row(leaf('a'), leaf('b'), leaf('c')), {
      width: 1002,
      height: 600,
    });
    for (const id of ['a', 'b', 'c']) {
      expect(boxes.get(id)!.width).toBeCloseTo(1000 / 3, 9);
      expect(boxes.get(id)!.height).toBe(600);
    }
  });

  it('divides a column by height and gives every child the width', () => {
    const boxes = panePixelBoxes(sized(col(leaf('a'), leaf('b')), [0.25, 0.75]), {
      width: 500,
      height: 801,
    });
    expect(boxes.get('a')).toEqual({ width: 500, height: 200 });
    expect(boxes.get('b')).toEqual({ width: 500, height: 600 });
  });

  it('nests: a column inside a row', () => {
    const layout = row(leaf('a'), col(leaf('b'), leaf('c')));
    const boxes = panePixelBoxes(layout, { width: 1001, height: 601 });
    expect(boxes.get('a')).toEqual({ width: 500, height: 601 });
    expect(boxes.get('b')).toEqual({ width: 500, height: 300 });
    expect(boxes.get('c')).toEqual({ width: 500, height: 300 });
  });

  it('nests: a row inside a column inside a row', () => {
    const layout = row(leaf('a'), col(leaf('b'), row(leaf('c'), leaf('d'))));
    const boxes = panePixelBoxes(layout, { width: 1001, height: 601 });
    expect(boxes.get('c')).toEqual({ width: 249.5, height: 300 });
    expect(boxes.get('d')).toEqual({ width: 249.5, height: 300 });
  });

  it('falls back to an even split when sizes disagree with the children, as the renderer does', () => {
    const layout: GroupLayout = {
      kind: 'branch',
      dir: 'row',
      children: [leaf('a'), leaf('b')],
      sizes: [1],
    };
    expect(widths(layout, { width: 1001, height: 600 })).toEqual([500, 500]);
  });
});

describe('growPanePlan: the worked examples (spec 3.4)', () => {
  it('flat four at 1152: 218 / 218 / 218 / 496', () => {
    const layout = row(leaf('a'), leaf('b'), leaf('c'), leaf('d'));
    const tree = { width: 1152, height: 900 };
    const plan = growPanePlan({ layout, groupId: 'd', tree, shortfall: 'width' });
    expect(plan).not.toBeNull();
    expectWithin(widths(apply(layout, plan!), tree), [218, 218, 218, 496]);
  });

  it('nested at 1152: 454 / 176 / 143 / 376', () => {
    const layout = row(leaf('a'), row(leaf('b'), row(leaf('c'), leaf('d'))));
    const tree = { width: 1152, height: 900 };
    expectWithin(widths(layout, tree), [575.5, 287.25, 143.125, 143.125], 0.01);
    const plan = growPanePlan({ layout, groupId: 'd', tree, shortfall: 'width' });
    expect(plan).not.toBeNull();
    expectWithin(widths(apply(layout, plan!), tree), [454, 176, 143, 376]);
  });

  it('two at 700: 203 / 496', () => {
    const layout = row(leaf('a'), leaf('b'));
    const tree = { width: 700, height: 900 };
    const plan = growPanePlan({ layout, groupId: 'b', tree, shortfall: 'width' });
    expectWithin(widths(apply(layout, plan!), tree), [203, 496]);
  });

  it('0.7 / 0.3 at 1000: 503 / 496', () => {
    const layout = sized(row(leaf('a'), leaf('b')), [0.7, 0.3]);
    const tree = { width: 1000, height: 900 };
    const plan = growPanePlan({ layout, groupId: 'b', tree, shortfall: 'width' });
    expectWithin(widths(apply(layout, plan!), tree), [503, 496]);
  });

  it('three stacked at 600: 156 / 286 / 156', () => {
    const layout = col(leaf('a'), leaf('b'), leaf('c'));
    const tree = { width: 1152, height: 600 };
    const plan = growPanePlan({ layout, groupId: 'b', tree, shortfall: 'height' });
    expect(plan).not.toBeNull();
    expectWithin(heights(apply(layout, plan!), tree), [156, 286, 156]);
  });
});

describe('growPanePlan: the attempt order', () => {
  it('attempt 1 takes room only from compact panes and leaves readable ones untouched', () => {
    // a readable (720), b and c compact (350, 329), c is the pane: b alone can give c its 496.
    const layout = sized(row(leaf('a'), leaf('b'), leaf('c')), [0.515, 0.25, 0.235]);
    const tree = { width: 1400, height: 900 };
    const before = widths(layout, tree);
    expect(before[0]).toBeGreaterThanOrEqual(PANE_MIN_WIDTH);
    expect(before[1]).toBeLessThan(PANE_MIN_WIDTH);
    expect(before[2]).toBeLessThan(PANE_MIN_WIDTH);
    const axis = planPaneAxis({ layout, groupId: 'c', tree, axis: 'width' });
    expect(axis?.attempt).toBe(1);
    const after = widths(apply(layout, axis!.resizes), tree);
    expect(after[0]).toBeCloseTo(before[0], 6);
    expect(after[2]).toBeCloseTo(PANE_GROW_WIDTH, 6);
  });

  it('attempt 2 lets a readable pane give room down to 362, never under', () => {
    const layout = sized(row(leaf('a'), leaf('b')), [0.7, 0.3]);
    const tree = { width: 1000, height: 900 };
    const axis = planPaneAxis({ layout, groupId: 'b', tree, axis: 'width' });
    expect(axis?.attempt).toBe(2);
    const after = widths(apply(layout, axis!.resizes), tree);
    expect(after[0]).toBeGreaterThanOrEqual(PANE_KEEP_WIDTH);
  });

  it('attempts 1 to 3 never turn a readable pane compact, and keep compact panes at card size', () => {
    const layout = row(leaf('a'), row(leaf('b'), row(leaf('c'), leaf('d'))));
    const tree = { width: 1152, height: 900 };
    const axis = planPaneAxis({ layout, groupId: 'd', tree, axis: 'width' });
    expect(axis?.attempt).toBe(3);
    const after = widths(apply(layout, axis!.resizes), tree);
    expect(after[0]).toBeGreaterThanOrEqual(PANE_MIN_WIDTH);
    expect(after[1]).toBeGreaterThanOrEqual(COMPACT_CARD_MIN_WIDTH - 1e-6);
    // c was already under card size; it keeps its current size.
    expect(after[2]).toBeCloseTo(143.125, 6);
  });

  it('attempt 4 runs only when 1 to 3 are infeasible', () => {
    const layout = col(leaf('a'), leaf('b'), leaf('c'));
    const tree = { width: 1152, height: 600 };
    const axis = planPaneAxis({ layout, groupId: 'b', tree, axis: 'height' });
    expect(axis?.attempt).toBe(4);
    // Three stacked at 680 tall: 320 + 2 × 188 + 2 = 698 does not fit, 286 + 2 × 188 + 2 = 664
    // does, so attempt 3 seats the pane and attempt 4 never runs.
    const taller = planPaneAxis({
      layout,
      groupId: 'b',
      tree: { width: 1152, height: 680 },
      axis: 'height',
    });
    expect(taller?.attempt).toBe(3);
  });

  it('a branch that does not hold the pane keeps its sizes when scaling them fits', () => {
    // The pair a, b (559 each, readable) and c (280, compact). Attempt 2 shrinks the pair to 903,
    // which still gives a and b 451 each at their own 0.5 / 0.5.
    const layout = sized(row(row(leaf('a'), leaf('b')), leaf('c')), [0.8, 0.2]);
    const tree = { width: 1400, height: 900 };
    const plan = growPanePlan({ layout, groupId: 'c', tree, shortfall: 'width' })!;
    expect(plan).not.toBeNull();
    // Only the root changes: the inner pair is scaled, not re-cut.
    expect(plan.map((entry) => entry.path)).toEqual([[]]);
    expectWithin(widths(apply(layout, plan), tree), [451, 451, 496]);
  });

  it('emits sizes that sum to 1 for every branch it changes', () => {
    const layout = row(leaf('a'), row(leaf('b'), row(leaf('c'), leaf('d'))));
    const plan = growPanePlan({
      layout,
      groupId: 'd',
      tree: { width: 1152, height: 900 },
      shortfall: 'width',
    })!;
    expect(plan.length).toBeGreaterThan(0);
    for (const entry of plan) {
      expect(entry.sizes.reduce((a, b) => a + b, 0)).toBeCloseTo(1, 9);
    }
  });
});

describe('growPanePlan: the axes', () => {
  it('a height shortfall touches only column branches', () => {
    const layout = row(leaf('a'), col(leaf('b'), leaf('c'), leaf('d')));
    const plan = growPanePlan({
      layout,
      groupId: 'c',
      tree: { width: 1400, height: 600 },
      shortfall: 'height',
    })!;
    expect(plan).not.toBeNull();
    const dirs = new Map(branchesOf(layout).map((b) => [b.path.join('.'), b.dir]));
    for (const entry of plan) expect(dirs.get(entry.path.join('.'))).toBe('col');
  });

  it('a both shortfall touches both kinds', () => {
    const layout = row(leaf('a'), leaf('x'), leaf('y'), col(leaf('b'), leaf('c'), leaf('d')));
    const tree = { width: 1400, height: 600 };
    const box = panePixelBoxes(layout, tree).get('c')!;
    expect(paneFit({ ...box, alone: false, previous: null }).shortfall).toBe('both');
    const plan = growPanePlan({ layout, groupId: 'c', tree, shortfall: 'both' })!;
    expect(plan).not.toBeNull();
    const dirs = new Map(branchesOf(layout).map((b) => [b.path.join('.'), b.dir]));
    expect(new Set(plan.map((entry) => dirs.get(entry.path.join('.'))))).toEqual(
      new Set(['row', 'col'])
    );
    const after = panePixelBoxes(apply(layout, plan), tree).get('c')!;
    expect(after.width).toBeGreaterThanOrEqual(PANE_SEAT_WIDTH - 1e-6);
    expect(after.height).toBeGreaterThanOrEqual(PANE_SEAT_HEIGHT - 1e-6);
  });

  it('a both shortfall is null when either axis has no room', () => {
    const layout = row(leaf('a'), col(leaf('b'), leaf('c'), leaf('d'), leaf('e'), leaf('f')));
    expect(
      growPanePlan({ layout, groupId: 'c', tree: { width: 700, height: 400 }, shortfall: 'both' })
    ).toBeNull();
  });

  it('a width shortfall with no row ancestor is null', () => {
    const layout = col(leaf('a'), leaf('b'));
    expect(
      growPanePlan({ layout, groupId: 'b', tree: { width: 340, height: 900 }, shortfall: 'width' })
    ).toBeNull();
  });

  it('an unmeasured tree is null', () => {
    const layout = row(leaf('a'), leaf('b'));
    for (const tree of [
      { width: 0, height: 900 },
      { width: 700, height: 0 },
      { width: Number.NaN, height: 900 },
    ]) {
      expect(growPanePlan({ layout, groupId: 'b', tree, shortfall: 'width' })).toBeNull();
    }
  });

  it('is null for a pane the layout does not hold', () => {
    expect(
      growPanePlan({
        layout: row(leaf('a'), leaf('b')),
        groupId: 'zz',
        tree: { width: 700, height: 900 },
        shortfall: 'width',
      })
    ).toBeNull();
  });
});

/** A seeded PRNG (mulberry32), so a failure names a layout that can be replayed. */
function random(seed: number) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** A random layout of `count` leaves: split a random leaf until there are enough. */
function randomLayout(next: () => number, count: number): GroupLayout {
  let layout: GroupLayout = leaf('g0');
  for (let index = 1; index < count; index++) {
    const ids = leafGroupIds(layout);
    const target = ids[Math.floor(next() * ids.length)];
    const dir = next() < 0.5 ? 'row' : 'col';
    const walk = (node: GroupLayout): GroupLayout => {
      if (node.kind === 'leaf') {
        if (node.groupId !== target) return node;
        return { kind: 'branch', dir, children: [node, leaf(`g${index}`)], sizes: [0.5, 0.5] };
      }
      // Sometimes join a same-direction parent, so flat rows and columns appear too.
      if (
        node.dir === dir &&
        node.children.some((c) => c.kind === 'leaf' && c.groupId === target)
      ) {
        if (next() < 0.5) {
          const children = [...node.children, leaf(`g${index}`)];
          return { ...node, children, sizes: children.map(() => 1 / children.length) };
        }
      }
      return { ...node, children: node.children.map(walk) };
    };
    layout = walk(layout);
  }
  const shuffle = (node: GroupLayout): GroupLayout => {
    if (node.kind === 'leaf') return node;
    const raw = node.children.map(() => 0.12 + next());
    const total = raw.reduce((a, b) => a + b, 0);
    return { ...node, sizes: raw.map((s) => s / total), children: node.children.map(shuffle) };
  };
  return shuffle(layout);
}

describe('growPanePlan: the property', () => {
  const trees = [
    { width: 688, height: 600 },
    { width: 1152, height: 900 },
  ];

  it('seats the pane and keeps every other pane at its attempt floor, for every plan it returns', () => {
    const next = random(20261116);
    let plans = 0;
    for (let sample = 0; sample < 400; sample++) {
      const count = 2 + Math.floor(next() * (MAX_GROUPS - 1));
      const layout = randomLayout(next, count);
      for (const tree of trees) {
        const boxes = panePixelBoxes(layout, tree);
        for (const groupId of leafGroupIds(layout)) {
          const box = boxes.get(groupId)!;
          const answer = paneFit({ ...box, alone: false, previous: null });
          if (answer.fit === 'full') continue;
          const plan = growPanePlan({ layout, groupId, tree, shortfall: answer.shortfall! });
          if (!plan) continue;
          plans++;
          const after = panePixelBoxes(apply(layout, plan), tree);
          const where = `${JSON.stringify(layout)} @ ${tree.width}x${tree.height} grow ${groupId}`;
          for (const axis of ['width', 'height'] as const) {
            const short =
              answer.shortfall === 'both' ||
              (axis === 'width' ? answer.shortfall === 'width' : answer.shortfall === 'height');
            if (!short) continue;
            const seat = axis === 'width' ? PANE_SEAT_WIDTH : PANE_SEAT_HEIGHT;
            expect(after.get(groupId)![axis], where).toBeGreaterThanOrEqual(seat - 1e-6);
            const detail = planPaneAxis({ layout, groupId, tree, axis })!;
            for (const [other, floor] of detail.floors) {
              expect(
                after.get(other)![axis],
                `${where}: ${other} on ${axis}`
              ).toBeGreaterThanOrEqual(floor - 1e-6);
            }
          }
        }
      }
    }
    // The generator must actually exercise the planner.
    expect(plans).toBeGreaterThan(100);
  });

  it('always has a width plan for a row chain of up to five at 688 by 600', () => {
    const next = random(7);
    const tree = { width: 688, height: 600 };
    for (let count = 2; count <= 5; count++) {
      for (let sample = 0; sample < 40; sample++) {
        // A nested chain, as drag-splitting builds it, and a flat row.
        let chain: GroupLayout = leaf(`g${count - 1}`);
        for (let index = count - 2; index >= 0; index--) chain = row(leaf(`g${index}`), chain);
        const flat = row(...Array.from({ length: count }, (_, i) => leaf(`g${i}`)));
        for (const base of [chain, flat]) {
          const layout = randomSizes(base, next);
          const boxes = panePixelBoxes(layout, tree);
          for (const groupId of leafGroupIds(layout)) {
            if (boxes.get(groupId)!.width >= PANE_MIN_WIDTH) continue;
            expect(
              growPanePlan({ layout, groupId, tree, shortfall: 'width' }),
              `${JSON.stringify(layout)} grow ${groupId}`
            ).not.toBeNull();
          }
        }
      }
    }
  });

  it('always has a height plan for a column chain of up to four at 688 by 600', () => {
    const next = random(11);
    const tree = { width: 688, height: 600 };
    for (let count = 2; count <= 4; count++) {
      for (let sample = 0; sample < 40; sample++) {
        let chain: GroupLayout = leaf(`g${count - 1}`);
        for (let index = count - 2; index >= 0; index--) chain = col(leaf(`g${index}`), chain);
        const flat = col(...Array.from({ length: count }, (_, i) => leaf(`g${i}`)));
        for (const base of [chain, flat]) {
          const layout = randomSizes(base, next);
          const boxes = panePixelBoxes(layout, tree);
          for (const groupId of leafGroupIds(layout)) {
            if (boxes.get(groupId)!.height >= PANE_MIN_HEIGHT) continue;
            expect(
              growPanePlan({ layout, groupId, tree, shortfall: 'height' }),
              `${JSON.stringify(layout)} grow ${groupId}`
            ).not.toBeNull();
          }
        }
      }
    }
  });

  it('offers no plan past the bounds: six in a row at 688, five stacked at 600', () => {
    const six = row(...Array.from({ length: 6 }, (_, i) => leaf(`g${i}`)));
    expect(
      growPanePlan({
        layout: six,
        groupId: 'g0',
        tree: { width: 688, height: 600 },
        shortfall: 'width',
      })
    ).toBeNull();
    const five = col(...Array.from({ length: 5 }, (_, i) => leaf(`g${i}`)));
    expect(
      growPanePlan({
        layout: five,
        groupId: 'g2',
        tree: { width: 688, height: 600 },
        shortfall: 'height',
      })
    ).toBeNull();
    // The attempt floors the bounds are built from.
    expect(COMPACT_CARD_MIN_HEIGHT).toBe(188);
  });
});

function randomSizes(layout: GroupLayout, next: () => number): GroupLayout {
  if (layout.kind === 'leaf') return layout;
  const raw = layout.children.map(() => 0.12 + next());
  const total = raw.reduce((a, b) => a + b, 0);
  return {
    ...layout,
    sizes: raw.map((s) => s / total),
    children: layout.children.map((child) => randomSizes(child, next)),
  };
}
