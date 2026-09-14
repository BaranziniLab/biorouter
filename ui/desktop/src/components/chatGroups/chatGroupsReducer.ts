import { UserAttachment } from '../../types/message';
import {
  ChatGroupsState,
  ChatGroup,
  ChatTab,
  ChatTabId,
  ChatGroupId,
  GroupLayout,
  firstLeaf,
  leafGroupIds,
} from './chatGroupsTypes';
import {
  MAX_GROUPS,
  removeLeaf,
  setSizesAtPath,
  splitLeaf,
  groupCountOf,
} from './chatGroupsLayout';
import { DropZone } from './dropZones';

export interface OpenTabPayload {
  sessionId: string;
  title?: string;
  userSetName?: boolean;
  pendingInitialMessage?: string;
  pendingInitialAttachments?: UserAttachment[];
  workflowId?: string;
  cwd?: string;
  /** Target group; defaults to activeGroupId. */
  groupId?: ChatGroupId;
  /**
   * Where in the target group's strip the tab goes. Omitted — every caller but
   * one — appends, which is what "open a tab" has always meant.
   *
   * The one caller that supplies it is a cross-window MERGE (tab tear-off,
   * design §5): the user aimed at a position between two tabs and the caret said
   * so, so landing the tab at the end instead would contradict the preview they
   * were just shown. Clamped into range rather than validated, because the index
   * arrives from another window's measurement of this one and a stale strip is a
   * normal event, not an error.
   *
   * Only the APPEND branch honours it. Dedupe (the session is already open here)
   * and adopt (the pre-session submit filling a blank tab) both land on an
   * existing tab, which already has a position the user can see; moving it would
   * be a reorder nobody asked for.
   */
  index?: number;
  /**
   * For a NEW-chat request (`sessionId: ''`) that is ARRIVING on /pair from
   * another route — the sidebar's New chat or Cmd+T pressed on Settings or Home,
   * or history landing on such an entry: when a tab already holds a new chat,
   * focus it instead of opening another blank one.
   *
   * On arrival every tab with no chat is one holding an unsent message, because
   * the load pruned the rest (`chatGroupsStorage.loadChatGroups`). That is the
   * case this exists for: a failed start says "Your message was kept." and, for
   * a credential, sends the person to Settings — and the ways back to /pair are
   * exactly these requests. Opening a blank tab beside the kept one put an empty
   * composer in front of them, next to a tab they had no reason to look for.
   *
   * NOT for a request made while /pair is already showing: pressing New chat
   * while looking at your tabs means another tab, as it always has.
   *
   * In a split, it never changes what any pane but the focused one shows: a tab
   * already on screen only takes focus, and a tab behind another pane's chat
   * stays there. Which tab, and why: `findUnsentTab`.
   */
  resumeUnsent?: boolean;
  /**
   * For a pre-session submit: the tab its message was typed in. The chat binds
   * to THAT tab or to a tab of its own — never to another blank tab.
   *
   * ⚠ The adopt branch used to take "the active blank tab, else any blank tab",
   * and a blank tab is not the same as an empty one once a tab can hold an
   * unsent message. Measured in the dev app: a start held in flight from tab X,
   * the person switched to tab Y holding "MY OWN UNSENT DRAFT" and an image, and
   * the chat that came back bound to Y, deleting its draft and image and leaving
   * X blank. And a message sent from Home after a failed start bound to the tab
   * the failure had just said "Your message was kept." about.
   */
  originTabId?: ChatTabId;
  /**
   * Which chat-less tabs are not blank (`utils/composerDrafts.unsentComposerTabs`),
   * sampled by the dispatcher because the reducer is pure. `drafted` tabs hold a
   * message the person has not sent and are never adopted, not even as the
   * origin — the chat gets its own tab and the draft stays where it was typed.
   * `sending` tabs have a message in flight and are adopted only by their own
   * chat, as the origin.
   */
  unsentTabs?: { drafted: readonly ChatTabId[]; sending: readonly ChatTabId[] };
}

