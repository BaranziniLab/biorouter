import { act, renderHook } from '@testing-library/react';
import { beforeEach, describe, expect, it } from 'vitest';

import type { SessionSummary } from '../../api';
import {
  applyHeldArrangement,
  arrangeSidebarChats,
  arrangeSidebarChatsCached,
  clampSidebarChatView,
  DEFAULT_SIDEBAR_CHAT_VIEW,
  folderLabels,
  groupSidebarChatsByDate,
  holdArrangement,
  normalizeFolder,
  parseCollapsedFolders,
  readSidebarChatView,
  SIDEBAR_CHAT_VIEW_STORAGE_KEY,
  linesToDrawRow,
  nextSidebarDrawRange,
  sidebarLinesInView,
  sliceSidebarLines,
  windowSidebarRows,
  sidebarRenderWindow,
  SIDEBAR_LINE_HEIGHT_PX,
  SIDEBAR_RENDER_OVERSCAN,
  SIDEBAR_RENDER_WINDOW,
  SIDEBAR_COLLAPSED_FOLDERS_MAX,
  SIDEBAR_GROUP_BY_OPTIONS,
  SIDEBAR_SORT_BY_OPTIONS,
  sidebarRowTitle,
  writeSidebarChatView,
  type SidebarChatView,
} from './sidebarChatView';
import { useSidebarChatView } from './useSidebarChatView';
import { groupByChatDate } from '../../utils/chatDateBuckets';
import { isDefaultSessionName } from '../../utils/sessionNameSync';

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

  it('sorts a chat by the title its row shows: a scheduled run by its job id (V1-R3-6)', () => {
    const chats = [
      chat('run', 'Scheduled job: daily-meditation', { session_type: 'scheduled' }),
      chat('person', 'Scheduled job: notes', { session_type: 'user' }),
      chat('cell', 'Cell atlas'),
      chat('morning', 'Morning notes'),
      chat('zoo', 'Zoo survey'),
    ];
    const order = arrangeSidebarChats(
      chats,
      { groupBy: 'none', sortBy: 'name' },
      NOW
    )[0].sessions.map((session) => session.id);
    // "daily-meditation" files under D, before "Morning notes" (by the stored name it would
    // follow it, under S); a chat a person titled "Scheduled job: …" keeps S.
    expect(order).toEqual(['cell', 'run', 'morning', 'person', 'zoo']);
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

describe('sidebarRowTitle: the kind is said once, in the glyph (V1-R3-6)', () => {
  it('shows a scheduled run as its job id, keyed on the kind', () => {
    const run = { session_type: 'scheduled' };
    expect(sidebarRowTitle(chat('a', 'Scheduled job: daily-meditation', run))).toBe(
      'daily-meditation'
    );
    // A chat a person titled this way keeps its title; a renamed run has nothing to drop.
    expect(sidebarRowTitle(chat('b', 'Scheduled job: notes', { session_type: 'user' }))).toBe(
      'Scheduled job: notes'
    );
    expect(sidebarRowTitle(chat('c', 'Morning check-in', run))).toBe('Morning check-in');
    // Only the prefix: kept, so a row never reads empty.
    expect(sidebarRowTitle(chat('d', 'Scheduled job:', run))).toBe('Scheduled job:');
  });

  it("shows an app's chat by the app's name, as History does", () => {
    expect(sidebarRowTitle(chat('e', 'app:living-news-router'))).toBe('Living news router');
  });

  it('trims, and leaves an empty name empty for the row to fill', () => {
    expect(sidebarRowTitle(chat('f', '  Cell atlas  '))).toBe('Cell atlas');
    expect(sidebarRowTitle(chat('g', '   '))).toBe('');
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

/**
 * R-28. The arrangement reads each row's sort key once, buckets dates once per
 * calendar day and groups folders in one pass. These pin it to the arrangement
 * it replaced, written out here as it was, over a few thousand generated chats
 * that tie, collide, carry default names and bad timestamps.
 */
describe('arrangeSidebarChats at scale (R-28)', () => {
  // A small deterministic generator, so a failure names a reproducible list.
  function generated(count: number): SessionSummary[] {
    let seed = 7;
    const next = () => {
      seed = (seed * 1_103_515_245 + 12_345) % 2_147_483_648;
      return seed / 2_147_483_648;
    };
    const names = ['Chart 9', 'Chart 10', 'alpha', 'Alpha', 'New chat', '', 'zebra', 'Écrit'];
    const folders = ['/Users/wgu', '/Users/wgu/gptr', '/Users/wgu/a/gptr', '/tmp/x/', ''];
    return Array.from({ length: count }, (_, index) => {
      // Mostly the last 400 days, some in the future, a few unreadable.
      const pick = next();
      const updated =
        pick < 0.02
          ? 'not a date'
          : new Date(
              NOW -
                Math.floor(next() * 400) * 86_400_000 +
                (pick < 0.05 ? 3_600_000 * 30 : -Math.floor(next() * 86_400_000))
            ).toISOString();
      const created = new Date(NOW - Math.floor(next() * 500) * 86_400_000).toISOString();
      return chat(
        `s${index % 997}-${index}`,
        `${names[index % names.length]}${index % 3 ? '' : ` ${index % 13}`}`,
        {
          updated_at: index % 11 === 0 ? created : updated,
          created_at: created,
          working_dir: `${folders[index % folders.length]}${index % 7 === 0 ? `/p${index % 4}` : ''}`,
        }
      );
    });
  }

  // The arrangement as it stood before R-28.
  function reference(sessions: readonly SessionSummary[], view: SidebarChatView) {
    const parse = (value: string | null | undefined) => {
      const parsed = value ? Date.parse(value) : Number.NaN;
      return Number.isNaN(parsed) ? 0 : parsed;
    };
    const activity = (s: SessionSummary) => parse(s.updated_at) || parse(s.created_at);
    const created = (s: SessionSummary) => parse(s.created_at) || parse(s.updated_at);
    const collator = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });
    const byName = (l: SessionSummary, r: SessionSummary) => {
      const ld = isDefaultSessionName(l.name?.trim());
      const rd = isDefaultSessionName(r.name?.trim());
      if (ld !== rd) return ld ? 1 : -1;
      return collator.compare(l.name ?? '', r.name ?? '') || l.id.localeCompare(r.id);
    };
    const time = view.sortBy === 'created' ? created : activity;
    const sorted = [...sessions].sort(
      view.sortBy === 'name' ? byName : (l, r) => time(r) - time(l) || l.id.localeCompare(r.id)
    );
    if (view.groupBy === 'none') return [['', sorted.map((s) => s.id)]];
    if (view.groupBy === 'date') {
      return groupByChatDate(sorted, time, NOW).map((g) => [g.label, g.items.map((s) => s.id)]);
    }
    const byFolder = new Map<string, SessionSummary[]>();
    for (const session of sorted) {
      const folder = normalizeFolder(session.working_dir);
      byFolder.set(folder, [...(byFolder.get(folder) ?? []), session]);
    }
    const labels = folderLabels([...byFolder.keys()], HOME);
    const groups = [...byFolder.entries()].map(([folder, members]) => ({
      key: `folder:${folder}`,
      label: labels.get(folder) ?? folder,
      members,
    }));
    const latest = (g: { members: SessionSummary[] }) => Math.max(...g.members.map(time));
    groups.sort((l, r) =>
      view.sortBy === 'name'
        ? collator.compare(l.label, r.label) || l.key.localeCompare(r.key)
        : latest(r) - latest(l) || l.key.localeCompare(r.key)
    );
    return groups.map((g) => [g.label, g.members.map((s) => s.id)]);
  }

  const MANY = generated(3_000);

  for (const groupBy of SIDEBAR_GROUP_BY_OPTIONS) {
    for (const sortBy of SIDEBAR_SORT_BY_OPTIONS) {
      it(`${groupBy} × ${sortBy} matches the arrangement it replaced`, () => {
        const view = { groupBy, sortBy };
        const arranged = arrangeSidebarChats(MANY, view, NOW, { homeDir: HOME }).map((group) => [
          group.label,
          group.sessions.map((session) => session.id),
        ]);
        expect(arranged).toEqual(reference(MANY, view));
      });
    }
  }

  it('buckets dates exactly as groupByChatDate does', () => {
    const times = MANY.map((session) => Date.parse(session.updated_at) || 0);
    const timeOf = (session: SessionSummary) => times[MANY.indexOf(session)];
    const ids = (groups: Array<{ key: string; label: string; items: SessionSummary[] }>) =>
      groups.map((group) => [group.key, group.label, group.items.map((session) => session.id)]);
    expect(ids(groupSidebarChatsByDate(MANY, timeOf, NOW))).toEqual(
      ids(groupByChatDate(MANY, timeOf, NOW))
    );
  });
});

describe('windowSidebarRows: draw what can be seen (R-28)', () => {
  type Entry = { key: string; header: number; rows: string[]; more: 'more' | null };
  const entry = (key: string, header: number, rows: number, more: 'more' | null = null): Entry => ({
    key,
    header,
    rows: Array.from({ length: rows }, (_, index) => `${key}${index}`),
    more,
  });
  const lines = (entries: Entry[]) =>
    entries.map((e) => `${e.key}:${e.rows.length}${e.more ? '+' : ''}`).join(' ');
  const window = (entries: Entry[], budget: number) => {
    const result = windowSidebarRows(entries, budget, (e) => e.header);
    return [lines(result.entries), result.cut];
  };

  it('draws everything that fits and says nothing was cut', () => {
    expect(window([entry('a', 1, 3), entry('b', 1, 2)], 7)).toEqual(['a:3 b:2', false]);
    expect(window([entry('all', 0, 5)], 5)).toEqual(['all:5', false]);
  });

  it('counts headers and Show more as lines, and cuts inside a group', () => {
    expect(window([entry('a', 1, 3), entry('b', 1, 4)], 6)).toEqual(['a:3 b:1', true]);
    expect(window([entry('a', 1, 5, 'more'), entry('b', 1, 1)], 9)).toEqual(['a:5+ b:1', false]);
    // Room for the next header and none of its rows: the list stops before it.
    expect(window([entry('a', 1, 5, 'more'), entry('b', 1, 1)], 7)).toEqual(['a:5+', true]);
    expect(window([entry('a', 1, 2), entry('b', 1, 3)], 4)).toEqual(['a:2', true]);
    // Out of room for Show more: the group is drawn without it, and it is a cut.
    expect(window([entry('a', 1, 5, 'more')], 6)).toEqual(['a:5', true]);
  });

  it('draws a collapsed folder header as one line', () => {
    expect(window([entry('a', 1, 0), entry('b', 1, 0), entry('c', 1, 0)], 2)).toEqual([
      'a:0 b:0',
      true,
    ]);
  });

  it('is not cut when only empty groups are left out', () => {
    expect(window([entry('a', 0, 2), entry('b', 0, 0)], 2)).toEqual(['a:2', false]);
  });

  it('keeps the rows in the order it was given', () => {
    const result = windowSidebarRows([entry('a', 0, 10)], 4, () => 0);
    expect(result.entries[0].rows).toEqual(['a0', 'a1', 'a2', 'a3']);
  });

  it('says how many lines reach a row, so a rename can draw it', () => {
    const entries = [entry('a', 1, 3, 'more'), entry('b', 1, 4)].map((e) => ({
      ...e,
      rows: e.rows.map((id) => ({ id })),
    }));
    const header = (e: { header: number }) => e.header;
    // a: header (1) + a0 a1 a2 (3) + Show more (1) = 5; b: header (6), then b0..b3.
    expect(linesToDrawRow(entries, 'a0', header)).toBe(2);
    expect(linesToDrawRow(entries, 'b2', header)).toBe(9);
    expect(linesToDrawRow(entries, 'nope', header)).toBeNull();
    const drawn = windowSidebarRows(entries, linesToDrawRow(entries, 'b2', header) ?? 0, header);
    expect(drawn.entries[drawn.entries.length - 1].rows.map((row) => row.id)).toContain('b2');
  });

  it('draws what the list shows plus the overscan, never more than the full window', () => {
    // The list heights a 700, 900, 1117 and 1440 px tall window gives the sidebar.
    expect(sidebarRenderWindow(413)).toBe(14 + SIDEBAR_RENDER_OVERSCAN);
    expect(sidebarRenderWindow(613)).toBe(21 + SIDEBAR_RENDER_OVERSCAN);
    expect(sidebarRenderWindow(830)).toBe(28 + SIDEBAR_RENDER_OVERSCAN);
    expect(sidebarRenderWindow(1153)).toBe(SIDEBAR_RENDER_WINDOW - 9);
    expect(sidebarRenderWindow(5000)).toBe(SIDEBAR_RENDER_WINDOW);
    // Never more than the list drew before, and still more than it shows (a
    // list taller than the full window is filled by the list's own fill check).
    for (const height of [1, 30, 300, 900, 1500]) {
      const lines = sidebarRenderWindow(height);
      expect(lines).toBeLessThanOrEqual(SIDEBAR_RENDER_WINDOW);
      expect(lines * SIDEBAR_LINE_HEIGHT_PX).toBeGreaterThan(height);
    }
  });

  it('draws the full window for a list not measured yet', () => {
    expect(sidebarRenderWindow(0)).toBe(SIDEBAR_RENDER_WINDOW);
    expect(sidebarRenderWindow(-1)).toBe(SIDEBAR_RENDER_WINDOW);
    expect(sidebarRenderWindow(Number.NaN)).toBe(SIDEBAR_RENDER_WINDOW);
  });
});

describe('sliceSidebarLines: draw only the lines near the viewport (R-28, V1-R2-2 round 4)', () => {
  type Entry = { key: string; header: number; rows: Array<{ id: string }>; more: 'more' | null };
  const entry = (key: string, header: number, rows: number, more: 'more' | null = null): Entry => ({
    key,
    header,
    rows: Array.from({ length: rows }, (_, index) => ({ id: `${key}${index}` })),
    more,
  });
  const header = (e: Entry) => e.header;
  // A group as drawn: `H` a drawn header, `h` an undrawn one, a row id, `+` Show more, `~n` a gap.
  const show = (sliced: ReturnType<typeof sliceSidebarLines<Entry>>) =>
    sliced
      .map(
        (e) =>
          (e.header ? (e.headerDrawn ? 'H ' : 'h ') : '') +
          e.lines
            .map((l) => (l.kind === 'row' ? l.row.id : l.kind === 'more' ? '+' : `~${l.lines}`))
            .join(' ')
      )
      .join(' | ');
  const lineCount = (sliced: ReturnType<typeof sliceSidebarLines<Entry>>) =>
    sliced.reduce(
      (sum, e) =>
        sum + e.header + e.lines.reduce((n, l) => n + (l.kind === 'gap' ? l.lines : 1), 0),
      0
    );

  it('draws every line for a list not measured yet (no range)', () => {
    const sliced = sliceSidebarLines([entry('a', 1, 3, 'more'), entry('b', 1, 2)], null, header);
    expect(show(sliced)).toBe('H a0 a1 a2 + | H b0 b1');
    expect(sliced.map((e) => e.rows.length)).toEqual([3, 2]);
    expect(sliced[0].more).toBe('more');
  });

  it('folds the rows outside the range into gaps that keep every line', () => {
    const entries = [entry('all', 0, 100)];
    const sliced = sliceSidebarLines(entries, { from: 40, to: 45 }, header);
    expect(show(sliced)).toBe('~40 all40 all41 all42 all43 all44 ~55');
    expect(lineCount(sliced)).toBe(100);
    // The rows drawn are the list's own, in its order, with nothing between them.
    expect(sliced[0].rows.map((row) => row.id)).toEqual([
      'all40',
      'all41',
      'all42',
      'all43',
      'all44',
    ]);
  });

  it('counts headers and Show more as lines, and folds Show more into a gap', () => {
    // Lines: a header 0, a0..a4 1-5, + 6, b header 7, b0..b2 8-10.
    const entries = [entry('a', 1, 5, 'more'), entry('b', 1, 3)];
    const sliced = sliceSidebarLines(entries, { from: 3, to: 5 }, header);
    expect(show(sliced)).toBe('h ~2 a2 a3 ~2 | h ~3');
    expect(sliced[0].more).toBeNull();
    expect(sliced.map((e) => e.rowsLine)).toEqual([1, 8]);
    expect(lineCount(sliced)).toBe(11);
    // From the header on: a header, then its rows, then Show more.
    expect(show(sliceSidebarLines(entries, { from: 0, to: 2 }, header))).toBe('H a0 ~5 | h ~3');
    expect(show(sliceSidebarLines(entries, { from: 6, to: 9 }, header))).toBe('h ~5 + | H b0 ~2');
  });

  it('always draws the rows it is told to keep', () => {
    const entries = [entry('all', 0, 50)];
    const keep = (row: { id: string }) => row.id === 'all7' || row.id === 'all30';
    const sliced = sliceSidebarLines(entries, { from: 20, to: 22 }, header, keep);
    expect(show(sliced)).toBe('~7 all7 ~12 all20 all21 ~8 all30 ~19');
    expect(lineCount(sliced)).toBe(50);
  });

  it('keys each gap by the row it starts at, unique within its group', () => {
    const sliced = sliceSidebarLines([entry('all', 0, 30)], { from: 10, to: 12 }, header);
    const keys = sliced[0].lines.flatMap((l) => (l.kind === 'gap' ? [l.key] : []));
    expect(keys).toEqual(['gap:0', 'gap:12']);
  });

  it('draws a collapsed folder header by the range like any other line', () => {
    const entries = [entry('a', 1, 0), entry('b', 1, 0), entry('c', 1, 0), entry('d', 1, 0)];
    expect(show(sliceSidebarLines(entries, { from: 1, to: 2 }, header))).toBe('h  | H  | h  | h ');
  });
});

describe('sidebarLinesInView: lines from the drawn groups (R-28)', () => {
  // Three groups: a header 24px tall over rows on a 30px pitch, as the DOM gives them.
  const boxes = [
    { rowsLine: 1, headerLines: 1, top: 0, rowsTop: 24 }, // 10 rows: lines 1-10
    { rowsLine: 12, headerLines: 1, top: 328, rowsTop: 360 }, // 100 rows: lines 12-111
    { rowsLine: 113, headerLines: 1, top: 3392, rowsTop: 3424 },
  ];
  const reads: number[] = [];
  const boxAt = (index: number) => {
    reads.push(index);
    return boxes[index];
  };

  it('reads a line on the row pitch below its group', () => {
    expect(sidebarLinesInView(3, boxAt, 0, 600)).toEqual({ first: 0, last: 20 });
    expect(sidebarLinesInView(3, boxAt, 30, 600)).toEqual({ first: 1, last: 20 });
    expect(sidebarLinesInView(3, boxAt, 400, 1000)).toEqual({ first: 13, last: 33 });
    // A y inside a header reads as the header's line.
    expect(sidebarLinesInView(3, boxAt, 340, 3400)).toEqual({ first: 11, last: 112 });
    expect(sidebarLinesInView(3, boxAt, 3500, 4000)).toEqual({ first: 115, last: 132 });
  });

  it('reads only the boxes a binary search visits', () => {
    const many = Array.from({ length: 1200 }, (_, index) => ({
      rowsLine: index * 2 + 1,
      headerLines: 1,
      top: index * 64,
      rowsTop: index * 64 + 32,
    }));
    let count = 0;
    const result = sidebarLinesInView(
      many.length,
      (index) => {
        count += 1;
        return many[index];
      },
      64 * 600,
      64 * 600 + 600
    );
    expect(result).toEqual({ first: 1200, last: 1218 });
    expect(count).toBeLessThan(30);
  });

  it('measures nothing when there is nothing to measure', () => {
    expect(sidebarLinesInView(0, boxAt, 0, 600)).toBeNull();
    expect(sidebarLinesInView(3, boxAt, 0, 0)).toBeNull();
    expect(sidebarLinesInView(3, boxAt, 0, 600, 0)).toBeNull();
  });
});

describe('nextSidebarDrawRange: a range that holds still while it covers the view (R-28)', () => {
  const o = SIDEBAR_RENDER_OVERSCAN;

  it('draws the lines in view plus the overscan on both sides', () => {
    expect(nextSidebarDrawRange(null, { first: 100, last: 120 })).toEqual({
      from: 100 - o,
      to: 121 + o,
    });
    expect(nextSidebarDrawRange(null, { first: 3, last: 23 })).toEqual({ from: 0, to: 24 + o });
  });

  it('keeps the same range (same object) while the view stays half an overscan inside it', () => {
    const current = nextSidebarDrawRange(null, { first: 100, last: 120 });
    expect(nextSidebarDrawRange(current, { first: 104, last: 124 })).toBe(current);
    expect(nextSidebarDrawRange(current, { first: 96, last: 116 })).toBe(current);
    const atTop = nextSidebarDrawRange(null, { first: 0, last: 20 });
    expect(nextSidebarDrawRange(atTop, { first: 2, last: 22 })).toBe(atTop);
  });

  it('moves with the view once it nears an edge, and never grows past view plus overscan', () => {
    const current = nextSidebarDrawRange(null, { first: 100, last: 120 });
    expect(nextSidebarDrawRange(current, { first: 110, last: 130 })).toEqual({
      from: 110 - o,
      to: 131 + o,
    });
    // A shorter view (the window shrank): the range shrinks with it.
    const tall = nextSidebarDrawRange(null, { first: 100, last: 200 });
    expect(nextSidebarDrawRange(tall, { first: 100, last: 110 })).toEqual({
      from: 100 - o,
      to: 111 + o,
    });
  });

  it('keeps what it drew when the list cannot be measured', () => {
    const current = { from: 10, to: 40 };
    expect(nextSidebarDrawRange(current, null)).toBe(current);
    expect(nextSidebarDrawRange(null, null)).toBeNull();
  });
});

describe('arrangeSidebarChatsCached: a view seen before is a lookup (R-28)', () => {
  const byName: SidebarChatView = { groupBy: 'none', sortBy: 'name' };
  const byFolder: SidebarChatView = { groupBy: 'folder', sortBy: 'activity' };
  const byDate = DEFAULT_SIDEBAR_CHAT_VIEW;
  const options = { homeDir: HOME };

  it('arranges exactly as arrangeSidebarChats does', () => {
    const held = [...CHATS];
    for (const view of [byName, byFolder, byDate]) {
      expect(arrangeSidebarChatsCached(held, view, NOW, options)).toEqual(
        arrangeSidebarChats(held, view, NOW, options)
      );
    }
  });

  it('hands back the same arrangement when switching back to a view, for the same list', () => {
    const held = [...CHATS];
    const first = arrangeSidebarChatsCached(held, byName, NOW, options);
    const other = arrangeSidebarChatsCached(held, byFolder, NOW, options);
    expect(other).not.toBe(first);
    expect(arrangeSidebarChatsCached(held, { ...byName }, NOW, options)).toBe(first);
    // Later the same day is the same arrangement, date groups included.
    const dated = arrangeSidebarChatsCached(held, byDate, NOW, options);
    expect(arrangeSidebarChatsCached(held, byDate, NOW + 60 * 60 * 1000, options)).toBe(dated);
  });

  it('arranges again for a new list, another home folder or another day', () => {
    const held = [...CHATS];
    const first = arrangeSidebarChatsCached(held, byName, NOW, options);
    // A changed row arrives as a new array: its new name places it.
    const renamed = held.map((session) =>
      session.id === 'e' ? { ...session, name: 'Aardvark' } : session
    );
    const again = arrangeSidebarChatsCached(renamed, byName, NOW, options);
    expect(again).not.toBe(first);
    expect(again[0].sessions[0].id).toBe('e');
    expect(arrangeSidebarChatsCached(held, byFolder, NOW, { homeDir: '/elsewhere' })).toEqual(
      arrangeSidebarChats(held, byFolder, NOW, { homeDir: '/elsewhere' })
    );
    // The next day "Today" becomes "Yesterday".
    const today = arrangeSidebarChatsCached(held, byDate, NOW, options);
    const tomorrow = arrangeSidebarChatsCached(held, byDate, NOW + 24 * 60 * 60 * 1000, options);
    expect(tomorrow).not.toBe(today);
    expect(tomorrow[0].label).toBe('Yesterday');
  });
});
