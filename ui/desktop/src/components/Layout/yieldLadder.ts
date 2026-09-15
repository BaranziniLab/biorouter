import { GroupLayout } from '../chatGroups/chatGroupsTypes';

/**
 * D-32 — THE YIELD LADDER.
 *
 * The active chat always wins. Everything else yields, in a fixed order, and the
 * order is the design. Widest-yields-first:
 *
 *   1. the sidebar collapses to an overlay (< 1120px) — AppLayout.sidebarAutoCollapseAction
 *   2. the preview panel narrows to its 360px floor, the conversation beside it
 *      narrows to its 440px floor, and below 800px the preview moves ABOVE the
 *      conversation and shares the pane's height — never covering it —
 *      previewPanelMode / previewSideWidth / previewStackHeight
 *   3. tab labels shrink to their TAB_MIN_WIDTH floor, then scroll, then
 *      collapse into a ▾ overflow menu, and NEVER wrap —
 *      shouldShowTabOverflowMenu
 *   4. a split merges back to one group rather than render two useless slivers —
 *      splitYieldAction
 *
 * Nothing here is new chrome. Each rung is something the app could already do;
 * the decision is only WHAT ORDER they let go in.
 *
 * Every rung is a pure function of (width, current state) and is unit-tested,
 * for the reason AppLayout's sidebar rule is: the bugs in this area are STATE
 * bugs (an auto-rule fighting the user's own click), not layout bugs, and jsdom
 * computes no layout — a geometric assertion there passes with the bug present.
 * The geometry is verified by driving the real app and measuring; the DECISIONS
 * are verified here.
 */

/**
 * The floor below which a chat pane stops being a chat.
 *
 * D-36 — read this before trusting the number. D-32 justified 360 with "below
 * this a pane cannot hold a 68ch measure". That rationale is FALSE and is
 * recorded as such: a pane spends ~56px on chrome (a 400px pane measures a
 * 344px column), and 68ch of the body face is ~500px anyway — so no floor near
 * 360 was ever going to seat 68 characters. The number was reasoned backwards
 * from one that felt right.
 *
 * 360 stays because it measures well and is what shipped, but it is defended
 * honestly now: below ~360 a pane stops being USABLE — the text is a gutter,
 * not a document. That is a weaker claim than the original and a true one.
 *
 * This is still the number every other rung yields to.
 */
export const CHAT_MIN_WIDTH = 360;

/**
 * The width below which the sidebar stops holding a column of its own, so the
 * chat's own title/controls must reserve room for the titlebar instead.
 *
 * ONE home, because this number had THREE (`AppLayout`, `TitlebarControls`,
 * and a third copy re-declared inside `BaseChat`). Rung 1 of the ladder fires
 * on it and rungs 2–4 inherit the room it frees, so a copy that drifted would
 * desynchronise the ladder from the titlebar silently — the layout would simply
 * be wrong at one width, with nothing failing. The titlebar reserve had already
 * drifted once this way.
 */
export const SIDEBAR_COMPACT_WIDTH = 1120;

/**
 * The preview panel's own floor beside a conversation: its tab strip, its
 * status strip and a file's text all fit at 360 with zero overflow (measured).
 * Narrower and the tab label is cut to "Lab |" and markdown runs ~37 characters
 * a line — the 285px floor a prototype tried, and was rejected for.
 */
export const PREVIEW_MIN_WIDTH = 360;

/**
 * The tab shrink floor. Mirrors `--tab-min-width` in main.css, and
 * `styles/tabStripFloor.test.ts` asserts the two are the same number.
 *
 * ⚠ It was 88, described as "a glyph, a few characters and the close control",
 * and that description was never measured: a tab spends 73px on its padding,
 * its leading glyph and the close control before the title gets a pixel, so 88
 * left the label 15px — one character and a clipped ellipsis. With six chats
 * open at 1440 the strip read `R.` `R.` `R.` `R.` `B..`. The arithmetic and the
 * trade are written out beside the token in main.css.
 */
export const TAB_MIN_WIDTH = 136;

