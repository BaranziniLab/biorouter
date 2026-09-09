import { GroupLayout } from '../chatGroups/chatGroupsTypes';

/**
 * D-32 — THE YIELD LADDER.
 *
 * The active chat always wins. Everything else yields, in a fixed order, and the
 * order is the design. Widest-yields-first:
 *
 *   1. the sidebar collapses to an overlay (< 1120px) — AppLayout.sidebarAutoCollapseAction
 *   2. the preview panel narrows to its 360px floor, then yields its column
 *      entirely rather than starve the transcript — previewPanelMode
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

/** The preview panel's own floor. Mirrors ARTIFACT_PANEL_MIN_WIDTH in BaseChat. */
export const PREVIEW_MIN_WIDTH = 360;

/**
 * Below this, a pane cannot seat a chat AND a preview side by side, so rung 2
 * fires: 360 + 360. There is no arrangement at 719px that leaves both floors
 * intact, which is why the panel yields its column instead of splitting the
 * difference and starving both.
 */
export const PREVIEW_YIELD_WIDTH = CHAT_MIN_WIDTH + PREVIEW_MIN_WIDTH;

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

export type PreviewPanelMode = 'side' | 'overlay';

/**
 * Where the preview panel goes for a pane of this width.
 *
 * 'side' — a real column beside the transcript. The panel narrows toward its
 *          360px floor first (BaseChat's clamp does that); the chat holds.
 * 'overlay' — the panel has yielded its column: it takes NO width from the
 *          transcript and floats over the pane instead, exactly as it already
 *          does on a mobile-width window.
 *
 * WHY PANE WIDTH, NOT WINDOW WIDTH. The shipped rule was `useIsMobile()`, i.e.
 * window < 930. That is right for one group and wrong the moment there are two:
 * in a split the pane is decoupled from the window, so a 2-up split in a 1000px
 * window gave each pane ~500px, the panel took its 360px floor, and the
 * transcript was left with 140px — the exact starvation this rung exists to
 * prevent, at a window width the old rule called "roomy". Measuring the PANE
 * generalises the shipped rule instead of replacing it (isMobile is still an
 * override, so nothing between 720 and 930 changes).
 *
 * WHY THIS RUNG NEVER OVERRIDES THE USER. It is a pure function of measured
 * width with NO memory — no auto-close, no "did we or did they" bookkeeping, no
 * way to fight a click. If the user opens an artifact in a 500px pane they get
 * it; the ladder only decides HOW it is presented, which is a fact about the
 * geometry rather than a preference anyone can hold. Closing the panel returns
 * the pane to the transcript at full width, and growing the pane back past 720
 * puts the panel back in its column with no state to get wrong.
 */
export function previewPanelMode(opts: { isMobile: boolean; paneWidth: number }): PreviewPanelMode {
  if (opts.isMobile) return 'overlay';
  // An unmeasured pane (0 before layout, NaN from a detached node) must not read
  // as "infinitely narrow" and slam the panel into an overlay on first paint.
  if (!Number.isFinite(opts.paneWidth) || opts.paneWidth <= 0) return 'side';
  return opts.paneWidth < PREVIEW_YIELD_WIDTH ? 'overlay' : 'side';
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
