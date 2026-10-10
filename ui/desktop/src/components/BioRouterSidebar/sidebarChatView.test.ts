import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';

import type { SessionSummary } from '../../api';
import {
  applyHeldArrangement,
  arrangeSidebarChats,
  clampSidebarChatView,
  DEFAULT_SIDEBAR_CHAT_VIEW,
  folderLabels,
  holdArrangement,
  parseCollapsedFolders,
  readSidebarChatView,
  SIDEBAR_CHAT_VIEW_STORAGE_KEY,
  SIDEBAR_COLLAPSED_FOLDERS_MAX,
  SIDEBAR_GROUP_BY_OPTIONS,
  SIDEBAR_SORT_BY_OPTIONS,
  writeSidebarChatView,
  type SidebarChatView,
} from './sidebarChatView';
import { useSidebarChatView } from './useSidebarChatView';

const NOW = new Date(2026, 9, 9, 12, 0, 0).getTime();
const day = (back: number, hour = 12, minute = 0) =>
  new Date(2026, 9, 9 - back, hour, minute, 0).toISOString();

function chat(id: string, name: string, overrides: Partial<SessionSummary> = {}): SessionSummary {
  return {
    id,
    name,
    created_at: day(10),
    updated_at: day(0),
    working_dir: '/Users/wgu/project',
    message_count: 1,
    user_set_name: false,
    ...overrides,
  } as SessionSummary;
}

// Five chats in three folders, with distinct activity and creation orders and
// one default name, so each of the nine combinations has one right answer.
const CHATS: SessionSummary[] = [
  chat('a', 'Zebra study', {
    updated_at: day(0, 9),
    created_at: day(40),
    working_dir: '/Users/wgu/gptr',
  }),
  chat('b', 'alpha cohort', {
    updated_at: day(0, 11),
    created_at: day(3),
    working_dir: '/Users/wgu/biorouter/',
  }),
  chat('c', 'New chat', { updated_at: day(1), created_at: day(1), working_dir: '/Users/wgu/gptr' }),
  chat('d', 'Chart 10', { updated_at: day(5), created_at: day(0, 8), working_dir: '/Users/wgu' }),
  chat('e', 'Chart 9', {
    updated_at: day(40),
    created_at: day(60),
    working_dir: '/Users/wgu/gptr',
  }),
];
const HOME = '/Users/wgu';

const shape = (view: SidebarChatView) =>
  arrangeSidebarChats(CHATS, view, NOW, { homeDir: HOME }).map((group) => [
    group.label,
    group.sessions.map((session) => session.id).join(''),
  ]);

