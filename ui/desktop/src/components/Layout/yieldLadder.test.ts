import { describe, expect, it } from 'vitest';
import { GroupLayout } from '../chatGroups/chatGroupsTypes';
import {
  CHAT_MIN_WIDTH,
  PREVIEW_DEFAULT_WIDTH_RATIO,
  PREVIEW_FOLD_THRESHOLD,
  PREVIEW_MAX_WIDTH,
  PREVIEW_MIN_WIDTH,
  PREVIEW_PREFERRED_CHAT_WIDTH,
  PREVIEW_SIDE_STRIP_HEIGHT,
  PREVIEW_SIDE_WIDTH,
  PREVIEW_STACK_MIN_HEIGHT,
  PREVIEW_STACK_EDGE_HEIGHT,
  PREVIEW_STACK_RATIO,
  PREVIEW_STACK_STRIP_HEIGHT,
  PREVIEW_TRANSCRIPT_MIN_HEIGHT,
  READABLE_CHAT_WIDTH,
  SplitYieldAction,
  layoutFitsWidth,
  layoutMinWidth,
  previewChatFloor,
  previewPanelMode,
  previewSideMaxWidth,
  previewSideWidth,
  previewStackDrag,
  previewStackHeight,
  shouldShowTabOverflowMenu,
  shouldCollapseComposerToolbar,
  COMPOSER_TOOLBAR_MIN_WIDTH,
  splitSnapshotIsStale,
  splitYieldAction,
  splitYieldFits,
  splitYieldSample,
} from './yieldLadder';

const leaf = (groupId: string): GroupLayout => ({ kind: 'leaf', groupId });
const row = (...children: GroupLayout[]): GroupLayout => ({
  kind: 'branch',
  dir: 'row',
  children,
  sizes: children.map(() => 1 / children.length),
});
const col = (...children: GroupLayout[]): GroupLayout => ({
  kind: 'branch',
  dir: 'col',
  children,
  sizes: children.map(() => 1 / children.length),
});

/**
 * Rung 2. Two bugs this encodes were MEASURED in the real app:
 *
 *   - a 2-up split in a 1000px window gave each pane ~500px; the preview panel
 *     took its 360px floor and left the transcript 140px (the old rule asked the
 *     WINDOW whether there was room, and the window said yes);
 *   - its replacement floated the preview OVER any pane under 720px, and in a
 *     576px pane (a two-pane split at 1440×900, sidebar open) it covered the
 *     transcript and the composer of the conversation it belonged to.
 *
 * Every threshold is asserted on BOTH sides, one pixel apart.
 */
describe('previewPanelMode (rung 2 — beside at 800px and wider, stacked below)', () => {
  it('is built from the two floors it protects: 360 + 440 = 800', () => {
    expect(PREVIEW_MIN_WIDTH).toBe(360);
    expect(READABLE_CHAT_WIDTH).toBe(440);
    expect(PREVIEW_SIDE_WIDTH).toBe(800);
  });

  it('keeps the preview beside the conversation at 800px', () => {
    expect(previewPanelMode({ paneWidth: 800 })).toBe('side');
    expect(previewPanelMode({ paneWidth: 1200 })).toBe('side');
  });

  it('stacks it one pixel below', () => {
    expect(previewPanelMode({ paneWidth: 799 })).toBe('stack');
    expect(previewPanelMode({ paneWidth: 799.5 })).toBe('stack');
  });

  it('THE REGRESSION: the operator’s 576px pane stacks — it is never covered', () => {
    // 1440×900, sidebar open, two panes. The old rule returned 'overlay' here,
    // which hid the transcript and the composer behind the preview.
    const mode = previewPanelMode({ paneWidth: 576 });
    expect(mode).toBe('stack');
    expect(mode).not.toBe('overlay' as never);
  });

  it('sees the pane, not the window: a 960px pane in a 1920px window stays beside', () => {
    expect(previewPanelMode({ paneWidth: 960 })).toBe('side');
    expect(previewPanelMode({ paneWidth: 816 })).toBe('side');
  });

  it('does not stack on an unmeasured pane', () => {
    for (const paneWidth of [0, -1, Number.NaN, Number.POSITIVE_INFINITY]) {
      expect(previewPanelMode({ paneWidth })).toBe('side');
    }
  });

  it('is pure, with one crossing and no hysteresis across a 1px sweep', () => {
    const modes: string[] = [];
    for (let width = 700; width <= 900; width += 1)
      modes.push(previewPanelMode({ paneWidth: width }));
    const crossings = modes.filter((mode, index) => index > 0 && mode !== modes[index - 1]);
    expect(crossings).toEqual(['side']);
    const down: string[] = [];
    for (let width = 900; width >= 700; width -= 1)
      down.push(previewPanelMode({ paneWidth: width }));
    expect(down.reverse()).toEqual(modes);
  });
});

