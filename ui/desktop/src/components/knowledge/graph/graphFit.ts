// ui/desktop/src/components/knowledge/graph/graphFit.ts

/**
 * `nodeRelSize` as passed to `<ForceGraph2D>`.
 *
 * ⚠ **This is the number force-graph pads its fit box with, and it is NOT the
 * radius the painter draws.** `getGraphBbox` computes each node's extent as
 * `sqrt(val || 1) * nodeRelSize`; the fixture's nodes carry no `val`, so every
 * node contributes exactly this, while `graphModel` gives a hub a radius of up
 * to 13.4. The difference is the overhang `graphFitPadding` has to pay for.
 */
export const NODE_REL_SIZE = 5.6;

/**
 * The padding handed to `zoomToFit`, in screen pixels.
 *
 * ⚠ **THE OLD VALUE WAS THE WHOLE REASON THE CANVAS LOOKED EMPTY**, and the
 * proof is algebraic rather than aesthetic. force-graph sets
 * `zoomK = min((W - 2p) / bboxW, (H - 2p) / bboxH)`, so after a fit the cluster
 * occupies exactly `viewport - 2 * padding` on the binding axis — a quantity no
 * force parameter can move. Re-running the shipped simulation with the x/y
 * strength at 0.02, 0.07 and 0.2 produced world bounding boxes differing by
 * 1.8x and all three rendered to the *identical* 612px height in a 1400x900
 * pane. Charge, link distance and the 900ms settle delay were all ruled out by
 * measurement; the padding was the cause.
 *
 * The previous expression was `compact ? 72 : Math.max(112, min(w,h) * 0.16)` —
 * 16% of the smaller dimension PER SIDE, so 32% of it gone before a node is
 * drawn, with a 112px floor that binds on any pane whose smaller dimension is
 * under 700px (i.e. essentially every real one). Measured canvas fill on the
 * real 43-node fixture was 30–37% at every pane size, the ratio being
 * scale-invariant because the padding was proportional. It is also a
 * regression: the value was a flat 84 before commit 87c73be4.
 *
 * ⚠ **DO NOT simply shrink it to a smaller constant.** force-graph's fit box
 * under-pads every node (see `NODE_REL_SIZE`) and does not know about labels at
 * all — `ForceGraphCanvas` draws them to the RIGHT of the mark, outside the box
 * entirely. The old oversized padding was silently paying for both. This
 * function pays for them explicitly instead, so the base padding can be small
 * without hub marks and labels clipping at the pane edge.
 *
 * Kept pure and exported for the reason `utils/messageClamp.ts` records: a
 * threshold you can only exercise by rendering a component is one nobody
 * re-tests — and jsdom cannot render this component at all, because force-graph
 * calls `canvas.getContext('2d')`.
 *
 * @param width  pane width in CSS pixels
 * @param height pane height in CSS pixels
 * @param maxNodeRadius the largest radius the painter will draw, from `GraphModel`
 */
export function graphFitPadding(width: number, height: number, maxNodeRadius: number): number {
  const minDim = Math.min(width, height);
  const compact = width < 560 || height < 430;

  // What the painter draws beyond what the fit box accounted for.
  const markOverhang = Math.max(0, maxNodeRadius - NODE_REL_SIZE);

  // Labels are screen-space and sit to the right of their mark, so they are
  // outside the bbox on one side only; `zoomToFit`'s padding is symmetric, so
  // this is a deliberate compromise rather than an exact allowance. A label
  // touching the edge is a far smaller problem than a canvas that is 70% empty,
  // which is what erring the other way produced.
  const labelAllowance = compact ? 20 : 34;

  const base = compact ? 14 : Math.max(22, minDim * 0.03);

  // The ceiling is what stops this ever drifting back to the old emptiness: at
  // 18% of the smaller dimension the cluster still fills nearly two thirds of
  // that axis.
  return Math.round(Math.min(base + markOverhang + labelAllowance, minDim * 0.18));
}

/**
 * The fraction of the binding axis the fitted cluster occupies, `(D - 2p) / D`.
 *
 * Exists so the guard can assert the OUTCOME rather than the input: a future
 * edit that quietly restores a large padding fails on what the user sees, not
 * on a literal someone has to recognise as wrong.
 */
export function fittedAxisFill(dimension: number, padding: number): number {
  if (dimension <= 0) return 0;
  return Math.max(0, (dimension - 2 * padding) / dimension);
}

/**
 * The most a fit may magnify the graph (W1-45).
 *
 * `zoomToFit` scales the graph's box to the pane and nothing capped it, so a knowledge base with
 * one page, the state every new base passes through, drew that page as one circle about 730px
 * across with its label clipped at the pane edge. Real graphs fit well under this (a few dozen
 * pages settle near 1), so the cap only binds on a lone node or a few close ones. At 2 an ordinary
 * node draws about 22px across, the size a hub draws in a fitted graph, and its label shows,
 * because labels appear from a zoom of 1.55. A person can still zoom in further by hand.
 */
export const MAX_FIT_ZOOM = 2;

/** force-graph's `getGraphBbox()`: the world box its own fit reads, each node padded by `NODE_REL_SIZE`. */
export interface GraphBox {
  x: [number, number];
  y: [number, number];
}

/** Where a fit leaves the camera: the world point at the pane's centre, and the zoom. */
export interface FitView {
  x: number;
  y: number;
  zoom: number;
}

/**
 * The fit `zoomToFit` makes, `min((W - 2p) / boxW, (H - 2p) / boxH)` centred on the box, with the
 * zoom capped at {@link MAX_FIT_ZOOM}. `null` for a box it cannot read or a pane too small for the
 * padding, and the caller then fits as `zoomToFit` does.
 */
export function graphFitView(
  width: number,
  height: number,
  padding: number,
  box: GraphBox | null | undefined
): FitView | null {
  if (!box) return null;
  const [x0, x1] = box.x;
  const [y0, y1] = box.y;
  if (![x0, x1, y0, y1, width, height, padding].every(Number.isFinite)) return null;
  const fit = Math.min((width - 2 * padding) / (x1 - x0), (height - 2 * padding) / (y1 - y0));
  if (Number.isNaN(fit) || fit <= 0) return null;
  return { x: (x0 + x1) / 2, y: (y0 + y1) / 2, zoom: Math.min(fit, MAX_FIT_ZOOM) };
}