export type ChatGroupsAction =
  | { type: 'openTab'; payload: OpenTabPayload }
  | { type: 'activateTab'; tabId: ChatTabId }
  | { type: 'closeTab'; tabId: ChatTabId }
  | { type: 'reorderTab'; draggedTabId: ChatTabId; targetTabId: ChatTabId }
  | { type: 'renameTab'; sessionId: string; title: string; userSetName?: boolean }
  | { type: 'setTabCwd'; sessionId: string; cwd: string }
  | { type: 'bindSession'; tabId: ChatTabId; sessionId: string }
  | { type: 'consumePending'; tabId: ChatTabId }
  | { type: 'setActiveGroup'; groupId: ChatGroupId }
  /**
   * The drop. `zone: 'center'` MOVES the tab into targetGroupId; an edge SPLITS
   * targetGroupId and moves the tab into the new half. One action for both,
   * because they are one gesture — the user aims and lets go, and the zone under
   * the cursor at that moment is the whole difference.
   */
  | { type: 'moveTabToGroup'; tabId: ChatTabId; targetGroupId: ChatGroupId; zone: DropZone }
  | { type: 'resizeBranch'; path: readonly number[]; sizes: readonly number[] }
  /**
   * Rung 4 of the yield ladder (D-32): the window got too narrow to give every
   * group a chat, so the split merges back to one rather than render two useless
   * slivers. Dispatched ONLY by the shell's width watcher, and only on a width
   * CROSSING — see yieldLadder.splitYieldAction.
   */
  | { type: 'mergeAllGroups' }
  /** …and the crossing back up, which gives the user their layout back. */
  | { type: 'restoreLayout'; snapshot: GroupLayoutSnapshot };

export const DEFAULT_TAB_TITLE = 'New chat';

/**
 * What rung 4 has to hand back when the window grows again.
 *
 * Deliberately NOT a copy of the state. It records the SHAPE the user built —
 * the tree, and which tabs lived where — and nothing about the tabs themselves,
 * because by the time it is restored the tab set has moved on: chats were
 * closed, new ones opened, sessions bound. Storing whole ChatTab objects would
 * resurrect a closed chat and stale a renamed one. The restore re-homes whatever
 * still exists and lets the rest go.
 *
 * In-memory only, held by the shell for the life of a merge. It is never
 * persisted: a split that survives a quit-and-relaunch by way of a rule the user
 * cannot see would be a layout arriving from nowhere.
 */
export interface GroupLayoutSnapshot {
  layout: GroupLayout;
  /** groupId → the tabs it held, in strip order. */
  tabIdsByGroup: Record<ChatGroupId, ChatTabId[]>;
  activeTabIdByGroup: Record<ChatGroupId, ChatTabId | null>;
  activeGroupId: ChatGroupId;
}

export function snapshotGroupLayout(state: ChatGroupsState): GroupLayoutSnapshot {
  const tabIdsByGroup: Record<ChatGroupId, ChatTabId[]> = {};
  const activeTabIdByGroup: Record<ChatGroupId, ChatTabId | null> = {};
  for (const groupId of leafGroupIds(state.layout)) {
    const group = state.groups[groupId];
    if (!group) continue;
    tabIdsByGroup[groupId] = group.tabs.map((t) => t.tabId);
    activeTabIdByGroup[groupId] = group.activeTabId;
  }
  return {
    layout: state.layout,
    tabIdsByGroup,
    activeTabIdByGroup,
    activeGroupId: state.activeGroupId,
  };
}

/**
 * Rung 4, the shrink half: every group's tabs into ONE group.
 *
 * The ACTIVE group survives, so the chat you are reading is still the chat you
 * are reading — the window got narrower, it did not change the subject. The tabs
 * arrive in LEAF ORDER (the tree walk, left-to-right / top-to-bottom), so the
 * merged strip reads the way the layout you just lost looked.
 *
 * Exported for its tests: this is a state transition, not geometry, and it is
 * provable without a browser.
 */
export function mergeAllGroups(state: ChatGroupsState): ChatGroupsState {
  const leaves = leafGroupIds(state.layout);
  if (leaves.length <= 1) return state;

  const survivorId = state.groups[state.activeGroupId]
    ? state.activeGroupId
    : firstLeaf(state.layout);
  const survivor = state.groups[survivorId];
  if (!survivor) return state;

  const tabs = leaves.flatMap((groupId) => state.groups[groupId]?.tabs ?? []);
  return {
    ...state,
    layout: { kind: 'leaf', groupId: survivorId },
    groups: {
      [survivorId]: {
        groupId: survivorId,
        tabs,
        // The tab you were looking at stays the tab you are looking at.
        activeTabId: survivor.activeTabId ?? tabs[0]?.tabId ?? null,
      },
    },
    activeGroupId: survivorId,
  };
}

