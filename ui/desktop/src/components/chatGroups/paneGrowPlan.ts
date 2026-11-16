import {
  CHAT_COLUMN_CHROME,
  COMPACT_CARD_MIN_HEIGHT,
  COMPACT_CARD_MIN_WIDTH,
  COMPACT_GLYPH_MIN_WIDTH,
  GROUP_SPLITTER_WIDTH,
  PANE_COMPOSER_BAR_HEIGHT,
  PANE_HEADER_HEIGHT,
  PANE_MIN_HEIGHT,
  PANE_MIN_WIDTH,
  PANE_RETURN_BUFFER,
  PREVIEW_TRANSCRIPT_MIN_HEIGHT,
  READABLE_CHAT_WIDTH,
  type PaneShortfall,
} from '../Layout/yieldLadder';
import { normalizeSizes } from './chatGroupsLayout';
import { ChatGroupId, GroupLayout, leafGroupIds } from './chatGroupsTypes';

/**
 * Make wider, taller or larger (rung 5 of the yield ladder, compact panes).
 *
 * A compact pane's one action resizes the split so the pane is full again. It
 * asks the layout for room in a fixed order and takes the first answer that
 * fits, so a pane the person can read is the last thing to give way:
 *
 *   | # | target      | a readable other         | a compact other            |
 *   | 1 | 496 / 320   | its current size         | min(current, 176 / 188)    |
 *   | 2 | 496 / 320   | 362 / 272                | min(current, 176 / 188)    |
 *   | 3 | 376 / 286   | 362 / 272                | min(current, 176 / 188)    |
 *   | 4 | 376 / 286   | 64 / 88                  | 64 / 88                    |
 *
 * Attempt 1 takes room only from panes that are already compact. Attempts 1 to
 * 3 never turn a readable pane compact and keep every compact pane at card
 * density (or the size it already has). Attempt 4 is the last resort before
 * Merge panes. When none fits the plan is null, and the action reads Merge
 * panes instead.
 *
 * Pure, with no DOM: the pixel model is the layout's fractions times the tree's
 * measured box, minus the 1px splitters, exactly as `ChatGroupBranch` lays it
 * out with `flex: <size> 1 0`.
 */

/** One `resizeBranch` dispatch: the branch's path from the root and its new sizes. */
export interface PaneResize {
  path: number[];
  sizes: number[];
}

/** The readable chat width plus the transcript's chrome: a pane that reads well. */
export const PANE_GROW_WIDTH = READABLE_CHAT_WIDTH + CHAT_COLUMN_CHROME; // 496
/** The band, the composer bar and the transcript a stacked preview always leaves. */
export const PANE_GROW_HEIGHT =
  PANE_HEADER_HEIGHT + PANE_COMPOSER_BAR_HEIGHT + PREVIEW_TRANSCRIPT_MIN_HEIGHT; // 320
/** 4px over the exit threshold absorbs flex rounding: the pane is certainly full after the click. */
export const PANE_SEAT_WIDTH = PANE_MIN_WIDTH + PANE_RETURN_BUFFER + 4; // 376
export const PANE_SEAT_HEIGHT = PANE_MIN_HEIGHT + PANE_RETURN_BUFFER + 4; // 286
/** A readable pane stays readable. */
export const PANE_KEEP_WIDTH = PANE_MIN_WIDTH + 2; // 362
export const PANE_KEEP_HEIGHT = PANE_MIN_HEIGHT + 2; // 272
/** The last resort: a pane keeps its glyph, or its band and 44px of body. */
export const PANE_YIELD_WIDTH = COMPACT_GLYPH_MIN_WIDTH; // 64
export const PANE_YIELD_HEIGHT = 88; // the band plus 44px of body

type Axis = 'width' | 'height';
type Box = { width: number; height: number };
type Branch = Extract<GroupLayout, { kind: 'branch' }>;

/** The branch direction that divides an axis: a row divides width, a column height. */
const DIVIDES: Record<Axis, 'row' | 'col'> = { width: 'row', height: 'col' };

