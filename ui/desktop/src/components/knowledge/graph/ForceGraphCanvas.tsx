// ui/desktop/src/components/knowledge/graph/ForceGraphCanvas.tsx
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import ForceGraph2D, { ForceGraphMethods } from 'react-force-graph-2d';
import { forceX, forceY } from 'd3-force';
import type { Graph, GraphEdge, GraphNode } from '../../../api/types.gen';
import { familyOfType, GRAPH_PALETTE } from '../../../styles/graphPalette';
import type { GraphCredibilityKey } from '../../../styles/graphPalette';
import { withAlpha } from './graphStyle';
import { useCanvasTheme } from './useCanvasTheme';
import type { CanvasTheme } from './useCanvasTheme';
import { NODE_RING_ALPHA } from './graphStyle';
import { graphFitPadding, NODE_REL_SIZE } from './graphFit';
import { isWindowResizing, motionDuration } from '../../../styles/motion';
import {
  announce,
  highestDegree,
  nextNodeInDirection,
  stepThrough,
  tabOrder,
} from './graphKeyboard';
import type { Direction } from './graphKeyboard';
// The mark — fill and silhouette — is shared with the inspector, so the two
// surfaces cannot disagree about the same node. See `nodeMark.ts`.
import { credibilityKey, fillFor, isHollow } from './nodeMark';
import { edgePredicate, isNegated, readablePredicate } from './graphModel';
import type { GraphModel, NodeMetrics } from './graphModel';
import { EMPTY_FACETS, facetsActive } from './graphFacets';
import type { FacetState } from './graphFacets';

/**
 * The typed graph canvas (ui-spec §5).
 *
 * Four encodings ride the mark and they are deliberately non-overlapping:
 * **shape** carries the family, **fill** carries the type within it, the **ring**
 * carries credibility, and the **edge treatment** carries negation, provenance
 * and direction. Nothing is carried by colour alone — the measured cross-family
 * colour distance under simulated dichromacy bottoms out at ΔE00 0.00, so a
 * palette of 28 marks cannot be made safe by choosing better hues.
 *
 * ⚠ **Not one hex literal lives in this file, and that is a rule rather than a
 * tidiness note.** Every fill comes from `GRAPH_PALETTE` (generated, solved
 * against the resolved `--background-muted` per mode) and every structural
 * colour — ink, ground, danger, muted, border — is RESOLVED off the live cascade
 * by `useCanvasTheme`, because a 2D canvas cannot parse `var(--…)`: the
 * assignment is silently dropped and the previous value stays. The file this
 * replaces hardcoded `rgba(119, 128, 145, 0.42)` for a resting edge and a
 * **green** focus edge in an app whose accent is coral, and drew its labels in a
 * near-black that was invisible on every dark ground.
 */

/** Screen-space geometry constants (§5.5, §5.7, §5.8). All divided by `globalScale` at use. */
const RING_GAP_PX = 1.8;
const RING_WIDTH_PX = 1.6;
/** Below this SCREEN diameter the credibility encoding is not drawn AT ALL (§5.5.2). */
const RING_LOD_MIN = 3.5;
/** The retracted badge goes with it, one notch later. */
const BADGE_LOD_MIN = 4;
const LABEL_GAP_PX = 6;
const GRID_SPACING_WORLD = 34;

const clamp01 = (v: number) => (v < 0 ? 0 : v > 1 ? 1 : v);

/**
 * §5.9's density ladder, recomputed once per frame in `onRenderFramePre` and
 * read by every painter from a ref — exactly one frame fresh, by construction.
 *
 * force-graph does no viewport culling: `paintNodes` and `paintLinks` both
 * iterate the full arrays every frame. So the pre-frame pass is where the
 * visible rect is computed, and it is what `nodeVisibility` / `linkVisibility`
 * read.
 */
interface DensityStyle {
  edgeAlpha: number;
  edgeWidth: number;
  nodeStrokeAlpha: number;
  nodeStrokeWidth: number;
}

interface WorldRect {
  x0: number;
  y0: number;
  x1: number;
  y1: number;
}

const FULL_DENSITY: DensityStyle = {
  edgeAlpha: 1,
  edgeWidth: 1,
  nodeStrokeAlpha: 1,
  nodeStrokeWidth: 1.1,
};

/** The alpha ladder for a resting / emphasised / dimmed edge, on the RESOLVED ink. */
const EDGE_ALPHA = { dim: 0.07, base: 0.18, emph1: 0.32, emph2: 0.46 } as const;
/** The same ladder for a negated edge, on the RESOLVED `--text-danger`. */
const NEG_ALPHA = { dim: 0.1, base: 0.34, emph1: 0.46, emph2: 0.62 } as const;

type PositionedNode = GraphNode & { x?: number; y?: number };
type LinkDatum = GraphEdge & { __i: number; source: PositionedNode; target: PositionedNode };

interface Props {
  graph: Graph;
  /**
   * Everything about the graph that is constant for its lifetime.
   *
   * Passed in rather than derived here because the facet rail and the legend
   * need the same counts, and because DR-9 is explicit that the label pass and
   * the radius model must not be paid `nodes × 60` times a second.
   */
  model: GraphModel;
  /** The active facets. Node ids that pass, or `null` for "no facet is active". */
  facets?: FacetState;
  passing?: Set<string> | null;
  selectedId: string | null;
  hoveredId: string | null;
  onHover: (id: string | null) => void;
  onNodeClick: (node: GraphNode) => void;
  /**
   * Open an edge's details.
   *
   * The canvas already tracked a hovered edge and painted its label, so every
   * §8.1 provenance field and §7.3 quantitative slot was one click away — and
   * that click went nowhere, because `onLinkClick` was never wired. Optional so
   * a caller that has no inspector (the preview-at-SHA reader) is not obliged
   * to invent one.
   */
  onLinkClick?: (edge: GraphEdge) => void;
  /// Optional: if set, nodes whose id is NOT in this set are dimmed and dashed
  /// (used in "preview at SHA" mode to ghost future-state additions).
  visibleSet: Set<string> | null;
}

