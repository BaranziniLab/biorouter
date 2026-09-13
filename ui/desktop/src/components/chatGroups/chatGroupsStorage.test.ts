import { describe, it, expect, beforeEach } from 'vitest';
import {
  saveChatGroups,
  loadChatGroups,
  loadChatGroupsOrInitial,
  chatGroupsStorageKey,
  getWindowId,
} from './chatGroupsStorage';
import { createInitialChatGroupsState, chatGroupsReducer } from './chatGroupsReducer';
import { ChatGroupsState, leafGroupIds } from './chatGroupsTypes';

const leafCount = (state: ChatGroupsState) => leafGroupIds(state.layout).length;

function seeded(): ChatGroupsState {
  return chatGroupsReducer(createInitialChatGroupsState(), {
    type: 'openTab',
    payload: {
      sessionId: 's1',
      title: 'Cohort',
      pendingInitialMessage: 'do not re-send me',
      pendingInitialAttachments: [],
    },
  });
}

beforeEach(() => {
  localStorage.clear();
  sessionStorage.clear();
});

describe('chatGroupsStorage', () => {
  it('round-trips a state', () => {
    const state = seeded();
    saveChatGroups(state, 'w1');
    const loaded = loadChatGroups('w1');
    expect(loaded?.groups['grp-1'].tabs[0].sessionId).toBe('s1');
    expect(loaded?.groups['grp-1'].tabs[0].title).toBe('Cohort');
    expect(loaded?.activeGroupId).toBe('grp-1');
  });

  it('NEVER persists pendingInitialMessage / pendingInitialAttachments', () => {
    saveChatGroups(seeded(), 'w1');

    // Assert on the raw serialized bytes, not just the parsed object: a queued
    // message that survives a reload re-sends, which is a data bug.
    const raw = localStorage.getItem(chatGroupsStorageKey('w1'))!;
    expect(raw).not.toContain('do not re-send me');
    expect(raw).not.toContain('pendingInitialMessage');
    expect(raw).not.toContain('pendingInitialAttachments');

    const loaded = loadChatGroups('w1')!;
    expect(loaded.groups['grp-1'].tabs[0].pendingInitialMessage).toBeUndefined();
    expect(loaded.groups['grp-1'].tabs[0].pendingInitialAttachments).toBeUndefined();
  });

  it("prunes sessionId:'' tabs on load and re-points activeTabId", () => {
    const state = chatGroupsReducer(seeded(), { type: 'openTab', payload: { sessionId: '' } });
    expect(state.groups['grp-1'].activeTabId).toBe(state.groups['grp-1'].tabs[1].tabId);
    saveChatGroups(state, 'w1');

    const loaded = loadChatGroups('w1')!;
    expect(loaded.groups['grp-1'].tabs).toHaveLength(1);
    expect(loaded.groups['grp-1'].tabs[0].sessionId).toBe('s1');
    // The pruned tab was active — focus must fall to a tab that still exists.
    expect(loaded.groups['grp-1'].activeTabId).toBe(loaded.groups['grp-1'].tabs[0].tabId);
  });

  it('keeps a tab with no chat only when it holds an unsent message', () => {
    // Coming back to /pair re-reads the layout from here, so this prune ran on
    // every trip to Settings: measured on 1.90.4, a new tab whose failed start
    // had said "Your message was kept." was gone from the strip on return.
    const withBlank = chatGroupsReducer(seeded(), { type: 'openTab', payload: { sessionId: '' } });
    const withTwoBlanks = chatGroupsReducer(withBlank, {
      type: 'openTab',
      payload: { sessionId: '' },
    });
    const [, unsent, blank] = withTwoBlanks.groups['grp-1'].tabs;
    // The unsent one was the tab in view when the person left.
    const leftOn = chatGroupsReducer(withTwoBlanks, { type: 'activateTab', tabId: unsent.tabId });
    saveChatGroups(leftOn, 'w1');

    const loaded = loadChatGroups('w1', { keepSessionlessTab: (tabId) => tabId === unsent.tabId })!;
    const tabs = loaded.groups['grp-1'].tabs.map((t) => t.tabId);
    expect(tabs).toContain(unsent.tabId);
    expect(tabs).not.toContain(blank.tabId);
    // And it is still the tab in view.
    expect(loaded.groups['grp-1'].activeTabId).toBe(unsent.tabId);

    // A reload is a new renderer: nothing holds a draft, so the prune is as it was.
    expect(loadChatGroups('w1')!.groups['grp-1'].tabs.map((t) => t.tabId)).toEqual([
      withTwoBlanks.groups['grp-1'].tabs[0].tabId,
    ]);
  });

  it('keeps a split pane whose only tab holds an unsent message', () => {
    const state = chatGroupsReducer(seeded(), { type: 'openTab', payload: { sessionId: '' } });
    const unsentTab = state.groups['grp-1'].tabs[1].tabId;
    const split = chatGroupsReducer(state, {
      type: 'moveTabToGroup',
      tabId: unsentTab,
      targetGroupId: 'grp-1',
      zone: 'right',
    });
    expect(leafCount(split)).toBe(2);
    saveChatGroups(split, 'w1');

    const loaded = loadChatGroups('w1', { keepSessionlessTab: (tabId) => tabId === unsentTab })!;
    expect(leafCount(loaded)).toBe(2);
    expect(
      Object.values(loaded.groups).some((g) => g.tabs.some((t) => t.tabId === unsentTab))
    ).toBe(true);
  });

  it('returns null on garbage, wrong version, and a dangling activeGroupId', () => {
    localStorage.setItem(chatGroupsStorageKey('w1'), 'not json {{{');
    expect(loadChatGroups('w1')).toBeNull();

    localStorage.setItem(chatGroupsStorageKey('w1'), JSON.stringify({ ...seeded(), version: 99 }));
    expect(loadChatGroups('w1')).toBeNull();

    localStorage.setItem(
      chatGroupsStorageKey('w1'),
      JSON.stringify({ ...seeded(), activeGroupId: 'ghost' })
    );
    expect(loadChatGroups('w1')).toBeNull();

    expect(loadChatGroups('never-written')).toBeNull();
  });

  it('falls back to a cold-boot state rather than throwing', () => {
    localStorage.setItem(chatGroupsStorageKey('w1'), '{]');
    const state = loadChatGroupsOrInitial('w1');
    expect(state.groups['grp-1'].tabs).toEqual([]);
    expect(state.activeGroupId).toBe('grp-1');
  });

  it('R3: two windowIds produce two DISJOINT keys and never clobber', () => {
    const a = seeded();
    const b = chatGroupsReducer(createInitialChatGroupsState(), {
      type: 'openTab',
      payload: { sessionId: 'other-window-session' },
    });

    saveChatGroups(a, 'w1');
    saveChatGroups(b, 'w2');

    expect(chatGroupsStorageKey('w1')).not.toBe(chatGroupsStorageKey('w2'));
    expect(loadChatGroups('w1')!.groups['grp-1'].tabs[0].sessionId).toBe('s1');
    expect(loadChatGroups('w2')!.groups['grp-1'].tabs[0].sessionId).toBe('other-window-session');
  });

  it('getWindowId is stable within a window and survives reload', () => {
    const first = getWindowId();
    expect(getWindowId()).toBe(first);
    // A reload keeps sessionStorage, so the window keeps its layout.
    expect(getWindowId()).toBe(first);
    // A different window = a fresh sessionStorage = a different id.
    sessionStorage.clear();
    expect(getWindowId()).not.toBe(first);
  });
});