/**
 * Rung 4, the grow half: give the layout back.
 *
 * Reconciles rather than replays. Between the merge and now the user has kept
 * working in the one group they had, so:
 *
 *   - a tab named by the snapshot that no longer exists is simply dropped;
 *   - a tab that was opened WHILE merged is named by no group, and goes to the
 *     group the user is actually in — a new chat must not be teleported into a
 *     background pane the moment the window widens;
 *   - a leaf that ends up with nothing is collapsed out of the tree rather than
 *     restored as an empty pane;
 *   - focus follows the tab the user is on, wherever it lands.
 *
 * Exported for its tests.
 */
export function restoreGroupLayout(
  state: ChatGroupsState,
  snapshot: GroupLayoutSnapshot
): ChatGroupsState {
  const living = new Map<ChatTabId, ChatTab>();
  for (const group of Object.values(state.groups)) {
    for (const tab of group.tabs) living.set(tab.tabId, tab);
  }
  const focusedTabId = state.groups[state.activeGroupId]?.activeTabId ?? null;

  const snapshotLeaves = leafGroupIds(snapshot.layout);
  if (snapshotLeaves.length === 0) return state;

  const claimed = new Set<ChatTabId>();
  const groups: Record<ChatGroupId, ChatGroup> = {};
  for (const groupId of snapshotLeaves) {
    const tabs = (snapshot.tabIdsByGroup[groupId] ?? [])
      .map((tabId) => living.get(tabId))
      .filter((tab): tab is ChatTab => Boolean(tab));
    tabs.forEach((tab) => claimed.add(tab.tabId));
    groups[groupId] = { groupId, tabs, activeTabId: null };
  }

  // Tabs opened while merged belong to the group the user has been working in —
  // which, after a merge, IS the survivor and is therefore a leaf of the
  // snapshot. Falling back keeps a hand-edited state renderable rather than
  // dropping tabs on the floor.
  const homeGroupId = groups[state.activeGroupId]
    ? state.activeGroupId
    : groups[snapshot.activeGroupId]
      ? snapshot.activeGroupId
      : snapshotLeaves[0];
  for (const group of Object.values(state.groups)) {
    for (const tab of group.tabs) {
      if (claimed.has(tab.tabId)) continue;
      claimed.add(tab.tabId);
      groups[homeGroupId] = { ...groups[homeGroupId], tabs: [...groups[homeGroupId].tabs, tab] };
    }
  }

  for (const groupId of snapshotLeaves) {
    const group = groups[groupId];
    const remembered = snapshot.activeTabIdByGroup[groupId];
    const active =
      remembered && group.tabs.some((t) => t.tabId === remembered)
        ? remembered
        : (group.tabs[0]?.tabId ?? null);
    groups[groupId] = { ...group, activeTabId: active };
  }

  // Drop leaves that lost everything. A restored empty pane is a pane the user
  // has to close by hand for a split they never asked to get back.
  let layout: GroupLayout | null = snapshot.layout;
  for (const groupId of snapshotLeaves) {
    if (groups[groupId].tabs.length > 0) continue;
    if (!layout) break;
    layout = removeLeaf(layout, groupId);
    delete groups[groupId];
  }
  if (!layout || Object.keys(groups).length === 0) return state;

  // The tab the user is on keeps focus, and its group is the focused group. The
  // window widening must not move the cursor.
  const focusedGroupId =
    (focusedTabId &&
      leafGroupIds(layout).find((groupId) =>
        groups[groupId]?.tabs.some((t) => t.tabId === focusedTabId)
      )) ||
    (groups[snapshot.activeGroupId] ? snapshot.activeGroupId : firstLeaf(layout));
  if (focusedTabId && groups[focusedGroupId]?.tabs.some((t) => t.tabId === focusedTabId)) {
    groups[focusedGroupId] = { ...groups[focusedGroupId], activeTabId: focusedTabId };
  }

  return { ...state, layout, groups, activeGroupId: focusedGroupId };
}

export function createInitialChatGroupsState(): ChatGroupsState {
  const groupId = 'grp-1';
  return {
    version: 1,
    layout: { kind: 'leaf', groupId },
    groups: { [groupId]: { groupId, tabs: [], activeTabId: null } },
    activeGroupId: groupId,
    seq: 1,
  };
}

