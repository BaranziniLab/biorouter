import { useCallback, useMemo, useState, useEffect, useRef, Fragment, ReactElement } from 'react';
import BaseChat from '../BaseChat';
import InAppTerminalDock from '../InAppTerminalDock';
import { ChatType } from '../../types/chat';
import { useChatGroups } from '../../contexts/ChatGroupsContext';
import { useTerminalDock } from '../../contexts/TerminalDockContext';
import { ChatTabStrip } from './ChatTabStrip';
import { Plus } from '../icons/app-icons';
import { ChatGroupSplitter } from './ChatGroupSplitter';
import { ChatDropOverlay, ChatTabGhost } from './ChatDropOverlay';
import { ChatTabDragProvider } from './ChatTabDragContext';
import { useTabDragReorder } from './useTabDragReorder';
import { CrossWindowPhase, ScreenPoint, payloadFromTab } from './tabTearOff';
import {
  countWindowTabs,
  measureStripBands,
  resolveMergeInsertion,
  type MergeInsertion,
} from './tabTearOffBridge';
import { DropTarget } from './dropZones';
import { groupCountOf } from './chatGroupsLayout';
import { GroupLayoutSnapshot, snapshotGroupLayout } from './chatGroupsReducer';
import { firstLeaf, leafGroupIds, GroupLayout, ChatGroupId } from './chatGroupsTypes';
import { splitSnapshotIsStale, splitYieldAction, splitYieldSample } from '../Layout/yieldLadder';
import { useIsMobile } from '../../hooks/use-mobile';
import { useSidebar } from '../ui/sidebar';
import { SIDEBAR_COMPACT_TITLE_WIDTH } from '../Layout/TitlebarControls';
import { setFocusedChatSession } from '../../utils/extensionErrorUtils';
import {
  getCachedSessionList,
  preloadSessionList,
  subscribeSessionList,
} from '../../utils/sessionListCache';
import type { SessionClassification } from '../../api';

interface ChatGroupsShellProps {
  /** Mirrors the focused chat up to App's hubChat, so AppSidebar's recents
   *  highlight and document.title keep working with ZERO sidebar edits. */
  onChatChange: (chat: ChatType) => void;
}

interface RenderGroupArgs {
  groupId: ChatGroupId;
}

/**
 * Renders the layout TREE.
 *
 * `path` addresses the branch being rendered, from the root: [] is the root,
 * [1, 0] is the first child of the second child. The resize action is
 * path-addressed for the same reason firstLeaf is a walk and not an index — a
 * position in a flattened list means a different node the moment the tree
 * reshapes, and a splitter that resizes the wrong branch is the kind of bug that
 * only shows up at depth 2.
 */
function renderLayout(
  layout: GroupLayout,
  path: readonly number[],
  renderGroup: (args: RenderGroupArgs) => ReactElement,
  renderBranch: (
    layout: Extract<GroupLayout, { kind: 'branch' }>,
    path: readonly number[],
    children: ReactElement[]
  ) => ReactElement
): ReactElement {
  if (layout.kind === 'leaf') return renderGroup({ groupId: layout.groupId });
  const children = layout.children.map((child, index) =>
    renderLayout(child, [...path, index], renderGroup, renderBranch)
  );
  return renderBranch(layout, path, children);
}

/**
 * Privacy tier per session id, for the tab strips (issue #56, R10).
 *
 * Reads the shared session-list cache and asks it to fill itself. The earlier
 * version of this comment claimed AppSidebar warmed the cache "at module
 * scope"; it does not. `preloadSessionList` lives inside AppSidebar's
 * `preloadHome()`, wired to `onFocus`/`onPointerEnter` on the Home nav entry,
 * so it fires only if the user points at Home. What actually warmed the cache
 * on a normal launch was the Hub index route mounting `SessionsInsights`, which
 * calls `refreshSessionList()` — an incidental side effect of an unrelated
 * screen, and absent in a window that opens straight onto a chat. So the strip
 * warms it here: `preloadSessionList()` returns early when the cache is
 * non-null and swallows its own errors, costing one fetch on a cold start.
 *
 * In jsdom, where the module is mocked or the fetch fails, the cache stays null
 * and every tab is simply unmarked — silence, never an assertion of Public.
 *
 * ⚠ What re-emits through `subscribeSessionList` is narrower than it looks.
 * Any `emitChange` reaches this hook — a completed `refreshSessionList`, the
 * name-channel patch, `updateCachedSessionList` (SessionListView's rename and
 * delete), `clearSessionListCache`. But `notifySessionListChanged`, the signal
 * whose own doc-comment says "call after create, diverge, delete or import",
 * has exactly ONE production caller: `useDiverge.ts`. Create and import do not
 * announce, so this cache learns of them only when some list surface mounts.
 *
 * ⚠ A stale cache fails in the UNSAFE direction, not the safe one. The tier is
 * a permanent ratchet server-side (`crates/biorouter/src/privacy/mod.rs`) — it
 * only ever rises public → private — so a cached `public`, or a session the
 * cache has never seen, leaves a now-private chat with no dot. There is no
 * failure mode in which this over-marks. `ChatTabStrip`'s `privacyTiers` prop
 * doc states the same thing, and the two must not drift apart again.
 *
 * ⚠ This USED to record a known gap, and the gap is closed — the note is kept
 * because the shape of the fix is what a future reader needs. It read: the
 * header pill is not live either, `privacy_tier` is never re-read on the turn
 * path, so a chat that ratchets to Private DURING its life shows no marker on
 * either chat-side surface until something reloads it; "closing this needs the
 * escalation to announce itself from the provider-bind path".
 *
 * It does now, twice over. The reply stream states the post-ratchet
 * classification in its own first frames, and `ChatStreamController.refresh
 * SessionBinding` patches the same four fields onto THIS cache as well as onto
 * its own snapshot — so the tab dot, the header pill and the composer read one
 * answer. A row rewritten by another process arrives the same way, through
 * `utils/sessionMetaSubscription`. History rows and the sidebar rail, which read
 * freshly-fetched lists, were always correct and are unchanged.
 */