/** `.br-group-splitter`'s flex-basis in main.css. */
export const GROUP_SPLITTER_WIDTH = 1;

// ---------------------------------------------------------------------------
// Rung 2 — the preview panel
// ---------------------------------------------------------------------------

/**
 * 'side'  — a full-height column on the pane's right, beside the conversation.
 * 'stack' — a sheet directly under the chat header, at the pane's full width,
 *           ABOVE the transcript and the composer. The pair shares the pane's
 *           HEIGHT instead of its width.
 *
 * There is no third shape, and that is the fix. Rung 2 used to have an
 * 'overlay' that floated the preview over the pane below 720px, and measured in
 * the real app it covered the pane's whole transcript and composer: in a 576px
 * pane (a two-pane split at 1440×900 with the sidebar open) the conversation the
 * preview belonged to was simply gone. Nothing here ever covers the
 * conversation; a user who wants the preview bigger drags its edge.
 */
export type PreviewPanelMode = 'side' | 'stack';

/**
 * THE NARROWEST CONVERSATION COLUMN BESIDE A PREVIEW: 440px.
 *
 * Measured on real transcript text at the transcript's own 14px/24px face: 440
 * is the first chat column in which no full line runs under 45 characters
 * (average 57.3, shortest 49) — the bottom of the 45–75 character band. The
 * composer's toolbar collapses behind its "+" below 528 (rung 3b), which is a
 * designed state, not a failure, so it does not set this floor.
 */
export const READABLE_CHAT_WIDTH = 440;

/**
 * THE SEAM: at or above this pane width the preview sits BESIDE the
 * conversation, below it the preview stacks. 360 + 440 = 800.
 *
 * 800 falls on no common split — a two-pane split reaches it only from a 1601px
 * window with the sidebar collapsed, or 1889px with it open — so a user does not
 * sit on the seam by accident. The rule it replaced (720) sat exactly on a 1440px
 * window. `measures.test.ts` pins the literal and its two terms.
 */
export const PREVIEW_SIDE_WIDTH = PREVIEW_MIN_WIDTH + READABLE_CHAT_WIDTH;

/**
 * The conversation column's PREFERRED width beside a preview — what the panel
 * gives up first. By default the panel narrows to its 360 floor while the chat
 * keeps 640, and only then does the chat narrow toward 440. Not a floor: the
 * user may drag the panel wider, down to READABLE_CHAT_WIDTH.
 */
export const PREVIEW_PREFERRED_CHAT_WIDTH = 640;

/** The side panel's ceiling, however wide the pane. */
export const PREVIEW_MAX_WIDTH = 920;

/** The side panel's default share of the pane, before the clamps. */
export const PREVIEW_DEFAULT_WIDTH_RATIO = 0.48;

/** `--chrome-height`: a SIDE panel's strip continues the chat header's band. */
export const PREVIEW_SIDE_STRIP_HEIGHT = 44;

/**
 * `--dock-height`: a STACKED sheet's strip, the terminal dock's treatment. It is
 * also the least a stacked preview is ever squeezed to and the height it folds
 * to, so its tabs and its close control stay reachable.
 */
export const PREVIEW_STACK_STRIP_HEIGHT = 36;

/**
 * A stacked sheet's floor: its strip plus ~160px of content — a chart's plot
 * area or eight lines of a file. A drag clamps here; short content may sit under
 * it (it has nothing to put in the rest).
 */
export const PREVIEW_STACK_MIN_HEIGHT = 200;

/**
 * The transcript a stacked sheet always leaves: the tail of the newest turn,
 * which is the one the preview is being discussed in. The conversation's floor
 * is this plus its MEASURED chrome (the composer bar), so a growing composer
 * shrinks the sheet rather than the transcript.
 */
export const PREVIEW_TRANSCRIPT_MIN_HEIGHT = 146;

/**
 * The stacked sheet's resize edge: an 8px hit area that sits in the transcript's
 * top padding, directly under the sheet's border — so the edge covers nothing
 * either side can use, and neither side's rows are sliced flush against the
 * hairline. It is part of the conversation's box, so the floor counts it.
 */