function findTabGroup(
  state: ChatGroupsState,
  predicate: (tab: ChatTab) => boolean
): { group: ChatGroup; tab: ChatTab } | null {
  for (const group of Object.values(state.groups)) {
    const tab = group.tabs.find(predicate);
    if (tab) return { group, tab };
  }
  return null;
}

/** BR-71: locate a session's tab anywhere in the layout. Public wrapper over
 * findTabGroup for the workspace command planner and tab annotations. */
export function findTabBySession(
  state: ChatGroupsState,
  sessionId: string
): { tabId: ChatTabId; groupId: ChatGroupId } | null {
  const hit = findTabGroup(state, (tab) => tab.sessionId === sessionId);
  return hit ? { tabId: hit.tab.tabId, groupId: hit.group.groupId } : null;
}

function withGroup(state: ChatGroupsState, groupId: ChatGroupId, next: ChatGroup): ChatGroupsState {
  return { ...state, groups: { ...state.groups, [groupId]: next } };
}

/**
 * The tab with no chat that `resumeUnsent` brings the person to, or null for the
 * blank tab New chat has always opened.
 *
 * New chat acts on ONE pane, the focused one: its visible tab gives way to the
 * tab the request lands on, and no other pane's does. Resuming keeps to that, so
 * in order:
 *
 *   1. A tab with no chat that a pane is ALREADY SHOWING — the focused pane's,
 *      then any pane's in layout order. Only focus moves; every pane keeps what
 *      it shows.
 *   2. A tab with no chat BEHIND the focused pane's visible tab. That visible tab
 *      goes behind a tab either way; the one in front is the kept message rather
 *      than a blank tab beside it. This is the single-pane case.
 *   3. Nothing. A tab with no chat behind ANOTHER pane's visible tab stays where
 *      it is, in that pane's strip: bringing it forward would put it in front of
 *      a chat in a pane New chat does not act on.
 *
 * ⚠ The first version searched the focused pane's whole strip before looking at
 * any other pane, then every pane's strip. Measured in the dev app: the left
 * pane showing a draft, the right pane showing a chat with a second draft behind
 * it; Settings, then New chat, and the right pane's chat was replaced by that
 * background draft. The focused pane on arrival is not even reliably the one the
 * person left: each remounting composer focuses itself, and a focus in a pane
 * makes it the focused one (`ChatGroupsShell`'s `onFocusGroup`).
 */
function findUnsentTab(state: ChatGroupsState): { group: ChatGroup; tab: ChatTab } | null {
  for (const groupId of [state.activeGroupId, ...leafGroupIds(state.layout)]) {
    const group = state.groups[groupId];
    const shown = group?.tabs.find((t) => t.tabId === group.activeTabId);
    if (group && shown && !shown.sessionId) return { group, tab: shown };
  }
  const focused = state.groups[state.activeGroupId];
  const behind = focused?.tabs.find((t) => !t.sessionId);
  return focused && behind ? { group: focused, tab: behind } : null;
}

/**
 * The existing tab a pre-session submit's chat fills, or null for a tab of its
 * own.
 *
 * WITH AN ORIGIN (a new tab's composer started it): that tab, wherever it is
 * now, if it still has no chat and holds no unsent draft. Nothing else — a tab
 * the person closed, or one they have typed a new message into while the start
 * was in flight, is not the chat's home, and neither is any other blank tab.
 *
 * WITHOUT ONE (Home, a launcher deep link, a diverge): the ACTIVE blank tab
 * first, then any blank tab in the target group — where "blank" now excludes a
 * tab holding a draft or a message in flight. "The blank tab" and "the leftmost
 * blank tab" were the same tab back when reaching two blanks took real effort;
 * Cmd+T makes two blanks a keystroke away. The fallback survives because a
 * launcher message arrives from outside the strip entirely, and filling a
 * waiting blank is the right home for it.
 */