describe('previewSideWidth (rung 2 — the side panel yields before the conversation does)', () => {
  it('defaults to 48% of the pane between the clamps', () => {
    expect(PREVIEW_DEFAULT_WIDTH_RATIO).toBe(0.48);
    expect(previewSideWidth({ paneWidth: 1600 })).toBe(768);
  });

  it('never exceeds 920, however wide the pane', () => {
    expect(PREVIEW_MAX_WIDTH).toBe(920);
    expect(previewSideWidth({ paneWidth: 3000 })).toBe(920);
  });

  it('narrows to 360 FIRST while the conversation keeps its preferred 640', () => {
    expect(PREVIEW_PREFERRED_CHAT_WIDTH).toBe(640);
    // 0.48 × 1100 = 528, but that would leave the chat 572 < 640: the panel gives.
    expect(previewSideWidth({ paneWidth: 1100 })).toBe(460);
    expect(1100 - previewSideWidth({ paneWidth: 1100 })).toBe(640);
    // 1000 − 640 = 360: the panel reaches its floor exactly as the chat reaches 640.
    expect(previewSideWidth({ paneWidth: 1000 })).toBe(360);
    expect(previewSideWidth({ paneWidth: 1001 })).toBe(361);
  });

  it('then the conversation narrows toward 440, reaching it at the 800px seam', () => {
    expect(previewSideWidth({ paneWidth: 999 })).toBe(360);
    expect(previewSideWidth({ paneWidth: 900 })).toBe(360);
    expect(800 - previewSideWidth({ paneWidth: 800 })).toBe(440);
  });

  it('clamps a drag to [360, min(920, pane − 440)] — the chat is never under 440', () => {
    expect(previewSideMaxWidth(1000)).toBe(560);
    expect(previewSideWidth({ paneWidth: 1000, userWidth: 900 })).toBe(560);
    expect(1000 - previewSideWidth({ paneWidth: 1000, userWidth: 900 })).toBe(440);
    expect(previewSideWidth({ paneWidth: 1000, userWidth: 100 })).toBe(360);
    expect(previewSideWidth({ paneWidth: 2000, userWidth: 1500 })).toBe(920);
    expect(previewSideMaxWidth(800)).toBe(360);
    expect(previewSideMaxWidth(801)).toBe(361);
  });

  it('keeps a drag the pane can still seat', () => {
    expect(previewSideWidth({ paneWidth: 1400, userWidth: 700 })).toBe(700);
  });
});

