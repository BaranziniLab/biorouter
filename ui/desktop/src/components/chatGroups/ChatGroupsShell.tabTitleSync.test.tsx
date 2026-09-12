import { render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../api';

/**
 * A tab that is NOT active must still learn its chat's real name.
 *
 * # The bug
 *
 * Found by driving the app on 2026-09-12. The daemon renames a chat after each
 * of its first few turns, with no signal (`SessionManager::maybe_update_name`
 * runs after the reply stream has closed). A tab title is the one name the
 * renderer PERSISTS, and only two things could ever correct one: the name
 * channel, and `handleSessionLoaded` — which comes from BaseChat, and only the
 * active tab mounts a BaseChat. So the sidebar read "Instruction-following
 * tests" while the same chat's background tab read "Penguin prompt test", on one
 * screen, unchanged across three reloads; making that tab active fixed it
 * instantly.
 *
 * `renameTab` already mirrored into every tab of that session — the reducer was
 * never the problem. What was missing is any path from the server's row to a tab
 * nobody is looking at. This pins that path: the shell reconciles its titles
 * against the session list it ALREADY reads and warms for the privacy dots.
 *
 * # Why it asserts dispatches, not DOM
 *
 * The strip reaches the DOM through BaseChat's `renderSessionTitle` render prop
 * and BaseChat is unmountable here (~10 providers), exactly as
 * `ChatGroupsShell.sessionName.test.tsx` and `.privacy.test.tsx` already
 * document. The dispatch IS the wiring that was absent.
 */

const dispatch = vi.fn();
let cachedList: Session[] | null = null;
let emit: (() => void) | null = null;

vi.mock('../BaseChat', () => ({
  default: () => <div data-testid="basechat" />,
}));

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: (listener: () => void) => {
    emit = listener;
    return () => {
      emit = null;
    };
  },
  preloadSessionList: () => {},
}));

vi.mock('../../hooks/chatStreamStore', () => ({
  useLiveSessionTiers: () => ({}),
}));

/**
 * `t-active` is the group's `activeTabId`; `t-background` and `t-mine` are not.
 * The background tab is the whole point — on `main` nothing could reach it.
 */
let tabs = [
  {
    tabId: 't-active',
    sessionId: 'sess-active',
    title: 'DELTA instruction test',
    userSetName: false,
  },
  { tabId: 't-background', sessionId: 'sess-bg', title: 'Penguin prompt test', userSetName: false },
];

vi.mock('../../contexts/ChatGroupsContext', () => ({
  useChatGroups: () => ({
    dispatch,
    state: {
      activeGroupId: 'g1',
      layout: { kind: 'leaf', groupId: 'g1' },
      groups: { g1: { id: 'g1', activeTabId: 't-active', tabs } },
    },
  }),
}));

vi.mock('../ui/sidebar', () => ({ useSidebar: () => ({ state: 'expanded', isMobile: false }) }));

import ChatGroupsShell from './ChatGroupsShell';

function row(id: string, name: string, userSetName = false): Session {
  return {
    id,
    name,
    user_set_name: userSetName,
    working_dir: '/tmp',
    message_count: 2,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
  } as unknown as Session;
}

function renameDispatches() {
  return dispatch.mock.calls.map((call) => call[0]).filter((a) => a?.type === 'renameTab');
}

describe('ChatGroupsShell — tab titles are reconciled against the session list', () => {
  beforeEach(() => {
    dispatch.mockClear();
    cachedList = null;
    emit = null;
    tabs = [
      {
        tabId: 't-active',
        sessionId: 'sess-active',
        title: 'DELTA instruction test',
        userSetName: false,
      },
      {
        tabId: 't-background',
        sessionId: 'sess-bg',
        title: 'Penguin prompt test',
        userSetName: false,
      },
    ];
  });

  it('renames a tab that is not the active one when the row says so', () => {
    cachedList = [
      row('sess-active', 'DELTA instruction test'),
      row('sess-bg', 'Instruction-following tests'),
    ];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  it('reconciles when the list arrives after the mount, not only on it', () => {
    // The cold-start order: the strip warms the cache, so the first read is
    // empty and the correction has to ride the subscription.
    cachedList = null;
    render(<ChatGroupsShell onChatChange={() => {}} />);
    expect(renameDispatches()).toEqual([]);

    cachedList = [row('sess-bg', 'Instruction-following tests')];
    emit?.();

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  /**
   * Guard 1 — the case the old code was protecting. A user rename sets
   * `userSetName` on the tab optimistically and `user_set_name` on the row, and
   * the daemon never auto-renames such a chat again. A user-named tab is skipped
   * outright, so no late row can snap it back and no keystroke can flicker.
   */
  it('never touches a tab the user named', () => {
    tabs = [{ tabId: 't-mine', sessionId: 'sess-bg', title: 'Penguin notes', userSetName: true }];
    cachedList = [row('sess-bg', 'Instruction-following tests')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([]);
  });

  /** Guard 2 — the placeholder is a lower bound on a name, never a correction. */
  it('never downgrades a named tab to the "New chat" placeholder', () => {
    cachedList = [row('sess-bg', 'New chat')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([]);
  });

  /** …but a tab still ON the placeholder does adopt the row's real name. */
  it('names a placeholder tab from the row', () => {
    tabs = [{ tabId: 't-new', sessionId: 'sess-bg', title: 'New chat', userSetName: false }];
    cachedList = [row('sess-bg', 'Instruction-following tests')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([
      {
        type: 'renameTab',
        sessionId: 'sess-bg',
        title: 'Instruction-following tests',
        userSetName: false,
      },
    ]);
  });

  /** A row that agrees with the tab must not dispatch — the strip re-renders on
   *  every streamed token, and a dispatch per emit would be a render loop. */
  it('dispatches nothing when every title already agrees', () => {
    cachedList = [
      row('sess-active', 'DELTA instruction test'),
      row('sess-bg', 'Penguin prompt test'),
    ];

    render(<ChatGroupsShell onChatChange={() => {}} />);
    emit?.();

    expect(renameDispatches()).toEqual([]);
  });

  /** A chat the list does not carry keeps what it has: silence, not a guess. */
  it('leaves a tab alone when the list has no row for it', () => {
    cachedList = [row('sess-active', 'DELTA instruction test')];

    render(<ChatGroupsShell onChatChange={() => {}} />);

    expect(renameDispatches()).toEqual([]);
  });
});