const GROW: Box = { width: PANE_GROW_WIDTH, height: PANE_GROW_HEIGHT };
const SEAT: Box = { width: PANE_SEAT_WIDTH, height: PANE_SEAT_HEIGHT };
const KEEP: Box = { width: PANE_KEEP_WIDTH, height: PANE_KEEP_HEIGHT };
const CARD: Box = { width: COMPACT_CARD_MIN_WIDTH, height: COMPACT_CARD_MIN_HEIGHT };
const YIELD: Box = { width: PANE_YIELD_WIDTH, height: PANE_YIELD_HEIGHT };

type Attempt = 1 | 2 | 3 | 4;
const ATTEMPTS: Attempt[] = [1, 2, 3, 4];

/** Sizes that changed by less than this are left alone. */
const SIZE_EPSILON = 0.0001;
/** Floating-point slack on a pixel comparison. */
const PX_EPSILON = 1e-9;

function measured(value: number): boolean {
  return Number.isFinite(value) && value > 0;
}

function sizesOf(branch: Branch): number[] {
  return normalizeSizes(branch.sizes, branch.children.length);
}

/** Each leaf's box from the layout fractions and the tree box (1px splitters, `<size> 1 0` flex). */
export function panePixelBoxes(layout: GroupLayout, tree: Box): Map<ChatGroupId, Box> {
  const boxes = new Map<ChatGroupId, Box>();
  const walk = (node: GroupLayout, box: Box) => {
    if (node.kind === 'leaf') {
      boxes.set(node.groupId, box);
      return;
    }
    const count = node.children.length;
    if (count === 0) return;
    const sizes = sizesOf(node);
    const along: Axis = node.dir === 'row' ? 'width' : 'height';
    const inner = box[along] - (count - 1) * GROUP_SPLITTER_WIDTH;
    node.children.forEach((child, index) => walk(child, { ...box, [along]: sizes[index] * inner }));
  };
  walk(layout, { width: tree.width, height: tree.height });
  return boxes;
}

/** The floor an OTHER leaf keeps on `axis` in `attempt`. */
function otherFloor(attempt: Attempt, axis: Axis, readable: boolean, current: number): number {
  if (attempt === 4) return YIELD[axis];
  if (!readable) return Math.min(current, CARD[axis]);
  return attempt === 1 ? current : KEEP[axis];
}

function targetOf(attempt: Attempt, axis: Axis): number {
  return attempt <= 2 ? GROW[axis] : SEAT[axis];
}

/** Does a branch that divides `axis` sit above the pane? Without one, no resize can widen it. */
function hasDividingAncestor(layout: GroupLayout, groupId: ChatGroupId, dir: 'row' | 'col') {
  const walk = (node: GroupLayout, divided: boolean): boolean => {
    if (node.kind === 'leaf') return node.groupId === groupId && divided;
    const here = divided || (node.dir === dir && node.children.length > 1);
    return node.children.some((child) => walk(child, here));
  };
  return walk(layout, false);
}

export interface PaneAxisPlan {
  axis: Axis;
  /** The attempt (1 to 4) that fitted. */
  attempt: Attempt;
  resizes: PaneResize[];
  /** Every other leaf's floor on this axis in that attempt, which the plan leaves it at or above. */
  floors: Map<ChatGroupId, number>;
}

/**
 * One axis of the plan. Width changes only `row` branches and height only `col`
 * branches, and a leaf's width depends only on row shares (its height only on
 * column shares), so the two axes never interact.
 */