interface Sized {
  width: number;
  height: number;
}

function useSize(): [React.RefObject<HTMLDivElement | null>, Sized] {
  const ref = useRef<HTMLDivElement | null>(null);
  const [size, setSize] = useState<Sized>({ width: 600, height: 400 });
  useEffect(() => {
    if (!ref.current) return;
    const el = ref.current;
    const update = () => {
      const r = el.getBoundingClientRect();
      setSize({
        width: Math.max(1, Math.floor(r.width)),
        height: Math.max(1, Math.floor(r.height)),
      });
    };
    const ro = new ResizeObserver(() => {
      update();
    });
    ro.observe(el);
    const raf = window.requestAnimationFrame(update);
    window.addEventListener('resize', update);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener('resize', update);
      ro.disconnect();
    };
  }, []);
  return [ref, size];
}

/**
 * §5.10's layout bands. A single parameter set cannot serve 40 nodes and 2,000:
 * the charge that spreads a small base legibly flings a large one off-screen.
 */
function layoutBand(count: number) {
  if (count <= 350) return { L: 92, strength: 0.22, charge: -135, distanceMax: 260 };
  if (count <= 900) return { L: 78, strength: 0.2, charge: -155, distanceMax: 300 };
  if (count <= 1500) return { L: 82, strength: 0.16, charge: -175, distanceMax: 340 };
  return { L: 96, strength: 0.12, charge: -200, distanceMax: 380 };
}

/**
 * How long a zoom-to-fit glides: the panel tier (`--dur-med`), 0 under reduced motion, and 0
 * while the window or the sidebar is being resized, because a camera move against a resize is
 * the jitter principle 9 forbids. It was a literal 500ms here and 400ms on Home.
 */
function fitDurationMs(): number {
  return isWindowResizing() ? 0 : motionDuration('med');
}