describe('arrangeSidebarChats: the nine views', () => {
  it('date × activity (the default) buckets by last activity, newest first', () => {
    expect(shape({ groupBy: 'date', sortBy: 'activity' })).toEqual([
      ['Today', 'ba'],
      ['Yesterday', 'c'],
      ['Previous 7 days', 'd'],
      ['August', 'e'],
    ]);
  });

  it('date × created buckets by creation', () => {
    expect(shape({ groupBy: 'date', sortBy: 'created' })).toEqual([
      ['Today', 'd'],
      ['Yesterday', 'c'],
      ['Previous 7 days', 'b'],
      ['August', 'ae'],
    ]);
  });

  it('date × name keeps the activity buckets and sorts names inside, default names last', () => {
    expect(shape({ groupBy: 'date', sortBy: 'name' })).toEqual([
      ['Today', 'ba'],
      ['Yesterday', 'c'],
      ['Previous 7 days', 'd'],
      ['August', 'e'],
    ]);
  });

  it('folder × activity orders folders by their latest activity', () => {
    expect(shape({ groupBy: 'folder', sortBy: 'activity' })).toEqual([
      ['biorouter', 'b'],
      ['gptr', 'ace'],
      ['~', 'd'],
    ]);
  });

  it('folder × created orders folders by their newest chat', () => {
    expect(shape({ groupBy: 'folder', sortBy: 'created' })).toEqual([
      ['~', 'd'],
      ['gptr', 'cae'],
      ['biorouter', 'b'],
    ]);
  });

  it('folder × name orders folders and chats A to Z, numbers numerically', () => {
    expect(shape({ groupBy: 'folder', sortBy: 'name' })).toEqual([
      ['~', 'd'],
      ['biorouter', 'b'],
      ['gptr', 'eac'],
    ]);
  });

  it('none × activity is one list with no header', () => {
    expect(shape({ groupBy: 'none', sortBy: 'activity' })).toEqual([['', 'bacde']]);
  });

  it('none × created', () => {
    expect(shape({ groupBy: 'none', sortBy: 'created' })).toEqual([['', 'dcbae']]);
  });

  it('none × name puts "Chart 9" before "Chart 10" and "New chat" last', () => {
    expect(shape({ groupBy: 'none', sortBy: 'name' })).toEqual([['', 'bedac']]);
  });

  it('covers every option the menu offers', () => {
    expect(SIDEBAR_GROUP_BY_OPTIONS).toEqual(['date', 'folder', 'none']);
    expect(SIDEBAR_SORT_BY_OPTIONS).toEqual(['activity', 'created', 'name']);
  });

  it('returns no groups for no chats', () => {
    expect(arrangeSidebarChats([], DEFAULT_SIDEBAR_CHAT_VIEW, NOW)).toEqual([]);
    expect(arrangeSidebarChats([], { groupBy: 'none', sortBy: 'name' }, NOW)).toEqual([]);
  });

  it('gives a folder group its tooltip path and raw folder', () => {
    const [group] = arrangeSidebarChats(
      [CHATS[1]],
      { groupBy: 'folder', sortBy: 'activity' },
      NOW,
      {
        homeDir: HOME,
      }
    );
    expect(group).toMatchObject({
      kind: 'folder',
      label: 'biorouter',
      path: '~/biorouter',
      workingDir: '/Users/wgu/biorouter',
      isCrew: false,
    });
  });
});

describe('folderLabels', () => {
  it('uses parent/basename when two basenames collide', () => {
    const labels = folderLabels(['/work/a/app', '/work/b/app', '/work/c/tools'], HOME);
    expect([...labels.values()]).toEqual(['a/app', 'b/app', 'tools']);
  });

  it('falls back to the full path when even parent/basename collides', () => {
    const labels = folderLabels(['/x/a/app', '/y/a/app'], HOME);
    expect([...labels.values()]).toEqual(['/x/a/app', '/y/a/app']);
  });

  it('labels home ~ and the Crew task folder Crew', () => {
    const labels = folderLabels(
      [HOME, '/Users/wgu/Library/Application Support/Biorouter/crew/tasks', ''],
      HOME
    );
    expect([...labels.values()]).toEqual(['~', 'Crew', 'No folder']);
  });

  it('marks the Crew task group so it offers no new chat', () => {
    const crewChat = chat('t', 'Crew · #general', {
      working_dir: '/data/biorouter/crew/tasks',
    });
    const [group] = arrangeSidebarChats([crewChat], { groupBy: 'folder', sortBy: 'activity' }, NOW);
    expect(group).toMatchObject({ label: 'Crew', isCrew: true });
  });
});