function useSessionPrivacyTiers(): Record<string, SessionClassification> {
  const [tiers, setTiers] = useState<Record<string, SessionClassification>>({});

  useEffect(() => {
    const read = () => {
      const next: Record<string, SessionClassification> = {};
      for (const session of getCachedSessionList() ?? []) {
        if (session.privacy_tier) next[session.id] = session.privacy_tier;
      }
      // Identity-stable when nothing changed, so a list refresh that touched
      // no tier does not re-render every strip in every pane.
      setTiers((prev) => {
        const prevKeys = Object.keys(prev);
        const same =
          prevKeys.length === Object.keys(next).length &&
          prevKeys.every((id) => prev[id] === next[id]);
        return same ? prev : next;
      });
    };
    read();
    // Subscribe BEFORE asking for the fetch. `preloadSessionList` is async but
    // makes no such promise, and a cache that resolved between `read()` and the
    // subscription would emit to nobody and leave the strip unmarked until the
    // next unrelated change.
    const unsubscribe = subscribeSessionList(read);
    preloadSessionList();
    return unsubscribe;
  }, []);

  return tiers;
}

export function ChatGroupsShell({ onChatChange }: ChatGroupsShellProps) {
  const groups = useChatGroups();
  const terminalDock = useTerminalDock();
  const privacyTiers = useSessionPrivacyTiers();

  const isMobile = useIsMobile();
  const { state: sidebarState } = useSidebar();
  const [isSidebarCompact, setIsSidebarCompact] = useState(
    () => typeof window !== 'undefined' && window.innerWidth < SIDEBAR_COMPACT_TITLE_WIDTH
  );
  useEffect(() => {
    const update = () => setIsSidebarCompact(window.innerWidth < SIDEBAR_COMPACT_TITLE_WIDTH);
    update();
    window.addEventListener('resize', update);
    return () => window.removeEventListener('resize', update);
  }, []);

  const isMacOS = (window?.electron?.platform || 'darwin') === 'darwin';
  const isCompactSidebarOverlayOpen = isSidebarCompact && !isMobile && sidebarState !== 'collapsed';
  const reserveTitlebarControls =
    isMacOS && (isMobile || isSidebarCompact || sidebarState === 'collapsed');

  // firstLeaf is a TREE WALK, never an array index. In a root `col` split both
  // strips sit at x=0 but only the TOP one collides with the traffic lights, so
  // an index would reserve the gap for the wrong strip — silently.
  const reservedGroupId = useMemo(() => (groups ? firstLeaf(groups.state.layout) : null), [groups]);

  const dispatch = groups?.dispatch;
  const handleSelect = useCallback(
    (tabId: string) => dispatch?.({ type: 'activateTab', tabId }),
    [dispatch]
  );
  const handleClose = useCallback(
    (tabId: string) => dispatch?.({ type: 'closeTab', tabId }),
    [dispatch]
  );
  const handleReorder = useCallback(
    (draggedTabId: string, targetTabId: string) =>
      dispatch?.({ type: 'reorderTab', draggedTabId, targetTabId }),
    [dispatch]
  );
  // The "+" at the end of a strip opens a fresh blank chat IN THAT group — the
  // same empty-session tab Cmd+T opens (sessionId '' is not deduped, so every
  // click is a new tab). Per-group, so a "+" on a split pane adds to that pane.
  const handleNewTab = useCallback(
    (groupId: ChatGroupId) => dispatch?.({ type: 'openTab', payload: { sessionId: '', groupId } }),
    [dispatch]
  );
  const handleDropToGroup = useCallback(
    (tabId: string, target: DropTarget) =>
      dispatch?.({
        type: 'moveTabToGroup',
        tabId,
        targetGroupId: target.groupId,
        zone: target.zone,
      }),
    [dispatch]
  );

  // ═════════════════════════════════════════════════════════════════════════
  // TAB TEAR-OFF AND MERGE — the renderer's half of the cross-window gesture.
  //
  // docs/design/astryx-adoption/tab-tear-off-and-merge.md, Phase 3. This window
  // plays BOTH roles and they never overlap in time:
  //
  //   as SOURCE — it holds the pointer capture, reports the screen point on
  //   every move that leaves its own content rect, and commits on release.
  //
  //   as TARGET — it receives no pointer events at all (measured, Phase 0), so
  //   its caret and its insert are driven entirely by IPC from main.
  //
  // NOTHING HERE ASKS WHETHER A TAB IS RUNNING. Design D1 refused to move a tab
  // with a turn in flight; it is superseded (§3.1) — the turn stream now has as
  // many subscribers as it likes, and the destination window rejoins the turn
  // off `/agent/resume` on load, from the sequence it has already painted.
  // ═════════════════════════════════════════════════════════════════════════

  const desktop = typeof window !== 'undefined' ? window.electron : undefined;
  const stateRefForDrag = useRef(groups?.state);
  stateRefForDrag.current = groups?.state;

  // `commitCrossWindow` needs the dragged tab and the grab offset, both of which
  // live in the hook it is an ARGUMENT to. A ref breaks that cycle without
  // making the callback identity change per pointermove — which would re-run the
  // hook's effect and drop its window listeners in the middle of a gesture.
  const dragRef = useRef<{ draggedTabId: string | null; grabOffset: { x: number; y: number } }>({
    draggedTabId: null,
    grabOffset: { x: 0, y: 0 },
  });

  /** The caret a MERGE is currently previewing in this window, if any. */
  const [remoteDrop, setRemoteDrop] = useState<MergeInsertion | null>(null);

  const reportCrossWindow = useCallback(
    (phase: CrossWindowPhase, point: ScreenPoint) => {
      if (!desktop?.tabDragMove) return;
      // `local` is reported exactly once, on the way back in — the hook
      // guarantees that. It means "no cross-window preview anywhere", which is
      // the same message `tabDragEnd` carries, so it uses the same door.
      if (phase.kind === 'local') desktop.tabDragEnd?.();
      else desktop.tabDragMove({ screenX: point.screenX, screenY: point.screenY });
    },
    [desktop]
  );

  const commitCrossWindow = useCallback(
    (phase: CrossWindowPhase, point: ScreenPoint) => {
      const state = stateRefForDrag.current;
      const dispatchNow = dispatch;
      if (!state || !dispatchNow || !desktop?.tabDragCommit) return;
      const draggedTabId = phase.kind === 'local' ? null : dragRef.current?.draggedTabId;
      if (!draggedTabId) return;
      const tab = leafGroupIds(state.layout)
        .flatMap((gid) => state.groups[gid]?.tabs ?? [])
        .find((candidate) => candidate.tabId === draggedTabId);
      if (!tab) return;

      // Obligation 2 / D5. `isOnlyTab` is sent rather than acted on here because
      // a tear-off of the last tab is a no-op while a MERGE of it is allowed and
      // closes this window — and only main knows which of those this drop is.
      const isOnlyTab = countWindowTabs(state) <= 1;

      void desktop
        .tabDragCommit({
          point: { screenX: point.screenX, screenY: point.screenY },
          grabOffset: dragRef.current?.grabOffset ?? { x: 0, y: 0 },
          tab: payloadFromTab(tab),
          isOnlyTab,
        })
        .then((result) => {
          // `noop` is the answer to everything that went wrong, and it means
          // exactly one thing: the tab stays. Never remove it speculatively.
          if (!result || result.outcome === 'noop') return;
          if (result.outcome === 'merge' && isOnlyTab) {
            // D6a — the target has ACKNOWLEDGED the insert, so this window is
            // now empty and closing it loses nothing. Closing outright rather
            // than closing the tab first: `closeTab` on the last tab would bounce
            // this window Home (useEmptyPairRedirect) for the frame before it
            // disappears.
            desktop.closeWindow?.();
            return;
          }
          dispatchNow({ type: 'closeTab', tabId: draggedTabId });
        })
        .catch(() => {
          // An IPC failure is not a reason to lose a chat.
        });
    },
    [desktop, dispatch]
  );

  // ONE gesture for every strip: the tint has to appear over the TARGET group,
  // which is a sibling of the source strip, so the drag cannot live inside a
  // strip. Handed down through ChatTabDragProvider.
  const drag = useTabDragReorder({
    onReorder: handleReorder,
    onDropToGroup: handleDropToGroup,
    onCrossWindow: reportCrossWindow,
    onCrossWindowCommit: commitCrossWindow,
  });
  dragRef.current = {
    draggedTabId: drag.draggedTabId,
    grabOffset: drag.ghost
      ? { x: drag.ghost.grabOffsetX, y: drag.ghost.grabOffsetY }
      : { x: 0, y: 0 },
  };

  const layoutForBands = groups?.state.layout;

  /**
   * Tell main where this window's tab strips are.
   *
   * Viewport-relative — main translates with `getContentBounds()`, which the
   * renderer cannot do for itself (`window.screenX` is the WINDOW origin, and
   * the difference from the content origin is the title bar, which is precisely
   * the band being measured).
   *
   * Reported on mount, on resize, and whenever the layout tree changes, because
   * a split gives this window a second strip and each one is an independent
   * merge target. NOT on window MOVE: bands are viewport-relative, so a move
   * cannot change them, and main re-reads the content rect on every pointermove
   * anyway.
   */
  useEffect(() => {
    const report = () => desktop?.tabDragRegisterBands?.(measureStripBands(document));
    // One frame late so the strips have been laid out — a `getBoundingClientRect`
    // taken in the same commit that added a pane measures the pane before flex
    // has divided the row.
    const raf = window.requestAnimationFrame(report);
    window.addEventListener('resize', report);
    return () => {
      window.cancelAnimationFrame(raf);
      window.removeEventListener('resize', report);
    };
    // `layoutForBands` alone: the group COUNT is derived from the tree, so a
    // split that adds a strip already changes this identity.
  }, [desktop, layoutForBands]);

  /**
   * TARGET-SIDE. Main forwards the source's screen point; this window converts
   * it to its own client coordinates and paints the ordinary insertion caret.
   *
   * `screenX − window.screenX` is the conversion, and it was measured to the
   * pixel in Phase 0 rather than assumed. This window must NOT raise or focus
   * itself here: it does not hold the pointer, and coming forward would take the
   * capture away from the window that does (design D3).
   */
  useEffect(() => {
    if (!desktop?.on) return;
    return desktop.on('tab-drag:preview', (_event, ...args) => {
      const payload = args[0] as
        | { active?: boolean; screenX?: number; screenY?: number }
        | undefined;
      if (!payload?.active || typeof payload.screenX !== 'number') {
        setRemoteDrop(null);
        return;
      }
      setRemoteDrop(
        resolveMergeInsertion(
          document,
          payload.screenX - window.screenX,
          (payload.screenY ?? 0) - window.screenY
        )
      );
    });
  }, [desktop]);

  /**
   * TARGET-SIDE, the commit. Insert at the caret the preview was showing, then
   * acknowledge — and only then does the source drop its copy (D6a). A refusal
   * (`false`) leaves the tab where it was, which is the right answer for every
   * way this can fail: no strip under the point, a layout that changed between
   * the preview and the release, a window mid-reload.
   *
   * ═══════════════════════════════════════════════════════════════════════
   * THE DEADLINE IS THE OTHER HALF OF D6a, AND IT USED TO BE MISSING HERE.
   *
   * Main gives up on an unanswered merge after a couple of seconds and tells
   * the source to KEEP its tab. This window had no deadline at all — so a
   * window busy with a heavy streaming turn could process the request long
   * after that, insert the tab, and acknowledge into a request that no longer
   * existed. The ack was dropped, the source kept its tab, and the SAME
   * SESSION was then open in two windows.
   *
   * `expiresAt` is main's deadline already backed off by a grace period, so a
   * request accepted here still has time on main's clock for the ack to get
   * back. Past it, refusing is not a degraded outcome: the tab simply stays
   * where the user last saw it.
   * ═══════════════════════════════════════════════════════════════════════
   */
  useEffect(() => {
    if (!desktop?.on || !dispatch) return;
    return desktop.on('tab-drag:merge', (_event, ...args) => {
      const payload = args[0] as
        | {
            requestId: number;
            tab?: {
              sessionId: string;
              title: string;
              userSetName: boolean;
              cwd?: string;
              workflowId?: string;
            };
            screenX: number;
            screenY: number;
            expiresAt?: number;
          }
        | undefined;
      setRemoteDrop(null);
      if (!payload?.tab) return;
      if (typeof payload.expiresAt === 'number' && Date.now() >= payload.expiresAt) {
        // Refuse EXPLICITLY rather than staying silent: main is still holding
        // the source's commit open, and an answer now ends it immediately
        // instead of making the user watch out the rest of the timeout.
        desktop.tabDragAckMerge?.(payload.requestId, false);
        return;
      }
      const insertion = resolveMergeInsertion(
        document,
        payload.screenX - window.screenX,
        payload.screenY - window.screenY
      );
      if (!insertion) {
        desktop.tabDragAckMerge?.(payload.requestId, false);
        return;
      }
      dispatch({
        type: 'openTab',
        payload: {
          sessionId: payload.tab.sessionId,
          title: payload.tab.title,
          userSetName: payload.tab.userSetName,
          cwd: payload.tab.cwd,
          workflowId: payload.tab.workflowId,
          groupId: insertion.groupId,
          index: insertion.index,
        },
      });
      dispatch({ type: 'setActiveGroup', groupId: insertion.groupId });
      desktop.tabDragAckMerge?.(payload.requestId, true);
    });
  }, [desktop, dispatch]);

  // A gesture that ends any way at all must leave no caret in any window. The
  // hook reports `local` on Escape and on re-entry, but a pointerup outside goes
  // straight to the commit path — which clears main's preview itself — and an
  // unmount mid-drag reports nothing at all.
  useEffect(() => () => desktop?.tabDragEnd?.(), [desktop]);

  /**
   * Mirror the loaded session's real name onto its tab.
   *
   * The rename mirror in ChatGroupsContext only listens to
   * `announceSessionName`, and BaseChat only announces on an explicit RENAME —
   * nothing announces when a session merely LOADS. So a tab opened from the
   * sidebar or a deep link kept its creation-time placeholder and sat there
   * reading "New chat" while document.title showed the real name. Caught by
   * driving the app; jsdom could not have shown it, because it needs a session
   * to actually load.
   */
  const handleSessionLoaded = useCallback(
    (session: { id: string; name: string; userSetName: boolean; workingDir?: string } | null) => {
      if (!session?.id) return;
      // ⚠ Recorded BEFORE the name guard below, and separately from it.
      //
      // `ChatTab.cwd` had no writer, so `payloadFromTab` always omitted it and
      // a torn-off window fell back to `os.homedir()` — every new chat opened
      // there was created in `~` instead of the project. Folding this into the
      // rename dispatch would inherit its `!session.name` guard and lose the
      // directory for any session that has not been named yet, which is
      // exactly a freshly created one.
      if (session.workingDir) {
        dispatch?.({ type: 'setTabCwd', sessionId: session.id, cwd: session.workingDir });
      }
      if (!session.name) return;
      dispatch?.({
        type: 'renameTab',
        sessionId: session.id,
        title: session.name,
        userSetName: session.userSetName,
      });
    },
    [dispatch]
  );

  const activeGroupId = groups?.state.activeGroupId;
  const handleFocusGroup = useCallback(
    (groupId: ChatGroupId) => {
      if (groupId === activeGroupId) return;
      dispatch?.({ type: 'setActiveGroup', groupId });
    },
    [dispatch, activeGroupId]
  );

  const layout = groups?.state.layout;
  const groupCount = useMemo(() => (layout ? groupCountOf(layout) : 1), [layout]);

  /**
   * Rung 4 of the yield ladder (D-32): a split merges back to one group rather
   * than render two useless slivers.
   *
   * Modelled on AppLayout's sidebar watcher, down to the shape of the rule,
   * because it is the same effect and would fail the same way. Three things make
   * it safe to let it move a layout the user built by hand:
   *
   *   1. it fires ONLY on a width CROSSING. `state` is read through a ref and is
   *      NOT a dep — a layout change must never re-run this, or splitting a
   *      narrow window by hand would be undone inside the same tick as the drop.
   *      That is the exact bug that made the sidebar's un-collapse button dead.
   *   2. it is REVERSIBLE, and only reverses what WE did: the snapshot is what
   *      we owe the user, and it is taken at the moment we take the split away.
   *   3. the user can always overrule it. Split again while merged and the
   *      snapshot is forfeit (splitSnapshotIsStale) — their layout is theirs, and
   *      growing the window will not throw it away to restore ours.
   *
   * Observed on the shell's own box, not the window: the sidebar collapsing
   * changes the room available to the groups without the window moving at all.
   * No feedback loop is possible — merging changes the TREE INSIDE this box, and
   * the box is sized by SidebarInset above it, so the callback cannot retrigger
   * itself. Hence no hysteresis: none is needed, and adding it on spec would only
   * make the crossing harder to reason about.
   */
  const treeRef = useRef<HTMLDivElement | null>(null);
  const stateRef = useRef(groups?.state);
  stateRef.current = groups?.state;
  const mergeSnapshotRef = useRef<GroupLayoutSnapshot | null>(null);
  const lastShellWidthRef = useRef<number | null>(null);

  useEffect(() => {
    const tree = treeRef.current;
    if (!tree || !dispatch) return;

    const sample = () => {
      const state = stateRef.current;
      if (!state) return;
      const width = tree.clientWidth;
      // An unmeasured box is not a narrow one. Without this, a 0-width sample —
      // first paint, a hidden window, a display change — reads as "everything
      // fits" and hands the split back at zero pixels.
      if (!(width > 0)) return;
      const groupCountNow = groupCountOf(state.layout);

      // Did the user build their own layout while we had theirs merged away? Then
      // ours is forfeit — checked BEFORE the fit, so the rest of this sample
      // judges the layout the user actually has.
      if (mergeSnapshotRef.current && splitSnapshotIsStale({ groupCount: groupCountNow })) {
        mergeSnapshotRef.current = null;
      }

      const { wasFitting, isFitting } = splitYieldSample({
        layout: state.layout,
        snapshotLayout: mergeSnapshotRef.current?.layout ?? null,
        lastWidth: lastShellWidthRef.current,
        width,
      });
      const action = splitYieldAction({
        wasFitting,
        isFitting,
        groupCount: groupCountNow,
        autoMerged: mergeSnapshotRef.current !== null,
      });
      // Recorded before the dispatch, exactly as the sidebar records wasCompact:
      // the re-render that follows must not read a stale previous side.
      lastShellWidthRef.current = width;

      if (action === 'merge') {
        mergeSnapshotRef.current = snapshotGroupLayout(state);
        dispatch({ type: 'mergeAllGroups' });
      } else if (action === 'restore') {
        const snapshot = mergeSnapshotRef.current;
        mergeSnapshotRef.current = null;
        if (snapshot) dispatch({ type: 'restoreLayout', snapshot });
      }
    };

    sample();

    if (typeof ResizeObserver === 'undefined') {
      window.addEventListener('resize', sample);
      return () => window.removeEventListener('resize', sample);
    }
    const observer = new ResizeObserver(sample);
    observer.observe(tree);
    return () => observer.disconnect();
  }, [dispatch]);

  // Terminals are keyed by tab id and outlive the tab's BaseChat (which remounts
  // on every tab switch). When a tab is CLOSED for good, nothing else disposes
  // its hidden-but-live terminal, so the shell hands the dock the set of tab ids
  // that still exist and it drops the rest — unmounting them, which disposes
  // their pty. Idempotent: retain is a no-op when nothing needs dropping.
  const allTabIds = useMemo(() => {
    if (!groups) return [] as string[];
    return leafGroupIds(groups.state.layout).flatMap(
      (gid) => groups.state.groups[gid]?.tabs.map((t) => t.tabId) ?? []
    );
  }, [groups]);
  const retainTerminals = terminalDock?.retain;
  useEffect(() => {
    retainTerminals?.(allTabIds);
  }, [retainTerminals, allTabIds]);

  if (!groups || !layout) return null;

  const renderGroup = ({ groupId }: RenderGroupArgs) => {
    const group = groups.state.groups[groupId];
    if (!group) return <div key={groupId} />;
    const activeTab = group.tabs.find((t) => t.tabId === group.activeTabId);
    const isActiveGroup = groupId === groups.state.activeGroupId;
    // Every terminal key this group can own: one per tab, plus the empty-tab
    // key. The pane renders only its OWN terminals (a terminal belongs to one
    // pane), and shows the one whose tab is active in THIS pane — so a 4-way
    // split can have four terminals open at once, each scoped to its pane and
    // each the width of its pane, never a bar spanning the whole window.
    const groupTabKeys = [...group.tabs.map((t) => t.tabId), `${groupId}-empty`];

    const strip = (
      <ChatTabStrip
        tabs={group.tabs}
        activeTabId={group.activeTabId}
        groupId={groupId}
        groupActive={isActiveGroup}
        runningSessionIds={groups.runningSessionIds}
        tabAnnotations={groups.tabAnnotations}
        privacyTiers={privacyTiers}
        // The MERGE caret. It cannot come from `dragOverTabId` like the local
        // one does: while a cross-window drag is in flight this window receives
        // no pointer events at all, so its own drag state is empty and the caret
        // is driven entirely by IPC from main (design D3, Phase 0).
        remoteDropBeforeTabId={remoteDrop?.groupId === groupId ? remoteDrop.beforeTabId : null}
        onSelect={handleSelect}
        onClose={handleClose}
        onReorder={handleReorder}
        reserveTitlebar={reserveTitlebarControls && groupId === reservedGroupId}
        isCompactSidebarOverlayOpen={isCompactSidebarOverlayOpen}
        endSlot={
          <button
            type="button"
            aria-label="New chat"
            title="New chat"
            data-testid="chat-tab-new"
            onClick={() => handleNewTab(groupId)}
            className="br-tab-new ml-0.5 flex h-6 w-6 items-center justify-center rounded-md text-text-muted transition-colors hover:bg-background-medium hover:text-text-default"
          >
            <Plus className="h-4 w-4" />
          </button>
        }
      />
    );

    return (
      <ChatGroupPane
        key={groupId}
        groupId={groupId}
        isActiveGroup={isActiveGroup}
        groupCount={groupCount}
        dropZone={drag.dropTarget?.groupId === groupId ? drag.dropTarget.zone : null}
        onFocusGroup={handleFocusGroup}
        terminalDock={terminalDock}
        groupTabKeys={groupTabKeys}
        // The tab is the chat's identity: keying on tabId (not sessionId) keeps
        // a tab's mount stable across a session bind. The SAME id keys the tab's
        // terminal, so it too survives the bind and switching tabs switches it.
        tabKey={activeTab?.tabId ?? `${groupId}-empty`}
        // No tab yet means this pane is the placeholder, and the placeholder is
        // always replaced rather than filled: the key above changes when the
        // real tab lands, which unmounts this whole subtree. Drawing a greeting
        // it will throw away made a new window show one heading, drop it, and
        // unroll a different one.
        suppressGreeting={activeTab === undefined}
        sessionId={activeTab?.sessionId ?? ''}
        initialMessage={activeTab?.pendingInitialMessage}
        initialAttachments={activeTab?.pendingInitialAttachments}
        // Spend the route cargo exactly once. The reducer has always had
        // `consumePending`; nothing ever dispatched it, so a tab kept the
        // message that created its session forever and re-sent it on every
        // remount (BR duplicate-submission bug, 2026-07-18).
        onInitialMessageConsumed={
          activeTab
            ? () => dispatch?.({ type: 'consumePending', tabId: activeTab.tabId })
            : undefined
        }
        renderSessionTitle={() => strip}
        onChatChange={onChatChange}
        onSessionLoaded={handleSessionLoaded}
      />
    );
  };

  const tree = renderLayout(layout, [], renderGroup, (branch, path, children) => (
    <ChatGroupBranch
      key={`branch-${path.join('-')}`}
      branch={branch}
      path={path}
      onResize={(sizes) => dispatch?.({ type: 'resizeBranch', path, sizes })}
    >
      {children}
    </ChatGroupBranch>
  ));

  return (
    <ChatTabDragProvider value={drag}>
      <div className="flex h-full min-h-0 w-full flex-col">
        {/* treeRef: rung 4 measures the room the GROUPS have, which the sidebar
            can change without the window moving. */}
        {/* Terminals render INSIDE their pane now (see ChatGroupPane), not here
            at the shell as a full-width bar below every group. A terminal belongs
            to one pane, is the width of that pane, and a split can show several
            at once — each scoped to its own session's working directory. */}
        <div ref={treeRef} className="flex min-h-0 flex-1">
          {tree}
        </div>
      </div>
      {/* The ghost renders at the SHELL, not in the source strip: it is fixed to
          the viewport and must not be clipped by the strip's overflow-x:auto. */}
      {/* `detached` is D7's "this will become a window" state: the tilt goes to
          0, it takes the dashed accent outline, and it CLAMPS to this window's
          frame — Electron cannot paint outside it, so an unclamped ghost simply
          flies past the edge and is clipped, which reads as the tab having been
          eaten rather than as one about to become a window. */}
      {drag.ghost && (
        <ChatTabGhost ghost={drag.ghost} detached={drag.crossWindow.kind !== 'local'} />
      )}
    </ChatTabDragProvider>
  );
}