export function ForceGraphCanvas({
  graph,
  model,
  facets = EMPTY_FACETS,
  passing = null,
  selectedId,
  hoveredId,
  onHover,
  onNodeClick,
  onLinkClick,
  visibleSet,
}: Props) {
  const fgRef = useRef<ForceGraphMethods | undefined>(undefined);
  const [containerRef, size] = useSize();

  // §5.12. `null` means the canvas has focus but no node is picked yet.
  const [keyboardFocusId, setKeyboardFocusId] = useState<string | null>(null);
  const [announcement, setAnnouncement] = useState('');
  const hintId = useId();

  // §5.11's four 0×0 probes. Non-inherited values (mono family, danger, muted,
  // border) cannot be read off the container, and there is deliberately no
  // danger *constant* — the status hues are per family, so a shared literal
  // would paint Parchment's red inside Alma Mater and Roche Limit, on the two
  // marks where a wrong red is a wrong MEANING.
  const monoProbe = useRef<HTMLElement | null>(null);
  const dangerProbe = useRef<HTMLElement | null>(null);
  const mutedProbe = useRef<HTMLElement | null>(null);
  const borderProbe = useRef<HTMLElement | null>(null);
  const probes = useMemo(
    () => ({ mono: monoProbe, danger: dangerProbe, muted: mutedProbe, border: borderProbe }),
    []
  );
  const theme = useCanvasTheme(containerRef, probes);

  const [hoveredEdge, setHoveredEdge] = useState<number | null>(null);

  // ⚠ **ONE fold-in point** (§5.12). `focusId` already drives six behaviours —
  // the neighbour dimming, the focus halo, the label priority ladder, the
  // incident-edge emphasis and both visibility culls — so keyboard focus enters
  // HERE and inherits all of them. Adding parallel `keyboardFocusId` checks at
  // those six sites is how the two focus notions drift apart.
  const focusId = keyboardFocusId ?? selectedId ?? hoveredId;
  const filtering = facetsActive(facets) && passing != null;

  // Convert API graph → force-graph data. react-force-graph mutates nodes
  // (adds x/y) and links (replaces source/target with node objects) — so we keep
  // our own stable copies, and carry the original edge index on `__i` so a
  // painter can find the edge it is drawing.
  const data = useMemo(() => {
    const nodes: PositionedNode[] = graph.nodes.map((n) => ({ ...n }));
    const links = graph.edges.map((e, i) => ({ ...e, source: e.from, target: e.to, __i: i }));
    return { nodes, links };
  }, [graph]);

  /**
   * §5.7's parallel-edge lanes.
   *
   * Keyed on the UNORDERED pair, so `A→B` and `B→A` share one lane set and bend
   * apart instead of overdrawing each other. Computed once per graph: it is
   * pure arithmetic over the edge list and cannot change between frames.
   */
  const lanes = useMemo(() => {
    const groups = new Map<string, number[]>();
    graph.edges.forEach((e, i) => {
      const a = String(e.from);
      const b = String(e.to);
      // The same `\u0000` collision guard the facet haystack uses: an id may
      // contain any other character, and `a|b` would collide with `a|b` split
      // the other way. Never a raw byte in the source.
      const key = a <= b ? `${a}\u0000${b}` : `${b}\u0000${a}`;
      const bucket = groups.get(key);
      if (bucket) bucket.push(i);
      else groups.set(key, [i]);
    });
    const out = new Float64Array(graph.edges.length);
    for (const bucket of groups.values()) {
      const n = bucket.length;
      bucket.forEach((edgeIndex, i) => {
        out[edgeIndex] = n > 1 ? i - (n - 1) / 2 : 0;
      });
    }
    return out;
  }, [graph]);

  // Per-frame state every painter reads. A ref rather than state: it is written
  // inside a render callback, and setting state there would loop.
  const densityRef = useRef<DensityStyle>(FULL_DENSITY);
  const rectRef = useRef<WorldRect | null>(null);
  const scaleRef = useRef(1);

  /**
   * The label width memo — half two of DR-9's fix.
   *
   * `measureText` is needed for the collision box and the font size changes at
   * every zoom step, so the naive version measures every labelled node every
   * frame. Advance widths are LINEAR in font size, so measuring once at a fixed
   * 100px and scaling is exact to within sub-pixel hinting — irrelevant for an
   * AABB. One measure per unique (text, weight) for the life of the graph.
   */
  const widthCache = useRef(new Map<string, number>());
  useEffect(() => {
    widthCache.current.clear();
  }, [graph, theme.fontFamily]);

  const themeRef = useRef<CanvasTheme>(theme);
  themeRef.current = theme;

  useEffect(() => {
    const fg = fgRef.current as
      | (ForceGraphMethods & {
          d3Force?: (name: string) => {
            strength?: (value: number) => unknown;
            distance?: (value: number) => unknown;
            distanceMax?: (value: number) => unknown;
          };
          zoomToFit?: (durationMs?: number, paddingPx?: number) => void;
        })
      | undefined;

    if (!fg) {
      return;
    }

    const band = layoutBand(graph.nodes.length);
    fg.d3Force?.('charge')?.strength?.(band.charge);
    // Cap the range of charge repulsion so a disconnected node is not flung to
    // the far edge of the canvas by the cumulative push of the whole cluster.
    fg.d3Force?.('charge')?.distanceMax?.(band.distanceMax);
    fg.d3Force?.('link')?.distance?.(band.L);
    fg.d3Force?.('link')?.strength?.(band.strength);

    // Gentle centering pull toward the origin. Linked nodes are held in place
    // by link tension, so this barely moves the connected hub — but an isolated
    // node (no incoming/outgoing edges) feels only this force and drifts back
    // toward the cluster instead of being pushed off-screen by charge repulsion.
    const fgWithForce = fg as typeof fg & {
      d3Force: (name: string, force?: unknown) => unknown;
    };
    fgWithForce.d3Force('x', forceX(0).strength(0.07));
    fgWithForce.d3Force('y', forceY(0).strength(0.07));

    // The largest radius the painter will actually draw, which is what
    // force-graph's own fit box does NOT account for — see `graphFit.ts`.
    let maxRadius = NODE_REL_SIZE;
    for (const m of model.nodes.values()) {
      if (m.radius > maxRadius) maxRadius = m.radius;
    }

    const timeout = window.setTimeout(() => {
      fg.zoomToFit?.(fitDurationMs(), graphFitPadding(size.width, size.height, maxRadius));
    }, 900);

    return () => window.clearTimeout(timeout);
  }, [graph, model, size.height, size.width]);

  /**
   * The pre-frame pass: visible rect, density, grid.
   *
   * Verified to run BEFORE `tickFrame()` → `paintLinks()` → `paintNodes()`,
   * which is what makes the ref exactly one statement fresh rather than one
   * frame stale.
   */
  const onRenderFramePre = (ctx: CanvasRenderingContext2D, globalScale: number) => {
    scaleRef.current = globalScale;
    const t = themeRef.current;

    const fg = fgRef.current as
      | (ForceGraphMethods & {
          screen2GraphCoords?: (x: number, y: number) => { x: number; y: number };
        })
      | undefined;

    let rect: WorldRect | null = null;
    if (typeof fg?.screen2GraphCoords === 'function') {
      const pad = 80;
      const a = fg.screen2GraphCoords(-pad, -pad);
      const b = fg.screen2GraphCoords(size.width + pad, size.height + pad);
      if (Number.isFinite(a?.x) && Number.isFinite(b?.x)) {
        rect = {
          x0: Math.min(a.x, b.x),
          y0: Math.min(a.y, b.y),
          x1: Math.max(a.x, b.x),
          y1: Math.max(a.y, b.y),
        };
      }
    }
    rectRef.current = rect;

    // Count what is actually on screen. A base can hold 2,000 nodes and show 40.
    let visibleNodes = 0;
    for (const n of data.nodes) {
      if (!rect || inRect(n, rect)) visibleNodes += 1;
    }
    let visibleEdges = 0;
    for (const l of data.links as unknown as LinkDatum[]) {
      if (!rect || inRect(l.source, rect) || inRect(l.target, rect)) visibleEdges += 1;
    }

    const edgeDensity = clamp01((visibleEdges - 260) / 2200);
    const nodeDensity = clamp01((visibleNodes - 220) / 1200);
    const zoomCrowd = clamp01((0.72 - globalScale) / 0.62);
    const fade = clamp01(Math.max(edgeDensity, nodeDensity) * zoomCrowd);
    const outlineFade = clamp01(Math.max(nodeDensity, edgeDensity * 0.75) * zoomCrowd);
    densityRef.current = {
      edgeAlpha: 1 - 0.82 * fade,
      edgeWidth: 1 - 0.34 * fade,
      nodeStrokeAlpha: 1 - outlineFade,
      nodeStrokeWidth: outlineFade >= 0.72 ? 0 : 1.1 * (1 - outlineFade / 0.72),
    };

    // The grid. Resolved ink at 0.045 — in dark mode a faint LIGHT dot field on
    // a dark ground, which a hardcoded `rgba(20, 24, 31, 0.045)` could never be.
    if (GRID_SPACING_WORLD * globalScale < 11 || !rect) return;
    const r = 0.8 / globalScale;
    ctx.fillStyle = withAlpha(t.ink, 0.045);
    const startX = Math.floor(rect.x0 / GRID_SPACING_WORLD) * GRID_SPACING_WORLD;
    const startY = Math.floor(rect.y0 / GRID_SPACING_WORLD) * GRID_SPACING_WORLD;
    for (let x = startX; x <= rect.x1; x += GRID_SPACING_WORLD) {
      for (let y = startY; y <= rect.y1; y += GRID_SPACING_WORLD) {
        ctx.beginPath();
        ctx.arc(x, y, r, 0, Math.PI * 2);
        ctx.fill();
      }
    }
  };

  /** §5.6's node alpha. Two dim levels, because they mean different things. */
  const nodeAlpha = (n: GraphNode, isFocus: boolean, isNeighbour: boolean): number => {
    if (filtering && !passing!.has(n.id)) return 0.12;
    if (focusId && !isFocus && !isNeighbour) return 0.26;
    if (visibleSet && !visibleSet.has(n.id)) return 0.26;
    return 1;
  };

  const paintNode = (raw: unknown, ctx: CanvasRenderingContext2D, globalScale: number) => {
    const n = raw as PositionedNode;
    if (typeof n.x !== 'number' || typeof n.y !== 'number') return;
    const m: NodeMetrics | undefined = model.nodes.get(n.id);
    if (!m) return;

    const t = themeRef.current;
    const density = densityRef.current;
    const r = m.radius;
    const screenDiameter = r * globalScale;
    const isFocus = focusId === n.id;
    const isNeighbour = !!(focusId && model.neighbours.get(focusId)?.has(n.id));
    const fill = fillFor(n, t.mode);

    ctx.globalAlpha = nodeAlpha(n, isFocus, isNeighbour);

    // Focus glow, painted BEFORE the fill so the mark sits on top of its halo.
    if (isFocus) {
      const outer = r + 13 / globalScale;
      const grad = ctx.createRadialGradient(n.x, n.y, r, n.x, n.y, outer);
      grad.addColorStop(0, withAlpha(fill, 0.34));
      grad.addColorStop(0.55, withAlpha(fill, 0.14));
      grad.addColorStop(1, withAlpha(fill, 0));
      ctx.fillStyle = grad;
      ctx.beginPath();
      ctx.arc(n.x, n.y, outer, 0, Math.PI * 2);
      ctx.fill();
    }

    // ⚠ **Every node is a circle.** The seven silhouettes and their level-of-
    // detail collapse are gone; `screenDiameter` no longer selects a path.
    ctx.beginPath();
    ctx.arc(n.x, n.y, r, 0, Math.PI * 2);

    if (m.external) {
      // A referenced entity with no page yet. Hollow and dashed, in ink — it
      // cannot fail contrast because it is drawn in the ink, and a hollow marker
      // says *placeholder* where the old pale `#D7DBE1` fill measured ≈1.2:1 on
      // the light ground and was simply not visible.
      ctx.fillStyle = t.ground;
      ctx.fill();
      ctx.setLineDash([2.5 / globalScale, 2 / globalScale]);
      ctx.lineWidth = 1 / globalScale;
      ctx.strokeStyle = withAlpha(t.ink, 0.45);
      ctx.stroke();
      ctx.setLineDash([]);
      ctx.globalAlpha = 1;
      return;
    }

    if (isHollow(n, t.mode)) {
      // Provenance & Context: an open ring rather than a filled disc (R-04).
      // The ground fill is not decoration — it occludes the edges that pass
      // beneath, so the ring reads as a mark rather than as a hole with lines
      // through it.
      //
      // 1.7px, not the 2.6 the first draft used: at 2.6 on a 7px radius the
      // stroke ate most of the mark and it read as a donut instead of a circle
      // that happens to be open. The legend swatch is thinned to match — the
      // key and the mark have to agree or the legend teaches the wrong thing.
      ctx.fillStyle = t.ground;
      ctx.fill();
      ctx.lineWidth = Math.max(1.7, density.nodeStrokeWidth * 1.55) / globalScale;
      ctx.strokeStyle = withAlpha(fill, isFocus ? 1 : density.nodeStrokeAlpha);
      ctx.stroke();
      ctx.globalAlpha = 1;
      return;
    }

    ctx.fillStyle = fill;
    ctx.fill();

    const ringKey = credibilityKey(n);
    const showsRing = ringKey != null && screenDiameter >= RING_LOD_MIN;

    if (!showsRing) {
      // The neutral separation ring, on the family shape's own path.
      const sw = isFocus
        ? Math.max(1.1, density.nodeStrokeWidth) / globalScale
        : density.nodeStrokeWidth / globalScale;
      const sa = isFocus ? NODE_RING_ALPHA : NODE_RING_ALPHA * density.nodeStrokeAlpha;
      if (sw > 0.05 / globalScale && sa > 0.03 && screenDiameter > 1.2) {
        ctx.lineWidth = sw;
        ctx.strokeStyle = withAlpha(t.ink, sa);
        ctx.stroke();
      }
    } else {
      paintCredibilityRing(ctx, n.x, n.y, r, globalScale, ringKey!, t);
    }

    // The retracted badge. A filled disc with a `!` in the RESOLVED GROUND —
    // the old `'#fff'` glyph was invisible on the light-mode badge.
    if (n.retracted && screenDiameter >= BADGE_LOD_MIN) {
      const bx = n.x + r * 0.7;
      const by = n.y - r * 0.7;
      const br = Math.max(3 / globalScale, r * 0.45);
      ctx.beginPath();
      ctx.arc(bx, by, br, 0, Math.PI * 2);
      ctx.fillStyle = GRAPH_PALETTE[t.mode].credibility.retracted;
      ctx.fill();
      ctx.fillStyle = t.ground;
      ctx.font = `700 ${br * 1.2}px ${t.fontFamily}`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('!', bx, by + br * 0.06);
    }

    ctx.globalAlpha = 1;
  };

  /**
   * The label pass, and it CANNOT live in `nodeCanvasObject`.
   *
   * force-graph paints nodes one at a time, so a later node overdraws an earlier
   * node's label and collision avoidance has no global view. Drawing here, after
   * every node and edge is down, is what makes the priority ladder mean
   * anything: a hub's label beats a leaf's for the same square of canvas.
   */
  const onRenderFramePost = (ctx: CanvasRenderingContext2D, globalScale: number) => {
    const t = themeRef.current;
    const rect = rectRef.current;

    interface Candidate {
      n: PositionedNode;
      m: NodeMetrics;
      pr: number;
      alpha: number;
    }
    const candidates: Candidate[] = [];

    for (const n of data.nodes) {
      if (typeof n.x !== 'number' || typeof n.y !== 'number') continue;
      if (rect && !inRect(n, rect)) continue;
      const m = model.nodes.get(n.id);
      if (!m) continue;

      let pr: number;
      if (filtering) {
        if (!passing!.has(n.id)) continue;
        pr = 2;
      } else if (selectedId === n.id) pr = 5;
      else if (hoveredId === n.id) pr = 4;
      else if (m.hub) pr = 3;
      else if (focusId && model.neighbours.get(focusId)?.has(n.id)) pr = 2;
      else if (globalScale >= 1.55) pr = 1;
      else continue;

      // An external node is a placeholder, not a page — it earns a label only
      // once the user is pointing at it.
      if (m.external && pr < 4) continue;

      const isFocus = focusId === n.id;
      const isNeighbour = !!(focusId && model.neighbours.get(focusId)?.has(n.id));
      candidates.push({
        n,
        m,
        pr,
        alpha: focusId && !isFocus && !isNeighbour ? 0.3 : 1,
      });
    }

    if (candidates.length === 0) return;
    candidates.sort((a, b) => b.pr - a.pr);

    const placed: { x: number; y: number; w: number; h: number }[] = [];
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    ctx.shadowColor = withAlpha(t.ground, 0.95);

    for (const c of candidates) {
      const bold = c.pr >= 4 || c.m.hub;
      const weight = bold ? 600 : 450;
      const fs = c.m.hub ? 12 : 11.5;
      const key = `${weight}|${c.m.display}`;
      let w100 = widthCache.current.get(key);
      if (w100 === undefined) {
        ctx.font = `${weight} 100px ${t.fontFamily}`;
        w100 = ctx.measureText(c.m.display).width;
        widthCache.current.set(key, w100);
      }
      const tw = (w100 * fs) / 100; // screen px

      const lx = c.n.x! + c.m.radius + LABEL_GAP_PX / globalScale;
      const ly = c.n.y!;
      const box = {
        x: lx - 1 / globalScale,
        y: ly - (fs / 2 + 1) / globalScale,
        w: (tw + 2) / globalScale,
        h: (fs + 2) / globalScale,
      };
      // Greedy, first-come-first-served: overlap means SKIP, never nudge and
      // never shrink — a nudged label no longer points at its node.
      if (placed.some((p) => overlaps(p, box))) continue;
      placed.push(box);

      ctx.globalAlpha = c.alpha;
      ctx.font = `${weight} ${fs / globalScale}px ${t.fontFamily}`;
      ctx.fillStyle = t.ink;
      ctx.shadowBlur = 4 / globalScale;
      // Called twice at the same point on purpose: the second call doubles the
      // shadow density, which is the halo mechanism and not a duplicated line.
      ctx.fillText(c.m.display, lx, ly);
      ctx.fillText(c.m.display, lx, ly);
      ctx.shadowBlur = 0;
    }

    ctx.globalAlpha = 1;
    ctx.shadowColor = 'transparent';
  };

  const paintLink = (raw: unknown, ctx: CanvasRenderingContext2D, globalScale: number) => {
    const l = raw as LinkDatum;
    const s = l.source;
    const e = l.target;
    if (typeof s?.x !== 'number' || typeof e?.x !== 'number') return;
    if (typeof s?.y !== 'number' || typeof e?.y !== 'number') return;

    const t = themeRef.current;
    const density = densityRef.current;

    const touchesFocus = !!focusId && (s.id === focusId || e.id === focusId);
    const failsFacet = filtering && (!passing!.has(s.id) || !passing!.has(e.id));
    const emph = hoveredEdge === l.__i && !failsFacet ? 2 : touchesFocus && !failsFacet ? 1 : 0;
    const dim = failsFacet || (!!focusId && !touchesFocus) || (hoveredEdge != null && emph === 0);

    const sm = model.nodes.get(s.id);
    const em = model.nodes.get(e.id);
    const sr = sm?.radius ?? 6;
    const er = em?.radius ?? 6;

    let dx = e.x - s.x;
    let dy = e.y - s.y;
    const rawLen = Math.hypot(dx, dy) || 1;
    const ux = dx / rawLen;
    const uy = dy / rawLen;

    // Canonicalising the perpendicular by id makes both directions of a pair
    // bend the SAME way, so `A→B` and `B→A` separate instead of overdrawing.
    const forward = String(s.id) <= String(e.id);
    const px = forward ? -uy : uy;
    const py = forward ? ux : -ux;

    const sx = s.x + ux * (sr + 1.5 / globalScale);
    const sy = s.y + uy * (sr + 1.5 / globalScale);
    const ex = e.x - ux * (er + 1.5 / globalScale);
    const ey = e.y - uy * (er + 1.5 / globalScale);
    const len = Math.max(1, rawLen - sr - er - 3 / globalScale);

    const lane = lanes[l.__i] ?? 0;
    // `bend` is screen-CONSTANT, so parallel edges stay legibly apart at every
    // zoom rather than collapsing as you pull back.
    const bend = Math.max(12, Math.min(44, 14 + globalScale * 6));
    const curved = Math.abs(lane) > 0.001;
    const cx = (sx + ex) / 2 + (px * lane * bend) / globalScale;
    const cy = (sy + ey) / 2 + (py * lane * bend) / globalScale;

    const stroke = (width: number, colour: string, dash: number[] | null) => {
      ctx.lineWidth = width;
      ctx.strokeStyle = colour;
      ctx.setLineDash(dash ?? []);
      ctx.beginPath();
      ctx.moveTo(sx, sy);
      if (curved) ctx.quadraticCurveTo(cx, cy, ex, ey);
      else ctx.lineTo(ex, ey);
      ctx.stroke();
      ctx.setLineDash([]);
    };

    // An emphasised edge never fades out under density.
    const restore = emph === 2 ? 0.92 : emph === 1 ? 0.72 : 0;
    const mul = Math.max(density.edgeAlpha, restore);
    const wm = density.edgeWidth / globalScale;
    const inkA = dim
      ? EDGE_ALPHA.dim
      : emph === 2
        ? EDGE_ALPHA.emph2
        : emph === 1
          ? EDGE_ALPHA.emph1
          : EDGE_ALPHA.base;

    ctx.globalAlpha = mul;

    // ── 1. Synthesized: derived from provenance, not authored. DOTTED. ──
    if (l.synthesized) {
      stroke(0.8 * wm, withAlpha(t.ink, dim ? 0.05 : 0.13), [1 / globalScale, 4 / globalScale]);
      ctx.globalAlpha = 1;
      return;
    }

    const negated = isNegated(l);

    // ── 2. Negative: DASHED, in the resolved danger ink. ──
    // A dotted texture for "the system inferred this" and a dashed one for
    // "this claim is negated" — never two dashes separated only by colour,
    // which at 1px is not a distinction the eye can make.
    if (negated) {
      // No danger fallback exists on purpose (the three families' reds differ),
      // so an unresolved probe means the treatment DEGRADES to the ink dash
      // rather than substituting one family's red into a semantic mark.
      const colour = t.danger ?? t.ink;
      const a = dim
        ? NEG_ALPHA.dim
        : emph === 2
          ? NEG_ALPHA.emph2
          : emph === 1
            ? NEG_ALPHA.emph1
            : NEG_ALPHA.base;
      stroke(1.1 * wm, withAlpha(colour, a), [4 / globalScale, 3 / globalScale]);
      if (emph === 2) paintEdgeLabel(ctx, globalScale, l, cx, cy, sx, sy, ex, ey, curved, len, t);
      ctx.globalAlpha = 1;
      return;
    }

    // ── 3/4. Curved lane, or a symmetric relation: a plain stroke. ──
    if (curved) {
      const w = emph === 2 ? 1.35 : emph === 1 ? 1.05 : 0.85;
      stroke(w * wm, withAlpha(t.ink, inkA), null);
      if (emph === 2) paintEdgeLabel(ctx, globalScale, l, cx, cy, sx, sy, ex, ey, curved, len, t);
      ctx.globalAlpha = 1;
      return;
    }

    // ── 5. The default: a tapered QUAD, not a stroke. ──
    // Both half-widths are floored at 0.5 screen px before the path is built:
    // at full density fade the thin end paints 0.55px, where antialiasing sets
    // the apparent width by coverage and the 2:1 taper that carries direction
    // compresses toward 1:1 — exactly when the graph is dense enough to need it.
    const w0 = Math.max(0.5, 0.85 * density.edgeWidth) / globalScale;
    const w1 = Math.max(0.5, 0.42 * density.edgeWidth) / globalScale;
    ctx.fillStyle = withAlpha(t.ink, inkA);
    ctx.beginPath();
    ctx.moveTo(sx + px * w0, sy + py * w0);
    ctx.lineTo(ex + px * w1, ey + py * w1);
    ctx.lineTo(ex - px * w1, ey - py * w1);
    ctx.lineTo(sx - px * w0, sy - py * w0);
    ctx.closePath();
    ctx.fill();

    if (emph === 2) paintEdgeLabel(ctx, globalScale, l, cx, cy, sx, sy, ex, ey, curved, len, t);
    ctx.globalAlpha = 1;
  };

  const nodeVisibility = (raw: unknown): boolean => {
    // Fail OPEN. A cull that cannot compute its rect must draw everything —
    // the failure mode of the other choice is a blank canvas that reads as a
    // data problem.
    const rect = rectRef.current;
    if (!rect || focusId) return true;
    return inRect(raw as PositionedNode, rect);
  };

  /**
   * §5.12's key model.
   *
   * ⚠ **Traversal runs over the CURRENT FILTER SET, not what is on screen.**
   * `passing` is the facet result; the viewport cull is deliberately not
   * consulted, because moving to an off-screen node and centring it is the
   * point. (The cull disables itself whenever anything is focused anyway, so a
   * focused canvas already paints every node.)
   *
   * ⚠ **Positions are read live off the force-graph data.** react-force-graph
   * mutates the node objects in `data.nodes` with `x`/`y` as the simulation
   * runs, so this is the settled layout rather than a stale copy — but a node
   * can be missing coordinates on the very first frames, hence the guard.
   */
  const onKeyDown = useCallback(
    (e: React.KeyboardEvent<HTMLDivElement>) => {
      const positioned = data.nodes.filter(
        (n) =>
          typeof n.x === 'number' &&
          typeof n.y === 'number' &&
          (!filtering || passing?.has(n.id) !== false)
      ) as (PositionedNode & { x: number; y: number })[];
      if (positioned.length === 0) return;

      const degreeOf = (id: string) => model.nodes.get(id)?.degree ?? 0;
      const focus = keyboardFocusId
        ? (positioned.find((n) => n.id === keyboardFocusId) ?? null)
        : null;

      const land = (node: (typeof positioned)[number] | null) => {
        if (!node) return;
        e.preventDefault();
        setKeyboardFocusId(node.id);
        const m = model.nodes.get(node.id);
        setAnnouncement(
          announce({
            identifier: node.identifier,
            label: m?.display ?? node.label,
            nodeType: node.node_type,
            family: node.node_type ? familyOfType(node.node_type, theme.mode) : null,
          })
        );
        // Bring it into view without changing the zoom the user chose.
        fgRef.current?.centerAt?.(node.x, node.y, 400);
      };

      switch (e.key) {
        case 'ArrowUp':
        case 'ArrowDown':
        case 'ArrowLeft':
        case 'ArrowRight': {
          const direction = e.key.slice(5).toLowerCase() as Direction;
          // With nothing focused yet, an arrow enters at the biggest hub rather
          // than doing nothing — the same node Home picks.
          land(
            focus
              ? nextNodeInDirection(focus, positioned, direction)
              : highestDegree(positioned, degreeOf)
          );
          return;
        }
        case 'Tab': {
          const order = tabOrder(positioned, degreeOf);
          land(stepThrough(order, keyboardFocusId, e.shiftKey ? -1 : 1));
          return;
        }
        case 'Enter':
        case ' ': {
          if (!focus) return;
          e.preventDefault();
          onNodeClick(focus);
          return;
        }
        case 'Home': {
          e.preventDefault();
          const top = highestDegree(positioned, degreeOf);
          land(top);
          fgRef.current?.zoomToFit?.(
            fitDurationMs(),
            graphFitPadding(size.width, size.height, NODE_REL_SIZE)
          );
          return;
        }
        case 'Escape': {
          // §5.12: clear the focus first; a SECOND press closes the inspector.
          // Handled in that order so one press never does both.
          if (keyboardFocusId) {
            e.preventDefault();
            setKeyboardFocusId(null);
            setAnnouncement('');
          }
          return;
        }
        default:
      }
    },
    [
      data.nodes,
      filtering,
      passing,
      model,
      keyboardFocusId,
      onNodeClick,
      theme.mode,
      size.width,
      size.height,
    ]
  );

  const linkVisibility = (raw: unknown): boolean => {
    const rect = rectRef.current;
    if (!rect || focusId) return true;
    const l = raw as LinkDatum;
    return inRect(l.source, rect) || inRect(l.target, rect);
  };

  return (
    <div
      data-testid="knowledge-graph-canvas"
      ref={containerRef}
      // ⚠ **§5.12: ONE tab stop, and the canvas had none.** It is the primary
      // content of the section and was reachable only with a pointer, which is
      // WCAG 2.1.1 at Level A. `application` rather than `img` or `group`
      // because the arrow keys are ours: a screen reader must pass them through
      // rather than use them to move its own virtual cursor.
      tabIndex={0}
      role="application"
      aria-label="Knowledge graph"
      aria-describedby={hintId}
      onKeyDown={onKeyDown}
      onBlur={() => setKeyboardFocusId(null)}
      className="relative h-full w-full overflow-hidden outline-none focus-visible:ring-2 focus-visible:ring-border-accent focus-visible:ring-inset"
    >
      {/* ⚠ **NOT `aria-hidden`, unlike the four probes below.** This is the
          channel that replaced the node shape encoding: colour is now the only
          visual encoding on the canvas and it does not survive dichromacy, so
          this announcement is what carries the type. `atomic` because the three
          parts are one statement rather than three updates. */}
      <span className="sr-only" aria-live="polite" aria-atomic="true">
        {announcement}
      </span>
      <span id={hintId} className="sr-only">
        Arrow keys move between nodes. Tab steps through them by connection count. Enter opens the
        focused node. Home returns to the most connected node. Escape clears the focus.
      </span>
      {/*
        §5.11's four probes. Zero-sized, `aria-hidden`, and each carrying exactly
        one existing utility — a canvas cannot read a custom property, and these
        are the only way to get a non-inherited token's USED value.
      */}
      <span
        ref={monoProbe}
        aria-hidden="true"
        data-testid="knowledge-graph-probe-mono"
        className="pointer-events-none absolute h-0 w-0 overflow-hidden font-mono"
      />
      <span
        ref={dangerProbe}
        aria-hidden="true"
        className="pointer-events-none absolute h-0 w-0 overflow-hidden text-text-danger"
      />
      <span
        ref={mutedProbe}
        aria-hidden="true"
        className="pointer-events-none absolute h-0 w-0 overflow-hidden text-text-muted"
      />
      <span
        ref={borderProbe}
        aria-hidden="true"
        className="pointer-events-none absolute h-0 w-0 overflow-hidden border border-border-subtle"
      />

      <ForceGraph2D
        ref={fgRef as unknown as React.MutableRefObject<ForceGraphMethods>}
        graphData={data}
        width={size.width}
        height={size.height}
        cooldownTicks={120}
        d3VelocityDecay={0.3}
        // 5.6 world units is the base radius the §5.9 thresholds were calibrated
        // against, so force-graph's shadow-canvas hit circles track the fills and
        // every zoom threshold transfers 1:1 with no recalibration.
        // Shared with `graphFit.ts`: the fit padding compensates for exactly the
        // gap between this value and the radius the painter draws.
        nodeRelSize={NODE_REL_SIZE}
        backgroundColor="transparent"
        onNodeHover={(n: unknown) => onHover((n as GraphNode | null)?.id ?? null)}
        onNodeClick={(n: unknown) => onNodeClick(n as GraphNode)}
        onLinkHover={(l: unknown) => setHoveredEdge((l as LinkDatum | null)?.__i ?? null)}
        // The ORIGINAL edge, off `__i`, never the force-graph datum: the library
        // replaces `source`/`target` with node objects in place, so handing the
        // datum straight out would give the inspector two mutated endpoints
        // instead of the two ids the contract defines.
        onLinkClick={(l: unknown) => {
          const i = (l as LinkDatum | null)?.__i;
          if (typeof i === 'number' && graph.edges[i]) onLinkClick?.(graph.edges[i]);
        }}
        // Without this the shadow canvas hit-tests the straight chord and
        // hovering a multi-edge picks the wrong one.
        linkCurvature={(raw: unknown) => {
          const l = raw as LinkDatum;
          const lane = lanes[l.__i] ?? 0;
          if (!lane) return 0;
          const gs = scaleRef.current || 1;
          const bend = Math.max(12, Math.min(44, 14 + gs * 6));
          const s = l.source;
          const e = l.target;
          const len =
            typeof s?.x === 'number' && typeof e?.x === 'number'
              ? Math.max(1, Math.hypot(e.x - s.x!, (e.y ?? 0) - (s.y ?? 0)))
              : 1;
          return (lane * (bend / gs)) / len;
        }}
        nodeVisibility={nodeVisibility}
        linkVisibility={linkVisibility}
        onRenderFramePre={onRenderFramePre}
        onRenderFramePost={onRenderFramePost}
        nodeCanvasObject={paintNode}
        linkCanvasObject={paintLink}
        linkCanvasObjectMode={() => 'replace'}
      />
    </div>
  );
}

