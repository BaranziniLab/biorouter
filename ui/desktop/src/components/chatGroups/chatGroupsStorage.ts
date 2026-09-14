import {
  ChatGroupsState,
  ChatGroup,
  GroupLayout,
  firstLeaf,
  leafGroupIds,
} from './chatGroupsTypes';
import { createInitialChatGroupsState } from './chatGroupsReducer';
import { removeLeaf } from './chatGroupsLayout';

export const CHAT_GROUPS_KEY_PREFIX = 'biorouter.chatgroups.v1';
const WINDOW_ID_KEY = 'biorouter.chatgroups.windowId';

/**
 * R3: two Electron windows share an origin and would clobber ONE localStorage
 * key. createChatWindow (main.ts:~894) spawns a real second renderer, so this
 * is not hypothetical. The now-removed dashboard mode shipped exactly this bug
 * in `biorouter.dashboard.v2`, unnoticed only because nobody opened two
 * dashboards at once. Tabs are used daily, so scope the key per window.
 *
 * The id is minted into sessionStorage rather than plumbed through appConfig:
 * sessionStorage is per-BrowserWindow by construction (each renderer gets its
 * own) and survives a reload of that window, which is exactly the scope the key
 * needs — and it costs zero main-process/IPC surface. Accepted residue, as the
 * plan allows: one stale key per crashed window.
 */
export function getWindowId(): string {
  try {
    const existing = sessionStorage.getItem(WINDOW_ID_KEY);
    if (existing) return existing;
    const minted = `w${Date.now().toString(36)}${Math.random().toString(36).slice(2, 8)}`;
    sessionStorage.setItem(WINDOW_ID_KEY, minted);
    return minted;
  } catch {
    // Storage unavailable (jsdom edge, privacy mode): fall back to a per-load
    // id. Persistence is lost, correctness is not.
    return 'w-ephemeral';
  }
}

export function chatGroupsStorageKey(windowId: string): string {
  return `${CHAT_GROUPS_KEY_PREFIX}:${windowId}`;
}

/** Route-state cargo must NEVER round-trip. A queued message that re-sends
 *  after a reload is a data bug, not a UX wrinkle. */
function stripTransient(group: ChatGroup): ChatGroup {
  return {
    ...group,
    tabs: group.tabs.map((tab) => {
      const { pendingInitialMessage: _m, pendingInitialAttachments: _a, ...persisted } = tab;
      void _m;
      void _a;
      return persisted;
    }),
  };
}

export function saveChatGroups(state: ChatGroupsState, windowId: string): void {
  try {
    const groups: Record<string, ChatGroup> = {};
    for (const [id, group] of Object.entries(state.groups)) {
      groups[id] = stripTransient(group);
    }
    localStorage.setItem(chatGroupsStorageKey(windowId), JSON.stringify({ ...state, groups }));
  } catch {
    // A full or unavailable quota must never take the chat down.
  }
}

/**
 * Validate the layout TREE, not just "layout is an object".
 *
 * Stage 2 only ever persisted a depth-0 leaf, so a shallow check was honest
 * then. A persisted branch is a recursive structure that arrives from
 * localStorage — i.e. from a previous app version, a hand-edited blob, or a
 * half-written record — and every one of those can produce a branch with no
 * children, which firstLeaf() throws on. A load path that throws white-screens
 * /pair with no way back except clearing storage by hand.
 */
function isValidLayout(value: unknown): value is GroupLayout {
  if (!value || typeof value !== 'object') return false;
  const node = value as Partial<GroupLayout>;
  if (node.kind === 'leaf') return typeof (node as { groupId?: unknown }).groupId === 'string';
  if (node.kind !== 'branch') return false;
  const branch = node as Extract<GroupLayout, { kind: 'branch' }>;
  if (branch.dir !== 'row' && branch.dir !== 'col') return false;
  if (!Array.isArray(branch.children) || branch.children.length === 0) return false;
  if (!Array.isArray(branch.sizes)) return false;
  return branch.children.every(isValidLayout);
}

function isValid(value: unknown): value is ChatGroupsState {
  if (!value || typeof value !== 'object') return false;
  const state = value as Partial<ChatGroupsState>;
  if (state.version !== 1) return false;
  if (!isValidLayout(state.layout)) return false;
  if (!state.groups || typeof state.groups !== 'object') return false;
  if (typeof state.activeGroupId !== 'string') return false;
  if (typeof state.seq !== 'number') return false;
  if (!state.groups[state.activeGroupId]) return false;
  return Object.values(state.groups).every((g) => Array.isArray(g?.tabs));
}