export const PREVIEW_STACK_EDGE_HEIGHT = 8;

/** An undragged stacked sheet takes at most half the height below the header. */
export const PREVIEW_STACK_RATIO = 0.5;

/**
 * A drag that takes the sheet's edge above the midpoint of [strip, floor] folds
 * it to its strip; anywhere between here and the floor it clamps to the floor.
 */
export const PREVIEW_FOLD_THRESHOLD = (PREVIEW_STACK_STRIP_HEIGHT + PREVIEW_STACK_MIN_HEIGHT) / 2;

function measured(value: number | null | undefined): value is number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0;
}

/**
 * Where the preview goes for a pane of this width.
 *
 * WHY PANE WIDTH, NOT WINDOW WIDTH. The shipped rule was `useIsMobile()`, i.e.
 * window < 930. That is right for one group and wrong the moment there are two:
 * in a split the pane is decoupled from the window. Measure the split box the
 * panel and the conversation share, never the window and never `vw`.
 *
 * A 12px return buffer prevents repeated flips during a window-edge drag.
 * The side floor remains 800px; a stacked preview returns beside the chat at 812px.
 */
export function previewPanelMode(opts: {
  paneWidth: number;
  previous?: PreviewPanelMode;
}): PreviewPanelMode {
  if (!measured(opts.paneWidth)) return opts.previous ?? 'side';
  const threshold = PREVIEW_SIDE_WIDTH + (opts.previous === 'stack' ? 12 : 0);
  return opts.paneWidth >= threshold ? 'side' : 'stack';
}

/**
 * The widest a SIDE panel may be dragged in this pane: the conversation never
 * goes under READABLE_CHAT_WIDTH, and the panel never over its ceiling.
 */
export function previewSideMaxWidth(paneWidth: number): number {
  if (!measured(paneWidth)) return PREVIEW_MIN_WIDTH;
  return Math.max(PREVIEW_MIN_WIDTH, Math.min(PREVIEW_MAX_WIDTH, paneWidth - READABLE_CHAT_WIDTH));
}

/**
 * A SIDE panel's width: the user's dragged width clamped to
 * [360, min(920, pane − 440)], or with no drag
 * `clamp(round(0.48·pane), 360, max(360, min(920, pane − 640)))` — the panel
 * yields to its 360 floor before the conversation narrows from 640.
 */
export function previewSideWidth(opts: { paneWidth: number; userWidth?: number | null }): number {
  const { paneWidth } = opts;
  const userWidth = opts.userWidth ?? null;
  if (userWidth !== null && Number.isFinite(userWidth)) {
    return Math.min(
      Math.max(Math.round(userWidth), PREVIEW_MIN_WIDTH),
      previewSideMaxWidth(paneWidth)
    );
  }
  if (!measured(paneWidth)) return PREVIEW_MIN_WIDTH;
  const defaultMax = Math.max(
    PREVIEW_MIN_WIDTH,
    Math.min(PREVIEW_MAX_WIDTH, paneWidth - PREVIEW_PREFERRED_CHAT_WIDTH)
  );
  return Math.min(
    Math.max(Math.round(paneWidth * PREVIEW_DEFAULT_WIDTH_RATIO), PREVIEW_MIN_WIDTH),
    defaultMax
  );
}

/**
 * The conversation's floor under a stacked sheet: its measured chrome (the
 * composer bar, 174px today; a replay's page header), the 8px resize edge, and
 * 146px of transcript below it.
 * An unmeasurable chrome counts as none — the CSS grid still holds the
 * transcript's own 146px, so the worst case is a sheet a composer's height too
 * tall for one frame, never a hidden transcript.
 */
export function previewChatFloor(chrome: number | null | undefined): number {
  const measuredChrome = typeof chrome === 'number' && Number.isFinite(chrome) && chrome > 0;
  return (
    (measuredChrome ? Math.round(chrome as number) : 0) +
    PREVIEW_STACK_EDGE_HEIGHT +
    PREVIEW_TRANSCRIPT_MIN_HEIGHT
  );
}