/* ── painters that need no closure over props ── */

function inRect(n: { x?: number; y?: number }, r: WorldRect): boolean {
  return (
    typeof n.x === 'number' &&
    typeof n.y === 'number' &&
    n.x >= r.x0 &&
    n.x <= r.x1 &&
    n.y >= r.y0 &&
    n.y <= r.y1
  );
}

function overlaps(
  a: { x: number; y: number; w: number; h: number },
  b: { x: number; y: number; w: number; h: number }
): boolean {
  return a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;
}

/**
 * The orbit ring (§5.5).
 *
 * A **circle** whatever the node's family shape, because a circular annulus
 * around a hexagon still reads as a ring and the alternative is seven ring path
 * generators. The 1.0px ground gap is load-bearing: without it the ring's
 * legibility would depend on ring-versus-FILL contrast, which cannot be
 * guaranteed across 28 fills × 7 tiers (`gray_lit` on `Publication` is 1.03:1 —
 * luminance-identical). With the gap the ring is read against the ground alone.
 *
 * The tier is an ARC COUNT, not a hue. A 1.6px stroke subtends 2–3 arcmin, well
 * inside the regime where the visual system reads luminance only, and the seven
 * ring hues collapse to ΔE00 1.13 under tritanopia. Counting is not a colour
 * judgement, so it survives all of it.
 */
