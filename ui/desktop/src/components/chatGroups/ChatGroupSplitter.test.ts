import { describe, expect, it } from 'vitest';
import { clampPairShares } from './ChatGroupSplitter';

/**
 * The splitter's clamp (compact panes, spec 8.3). Make wider, and a layout from storage, can leave a
 * pane under 12% of its pair. Clamping that pane up to 12% on the first pixel of a drag made it jump;
 * a pane already under the floor may grow, and never shrinks further.
 */
describe('clampPairShares', () => {
  it('leaves a pane at 5% of its pair where it is on a zero shift', () => {
    const [a, b] = clampPairShares(0.05, 0.45, 0);
    expect(a).toBeCloseTo(0.05, 12);
    expect(b).toBeCloseTo(0.45, 12);
  });

  it('lets a pane under the floor grow', () => {
    const [a, b] = clampPairShares(0.05, 0.45, 0.1);
    expect(a).toBeCloseTo(0.15, 12);
    expect(b).toBeCloseTo(0.35, 12);
  });

  it('never shrinks a pane under the floor any further', () => {
    const [a, b] = clampPairShares(0.05, 0.45, -0.03);
    expect(a).toBeCloseTo(0.05, 12);
    expect(b).toBeCloseTo(0.45, 12);
    // The same for the second pane of the pair.
    const [c, d] = clampPairShares(0.45, 0.05, 0.03);
    expect(c).toBeCloseTo(0.45, 12);
    expect(d).toBeCloseTo(0.05, 12);
  });

  it('clamps panes above 12% to 12% of the pair, as before', () => {
    const [a, b] = clampPairShares(0.25, 0.25, -0.4);
    expect(a).toBeCloseTo(0.06, 12); // 12% of a 0.5 pair
    expect(b).toBeCloseTo(0.44, 12);
    const [c, d] = clampPairShares(0.25, 0.25, 0.4);
    expect(c).toBeCloseTo(0.44, 12);
    expect(d).toBeCloseTo(0.06, 12);
  });

  it('keeps the pair’s combined share', () => {
    for (const shift of [-0.3, -0.01, 0, 0.02, 0.3]) {
      const [a, b] = clampPairShares(0.2, 0.3, shift);
      expect(a + b).toBeCloseTo(0.5, 12);
    }
  });
});