function adoptionTarget(
  state: ChatGroupsState,
  group: ChatGroup,
  payload: OpenTabPayload
): { group: ChatGroup; tab: ChatTab } | null {
  const drafted = new Set(payload.unsentTabs?.drafted ?? []);
  const sending = new Set(payload.unsentTabs?.sending ?? []);
  if (payload.originTabId) {
    const hit = findTabGroup(state, (t) => t.tabId === payload.originTabId);
    if (!hit || hit.tab.sessionId || drafted.has(hit.tab.tabId)) return null;
    return hit;
  }
  const blank = (t: ChatTab) => !t.sessionId && !drafted.has(t.tabId) && !sending.has(t.tabId);
  const tab =
    group.tabs.find((t) => t.tabId === group.activeTabId && blank(t)) ?? group.tabs.find(blank);
  return tab ? { group, tab } : null;
}

/**
 * Open a chat as a tab.
 *
 * Every open is a REAL tab. There is no preview/italic slot and nothing is ever
 * recycled out from under the user: this reducer used to implement VS Code's
 * enablePreview (single click = italic tab, reused in place), and the user
 * rejected it after living with it — clicking a chat in Recents must leave the
 * chat you were reading exactly where it was. Two rules, and only two:
 *
 *   DEDUPE  — a sessionId already open ANYWHERE activates that tab (and focuses
 *             its group) instead of opening a second one. "New chats that are
 *             not already launched as a tab will always launch as a tab."
 *   ADOPT   — the pre-session submit path, and it alone, fills the empty tab the
 *             user is already looking at. See below.
 */
function openTab(state: ChatGroupsState, action: ChatGroupsAction & { type: 'openTab' }) {
  const { payload } = action;

  // Dedupe: a sessionId already open ANYWHERE activates its tab and its group
  // rather than duplicating. Generalizes the artifactSourceKey dedupe in
  // ArtifactViewer, so the two surfaces cannot drift.
  if (payload.sessionId) {
    const hit = findTabGroup(state, (tab) => tab.sessionId === payload.sessionId);
    if (hit) {
      return {
        ...withGroup(state, hit.group.groupId, {
          ...hit.group,
          activeTabId: hit.tab.tabId,
        }),
        activeGroupId: hit.group.groupId,
      };
    }
  }

  // A started chat whose origin tab still exists but cannot take it (the person
  // typed a new message there while the start was in flight) opens BESIDE that
  // tab, in its pane: the origin's surface is still mid-start, and becoming the
  // inactive tab of its own pane is what rebuilds it fresh.
  const isPreSessionSubmit =
    payload.pendingInitialMessage !== undefined || payload.pendingInitialAttachments !== undefined;
  const origin =
    payload.originTabId && isPreSessionSubmit
      ? findTabGroup(state, (t) => t.tabId === payload.originTabId)
      : null;
  const groupId = payload.groupId ?? origin?.group.groupId ?? state.activeGroupId;
  const group = state.groups[groupId];
  if (!group) return state;

  const nextTab = (tabId: ChatTabId): ChatTab => ({
    tabId,
    sessionId: payload.sessionId,
    title: payload.title ?? DEFAULT_TAB_TITLE,
    userSetName: payload.userSetName ?? false,
    pendingInitialMessage: payload.pendingInitialMessage,
    pendingInitialAttachments: payload.pendingInitialAttachments,
    workflowId: payload.workflowId,
    cwd: payload.cwd,
  });

  // ADOPT — the ONLY path that may fill an existing tab, and it is not an "open"
  // in the user's sense at all.
  //
  // An empty tab (sessionId '') is a tab the user opened and has not yet bound to
  // a session. When BaseChat's pre-session submit finally creates one and
  // navigates, that arrives here as an openTab — and it must fill the empty tab
  // in place, keeping its tabId, rather than opening a second tab beside it and
  // orphaning the blank one the user is staring at. WHICH empty tab is
  // `adoptionTarget`'s decision: the one the message was typed in, and never a
  // tab holding a message the person has not sent.
  //
  // Gated on the route-state cargo (`pendingInitial*`) because that cargo is what
  // makes this the submit path: only a submit carries the message that created
  // the session. A Recents click carries none, so it can never land here and can
  // never consume a blank tab — it always opens its own. That gate is the whole
  // reason this branch is safe to keep; without it "open in a new tab" would
  // silently become "replace the blank tab" and we would be back to the
  // behaviour the user rejected.
  if (payload.sessionId && isPreSessionSubmit) {
    const target = adoptionTarget(state, group, payload);
    if (target) {
      return {
        ...withGroup(state, target.group.groupId, {
          ...target.group,
          tabs: target.group.tabs.map((t) =>
            t.tabId === target.tab.tabId ? nextTab(target.tab.tabId) : t
          ),
          activeTabId: target.tab.tabId,
        }),
        activeGroupId: target.group.groupId,
      };
    }
  }

  if (!payload.sessionId && payload.resumeUnsent) {
    const unsent = findUnsentTab(state);
    if (unsent) {
      return {
        ...withGroup(state, unsent.group.groupId, {
          ...unsent.group,
          activeTabId: unsent.tab.tabId,
        }),
        activeGroupId: unsent.group.groupId,
      };
    }
  }

  const tabId = `tab-${state.seq + 1}`;
  // Append unless a position was asked for. `index` is CLAMPED, not rejected:
  // it is measured in another window against a strip that may have changed
  // between the measurement and this commit, and the honest response to "insert
  // before a tab that has since closed" is "insert at the end", not "drop the
  // tab on the floor". See OpenTabPayload.index.
  const tabs = [...group.tabs];
  const at =
    payload.index === undefined || !Number.isFinite(payload.index)
      ? tabs.length
      : Math.min(Math.max(Math.trunc(payload.index), 0), tabs.length);
  tabs.splice(at, 0, nextTab(tabId));
  return {
    ...withGroup(state, groupId, {
      ...group,
      tabs,
      activeTabId: tabId,
    }),
    activeGroupId: groupId,
    seq: state.seq + 1,
  };
}