export interface LoadChatGroupsOptions {
  /**
   * Keep this tab although it has no chat. Asked only about tabs with
   * `sessionId: ''`.
   *
   * The provider answers "it holds an unsent message" (`utils/composerDrafts.ts`).
   * That store is renderer memory, so the answer is yes only when the provider
   * is being re-mounted inside the same renderer — the user went to Settings or
   * Home and came back — and no after a reload, which therefore prunes exactly
   * as it always did.
   */
  keepSessionlessTab?: (tabId: string) => boolean;
}

/** Returns null on ANY shape mismatch -> the caller takes the cold-boot path. */
export function loadChatGroups(
  windowId: string,
  options: LoadChatGroupsOptions = {}
): ChatGroupsState | null {
  try {
    const raw = localStorage.getItem(chatGroupsStorageKey(windowId));
    if (!raw) return null;
    const parsed: unknown = JSON.parse(raw);
    if (!isValid(parsed)) return null;

    const groups: Record<string, ChatGroup> = {};
    for (const [id, group] of Object.entries(parsed.groups)) {
      // A tab with sessionId '' never resolved its createSession. Blank, it
      // cannot be restored into anything meaningful, so it is dropped rather
      // than rendered as a permanently blank tab.
      //
      // ⚠ Unless it holds a message the person has not sent. Leaving /pair
      // unmounts this provider and coming back re-reads it from here, so this
      // prune ran on every trip to Settings — measured on 1.90.4: a new tab
      // whose failed start had just said "Your message was kept." was gone
      // from the strip, and the message with it, while that toast was still on
      // screen.
      const tabs = group.tabs.filter(
        (tab) =>
          typeof tab?.sessionId === 'string' &&
          (tab.sessionId !== '' || (options.keepSessionlessTab?.(tab.tabId) ?? false))
      );
      const activeTabId = tabs.some((t) => t.tabId === group.activeTabId)
        ? group.activeTabId
        : (tabs[0]?.tabId ?? null);
      groups[id] = { ...group, tabs, activeTabId };
    }
    return reconcile({ ...parsed, groups });
  } catch {
    return null;
  }
}

/**
 * Make a restored SPLIT consistent with the groups that actually survived load.
 *
 * The tab prune above can empty a group (every one of its tabs was an unbound
 * `sessionId: ''`). At depth 0 that is fine and intended — one empty group
 * renders BaseChat's empty state. In a split it is not: the layout would still
 * hold a leaf for it and the user would reopen the app to a dead half-pane with
 * no tabs and no way to fill it, next to their real chat.
 *
 * Symmetrically, a leaf naming a group that is not in `groups` at all (a
 * truncated or hand-edited blob) would render nothing at all, and a group not
 * named by any leaf is unreachable state that persists forever.
 *
 * Returns null — the cold-boot path — only if nothing survives.
 */
function reconcile(state: ChatGroupsState): ChatGroupsState | null {
  let layout: GroupLayout | null = state.layout;

  const dropIf = (predicate: (groupId: string) => boolean) => {
    for (const groupId of leafGroupIds(layout as GroupLayout)) {
      if (!predicate(groupId)) continue;
      // Never remove the last leaf: one empty group is a valid, intended state.
      if (leafGroupIds(layout as GroupLayout).length <= 1) break;
      layout = removeLeaf(layout as GroupLayout, groupId);
      if (!layout) return;
    }
  };

  dropIf((groupId) => !state.groups[groupId]);
  if (!layout) return null;
  dropIf((groupId) => (state.groups[groupId]?.tabs.length ?? 0) === 0);
  if (!layout) return null;

  const liveLeaves = leafGroupIds(layout);
  // A single surviving leaf whose group is missing entirely is unrecoverable.
  if (!state.groups[liveLeaves[0]]) return null;

  // Drop groups no leaf names — otherwise they accumulate in storage forever.
  const groups: Record<string, ChatGroup> = {};
  for (const groupId of liveLeaves) groups[groupId] = state.groups[groupId];

  const activeGroupId = groups[state.activeGroupId] ? state.activeGroupId : firstLeaf(layout);
  return { ...state, layout, groups, activeGroupId };
}

export function loadChatGroupsOrInitial(
  windowId: string,
  options: LoadChatGroupsOptions = {}
): ChatGroupsState {
  return loadChatGroups(windowId, options) ?? createInitialChatGroupsState();
}

/** Prune keys belonging to windows that are gone. Best-effort; called on
 *  beforeunload for this window's own key only when it holds nothing worth
 *  keeping. */
export function clearChatGroups(windowId: string): void {
  try {
    localStorage.removeItem(chatGroupsStorageKey(windowId));
  } catch {
    // ignore
  }
}
