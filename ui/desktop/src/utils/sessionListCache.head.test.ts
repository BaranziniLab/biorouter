/**
 * V1-R3-2: History opened on the full listing of every chat on the machine, 0.7
 * to 1 s on a store of 5,542, to show sixteen rows. It now reads the newest page
 * and lays it over the cache (`refreshSessionListHead`); the rows after the page
 * keep what the cache held.
 *
 * What these pin is what makes that safe: the page is merged by the daemon's own
 * order, a chat that is gone from the stretch the page covers goes with it, a
 * rename or a raise that outran the page is not undone, a head read and a full
 * read never settle against the same facts at once, and a cold cache is never
 * given a partial list.
 */
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../api';
import {
  SESSION_LIST_HEAD_SIZE,
  SESSION_LIST_STALE_AFTER_MS,
  clearSessionListCache,
  getCachedSessionList,
  mergeSessionListHead,
  refreshSessionList,
  refreshSessionListHead,
  subscribeSessionList,
} from './sessionListCache';
import { announceSessionName } from './sessionNameSync';
import { announceSessionRowChanged } from './sessionRowSync';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  updateSessionName: vi.fn(),
  getSession: vi.fn(),
}));

vi.mock('../api', () => ({
  listSessions: mocks.listSessions,
  updateSessionName: mocks.updateSessionName,
  getSession: mocks.getSession,
}));

