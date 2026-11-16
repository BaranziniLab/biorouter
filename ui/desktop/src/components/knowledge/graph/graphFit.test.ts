import { describe, expect, it } from 'vitest';
import {
  fittedAxisFill,
  graphFitPadding,
  graphFitView,
  MAX_FIT_ZOOM,
  NODE_REL_SIZE,
} from './graphFit';

/**
 * The canvas cannot be rendered in jsdom — force-graph calls
 * `canvas.getContext('2d')` — so the fit is guarded here, at the pure function,
 * which is also where the defect lived: a single padding expression.
 *
 * ⚠ **These are CANVAS boxes, not pane boxes, and the distinction invalidated
 * an earlier version of this file.** `ForceGraphCanvas` calls
 * `graphFitPadding(size.width, size.height, …)` where `size` comes from a
 * ResizeObserver on the canvas wrapper — so the header, the subject band, the
 * filter bar and the card padding are already gone. Measured in a browser: a
 * 1690x760 pane yields an 880x486 canvas, i.e. ~274px of chrome above it.
 *
 * The first fixture set here was pane-sized (900, 800 and 700 tall) and asserted
 * a fill this function never actually produces in the app, because at the app's
 * own window sizes the canvas is 294–800 tall and the binding axis is almost
 * always the SHORT one. A fixture that cannot occur is worse than no fixture: it
 * passes, and it reports a number nobody will see.
 */
const CANVAS_BOXES = [
  [694, 294], // ~600px window, the app's minimum — canvas is compact here
  [694, 486], // 992px viewport / 760px pane, the single-column step
  [880, 486], // 1690px pane, three columns
  [1098, 594], // ~900px window
  [1240, 774], // ~1080px window
] as const;

describe('graphFitPadding', () => {
  /**
   * The regression this exists to prevent. The old expression was
   * `compact ? 72 : Math.max(112, min(w,h) * 0.16)`, and across the canvas boxes
   * above it left the cluster on only 51–68% of the binding axis, which is the
   * 30–37% *area* the canvas measured at.
   *
   * 0.70 is the floor because it is what the shipped function actually
   * guarantees across the real range — 71.4% at the compact end rising to 83.2%
   * at the largest. Asserting 0.80 here would be asserting the best case and
   * calling it the contract.
   */
  it('leaves the cluster at least 70% of the binding axis on every real canvas', () => {
    for (const [w, h] of CANVAS_BOXES) {
      const p = graphFitPadding(w, h, 13.4);
      const fill = fittedAxisFill(Math.min(w, h), p);
      expect(fill, `${w}x${h} padding ${p}`).toBeGreaterThanOrEqual(0.7);
    }
  });

  /** The other half of the contract: the gain is real at every size, not on average. */
  it('gains at least 14 points of fill over the old expression everywhere', () => {
    const old = (w: number, h: number) =>
      w < 560 || h < 430 ? 72 : Math.max(112, Math.min(w, h) * 0.16);
    for (const [w, h] of CANVAS_BOXES) {
      const d = Math.min(w, h);
      const gain = fittedAxisFill(d, graphFitPadding(w, h, 13.4)) - fittedAxisFill(d, old(w, h));
      expect(gain, `${w}x${h}`).toBeGreaterThanOrEqual(0.14);
    }
  });

  it('beats the old expression at every size it was measured against', () => {
    const old = (w: number, h: number) =>
      w < 560 || h < 430 ? 72 : Math.max(112, Math.min(w, h) * 0.16);
    for (const [w, h] of [
      [1400, 900],
      [1200, 800],
      [900, 700],
      [520, 400],
    ] as const) {
      expect(graphFitPadding(w, h, 13.4), `${w}x${h}`).toBeLessThan(old(w, h));
    }
  });

  /**
   * force-graph's `getGraphBbox` pads each node by `nodeRelSize`, not by the
   * radius the painter uses, so a bigger hub must buy more padding — otherwise
   * shrinking the base value clips the mark at the pane edge.
   */
  it('pays for the gap between the painted radius and the fit box', () => {
    const small = graphFitPadding(1400, 900, NODE_REL_SIZE);
    const hub = graphFitPadding(1400, 900, 13.4);
    expect(hub).toBeGreaterThan(small);
    expect(hub - small).toBeCloseTo(13.4 - NODE_REL_SIZE, 0);
  });

  it('never returns more than 18% of the smaller dimension, so it cannot drift back', () => {
    for (const [w, h] of [
      [1400, 900],
      [600, 480],
      [3000, 2000],
    ] as const) {
      expect(graphFitPadding(w, h, 40)).toBeLessThanOrEqual(Math.min(w, h) * 0.18);
    }
  });

  it('is tighter on a compact pane, where every pixel is scarcer', () => {
    expect(graphFitPadding(520, 400, 13.4)).toBeLessThan(graphFitPadding(1400, 900, 13.4));
  });
});