/**
 * Drop a group that has just lost its last tab — but ONLY when it is not the
 * last group.
 *
 * The single-group case is a deliberate exception: closing the last tab of the
 * last group leaves an EMPTY group — the reducer itself never navigates and
 * never deletes the session. That is the Stage-2 invariant and it must survive
 * the split unchanged, which is why this is gated on the group COUNT rather
 * than on "is there a branch". (What the ROUTE does with that empty layout is
 * a separate, later decision: /pair now redirects Home when the whole layout
 * is empty — see useEmptyPairRedirect and the closeTab comment below.)
 */
function collapseEmptyGroup(state: ChatGroupsState, groupId: ChatGroupId): ChatGroupsState {
  const group = state.groups[groupId];
  if (!group || group.tabs.length > 0) return state;
  if (groupCountOf(state.layout) <= 1) return state;

  const layout = removeLeaf(state.layout, groupId);
  // removeLeaf returning null means we just removed the only leaf, which the
  // count guard above already excluded. Refuse rather than produce a null tree.
  if (!layout) return state;

  const groups = { ...state.groups };
  delete groups[groupId];

  // activeGroupId must ALWAYS name a live leaf. If the group that just died was
  // the active one, focus falls to the first surviving leaf.
  const activeGroupId = groups[state.activeGroupId] ? state.activeGroupId : firstLeaf(layout);
  return { ...state, layout, groups, activeGroupId };
}

function closeTab(state: ChatGroupsState, tabId: ChatTabId): ChatGroupsState {
  const hit = findTabGroup(state, (t) => t.tabId === tabId);
  if (!hit) return state;
  const { group } = hit;

  const closingIndex = group.tabs.findIndex((t) => t.tabId === tabId);
  const tabs = group.tabs.filter((t) => t.tabId !== tabId);

  // Successor = Math.min(closingIndex, remaining.length - 1) — identical to
  // ArtifactViewer's, so the two tab surfaces cannot drift.
  if (group.activeTabId !== tabId) {
    return collapseEmptyGroup(withGroup(state, group.groupId, { ...group, tabs }), group.groupId);
  }
  const successor = tabs[Math.min(closingIndex, tabs.length - 1)] ?? null;
  // Closing the last tab of the last group leaves an EMPTY group, and it does
  // NOT delete the session — there is no createdHere here, by construction.
  // The REDUCER still never navigates; but the empty pane is no longer the
  // resting state: /pair itself redirects Home (the Hub) when the whole layout
  // has zero tabs and nothing is en route to becoming one (issue #38 —
  // useEmptyPairRedirect, mounted in App.tsx's PairRouteContent, with gates
  // for deep links, new-chat, workflows and pending Cmd+T).
  //
  // In a SPLIT, closing the last tab of a non-last group collapses that group
  // out of the tree instead: an empty half of a split is a dead pane the user
  // has to close twice — and the survivor keeps the layout non-empty, so the
  // redirect never fires for a collapsed half.
  return collapseEmptyGroup(
    withGroup(state, group.groupId, { ...group, tabs, activeTabId: successor?.tabId ?? null }),
    group.groupId
  );
}