function paintCredibilityRing(
  ctx: CanvasRenderingContext2D,
  x: number,
  y: number,
  r: number,
  globalScale: number,
  key: GraphCredibilityKey,
  theme: CanvasTheme
): void {
  const palette = GRAPH_PALETTE[theme.mode];
  const ringR = r + RING_GAP_PX / globalScale;
  const treatment = palette.ringArcs[key];

  ctx.strokeStyle = palette.credibility[key];
  ctx.lineWidth = RING_WIDTH_PX / globalScale;
  ctx.setLineDash([]);

  if (treatment === 'solid') {
    ctx.beginPath();
    ctx.arc(x, y, ringR, 0, Math.PI * 2);
    ctx.stroke();
    return;
  }

  if (treatment === 'dashed') {
    // Eight equal dashes: `web` and `personal` are ONE category on the canvas —
    // *not academic* — and their hue difference is a bonus for trichromatic
    // vision rather than an encoding. The inspector carries the exact tier.
    const dash = (Math.PI * 2 * ringR) / 16;
    ctx.setLineDash([dash, dash]);
    ctx.beginPath();
    ctx.arc(x, y, ringR, 0, Math.PI * 2);
    ctx.stroke();
    ctx.setLineDash([]);
    return;
  }

  const n = treatment;
  // The gap subtends 3 screen px at the ring radius, so the arcs stay countable
  // as the ring shrinks instead of merging into a solid circle.
  const gapAngle = Math.min(0.5, Math.max(0.12, 3 / globalScale / ringR));
  const span = (Math.PI * 2) / n - gapAngle;
  if (span <= 0) return;
  for (let i = 0; i < n; i++) {
    const start = -Math.PI / 2 + ((Math.PI * 2) / n) * i;
    ctx.beginPath();
    ctx.arc(x, y, ringR, start, start + span);
    ctx.stroke();
  }
}