/**
 * How tall a STACKED sheet is.
 *
 *   bodyHeight   H — the split box's height below the chat header
 *   chatFloor    F — previewChatFloor(measured chrome)
 *   ratio        the share of H the user dragged it to; null = never dragged
 *   contentHeight the whole sheet's natural height (strip + content), or null
 *                when the content cannot say (loading, an image, a live page)
 *   folded       the user folded it to its strip
 *
 * Not dragged: `clamp(min(round(0.5·H), content), min(200, content), H − F)` —
 * half, or less when the content is shorter, so a three-line file takes 156px
 * rather than half the pane. Dragged: `clamp(ratio·H, 200, H − F)`, ignoring the
 * content: the user's size is theirs until the panel closes.
 *
 * WHEN THE FLOORS COLLIDE the conversation wins: if H − F is under the sheet's
 * floor the sheet is `max(36, H − F)`, yielding down to its strip and never
 * past it, so it can always be closed.
 */
export function previewStackHeight(opts: {
  bodyHeight: number;
  chatFloor: number;
  ratio?: number | null;
  contentHeight?: number | null;
  folded?: boolean;
}): number {
  const { bodyHeight: H } = opts;
  if (!measured(H)) return 0;
  if (opts.folded) return PREVIEW_STACK_STRIP_HEIGHT;
  const ceiling = H - (Number.isFinite(opts.chatFloor) ? Math.max(0, opts.chatFloor) : 0);
  const ratio = opts.ratio ?? null;
  const dragged = ratio !== null && Number.isFinite(ratio);
  // Content never asks for less than the strip it is shown under.
  const content =
    !dragged && measured(opts.contentHeight)
      ? Math.max(PREVIEW_STACK_STRIP_HEIGHT, Math.ceil(opts.contentHeight))
      : null;

  const floor =
    content === null ? PREVIEW_STACK_MIN_HEIGHT : Math.min(PREVIEW_STACK_MIN_HEIGHT, content);
  if (ceiling < floor) return Math.max(PREVIEW_STACK_STRIP_HEIGHT, Math.round(ceiling));

  const preferred = dragged
    ? Math.round(Math.min(Math.max(ratio, 0), 1) * H)
    : content === null
      ? Math.round(H * PREVIEW_STACK_RATIO)
      : Math.min(Math.round(H * PREVIEW_STACK_RATIO), content);
  return Math.min(Math.max(preferred, floor), Math.round(ceiling));
}

/**
 * The outcome of dragging a stacked sheet's bottom edge to `wantedHeight`.
 *
 * Above the fold threshold (the midpoint of the strip and the floor) the sheet
 * folds to its strip; otherwise it takes `clamp(wanted, 200, H − F)` and the
 * ratio that height is of H, which is what persists across later resizes.
 */
export function previewStackDrag(opts: {
  bodyHeight: number;
  chatFloor: number;
  wantedHeight: number;
}): { folded: true; height: number } | { folded: false; height: number; ratio: number } {
  const { bodyHeight: H } = opts;
  if (!Number.isFinite(opts.wantedHeight) || opts.wantedHeight < PREVIEW_FOLD_THRESHOLD) {
    return { folded: true, height: PREVIEW_STACK_STRIP_HEIGHT };
  }
  if (!measured(H)) return { folded: false, height: 0, ratio: PREVIEW_STACK_RATIO };
  const height = previewStackHeight({
    bodyHeight: H,
    chatFloor: opts.chatFloor,
    ratio: opts.wantedHeight / H,
  });
  return { folded: false, height, ratio: height / H };
}

// ---------------------------------------------------------------------------
// Rung 3 — the tab strip
// ---------------------------------------------------------------------------

/** Sub-pixel slack: a fractional scrollWidth must not summon the menu. */
export const TAB_OVERFLOW_EPSILON = 1;