interface ChatGroupBranchProps {
  branch: Extract<GroupLayout, { kind: 'branch' }>;
  path: readonly number[];
  onResize: (sizes: number[]) => void;
  children: ReactElement[];
}

function ChatGroupBranch({ branch, onResize, children }: ChatGroupBranchProps) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  // Sizes are normalized on the branch by the reducer, but a persisted or
  // mid-migration tree can arrive with a sizes array that disagrees with the
  // children count. Falling back to an even split keeps a bad blob renderable
  // instead of collapsing panes to flexBasis: undefined.
  const sizes =
    branch.sizes.length === children.length
      ? branch.sizes
      : children.map(() => 1 / children.length);

  return (
    <div
      ref={containerRef}
      data-testid="chat-group-branch"
      data-dir={branch.dir}
      className={
        branch.dir === 'row'
          ? 'flex min-h-0 min-w-0 flex-1'
          : 'flex min-h-0 min-w-0 flex-1 flex-col'
      }
    >
      {children.map((child, index) => (
        <Fragment key={child.key ?? index}>
          {/* Splitter i lives between pane i and pane i+1. */}
          {index > 0 && (
            <ChatGroupSplitter
              dir={branch.dir}
              index={index - 1}
              sizes={sizes}
              containerRef={containerRef}
              onResize={onResize}
            />
          )}
          <div
            className="flex min-h-0 min-w-0"
            // `<size> 1 0` — grow PROPORTIONAL to the fraction, from a zero
            // basis. Not `0 0 <pct>%`: the splitters are flex siblings that
            // occupy real pixels, so percentages of the container would sum past
            // 100% and overflow. From a zero basis the panes divide exactly
            // whatever is left after the handles, in the right ratio.
            //
            // min-w-0 / min-h-0 is what makes the ratio hold: flex items default
            // to min-width:auto, so one long unbreakable transcript line would
            // otherwise push its pane past its share and silently rewrite the
            // split the user set.
            style={{ flex: `${sizes[index]} 1 0` }}
          >
            {child}
          </div>
        </Fragment>
      ))}
    </div>
  );
}