function moveTabToGroup(
  state: ChatGroupsState,
  action: ChatGroupsAction & { type: 'moveTabToGroup' }
): ChatGroupsState {
  const hit = findTabGroup(state, (t) => t.tabId === action.tabId);
  if (!hit) return state;
  const source = hit.group;
  const target = state.groups[action.targetGroupId];
  if (!target) return state;

  // Narrowed once, here, rather than testing `zone !== 'center'` at each use:
  // splitLeaf's parameter EXCLUDES 'center' (there is no such thing as a centre
  // split), and a boolean flag does not carry that proof to the call site.
  const splitZone = action.zone === 'center' ? null : action.zone;
  const isSplit = splitZone !== null;

  // Dropping a tab into the centre of its own group is a no-op, not a move — the
  // reorder gesture owns that case. And splitting a group off a tab that is that
  // group's ONLY tab would create a fresh group and leave an empty one behind,
  // i.e. a lot of motion to arrive back where you started.
  if (source.groupId === action.targetGroupId) {
    if (!isSplit) return state;
    if (source.tabs.length <= 1) return state;
  }

  if (isSplit && groupCountOf(state.layout) >= MAX_GROUPS) return state;

  const remaining = source.tabs.filter((t) => t.tabId !== action.tabId);
  const closingIndex = source.tabs.findIndex((t) => t.tabId === action.tabId);
  const sourceActiveTabId =
    source.activeTabId === action.tabId
      ? (remaining[Math.min(closingIndex, remaining.length - 1)]?.tabId ?? null)
      : source.activeTabId;

  let next: ChatGroupsState = {
    ...state,
    groups: {
      ...state.groups,
      [source.groupId]: { ...source, tabs: remaining, activeTabId: sourceActiveTabId },
    },
  };

  let landingGroupId = action.targetGroupId;

  if (splitZone) {
    landingGroupId = `grp-${state.seq + 1}`;
    next = {
      ...next,
      seq: state.seq + 1,
      layout: splitLeaf(next.layout, action.targetGroupId, landingGroupId, splitZone),
      groups: {
        ...next.groups,
        [landingGroupId]: { groupId: landingGroupId, tabs: [], activeTabId: null },
      },
    };
  }

  // Re-read the landing group from `next`: when the target IS the source (a
  // split off one's own group) the source's tab list has already been rewritten
  // above, and appending to the stale `target` would resurrect the moved tab.
  const landing = next.groups[landingGroupId];
  next = {
    ...next,
    groups: {
      ...next.groups,
      [landingGroupId]: {
        ...landing,
        tabs: [...landing.tabs, hit.tab],
        activeTabId: hit.tab.tabId,
      },
    },
    // The group you dropped into is the one you are now looking at.
    activeGroupId: landingGroupId,
  };

  return collapseEmptyGroup(next, source.groupId);
}