/**
 * Whether the strip needs its ▾ overflow menu.
 *
 * The strip already shrinks to TAB_MIN_WIDTH and then scrolls (main.css:
 * `flex-wrap: nowrap; overflow-x: auto`), and it must NEVER wrap — a wrapped
 * second row moves every tab under the cursor. What was missing is the last
 * step: once tabs are scrolled out of sight, a way to reach them without
 * scrubbing.
 *
 * The signal is the browser's own measurement rather than re-deriving the box
 * math from the CSS: two copies of "how wide is a tab" would drift, and only one
 * of them ships.
 *
 * NO HYSTERESIS, AND IT CANNOT LOOP — the menu button lives OUTSIDE the strip's
 * scroll box, so showing it narrows `clientWidth`, which can only make an
 * overflowing strip overflow more, and hiding it widens `clientWidth`, which can
 * only make a fitting strip fit better. Both directions are monotone, so there
 * is no width at which the two states chase each other. (Had the button been a
 * sticky child INSIDE the strip it would have latched: its own width would keep
 * the overflow it was summoned by alive forever.)
 */
export function shouldShowTabOverflowMenu(opts: {
  scrollWidth: number;
  clientWidth: number;
  tabCount: number;
}): boolean {
  // A menu that can only offer you the tab you are already on is noise.
  if (opts.tabCount < 2) return false;
  if (!Number.isFinite(opts.scrollWidth) || !Number.isFinite(opts.clientWidth)) return false;
  return opts.scrollWidth - opts.clientWidth > TAB_OVERFLOW_EPSILON;
}

// ---------------------------------------------------------------------------
// Rung 3b — the composer toolbar
// ---------------------------------------------------------------------------

/**
 * The width the composer's bottom control row needs with every picker expanded:
 * working directory, extensions/skills/knowledge, reasoning effort, model,
 * context gauge, cost, and the send button, with their gaps and dividers. Below
 * this the row can no longer lay them out without overlap, so they collapse
 * behind a single "+" at the lower-left and the row becomes just `[+] … [Send]`.
 *
 * A fixed threshold against the row's OWN width, not `scrollWidth > clientWidth`,
 * and for the same reason rung 3's menu button lives outside the scroll box:
 * collapsing REMOVES the controls, which changes the content width, so a
 * content-overflow rule would collapse (content now fits) → expand (overflows) →
 * collapse forever. The row's own box is fixed by the pane regardless of what it
 * holds, so measuring it against a constant is monotone and cannot oscillate.
 *
 * A pane can be as narrow as CHAT_MIN_WIDTH (360); the collapsed row is far
 * narrower than that, so at or above the pane floor the composer never overlaps.
 */
export const COMPOSER_TOOLBAR_MIN_WIDTH = 480;

export function shouldCollapseComposerToolbar(opts: { availableWidth: number }): boolean {
  // An unmeasured box is not a narrow one: 0/NaN on first paint must not flash
  // the collapsed state before the real width is known.
  if (!Number.isFinite(opts.availableWidth) || opts.availableWidth <= 0) return false;
  return opts.availableWidth < COMPOSER_TOOLBAR_MIN_WIDTH;
}

// ---------------------------------------------------------------------------
// Rung 4 — the split
// ---------------------------------------------------------------------------

/**
 * The narrowest width at which this layout still gives every group a chat.
 *
 * A TREE WALK, never `groupCount * CHAT_MIN`: a `col` split stacks its children,
 * so two groups one above the other need the width of ONE, not two. Counting
 * leaves would merge a perfectly good vertical split the moment a horizontal one
 * of the same size would have been too narrow.
 */
export function layoutMinWidth(layout: GroupLayout): number {
  if (layout.kind === 'leaf') return CHAT_MIN_WIDTH;
  if (layout.children.length === 0) return CHAT_MIN_WIDTH;
  const children = layout.children.map(layoutMinWidth);
  if (layout.dir === 'col') return Math.max(...children);
  return children.reduce((a, b) => a + b, 0) + (children.length - 1) * GROUP_SPLITTER_WIDTH;
}