describe('previewStackHeight (rung 2 — a stacked sheet shares the height, never the conversation)', () => {
  const H = 856; // 1440×900, below a 44px header
  const F = previewChatFloor(174); // the composer bar + the 8px edge + 146 of transcript

  it('pins its numbers', () => {
    expect(PREVIEW_STACK_STRIP_HEIGHT).toBe(36);
    expect(PREVIEW_SIDE_STRIP_HEIGHT).toBe(44);
    expect(PREVIEW_STACK_MIN_HEIGHT).toBe(200);
    expect(PREVIEW_TRANSCRIPT_MIN_HEIGHT).toBe(146);
    expect(PREVIEW_STACK_RATIO).toBe(0.5);
    expect(PREVIEW_FOLD_THRESHOLD).toBe(118);
    expect(PREVIEW_STACK_EDGE_HEIGHT).toBe(8);
    expect(F).toBe(328);
  });

  it('takes half the body when the content is tall or unknown', () => {
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F })).toBe(428);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 5000 })).toBe(428);
  });

  it('fits short content instead of leaving half the pane empty', () => {
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 156 })).toBe(156);
    // Content may sit under the 200 floor — it has nothing to put in the rest…
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 199.2 })).toBe(200);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 90 })).toBe(90);
    // …but never under the strip it is shown beneath.
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 14 })).toBe(36);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 36 })).toBe(36);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, contentHeight: 37 })).toBe(37);
  });

  it('ignores the content once dragged, and keeps the dragged share across resizes', () => {
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, ratio: 0.3, contentHeight: 90 })).toBe(
      257
    );
    expect(previewStackHeight({ bodyHeight: 1000, chatFloor: F, ratio: 0.3 })).toBe(300);
    // …clamped to the floor and to the conversation's floor.
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, ratio: 0.05 })).toBe(200);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, ratio: 0.95 })).toBe(H - F);
    expect(H - F).toBe(528);
  });

  it('always leaves the conversation its floor, at both sides of the collision', () => {
    // H − F = 200: the sheet gets exactly its floor.
    expect(previewStackHeight({ bodyHeight: 528, chatFloor: F })).toBe(200);
    // H − F = 199: below its floor, the sheet yields to what is left…
    expect(previewStackHeight({ bodyHeight: 527, chatFloor: F })).toBe(199);
    // …down to its strip and never past it.
    expect(previewStackHeight({ bodyHeight: 364, chatFloor: F })).toBe(36);
    expect(previewStackHeight({ bodyHeight: 363, chatFloor: F })).toBe(36);
    expect(previewStackHeight({ bodyHeight: 300, chatFloor: F })).toBe(36);
    for (let body = 364; body <= 2000; body += 7) {
      for (const contentHeight of [null, 60, 156, 420, 5000]) {
        const sheet = previewStackHeight({ bodyHeight: body, chatFloor: F, contentHeight });
        expect(body - sheet).toBeGreaterThanOrEqual(F);
        expect(sheet).toBeGreaterThanOrEqual(36);
      }
    }
  });

  it('folds to its strip whatever else is true', () => {
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, folded: true })).toBe(36);
    expect(previewStackHeight({ bodyHeight: H, chatFloor: F, ratio: 0.9, folded: true })).toBe(36);
  });

  it('is zero on an unmeasured body', () => {
    for (const bodyHeight of [0, -5, Number.NaN]) {
      expect(previewStackHeight({ bodyHeight, chatFloor: F })).toBe(0);
    }
  });

  it('reads a measured chrome and survives an unmeasurable one', () => {
    expect(previewChatFloor(230)).toBe(384);
    expect(previewChatFloor(null)).toBe(154);
    expect(previewChatFloor(Number.NaN)).toBe(154);
  });
});

describe('previewStackDrag (rung 2 — the edge drags, clamps, and folds)', () => {
  const H = 856;
  const F = 328;

  it('folds when dragged above the midpoint of [36, 200], and not one pixel later', () => {
    expect(previewStackDrag({ bodyHeight: H, chatFloor: F, wantedHeight: 117.9 })).toEqual({
      folded: true,
      height: 36,
    });
    expect(previewStackDrag({ bodyHeight: H, chatFloor: F, wantedHeight: 118 })).toEqual({
      folded: false,
      height: 200,
      ratio: 200 / H,
    });
  });

  it('clamps to the floor between the threshold and 200, and to H − F at the bottom', () => {
    expect(previewStackDrag({ bodyHeight: H, chatFloor: F, wantedHeight: 150 }).height).toBe(200);
    expect(previewStackDrag({ bodyHeight: H, chatFloor: F, wantedHeight: 300 }).height).toBe(300);
    expect(previewStackDrag({ bodyHeight: H, chatFloor: F, wantedHeight: 900 }).height).toBe(528);
  });
});