/**
 * The label on the hovered or selected edge, and only that one.
 *
 * The dash is the channel that is always present; the word and the strike are
 * confirmation once the user has committed attention to one edge — which is why
 * the redundancy exists here and nowhere else.
 */
function paintEdgeLabel(
  ctx: CanvasRenderingContext2D,
  globalScale: number,
  edge: GraphEdge,
  cx: number,
  cy: number,
  sx: number,
  sy: number,
  ex: number,
  ey: number,
  curved: boolean,
  len: number,
  theme: CanvasTheme
): void {
  if (len * globalScale <= 26) return;
  if (!edgePredicate(edge)) return;

  const mx = curved ? 0.25 * sx + 0.5 * cx + 0.25 * ex : (sx + ex) / 2;
  const my = curved ? 0.25 * sy + 0.5 * cy + 0.25 * ey : (sy + ey) / 2;
  const negated = isNegated(edge);
  const colour = negated ? (theme.danger ?? theme.ink) : theme.ink;
  const text = readablePredicate(edge);

  ctx.save();
  ctx.font = `500 ${11 / globalScale}px ${theme.monoFamily}`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillStyle = colour;
  ctx.shadowColor = withAlpha(theme.ground, 0.95);
  ctx.shadowBlur = 4 / globalScale;
  ctx.fillText(text, mx, my);
  ctx.fillText(text, mx, my);
  ctx.shadowBlur = 0;
  ctx.shadowColor = 'transparent';

  if (negated) {
    const tw = ctx.measureText(text).width;
    ctx.strokeStyle = colour;
    ctx.lineWidth = 1 / globalScale;
    ctx.setLineDash([]);
    ctx.beginPath();
    ctx.moveTo(mx - tw / 2, my);
    ctx.lineTo(mx + tw / 2, my);
    ctx.stroke();
  }
  ctx.restore();
}