export function layoutFitsWidth(layout: GroupLayout, availableWidth: number): boolean {
  // Refuse to act on a width we have not really measured. A zero-width sample
  // (first paint, a hidden window) would otherwise read as "nothing fits" and
  // merge the user's split before they ever saw it.
  if (!Number.isFinite(availableWidth) || availableWidth <= 0) return true;
  return availableWidth >= layoutMinWidth(layout);
}

/**
 * Which layout the fit is judged against.
 *
 * While we hold a snapshot, the question is NOT "does the single merged group
 * fit" — it trivially does, at every width — it is "does the layout we still owe
 * the user fit yet". Judging the merged tree instead would read the next
 * shrink-step as a crossing back INTO fitting and re-split the window at 500px.
 */
export function splitYieldFits(opts: {
  layout: GroupLayout;
  snapshotLayout: GroupLayout | null;
  availableWidth: number;
}): boolean {
  return layoutFitsWidth(opts.snapshotLayout ?? opts.layout, opts.availableWidth);
}

/**
 * Has the user made the snapshot ours to forget?
 *
 * If they split again while we had their layout merged away, that new split is
 * theirs and the snapshot no longer describes anything we owe them. Restoring it
 * later would throw away a layout the user built by hand — the same class of bug
 * as the sidebar effect that swallowed the un-collapse click.
 */
export function splitSnapshotIsStale(opts: { groupCount: number }): boolean {
  return opts.groupCount > 1;
}

/**
 * The two sides of the threshold for ONE width sample.
 *
 * The bucket is "does the layout we owe fit at width W" — and BOTH sides are
 * computed against the SAME reference layout, one with the previous width and
 * one with the current. That is the whole trick, and it is not decoration:
 *
 * A watcher that simply remembered `wasFitting` as a boolean would treat a
 * LAYOUT change as a crossing. The user splits a 600px window by hand: last
 * sample said "fits" (one leaf needs 360), this sample says "does not fit" (two
 * leaves need 721) — a crossing appears out of nowhere and the ladder merges the
 * split inside the same tick as the drop. That is exactly the sidebar bug
 * (an effect that re-runs on the user's own state change and always wins),
 * rebuilt from scratch. Re-deriving the previous side from the previous WIDTH
 * makes a layout change at a stable width provably a no-op.
 */
export function splitYieldSample(opts: {
  layout: GroupLayout;
  snapshotLayout: GroupLayout | null;
  /** The previous width sample, or null if we have never sampled. */
  lastWidth: number | null;
  width: number;
}): { wasFitting: boolean | null; isFitting: boolean } {
  const reference = opts.snapshotLayout ?? opts.layout;
  return {
    wasFitting: opts.lastWidth === null ? null : layoutFitsWidth(reference, opts.lastWidth),
    isFitting: layoutFitsWidth(reference, opts.width),
  };
}

export type SplitYieldAction = 'merge' | 'restore' | 'none';

/**
 * What the width watcher should do to the split, given a width sample.
 *
 * Deliberately the same shape as AppLayout's `sidebarAutoCollapseAction`, and
 * for the same reason: this reacts ONLY to a width CROSSING, never to the
 * layout itself. Anything other than a crossing returns 'none' — if the width
 * did not change buckets, the layout is the one the user asked for and we must
 * not touch it. That is what lets a user split a narrow window by hand and keep
 * the split: no crossing, no merge, no fight.
 *
 * `wasFitting` is the side of the threshold we were on last sample, or null if
 * we have never sampled.
 */
export function splitYieldAction(opts: {
  wasFitting: boolean | null;
  isFitting: boolean;
  groupCount: number;
  autoMerged: boolean;
}): SplitYieldAction {
  const { wasFitting, isFitting, groupCount, autoMerged } = opts;
  // Not a crossing → the user's layout stands.
  if (wasFitting === isFitting) return 'none';
  // Crossed INTO too-narrow: only merge if there is actually a split to merge.
  if (!isFitting) return groupCount > 1 ? 'merge' : 'none';
  // Crossed OUT of too-narrow: only give the split back if WE took it away.
  return autoMerged ? 'restore' : 'none';
}