describe('shouldCollapseComposerToolbar (rung 3b — the composer collapses to a +)', () => {
  it('collapses once the row is narrower than the pickers need', () => {
    expect(shouldCollapseComposerToolbar({ availableWidth: COMPOSER_TOOLBAR_MIN_WIDTH - 1 })).toBe(
      true
    );
  });

  it('stays expanded at and above the threshold', () => {
    expect(shouldCollapseComposerToolbar({ availableWidth: COMPOSER_TOOLBAR_MIN_WIDTH })).toBe(
      false
    );
    expect(shouldCollapseComposerToolbar({ availableWidth: 1200 })).toBe(false);
  });

  it('does not flash collapsed on an unmeasured (0/NaN) box at first paint', () => {
    expect(shouldCollapseComposerToolbar({ availableWidth: 0 })).toBe(false);
    expect(shouldCollapseComposerToolbar({ availableWidth: NaN })).toBe(false);
  });

  it('is monotone: measuring the row’s OWN box, not its content, so it cannot oscillate', () => {
    // The collapsed state removes controls, changing content width — but this
    // rule reads the container width, which the pane fixes regardless. So for a
    // given width the decision is the same whether currently collapsed or not.
    for (let w = 200; w <= 900; w += 13) {
      const decision = shouldCollapseComposerToolbar({ availableWidth: w });
      // Re-measuring the same box yields the same decision — no dependence on
      // the current collapsed/expanded content.
      expect(shouldCollapseComposerToolbar({ availableWidth: w })).toBe(decision);
    }
  });
});

describe('shouldShowTabOverflowMenu (rung 3 — shrink, scroll, then ▾, never wrap)', () => {
  it('shows the menu once tabs are scrolled out of sight', () => {
    expect(shouldShowTabOverflowMenu({ scrollWidth: 800, clientWidth: 300, tabCount: 6 })).toBe(
      true
    );
  });

  it('stays away while the tabs fit', () => {
    expect(shouldShowTabOverflowMenu({ scrollWidth: 300, clientWidth: 300, tabCount: 3 })).toBe(
      false
    );
  });

  it('ignores sub-pixel overflow', () => {
    expect(shouldShowTabOverflowMenu({ scrollWidth: 300.4, clientWidth: 300, tabCount: 3 })).toBe(
      false
    );
  });

  it('never offers a menu that could only list the tab you are on', () => {
    expect(shouldShowTabOverflowMenu({ scrollWidth: 900, clientWidth: 40, tabCount: 1 })).toBe(
      false
    );
    expect(shouldShowTabOverflowMenu({ scrollWidth: 900, clientWidth: 40, tabCount: 0 })).toBe(
      false
    );
  });

  it('cannot oscillate: the button lives outside the scroll box, so both directions are monotone', () => {
    // Showing the ▾ costs the strip BUTTON_W of clientWidth. Model both states at
    // the same window and assert the pair is never in disagreement — that is what
    // "no hysteresis needed" actually claims, and it is the claim a sticky
    // in-strip button would have failed.
    const BUTTON_W = 30;
    const CONTENT = 800;
    for (let strip = 0; strip <= 1200; strip += 7) {
      const shown = shouldShowTabOverflowMenu({
        scrollWidth: CONTENT,
        clientWidth: strip - BUTTON_W,
        tabCount: 6,
      });
      const hidden = shouldShowTabOverflowMenu({
        scrollWidth: CONTENT,
        clientWidth: strip,
        tabCount: 6,
      });
      // The only forbidden pair is "hidden says show, and shown says hide" —
      // i.e. each state's measurement demanding the other. Adding width can
      // never create overflow, so this must hold at every width.
      expect(hidden && !shown).toBe(false);
    }
  });
});

describe('layoutMinWidth (rung 4 — what the tree actually costs in width)', () => {
  it('a single group costs one chat floor', () => {
    expect(layoutMinWidth(leaf('a'))).toBe(CHAT_MIN_WIDTH);
  });

  it('a row of two costs both floors plus the splitter', () => {
    expect(layoutMinWidth(row(leaf('a'), leaf('b')))).toBe(721);
  });

  it('a row of four costs four floors plus three splitters', () => {
    expect(layoutMinWidth(row(leaf('a'), leaf('b'), leaf('c'), leaf('d')))).toBe(1443);
  });

  it('a COLUMN of two costs the width of ONE — stacked groups do not divide width', () => {
    // This is why it is a tree walk and not `groupCount * CHAT_MIN`. Counting
    // leaves would merge a perfectly usable vertical split at 700px.
    expect(layoutMinWidth(col(leaf('a'), leaf('b')))).toBe(CHAT_MIN_WIDTH);
    expect(layoutFitsWidth(col(leaf('a'), leaf('b')), 700)).toBe(true);
  });

  it('a column of rows costs the widest row', () => {
    expect(layoutMinWidth(col(row(leaf('a'), leaf('b')), leaf('c')))).toBe(721);
  });

  it('nests: a row whose child is a row', () => {
    expect(layoutMinWidth(row(leaf('a'), row(leaf('b'), leaf('c'))))).toBe(360 + 1 + 721);
  });

  it('treats an unmeasured width as fitting rather than merging a split nobody saw', () => {
    for (const width of [0, -5, Number.NaN]) {
      expect(layoutFitsWidth(row(leaf('a'), leaf('b')), width)).toBe(true);
    }
  });
});