export function chatGroupsReducer(
  state: ChatGroupsState,
  action: ChatGroupsAction
): ChatGroupsState {
  switch (action.type) {
    case 'openTab':
      return openTab(state, action);

    case 'activateTab': {
      const hit = findTabGroup(state, (t) => t.tabId === action.tabId);
      if (!hit) return state;
      if (hit.group.activeTabId === action.tabId && state.activeGroupId === hit.group.groupId) {
        return state;
      }
      return {
        ...withGroup(state, hit.group.groupId, { ...hit.group, activeTabId: action.tabId }),
        activeGroupId: hit.group.groupId,
      };
    }

    case 'closeTab':
      return closeTab(state, action.tabId);

    case 'reorderTab': {
      const hit = findTabGroup(state, (t) => t.tabId === action.draggedTabId);
      if (!hit) return state;
      const { group } = hit;
      const draggedIndex = group.tabs.findIndex((t) => t.tabId === action.draggedTabId);
      const targetIndex = group.tabs.findIndex((t) => t.tabId === action.targetTabId);
      if (draggedIndex < 0 || targetIndex < 0 || draggedIndex === targetIndex) return state;
      const tabs = [...group.tabs];
      const [dragged] = tabs.splice(draggedIndex, 1);
      tabs.splice(targetIndex, 0, dragged);
      return withGroup(state, group.groupId, { ...group, tabs });
    }

    case 'setTabCwd': {
      // Record the session's working directory on every tab bound to it.
      //
      // ⚠ **`ChatTab.cwd` had no writer at all**, which is why this exists.
      // The field, `payloadFromTab`'s handling of it and `main.ts`'s
      // `req.tab?.cwd` were all in place and all correct — but nothing ever
      // SET it, so `payloadFromTab` always omitted it and a torn-off window
      // fell through to `os.homedir()`. The torn-off chat itself was fine
      // (`resumeSessionId` travels and the daemon re-anchors per session), so
      // the damage was one step removed and easy to miss: every NEW chat
      // opened in that window was created in `~` instead of the project the
      // user tore off from.
      //
      // Written from the loaded SESSION rather than at the `openTab` call
      // sites, deliberately. There are seven of those and the bug is exactly
      // what happens when one of them forgets; keying on the session means a
      // tab gets its directory however it was opened, and picks up a later
      // `update_working_dir` for free.
      let changed = false;
      const groups: Record<ChatGroupId, ChatGroup> = {};
      for (const [groupId, group] of Object.entries(state.groups)) {
        let groupChanged = false;
        const tabs = group.tabs.map((t) => {
          if (t.sessionId !== action.sessionId || t.cwd === action.cwd) return t;
          groupChanged = true;
          return { ...t, cwd: action.cwd };
        });
        changed = changed || groupChanged;
        groups[groupId] = groupChanged ? { ...group, tabs } : group;
      }
      return changed ? { ...state, groups } : state;
    }

    case 'renameTab': {
      // Mirrors a session rename into every tab bound to that session.
      let changed = false;
      const groups: Record<ChatGroupId, ChatGroup> = {};
      for (const [groupId, group] of Object.entries(state.groups)) {
        const tabs = group.tabs.map((t) => {
          if (t.sessionId !== action.sessionId || t.title === action.title) return t;
          changed = true;
          return { ...t, title: action.title, userSetName: action.userSetName ?? t.userSetName };
        });
        groups[groupId] = changed ? { ...group, tabs } : group;
      }
      return changed ? { ...state, groups } : state;
    }

    case 'bindSession': {
      const hit = findTabGroup(state, (t) => t.tabId === action.tabId);
      if (!hit) return state;
      return withGroup(state, hit.group.groupId, {
        ...hit.group,
        tabs: hit.group.tabs.map((t) =>
          t.tabId === action.tabId ? { ...t, sessionId: action.sessionId } : t
        ),
      });
    }

    case 'consumePending': {
      // Route-state cargo is consumed exactly once, by BaseChat on mount.
      const hit = findTabGroup(state, (t) => t.tabId === action.tabId);
      if (!hit) return state;
      if (!hit.tab.pendingInitialMessage && !hit.tab.pendingInitialAttachments) return state;
      return withGroup(state, hit.group.groupId, {
        ...hit.group,
        tabs: hit.group.tabs.map((t) =>
          t.tabId === action.tabId
            ? { ...t, pendingInitialMessage: undefined, pendingInitialAttachments: undefined }
            : t
        ),
      });
    }

    case 'setActiveGroup':
      // activeGroupId must ALWAYS name a live leaf.
      if (!state.groups[action.groupId]) return state;
      if (state.activeGroupId === action.groupId) return state;
      return { ...state, activeGroupId: action.groupId };

    case 'moveTabToGroup':
      return moveTabToGroup(state, action);

    case 'resizeBranch': {
      const layout = setSizesAtPath(state.layout, action.path, action.sizes);
      return layout === state.layout ? state : { ...state, layout };
    }

    case 'mergeAllGroups':
      return mergeAllGroups(state);

    case 'restoreLayout':
      return restoreGroupLayout(state, action.snapshot);

    default:
      return state;
  }
}

export function activeGroupOf(state: ChatGroupsState): ChatGroup | undefined {
  return state.groups[state.activeGroupId];
}

export function activeTabOf(state: ChatGroupsState): ChatTab | undefined {
  const group = activeGroupOf(state);
  if (!group || !group.activeTabId) return undefined;
  return group.tabs.find((t) => t.tabId === group.activeTabId);
}

/** The focused session id, or '' when the active group is empty. */
export function activeSessionIdOf(state: ChatGroupsState): string {
  return activeTabOf(state)?.sessionId ?? '';
}