describe('chatGroupsStorage — a persisted SPLIT round-trips and is reconciled', () => {
  const KEY = () => chatGroupsStorageKey(getWindowId());

  /** grp-1[s1] | grp-2[s2] */
  function splitState(): ChatGroupsState {
    const two = chatGroupsReducer(
      chatGroupsReducer(createInitialChatGroupsState(), {
        type: 'openTab',
        payload: { sessionId: 's1', title: 's1' },
      }),
      { type: 'openTab', payload: { sessionId: 's2', title: 's2' } }
    );
    return chatGroupsReducer(two, {
      type: 'moveTabToGroup',
      tabId: two.groups['grp-1'].tabs[1].tabId,
      targetGroupId: 'grp-1',
      zone: 'right',
    });
  }

  it('round-trips a branch layout', () => {
    const state = splitState();
    saveChatGroups(state, getWindowId());
    const loaded = loadChatGroups(getWindowId());
    expect(loaded?.layout).toEqual(state.layout);
    expect(Object.keys(loaded!.groups).sort()).toEqual(Object.keys(state.groups).sort());
    expect(loaded?.activeGroupId).toBe(state.activeGroupId);
  });

  it('drops a leaf whose group emptied on load, instead of restoring a dead pane', () => {
    // Every tab in grp-2 was an unbound sessionId:'' tab, so the tab prune
    // empties it. At depth 0 an empty group is fine (BaseChat's empty state); in
    // a SPLIT it would be a half-pane with no tabs and no way to fill it.
    const state = splitState();
    const [, secondId] = Object.keys(state.groups);
    const wounded = {
      ...state,
      groups: {
        ...state.groups,
        [secondId]: {
          ...state.groups[secondId],
          tabs: state.groups[secondId].tabs.map((t) => ({ ...t, sessionId: '' })),
        },
      },
    };
    localStorage.setItem(KEY(), JSON.stringify(wounded));

    const loaded = loadChatGroups(getWindowId());
    expect(loaded?.layout).toEqual({ kind: 'leaf', groupId: 'grp-1' });
    expect(loaded?.groups[secondId]).toBeUndefined();
    expect(loaded?.activeGroupId).toBe('grp-1');
  });

  it('drops a leaf naming a group that is not in `groups` at all', () => {
    const state = splitState();
    const groups = { ...state.groups };
    const [, secondId] = Object.keys(groups);
    delete groups[secondId];
    localStorage.setItem(KEY(), JSON.stringify({ ...state, groups, activeGroupId: 'grp-1' }));

    const loaded = loadChatGroups(getWindowId());
    expect(loaded?.layout).toEqual({ kind: 'leaf', groupId: 'grp-1' });
    expect(loaded?.groups['grp-1']).toBeDefined();
  });

  it('rejects a branch with no children rather than white-screening on firstLeaf', () => {
    const state = splitState();
    localStorage.setItem(
      KEY(),
      JSON.stringify({ ...state, layout: { kind: 'branch', dir: 'row', children: [], sizes: [] } })
    );
    expect(loadChatGroups(getWindowId())).toBeNull();
    // ...and the caller cold-boots rather than throwing.
    expect(loadChatGroupsOrInitial(getWindowId()).layout).toEqual({
      kind: 'leaf',
      groupId: 'grp-1',
    });
  });

  it('rejects a branch with a bogus direction', () => {
    const state = splitState();
    localStorage.setItem(
      KEY(),
      JSON.stringify({
        ...state,
        layout: {
          kind: 'branch',
          dir: 'diagonal',
          children: [{ kind: 'leaf', groupId: 'grp-1' }],
          sizes: [1],
        },
      })
    );
    expect(loadChatGroups(getWindowId())).toBeNull();
  });
});
