import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from 'react';
import { flushSync } from 'react-dom';
import type { SessionClassification } from '../../api';
import { useRunningChats } from '../../hooks/chatStreamStore';
import { ChatState } from '../../types/chatState';
import { composerDraftKeyForTab, existingChatComposerDraftKey } from '../../utils/composerDrafts';
import type { ChatKindSource } from '../chats/chatKind';
import { dictationActivityFor, subscribeDictation } from '../dictation/dictationSession';
import {
  compactBandControls,
  compactPaneDensity,
  type CompactPaneDensity,
  type PaneShortfall,
} from '../Layout/yieldLadder';
import { TabOverflowMenu } from './ChatTabStrip';
import type { ChatGroupId, ChatTab, ChatTabId, GroupLayout } from './chatGroupsTypes';
import { CompactPaneView, type CompactPaneState } from './CompactPaneView';
import { compactPaneCopy } from './copy';
import { growPanePlan, type PaneResize } from './paneGrowPlan';

/** What a pane's chat is, for its placeholder: the same inputs its tab uses. */
export interface PaneIdentity {
  title: string;
  kindSource: ChatKindSource;
  /** Absent means not yet known; never drawn as Public. */
  tier?: SessionClassification | null;
  sessionId: string;
}

/** Everything the shell hands a pane so it can draw and act as a compact pane. */
export interface CompactPaneInputs {
  identity: PaneIdentity;
  /** For the band's ▾: the same inputs the strip's overflow menu takes. */
  tabMenu: {
    tabs: ChatTab[];
    activeTabId: ChatTabId | null;
    runningSessionIds: readonly string[];
    onSelectTab: (tabId: ChatTabId) => void;
    renderGlyph: (tab: ChatTab) => React.ReactNode;
    titleOf: (tab: ChatTab) => string;
  };
  layout: GroupLayout;
  treeRef: React.RefObject<HTMLDivElement | null>;
  /** 172 for the first leaf while reserveTitlebarControls is true, else 0. */
  bandInsetStart: number;
  onGrow: (plan: PaneResize[]) => void;
  onMerge: () => void;
}

export interface CompactPaneProps extends CompactPaneInputs {
  groupId: ChatGroupId;
  tabKey: string;
  isActiveGroup: boolean;
  shortfall: PaneShortfall;
  animateIn: boolean;
  /**
   * The pane's box when it turned compact, so the first render already has the
   * right density and the frame's focus lands on the action that stays.
   */
  initialBox?: { width: number; height: number };
}

interface Fit {
  density: CompactPaneDensity;
  bandAction: boolean;
  bandMenu: boolean;
  canGrow: boolean;
}

const sameFit = (a: Fit, b: Fit) =>
  a.density === b.density &&
  a.bandAction === b.bandAction &&
  a.bandMenu === b.bandMenu &&
  a.canGrow === b.canGrow;

/**
 * Where the action is drawn: in the body (card, glyph), in the band, or nowhere
 * (band density with under 28px of band left). With no action the placeholder
 * root itself is what holds focus.
 */
const placementOf = (fit: Fit) =>
  fit.density !== 'band' ? 'body' : fit.bandAction ? 'band' : 'none';

const ACTION_SELECTOR = '[data-compact-pane-action]';

/**
 * The compact pane's container (rung 5): it measures its own box for the
 * density and what the band can hold, plans Make wider against the tree, reads
 * the chat's run and dictation state, and keeps the one live announcement.
 *
 * Mounted only while the pane is compact, so its re-renders on resize cost the
 * hidden chat nothing. A change of density is committed with `flushSync`, so a
 * drag never shows a frame of the wrong layout.
 */