vi.mock('./userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

/** `n` seconds after a fixed instant: a larger `n` is a NEWER chat. */
const at = (n: number) => new Date(Date.UTC(2026, 9, 11, 12, 0, n)).toISOString();
const chat = (id: string, n: number, extra: Partial<Session> = {}): Session =>
  ({
    id,
    name: `Chat ${id}`,
    user_set_name: false,
    working_dir: '/x',
    created_at: at(n),
    updated_at: at(n),
    message_count: 2,
    privacy_tier: 'public',
    privacy_reason: null,
    ...extra,
  }) as Session;

const ids = (rows: Session[] | null | undefined) => (rows ?? []).map((row) => row.id);
const page = (sessions: Session[], hasMore: boolean) => ({
  data: hasMore ? { sessions, has_more: true, next_cursor: 'next' } : { sessions },
});

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('mergeSessionListHead', () => {
  const cached = [chat('a', 50), chat('b', 40), chat('c', 30), chat('d', 20), chat('e', 10)];

  it('lays the page over the cache and keeps the rows after it', () => {
    const head = [chat('c', 60, { name: 'Renamed' }), chat('a', 50)];
    const merged = mergeSessionListHead(cached, head);
    expect(ids(merged)).toEqual(['c', 'a', 'b', 'd', 'e']);
    expect(merged[0].name).toBe('Renamed');
    // The rows after the page are the cache's own objects, so a row that did not
    // change does not redraw.
    expect(merged[2]).toBe(cached[1]);
    expect(merged[4]).toBe(cached[4]);
  });

  it('drops a cached row the page covers but does not hold', () => {
    // 'b' sorts before the page's last row (c, at 30) yet is not in the page:
    // the daemon no longer lists it (deleted, or private to this caller).
    const merged = mergeSessionListHead(cached, [chat('a', 50), chat('x', 45), chat('c', 30)]);
    expect(ids(merged)).toEqual(['a', 'x', 'c', 'd', 'e']);
  });

  it('keeps a row this window changed while the page was out, though the page lacks it', () => {
    // 'b' was renamed after the page was issued, so its cached time is past the page.
    const renamed = [chat('b', 90, { name: 'Renamed' }), ...cached.filter((s) => s.id !== 'b')];
    const merged = mergeSessionListHead(renamed, [chat('a', 50), chat('c', 30)], new Set(['b']));
    expect(ids(merged)).toEqual(['a', 'c', 'b', 'd', 'e']);
    expect(ids(mergeSessionListHead(renamed, [chat('a', 50), chat('c', 30)]))).toEqual([
      'a',
      'c',
      'd',
      'e',
    ]);
  });

  it('breaks a tie on the last row by id, the way the daemon orders', () => {
    const tied = [chat('b1', 40), chat('b3', 40), chat('b5', 40), chat('z', 10)];
    // The page ends at b3: b5 sorts after it, b1 before it (so it is gone).
    const merged = mergeSessionListHead(tied, [chat('b3', 40, { name: 'new' })]);
    expect(ids(merged)).toEqual(['b3', 'b5', 'z']);
  });

  it('keeps a cached row whose date cannot be read', () => {
    const odd = [chat('a', 50), chat('odd', 0, { updated_at: 'garbage' }), chat('z', 5)];
    expect(ids(mergeSessionListHead(odd, [chat('a', 50)]))).toEqual(['a', 'odd', 'z']);
  });

  it('is the page alone when the page is the whole list', () => {
    expect(mergeSessionListHead(cached, [])).toEqual([]);
  });
});

describe('refreshSessionListHead', () => {
  it('asks for the newest page only, with the proof, and lays it over a warm cache', async () => {
    mocks.listSessions.mockResolvedValueOnce(
      page([chat('a', 50), chat('b', 40), chat('c', 30), chat('d', 20)], false)
    );
    await refreshSessionList();
    const tail = getCachedSessionList()!.slice(1);

    mocks.listSessions.mockResolvedValueOnce(
      page([chat('c', 70, { name: 'Moved up' }), chat('a', 50)], true)
    );
    const heard = vi.fn();
    const unsubscribe = subscribeSessionList(heard);
    const head = await refreshSessionListHead();
    unsubscribe();

    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({
        headers: { 'X-User-Action': 'test-proof' },
        query: { include_subagents: false, limit: SESSION_LIST_HEAD_SIZE },
      })
    );
    expect(head.partial).toBe(false);
    expect(ids(head.sessions)).toEqual(['c', 'a', 'b', 'd']);
    expect(head.sessions[0].name).toBe('Moved up');
    // …in the cache too, and announced once, so every reader sees the same list.
    expect(getCachedSessionList()).toBe(head.sessions);
    expect(heard).toHaveBeenCalledTimes(1);
    expect(head.sessions[2]).toBe(tail[0]);
    expect(head.sessions[3]).toBe(tail[2]);
  });

  it('takes a page that is the whole list as a full read, whatever the daemon says of paging', async () => {
    // A short store, or a daemon that has never heard of `limit`: no `has_more`.
    mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
    await refreshSessionList();
    mocks.listSessions.mockResolvedValueOnce(page([chat('b', 60)], false));

    const head = await refreshSessionListHead();
    expect(ids(head.sessions)).toEqual(['b']);
    expect(ids(getCachedSessionList())).toEqual(['b']);
    expect(head.partial).toBe(false);
  });

  it('never hands a cold cache a partial list, and starts the read that fills it', async () => {
    mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], true));
    const fullList = [chat('a', 50), chat('b', 40), chat('c', 30), chat('d', 20)];
    mocks.listSessions.mockResolvedValueOnce(page(fullList, false));

    const head = await refreshSessionListHead();
    expect(head).toMatchObject({ partial: true });
    expect(ids(head.sessions)).toEqual(['a', 'b']);
    // The cache is the full list's to fill: Home, the tab strip and the sidebar
    // read it and expect every chat.
    expect(getCachedSessionList()).toBeNull();

    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));
    expect(mocks.listSessions.mock.calls[1][0].query).toEqual({ include_subagents: false });
    await vi.waitFor(() => expect(ids(getCachedSessionList())).toEqual(ids(fullList)));
  });

  it('reads the whole list when sub-agent runs are asked for, and when the identity changes', async () => {
    mocks.listSessions.mockResolvedValue(page([chat('a', 50), chat('child', 45)], false));

    const withChildren = await refreshSessionListHead(true);
    expect(withChildren.partial).toBe(false);
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: { include_subagents: true } })
    );

    // Back to the default list: a change of identity, so the full read decides it.
    await refreshSessionListHead(false);
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: { include_subagents: false } })
    );
  });

  it('waits for a full read already in flight instead of racing it', async () => {
    const full = deferred<unknown>();
    mocks.listSessions.mockReturnValueOnce(full.promise);
    const inFlight = refreshSessionList();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));

    const head = refreshSessionListHead();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);

    full.resolve(page([chat('a', 50)], false));
    await inFlight;
    expect(ids((await head).sessions)).toEqual(['a']);
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
  });

  it('puts a full read behind a head read in flight, so the two never share one set of facts', async () => {
    mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
    await refreshSessionList();

    const headAnswer = deferred<unknown>();
    mocks.listSessions.mockReturnValueOnce(headAnswer.promise);
    const head = refreshSessionListHead();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    // A membership change asks for a full read while the head read is out.
    mocks.listSessions.mockResolvedValueOnce(
      page([chat('n', 90), chat('a', 50), chat('b', 40)], false)
    );
    const full = refreshSessionList();
    await new Promise((resolve) => setTimeout(resolve, 10));
    expect(mocks.listSessions).toHaveBeenCalledTimes(2);

    headAnswer.resolve(page([chat('a', 55), chat('b', 40)], false));
    await head;
    expect(ids(await full)).toEqual(['n', 'a', 'b']);
    expect(mocks.listSessions).toHaveBeenCalledTimes(3);
    // The later read is the one the cache holds.
    expect(ids(getCachedSessionList())).toEqual(['n', 'a', 'b']);
  });

  it('keeps a rename announced while the page was in flight, and moves the chat up', async () => {
    mocks.listSessions.mockResolvedValueOnce(
      page([chat('a', 50), chat('b', 40), chat('c', 30)], false)
    );
    await refreshSessionList();

    const answer = deferred<unknown>();
    mocks.listSessions.mockReturnValueOnce(answer.promise);
    const head = refreshSessionListHead();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    // A saved rename of a chat that is NOT in the page (it sits in the tail).
    announceSessionName({
      sessionId: 'c',
      name: 'Saved rename',
      userSetName: true,
      origin: 'user',
      activityAt: at(80),
    });
    answer.resolve(page([chat('a', 50), chat('b', 40)], true));
    const result = await head;

    expect(ids(result.sessions)).toEqual(['c', 'a', 'b']);
    expect(result.sessions[0]).toMatchObject({ name: 'Saved rename', user_set_name: true });
  });

  it('does not draw a chat public that a raise announced while the page was in flight made private', async () => {
    mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
    await refreshSessionList();

    const answer = deferred<unknown>();
    mocks.listSessions.mockReturnValueOnce(answer.promise);
    const head = refreshSessionListHead();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    mocks.getSession.mockResolvedValue({
      data: { id: 'a', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
    });
    announceSessionRowChanged('a');
    await vi.waitFor(() =>
      expect(getCachedSessionList()?.[0]).toMatchObject({ privacy_tier: 'private' })
    );

    // The page was issued before the raise and still says public.
    answer.resolve(page([chat('a', 50), chat('b', 40)], false));
    const result = await head;
    expect(result.sessions[0]).toMatchObject({ id: 'a', privacy_tier: 'private' });
  });

  it('publishes nothing when the cache was cleared while the page was in flight', async () => {
    mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
    await refreshSessionList();

    const answer = deferred<unknown>();
    mocks.listSessions.mockReturnValueOnce(answer.promise);
    const head = refreshSessionListHead();
    await vi.waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));

    clearSessionListCache();
    answer.resolve(page([chat('z', 99)], true));
    await head;
    expect(getCachedSessionList()).toBeNull();
  });

  describe('the rows after the page', () => {
    beforeEach(() => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout', 'Date'] });
      vi.setSystemTime(new Date('2026-10-11T12:00:00Z'));
    });

    it('are re-read in the background once they have gone unchecked for a minute', async () => {
      mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
      await refreshSessionList();

      vi.setSystemTime(Date.now() + SESSION_LIST_STALE_AFTER_MS + 1);
      mocks.listSessions.mockResolvedValueOnce(page([chat('a', 55)], true));
      mocks.listSessions.mockResolvedValueOnce(page([chat('a', 55)], false));
      await refreshSessionListHead();
      expect(mocks.listSessions).toHaveBeenCalledTimes(2);

      await vi.advanceTimersByTimeAsync(2_000);
      expect(mocks.listSessions).toHaveBeenCalledTimes(3);
      // The quiet read is the whole listing: no `limit`.
      expect(mocks.listSessions.mock.calls[2][0].query).toEqual({ include_subagents: false });
      expect(ids(getCachedSessionList())).toEqual(['a']);
    });

    it('are left alone on a quick revisit', async () => {
      mocks.listSessions.mockResolvedValueOnce(page([chat('a', 50), chat('b', 40)], false));
      await refreshSessionList();

      vi.setSystemTime(Date.now() + SESSION_LIST_STALE_AFTER_MS - 5_000);
      mocks.listSessions.mockResolvedValueOnce(page([chat('a', 55)], true));
      await refreshSessionListHead();
      await vi.advanceTimersByTimeAsync(5_000);

      expect(mocks.listSessions).toHaveBeenCalledTimes(2);
    });
  });
});