describe('fittedAxisFill', () => {
  it('is the identity force-graph itself uses: (D - 2p) / D', () => {
    expect(fittedAxisFill(900, 144)).toBeCloseTo(612 / 900, 6);
    expect(fittedAxisFill(900, 0)).toBe(1);
  });

  it('never goes negative on a padding wider than the pane', () => {
    expect(fittedAxisFill(100, 200)).toBe(0);
  });
});

/**
 * W1-45: a fit scales the graph's box to the pane, and nothing capped it, so a knowledge base with
 * one page drew that page as one circle about 730px across, its label clipped at the pane edge.
 */
describe('graphFitView', () => {
  /** force-graph's `getGraphBbox` for nodes at these points: each one padded by `NODE_REL_SIZE`. */
  const boxOf = (...points: Array<[number, number]>) => ({
    x: [
      Math.min(...points.map(([x]) => x)) - NODE_REL_SIZE,
      Math.max(...points.map(([x]) => x)) + NODE_REL_SIZE,
    ] as [number, number],
    y: [
      Math.min(...points.map(([, y]) => y)) - NODE_REL_SIZE,
      Math.max(...points.map(([, y]) => y)) + NODE_REL_SIZE,
    ] as [number, number],
  });

  it('a lone node is not zoomed past the cap', () => {
    for (const [w, h] of CANVAS_BOXES) {
      const padding = graphFitPadding(w, h, NODE_REL_SIZE);
      const view = graphFitView(w, h, padding, boxOf([3, -2]));
      // `zoomToFit` alone would choose (h - 2p) / 11.2, over 20x here.
      expect(view, `${w}x${h}`).toEqual({ x: 3, y: -2, zoom: MAX_FIT_ZOOM });
      // Drawn at a normal size, centred: the mark is a small part of the pane, not all of it.
      expect(2 * NODE_REL_SIZE * view!.zoom, `${w}x${h}`).toBeLessThan(Math.min(w, h) * 0.1);
    }
  });

  it('caps two close nodes as well, and keeps them centred', () => {
    const view = graphFitView(
      880,
      486,
      graphFitPadding(880, 486, NODE_REL_SIZE),
      boxOf([0, 0], [40, 10])
    );
    expect(view).toEqual({ x: 20, y: 5, zoom: MAX_FIT_ZOOM });
  });

  it('leaves a graph that fills the pane at the zoom zoomToFit would choose', () => {
    // A cluster 600 by 330 world units, the size a few dozen pages settle to.
    const box = boxOf([-300, -165], [300, 165]);
    const padding = graphFitPadding(880, 486, 13.4);
    const view = graphFitView(880, 486, padding, box)!;
    const zoomToFit = Math.min(
      (880 - 2 * padding) / (box.x[1] - box.x[0]),
      (486 - 2 * padding) / (box.y[1] - box.y[0])
    );
    expect(zoomToFit).toBeLessThan(MAX_FIT_ZOOM);
    expect(view.zoom).toBeCloseTo(zoomToFit, 9);
    expect([view.x, view.y]).toEqual([0, 0]);
  });

  it('answers nothing for a box it cannot read, so the caller fits as before', () => {
    expect(graphFitView(880, 486, 40, null)).toBeNull();
    expect(graphFitView(880, 486, 40, { x: [0, Number.NaN], y: [0, 1] })).toBeNull();
    expect(graphFitView(60, 486, 40, boxOf([0, 0]))).toBeNull();
  });

  /** Labels show from a zoom of 1.55 (`ForceGraphCanvas`'s label pass), so a lone page is named. */
  it('caps at a zoom where an ordinary node is labelled', () => {
    expect(MAX_FIT_ZOOM).toBeGreaterThanOrEqual(1.55);
  });
});