describe('splitYieldFits (which layout the fit is judged against)', () => {
  const split = row(leaf('a'), leaf('b'));

  it('judges the live layout when we hold no snapshot', () => {
    expect(splitYieldFits({ layout: split, snapshotLayout: null, availableWidth: 900 })).toBe(true);
    expect(splitYieldFits({ layout: split, snapshotLayout: null, availableWidth: 700 })).toBe(
      false
    );
  });

  it('THE TRAP: while merged, judges the layout we OWE, not the merged leaf', () => {
    // After a merge the live layout is one leaf, which fits at every width. Ask
    // it and the next shrink-step reads as a crossing back INTO fitting — and
    // the window re-splits at 500px, which is the sidebar bug in a new costume.
    expect(splitYieldFits({ layout: leaf('a'), snapshotLayout: split, availableWidth: 500 })).toBe(
      false
    );
    expect(splitYieldFits({ layout: leaf('a'), snapshotLayout: split, availableWidth: 900 })).toBe(
      true
    );
  });
});

describe('splitSnapshotIsStale', () => {
  it('is ours to keep while the merged layout is still one group', () => {
    expect(splitSnapshotIsStale({ groupCount: 1 })).toBe(false);
  });

  it('is forfeit the moment the user splits again by hand', () => {
    expect(splitSnapshotIsStale({ groupCount: 2 })).toBe(true);
  });
});

/**
 * The watcher, modelled exactly as the shell runs it. Every rung-4 sequence test
 * drives THIS, so the composition under test cannot drift from the composition
 * that ships.
 */
function makeWatcher(initial: GroupLayout) {
  let layout = initial;
  let snapshot: GroupLayout | null = null;
  let lastWidth: number | null = null;

  const sample = (width: number): SplitYieldAction => {
    const groupCount = layout.kind === 'leaf' ? 1 : layout.children.length;
    if (snapshot && splitSnapshotIsStale({ groupCount })) snapshot = null;
    const { wasFitting, isFitting } = splitYieldSample({
      layout,
      snapshotLayout: snapshot,
      lastWidth,
      width,
    });
    const action = splitYieldAction({
      wasFitting,
      isFitting,
      groupCount,
      autoMerged: snapshot !== null,
    });
    lastWidth = width;
    if (action === 'merge') {
      snapshot = layout;
      layout = leaf('a');
    } else if (action === 'restore') {
      layout = snapshot!;
      snapshot = null;
    }
    return action;
  };

  return {
    sample,
    split: (next: GroupLayout) => {
      layout = next;
    },
    get layout() {
      return layout;
    },
    get snapshot() {
      return snapshot;
    },
  };
}

describe('splitYieldSample (the crossing is on WIDTH, never on the layout)', () => {
  it('THE REGRESSION: splitting by hand at a stable width is not a crossing', () => {
    // A watcher that cached `wasFitting` as a bare boolean would say
    // was=true (one leaf fits 600) / is=false (two leaves need 721) and merge
    // the user's split inside the same tick as their drop — the sidebar bug,
    // rebuilt. Re-deriving the previous side from the previous WIDTH makes the
    // two sides agree, because the width did not move.
    const before = splitYieldSample({
      layout: row(leaf('a'), leaf('b')),
      snapshotLayout: null,
      lastWidth: 600,
      width: 600,
    });
    expect(before.wasFitting).toBe(before.isFitting);
    expect(splitYieldAction({ ...before, groupCount: 2, autoMerged: false })).toBe('none');
  });

  it('reports no previous side on the first sample', () => {
    expect(
      splitYieldSample({ layout: leaf('a'), snapshotLayout: null, lastWidth: null, width: 500 })
        .wasFitting
    ).toBeNull();
  });
});