export function planPaneAxis(opts: {
  layout: GroupLayout;
  groupId: ChatGroupId;
  tree: Box;
  axis: Axis;
}): PaneAxisPlan | null {
  const { layout, groupId, tree, axis } = opts;
  if (!measured(tree.width) || !measured(tree.height)) return null;
  const boxes = panePixelBoxes(layout, tree);
  if (!boxes.has(groupId)) return null;
  const divides = DIVIDES[axis];
  if (!hasDividingAncestor(layout, groupId, divides)) return null;

  const readable = (box: Box) => box.width >= PANE_MIN_WIDTH && box.height >= PANE_MIN_HEIGHT;

  for (const attempt of ATTEMPTS) {
    const target = targetOf(attempt, axis);
    const floors = new Map<ChatGroupId, number>();
    for (const [id, box] of boxes) {
      if (id !== groupId) floors.set(id, otherFloor(attempt, axis, readable(box), box[axis]));
    }

    // The walk `layoutMinWidth` does, with a floor per leaf.
    const needs = new Map<GroupLayout, number>();
    const need = (node: GroupLayout): number => {
      let value: number;
      if (node.kind === 'leaf') {
        value = node.groupId === groupId ? target : (floors.get(node.groupId) ?? 0);
      } else if (node.children.length === 0) {
        value = 0;
      } else {
        const children = node.children.map(need);
        value =
          node.dir === divides
            ? children.reduce((a, b) => a + b, 0) +
              (node.children.length - 1) * GROUP_SPLITTER_WIDTH
            : Math.max(...children);
      }
      needs.set(node, value);
      return value;
    };
    if (need(layout) > tree[axis] + PX_EPSILON) continue;

    const resizes: PaneResize[] = [];
    // Top down: each branch on this axis hands its children their need, then the
    // slack in proportion to how far over its need each child is now.
    const assign = (node: GroupLayout, current: number, next: number, path: number[]) => {
      if (node.kind === 'leaf' || node.children.length === 0) return;
      const count = node.children.length;
      if (node.dir !== divides) {
        node.children.forEach((child, index) => assign(child, current, next, [...path, index]));
        return;
      }
      const sizes = sizesOf(node);
      const splitters = (count - 1) * GROUP_SPLITTER_WIDTH;
      const innerNow = current - splitters;
      const innerNext = next - splitters;
      const nowOf = sizes.map((size) => size * innerNow);
      const needOf = node.children.map((child) => needs.get(child) ?? 0);

      let nextOf: number[];
      const holdsPane = leafGroupIds(node).includes(groupId);
      const scaled = sizes.map((size) => size * innerNext);
      if (!holdsPane && scaled.every((value, index) => value >= needOf[index] - PX_EPSILON)) {
        // A branch that does not hold the pane keeps its own proportions when they fit.
        nextOf = scaled;
      } else {
        const slack = Math.max(0, innerNext - needOf.reduce((a, b) => a + b, 0));
        const surplus = nowOf.map((value, index) => Math.max(0, value - needOf[index]));
        const totalSurplus = surplus.reduce((a, b) => a + b, 0);
        const totalNeed = needOf.reduce((a, b) => a + b, 0);
        nextOf = needOf.map((value, index) => {
          if (totalSurplus > 0) return value + (slack * surplus[index]) / totalSurplus;
          if (totalNeed > 0) return value + (slack * value) / totalNeed;
          return innerNext / count;
        });
        const nextSizes = innerNext > 0 ? nextOf.map((value) => value / innerNext) : sizes;
        if (nextSizes.some((size, index) => Math.abs(size - sizes[index]) > SIZE_EPSILON)) {
          resizes.push({ path, sizes: nextSizes });
        }
      }
      node.children.forEach((child, index) =>
        assign(child, nowOf[index], nextOf[index], [...path, index])
      );
    };
    assign(layout, tree[axis], tree[axis], []);
    return { axis, attempt, resizes, floors };
  }
  return null;
}

/**
 * The resizes that make `groupId` full again, or null when the layout has no
 * room for it (the action is then Merge panes). A `'both'` shortfall needs both
 * axes: if either has no feasible attempt, the plan is null.
 */
export function growPanePlan(opts: {
  layout: GroupLayout;
  groupId: ChatGroupId;
  tree: Box;
  shortfall: PaneShortfall;
}): PaneResize[] | null {
  const axes: Axis[] = opts.shortfall === 'both' ? ['width', 'height'] : [opts.shortfall];
  const resizes: PaneResize[] = [];
  for (const axis of axes) {
    const plan = planPaneAxis({
      layout: opts.layout,
      groupId: opts.groupId,
      tree: opts.tree,
      axis,
    });
    if (!plan) return null;
    resizes.push(...plan.resizes);
  }
  // A plan that moves nothing would be a button that does nothing.
  return resizes.length > 0 ? resizes : null;
}