export function CompactPane({
  identity,
  tabMenu,
  layout,
  treeRef,
  bandInsetStart,
  onGrow,
  onMerge,
  groupId,
  tabKey,
  isActiveGroup,
  shortfall,
  animateIn,
  initialBox,
}: CompactPaneProps) {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const tabCount = tabMenu.tabs.length;

  const measure = useCallback(
    (paneBox?: { width: number; height: number }): Fit => {
      const box = paneBox ?? rootRef.current?.getBoundingClientRect() ?? { width: 0, height: 0 };
      const density = compactPaneDensity(box.width, box.height);
      const band = compactBandControls({
        width: box.width,
        insetStart: bandInsetStart,
        density,
        tabCount,
      });
      const tree = treeRef.current?.getBoundingClientRect();
      const canGrow =
        !!tree &&
        growPanePlan({
          layout,
          groupId,
          tree: { width: tree.width, height: tree.height },
          shortfall,
        }) !== null;
      return { density, bandAction: band.action, bandMenu: band.menu, canGrow };
    },
    [bandInsetStart, tabCount, treeRef, layout, groupId, shortfall]
  );

  const [fit, setFit] = useState<Fit>(() => measure(initialBox));
  const fitRef = useRef(fit);
  fitRef.current = fit;
  /**
   * Set when focus was in the placeholder (on the action, the ▾, or the root
   * standing in for a missing action) as a change of fit moved the action
   * between the body, the band and nowhere, or took the ▾ away.
   */
  const carryFocusRef = useRef(false);

  const apply = useCallback((next: Fit, commit: 'sync' | 'layout') => {
    const current = fitRef.current;
    if (sameFit(current, next)) return;
    const root = rootRef.current;
    const active = document.activeElement;
    if (
      (placementOf(current) !== placementOf(next) || current.bandMenu !== next.bandMenu) &&
      root &&
      active instanceof HTMLElement &&
      root.contains(active)
    ) {
      carryFocusRef.current = true;
    }
    fitRef.current = next;
    if (commit === 'sync') flushSync(() => setFit(next));
    else setFit(next);
  }, []);

  // Measured once on mount and whenever what the answer depends on changes
  // (the layout, the shortfall, the tab count, the titlebar reserve)...
  useLayoutEffect(() => {
    apply(measure(), 'layout');
  }, [apply, measure]);

  // ...and on every resize of its own box.
  useLayoutEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    if (typeof ResizeObserver === 'undefined') {
      const onResize = () => apply(measure(), 'sync');
      window.addEventListener('resize', onResize);
      return () => window.removeEventListener('resize', onResize);
    }
    const observer = new ResizeObserver(() => apply(measure(), 'sync'));
    observer.observe(root);
    return () => observer.disconnect();
  }, [apply, measure]);

  // A change of fit unmounted the control that held focus (the action moved
  // between the body, the band and nowhere, or the ▾ left the band), or the
  // root was standing in for an action that now exists: carry focus to the
  // action, or to the root when there is none. A control that is still there
  // keeps it, and focus that left the pane is left alone.
  const placement = placementOf(fit);
  const bandMenu = fit.bandMenu;
  useLayoutEffect(() => {
    if (!carryFocusRef.current) return;
    carryFocusRef.current = false;
    const root = rootRef.current;
    const active = document.activeElement;
    if (!root || (active && active !== document.body && active !== root)) return;
    (root.querySelector<HTMLElement>(ACTION_SELECTOR) ?? root).focus({ preventScroll: true });
  }, [placement, bandMenu]);

  // The run state, from the registry the strip's dot and the sidebar read.
  const running = useRunningChats();
  const entry = identity.sessionId
    ? running.find((candidate) => candidate.sessionId === identity.sessionId)
    : undefined;
  const live = !!entry && !entry.completedAt;
  const waiting = live && entry?.chatState === ChatState.WaitingForUserInput;

  // A hidden microphone is never silent. The key is the composer's own, the
  // expression BaseChat mounts it with (pinned by styles/paneFloor.test.ts).
  const dictationKey = identity.sessionId
    ? existingChatComposerDraftKey(tabKey, identity.sessionId)
    : composerDraftKeyForTab(tabKey);
  const dictation = useSyncExternalStore(
    subscribeDictation,
    () => dictationActivityFor(dictationKey),
    () => dictationActivityFor(dictationKey)
  );
  const listening = dictation === 'arming' || dictation === 'recording';

  // The one slot shows the most urgent fact.
  const state: CompactPaneState = listening
    ? 'listening'
    : waiting
      ? 'waiting'
      : live
        ? 'running'
        : 'idle';

  // One polite announcement, on the way INTO waiting while this pane is
  // mounted; silent on mount and for every other change, so several running
  // panes never talk at once and a pending approval in a hidden pane is never
  // silent.
  const [announcement, setAnnouncement] = useState('');
  const wasWaitingRef = useRef(waiting);
  useEffect(() => {
    if (waiting && !wasWaitingRef.current) {
      setAnnouncement(compactPaneCopy.announceWaiting(identity.title));
    } else if (!waiting) {
      setAnnouncement('');
    }
    wasWaitingRef.current = waiting;
  }, [waiting, identity.title]);

  const run = () => {
    const tree = treeRef.current?.getBoundingClientRect();
    if (fit.canGrow) {
      // Planned again from fresh rects: the layout may have moved since the label.
      const plan = tree
        ? growPanePlan({
            layout,
            groupId,
            tree: { width: tree.width, height: tree.height },
            shortfall,
          })
        : null;
      if (plan) {
        onGrow(plan);
        return;
      }
      // The label said grow; a press never merges on it. Say what a press would do now.
      apply({ ...fitRef.current, canGrow: false }, 'layout');
      return;
    }
    onMerge();
  };

  const label = fit.canGrow ? compactPaneCopy.grow[shortfall] : compactPaneCopy.merge;

  return (
    <CompactPaneView
      ref={rootRef}
      title={identity.title}
      kindSource={identity.kindSource}
      tier={identity.tier}
      isActiveGroup={isActiveGroup}
      state={state}
      live={live}
      shortfall={shortfall}
      density={fit.density}
      band={{
        insetStart: bandInsetStart,
        action: fit.bandAction,
        menu: fit.bandMenu ? <TabOverflowMenu {...tabMenu} /> : null,
      }}
      action={{ kind: fit.canGrow ? 'grow' : 'merge', label, run }}
      announcement={announcement}
      animateIn={animateIn}
    />
  );
}