interface ChatGroupPaneProps {
  groupId: ChatGroupId;
  isActiveGroup: boolean;
  groupCount: number;
  dropZone: import('./dropZones').DropZone | null;
  onFocusGroup: (groupId: ChatGroupId) => void;
  terminalDock: ReturnType<typeof useTerminalDock>;
  /** Every terminal key this group can own (one per tab + the empty-tab key). */
  groupTabKeys: string[];
  tabKey: string;
  /** See `BaseChat`'s prop: the placeholder pane skips the rotating greeting. */
  suppressGreeting: boolean;
  sessionId: string;
  initialMessage?: string;
  initialAttachments?: import('../../types/message').UserAttachment[];
  onInitialMessageConsumed?: () => void;
  renderSessionTitle: () => ReactElement;
  onChatChange: (chat: ChatType) => void;
  onSessionLoaded: (
    session: { id: string; name: string; userSetName: boolean; workingDir?: string } | null
  ) => void;
}

/**
 * One group: the drop hit-test target, the focus target, and the host for its
 * active tab's chat.
 */
function ChatGroupPane({
  groupId,
  isActiveGroup,
  groupCount,
  dropZone,
  onFocusGroup,
  terminalDock,
  groupTabKeys,
  tabKey,
  sessionId,
  suppressGreeting,
  initialMessage,
  initialAttachments,
  onInitialMessageConsumed,
  renderSessionTitle,
  onChatChange,
  onSessionLoaded,
}: ChatGroupPaneProps) {
  // Extensions load per-session, so in a 4-way split four chats each finish
  // their own load and each has something to report. Telling the toast layer
  // which chat you are actually in lets a background group's clean load stay
  // silent instead of stacking over the transcript you are reading. Failures
  // still speak up from any group — see showExtensionLoadResults.
  useEffect(() => {
    if (isActiveGroup && sessionId) {
      setFocusedChatSession(sessionId);
    }
  }, [isActiveGroup, sessionId]);

  return (
    <div
      // The drag hit-test looks for exactly this attribute
      // (dropZones.dropTargetAtPoint). It must sit on the element whose
      // getBoundingClientRect IS the group's box, or the zones would be measured
      // against the wrong rectangle and the tint would land off the group.
      data-chat-group-id={groupId}
      data-active-group={isActiveGroup ? 'true' : 'false'}
      // flex COLUMN: the chat fills the pane and this pane's terminal (if any)
      // stacks below it, inside the pane — so the terminal is the pane's width,
      // not the window's. The box is still exactly the group's rectangle, which
      // is what the drag hit-test measures against.
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      // CAPTURE phase, both: clicking anywhere in a group focuses it, including
      // on controls that stopPropagation on the bubble (the composer's buttons,
      // the strip's close ×). Focus must not depend on WHERE in the pane you
      // clicked. focus-capture covers the keyboard path — tabbing into a pane
      // focuses its group without any pointer at all.
      onPointerDownCapture={() => onFocusGroup(groupId)}
      onFocusCapture={() => onFocusGroup(groupId)}
    >
      <div className="flex min-h-0 min-w-0 flex-1">
        <BaseChat
          key={tabKey}
          // Scopes this tab's terminal. Same id as the React key, so the terminal
          // is tied to the tab, not the (rebindable) session.
          terminalKey={tabKey}
          setChat={onChatChange}
          sessionId={sessionId}
          initialMessage={initialMessage}
          initialAttachments={initialAttachments}
          onInitialMessageConsumed={onInitialMessageConsumed}
          suppressEmptyState={false}
          suppressGreeting={suppressGreeting}
          renderSessionTitle={renderSessionTitle}
          onSessionUpdate={onSessionLoaded}
          // The preview panel follows the ACTIVE group. State is kept, only the
          // render is gated — see BaseChat's artifactPanelEnabled doc.
          artifactPanelEnabled={isActiveGroup}
          // A session-scoped chat must never resize the OS window once it is not
          // the only one: a background group opening an artifact would resize the
          // window out from under the group you are actually looking at.
          allowWindowResize={groupCount === 1}
        />
      </div>
      {/* This pane's OWN terminals, stacked below its chat inside the pane's
          flex column — so a terminal is the width of its pane and belongs to
          exactly one pane, never a full-window bar. Every terminal in the group
          stays mounted so its shell keeps running; only the one whose tab is
          active in THIS pane is visible (open). `onClose` HIDES a terminal (its
          panes live on); `onEmptied`, fired when its last pane closes, DESTROYS
          it. Each reads its own frozen cwd — its tab's session folder, captured
          on open. */}
      {terminalDock?.terminals
        .filter((terminal) => groupTabKeys.includes(terminal.key))
        .map((terminal) => (
          <InAppTerminalDock
            key={terminal.key}
            // Also its scope for "Run this code block": a Run clicked in this
            // tab's transcript must land in THIS terminal, not in whichever
            // pane happens to hold focus.
            dockKey={terminal.key}
            open={terminal.showing && terminal.key === tabKey}
            workingDir={terminal.workingDir}
            onClose={() => terminalDock.setOpen(terminal.key, false)}
            onEmptied={() => terminalDock.remove(terminal.key)}
          />
        ))}
      {dropZone && <ChatDropOverlay zone={dropZone} />}
    </div>
  );
}

export default ChatGroupsShell;