describe('splitYieldAction (rung 4 — merge rather than render two useless slivers)', () => {
  it('replays a full shrink/grow sweep without ever fighting the user', () => {
    // The sidebar's regression test, ported to the split — because this is the
    // same effect shape and would fail the same way.
    const w = makeWatcher(row(leaf('a'), leaf('b'))); // user splits 2-up

    expect(w.sample(1400)).toBe('none'); // wide, split stands
    expect(w.sample(900)).toBe('none'); // still fits (721)
    expect(w.sample(700)).toBe('merge'); // crossed the floor
    expect(w.layout).toEqual(leaf('a'));
    expect(w.sample(700)).toBe('none'); // the watcher re-runs after the merge: no-op
    expect(w.sample(500)).toBe('none'); // shrink further: judged against the snapshot
    expect(w.layout).toEqual(leaf('a'));
    expect(w.sample(900)).toBe('restore'); // back over the floor: the split returns
    expect(w.layout).toEqual(row(leaf('a'), leaf('b')));
    expect(w.sample(900)).toBe('none'); // and settles
    expect(w.snapshot).toBeNull();
  });

  it('keeps a split the user made BY HAND in a too-narrow window', () => {
    // The user wins. No width crossing happened, so the ladder is silent — and
    // stays silent however many times the watcher re-runs.
    const w = makeWatcher(leaf('a'));
    expect(w.sample(600)).toBe('none');
    w.split(row(leaf('a'), leaf('b'))); // the user splits anyway, at 600px
    expect(w.sample(600)).toBe('none');
    expect(w.sample(600)).toBe('none');
    expect(w.layout.kind).toBe('branch'); // their split survives
  });

  it('forgets the snapshot when the user re-splits while merged, and never resurrects it', () => {
    const w = makeWatcher(row(leaf('a'), leaf('b')));
    expect(w.sample(1400)).toBe('none');
    expect(w.sample(700)).toBe('merge');
    w.split(col(leaf('a'), leaf('c'))); // the user builds their OWN layout while merged
    expect(w.sample(700)).toBe('none'); // the snapshot is forfeit here
    expect(w.snapshot).toBeNull();
    expect(w.sample(1400)).toBe('none'); // growing back must NOT restore the old split
    expect(w.layout).toEqual(col(leaf('a'), leaf('c'))); // the user's layout stands
  });

  it('merges a persisted split that loads into an already-too-narrow window', () => {
    const w = makeWatcher(row(leaf('a'), leaf('b')));
    expect(w.sample(700)).toBe('merge'); // first sample, no previous side
    expect(w.sample(1400)).toBe('restore'); // and it is still owed back
  });

  it('does nothing when the width did not cross the threshold', () => {
    expect(
      splitYieldAction({ wasFitting: false, isFitting: false, groupCount: 2, autoMerged: false })
    ).toBe('none');
  });

  it('never fights the user at a stable width, in any state combination', () => {
    for (const fitting of [true, false]) {
      for (const groupCount of [1, 2, 4]) {
        for (const autoMerged of [true, false]) {
          expect(
            splitYieldAction({
              wasFitting: fitting,
              isFitting: fitting,
              groupCount,
              autoMerged,
            })
          ).toBe('none');
        }
      }
    }
  });

  it('merges on crossing INTO too-narrow while there is a split', () => {
    expect(
      splitYieldAction({ wasFitting: true, isFitting: false, groupCount: 2, autoMerged: false })
    ).toBe('merge');
  });

  it('merges on the very first sample in a window that is already too narrow', () => {
    expect(
      splitYieldAction({ wasFitting: null, isFitting: false, groupCount: 3, autoMerged: false })
    ).toBe('merge');
  });

  it('has nothing to merge when there is only one group', () => {
    expect(
      splitYieldAction({ wasFitting: true, isFitting: false, groupCount: 1, autoMerged: false })
    ).toBe('none');
  });

  it('restores on crossing OUT only if WE merged', () => {
    expect(
      splitYieldAction({ wasFitting: false, isFitting: true, groupCount: 1, autoMerged: true })
    ).toBe('restore');
    expect(
      splitYieldAction({ wasFitting: false, isFitting: true, groupCount: 1, autoMerged: false })
    ).toBe('none');
  });
});