describe('the stored view', () => {
  beforeEach(() => window.localStorage.clear());

  it('clamps unknown values field by field', () => {
    expect(clampSidebarChatView({ groupBy: 'folder', sortBy: 'priority' })).toEqual({
      groupBy: 'folder',
      sortBy: 'activity',
    });
    expect(clampSidebarChatView('garbage')).toEqual(DEFAULT_SIDEBAR_CHAT_VIEW);
  });

  it('round-trips through storage with a version', () => {
    writeSidebarChatView({ groupBy: 'none', sortBy: 'name' });
    expect(JSON.parse(window.localStorage.getItem(SIDEBAR_CHAT_VIEW_STORAGE_KEY)!)).toEqual({
      v: 1,
      groupBy: 'none',
      sortBy: 'name',
    });
    expect(readSidebarChatView()).toEqual({ groupBy: 'none', sortBy: 'name' });
  });

  it('falls back to the default on unreadable JSON and on a storage that throws', () => {
    window.localStorage.setItem(SIDEBAR_CHAT_VIEW_STORAGE_KEY, '{nope');
    expect(readSidebarChatView()).toEqual(DEFAULT_SIDEBAR_CHAT_VIEW);

    const throwing = {
      getItem: () => {
        throw new Error('blocked');
      },
      setItem: () => {
        throw new Error('blocked');
      },
    };
    expect(readSidebarChatView(throwing)).toEqual(DEFAULT_SIDEBAR_CHAT_VIEW);
    expect(() =>
      writeSidebarChatView({ groupBy: 'folder', sortBy: 'name' }, throwing)
    ).not.toThrow();
  });

  it('follows a change made in another window', () => {
    const { result } = renderHook(() => useSidebarChatView());
    expect(result.current[0]).toEqual(DEFAULT_SIDEBAR_CHAT_VIEW);

    act(() => {
      window.dispatchEvent(
        new StorageEvent('storage', {
          key: SIDEBAR_CHAT_VIEW_STORAGE_KEY,
          newValue: JSON.stringify({ v: 1, groupBy: 'folder', sortBy: 'created' }),
        })
      );
    });
    expect(result.current[0]).toEqual({ groupBy: 'folder', sortBy: 'created' });

    act(() => result.current[1]({ groupBy: 'none', sortBy: 'name' }));
    expect(result.current[0]).toEqual({ groupBy: 'none', sortBy: 'name' });
    expect(readSidebarChatView()).toEqual({ groupBy: 'none', sortBy: 'name' });
  });

  it('caps the remembered collapsed folders', () => {
    const many = Array.from({ length: SIDEBAR_COLLAPSED_FOLDERS_MAX + 5 }, (_, i) => `/f/${i}`);
    expect(parseCollapsedFolders(JSON.stringify(many))).toHaveLength(SIDEBAR_COLLAPSED_FOLDERS_MAX);
    expect(parseCollapsedFolders('{"not":"an array"}')).toEqual([]);
  });
});

describe('applyHeldArrangement: never move a row under the pointer', () => {
  const view = DEFAULT_SIDEBAR_CHAT_VIEW;
  const arranged = (sessions: SessionSummary[]) => arrangeSidebarChats(sessions, view, NOW);
  const ids = (groups: ReturnType<typeof arranged>) =>
    groups.map((group) => [group.label, group.sessions.map((s) => s.id).join('')]);

  it('keeps the held order when a chat moves to the top, and shows its new name', () => {
    const before = arranged(CHATS);
    const held = holdArrangement(before, view);
    const moved = CHATS.map((session) =>
      session.id === 'd' ? { ...session, name: 'Renamed', updated_at: day(0, 11, 30) } : session
    );
    const shown = applyHeldArrangement(held, arranged(moved));
    expect(ids(shown)).toEqual(ids(before));
    expect(shown.flatMap((group) => group.sessions).find((s) => s.id === 'd')?.name).toBe(
      'Renamed'
    );
  });

  it('lets a removed row go and a row below every held row come in', () => {
    const held = holdArrangement(arranged(CHATS.slice(0, 4)), view);
    const shown = applyHeldArrangement(
      held,
      arranged(CHATS.filter((session) => session.id !== 'c'))
    );
    expect(ids(shown)).toEqual([
      ['Today', 'ba'],
      ['Previous 7 days', 'd'],
      ['August', 'e'],
    ]);
  });

  it('holds back a new row that would land above held rows', () => {
    const held = holdArrangement(arranged(CHATS), view);
    const fresh = chat('z', 'Brand new', { updated_at: day(0, 11, 59) });
    const shown = applyHeldArrangement(held, arranged([fresh, ...CHATS]));
    expect(shown.flatMap((group) => group.sessions).map((s) => s.id)).not.toContain('z');
  });
});
