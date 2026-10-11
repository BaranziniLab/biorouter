import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import useSidebarSessions, {
  appendSessionPage,
  SIDEBAR_SESSION_LOAD_ALL_CAP,
  SIDEBAR_SESSION_PAGE_SIZE,
} from './useSidebarSessions';
import { notifySessionListChanged } from '../../utils/sessionListCache';
import { announceSessionName } from '../../utils/sessionNameSync';

const mocks = vi.hoisted(() => ({
  listSidebarSessions: vi.fn(),
  getSession: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSidebarSessions: mocks.listSidebarSessions,
  getSession: mocks.getSession,
}));

// The proof the desktop sends. Since issue #56's QA sweep (2026-09-10) the
// daemon answers a request without it as a public model — private chats and
// knowledge bases omitted or refused — so each call here must carry it.
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

function makeSummary(index: number): SessionSummary {
  const timestamp = new Date(Date.parse('2026-07-15T12:00:00.000Z') - index * 60_000).toISOString();
  return {
    id: `session-${index}`,
    name: `Chat ${index}`,
    working_dir: `/workspace/project-${index}`,
    created_at: timestamp,
    updated_at: timestamp,
    message_count: index,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
});

describe('appendSessionPage', () => {
  it('appends new summaries while replacing duplicate session ids in place', () => {
    const updated = { ...makeSummary(0), name: 'Renamed chat' };

    expect(appendSessionPage([makeSummary(0)], [updated, makeSummary(1)])).toEqual([
      updated,
      makeSummary(1),
    ]);
  });
});

describe('useSidebarSessions', () => {
  it('loads lightweight session summaries one page at a time', async () => {
    const firstPage = Array.from({ length: SIDEBAR_SESSION_PAGE_SIZE }, (_, index) =>
      makeSummary(index)
    );
    const secondPage = [makeSummary(10), makeSummary(11)];
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: firstPage, has_more: true, next_cursor: 'cursor-page-2' },
      })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions());

    await waitFor(() => expect(result.current.sessions).toHaveLength(10));
    expect(result.current.hasMore).toBe(true);
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(1, {
      query: { limit: 10 },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });

    act(() => result.current.loadMore());

    await waitFor(() => expect(result.current.sessions).toHaveLength(12));
    expect(result.current.hasMore).toBe(false);
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(2, {
      query: { limit: 10, cursor: 'cursor-page-2' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });

  it('refreshes recent activity without discarding pages already loaded by scrolling', async () => {
    const firstPage = Array.from({ length: 10 }, (_, index) => makeSummary(index));
    const secondPage = Array.from({ length: 10 }, (_, index) => makeSummary(index + 10));
    const refreshedFirstPage = [
      { ...makeSummary(0), name: 'Refreshed chat' },
      ...firstPage.slice(1),
    ];
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: firstPage, has_more: true, next_cursor: 'cursor-page-2' },
      })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: true, next_cursor: 'cursor-page-3' },
      })
      .mockResolvedValueOnce({
        data: { sessions: refreshedFirstPage, has_more: true, next_cursor: 'cursor-page-2' },
      })
      .mockResolvedValueOnce({
        data: {
          sessions: [makeSummary(20)],
          has_more: false,
          next_cursor: null,
        },
      });

    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(10));

    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.sessions).toHaveLength(20));

    act(() => window.dispatchEvent(new Event('message-stream-finished')));
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(3), {
      timeout: 1_000,
    });
    await waitFor(() => expect(result.current.sessions[0].name).toBe('Refreshed chat'));

    expect(result.current.sessions).toHaveLength(20);
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(3, {
      query: { limit: 10 },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });

    // …and the refresh must not rewind the tail. `next_cursor` is opaque and
    // names the last row of the page that issued it, so a refresh of the HEAD
    // carries the cursor for page 2 — adopting it would make "Load more" refetch
    // rows the list already holds and appear to do nothing. The furthest cursor
    // wins.
    act(() => result.current.loadMore());
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(4));
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(4, {
      query: { limit: 10, cursor: 'cursor-page-3' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });
});

/**
 * M11. A deleted chat left History, the tab strip and the database immediately
 * and stayed in the sidebar until a renderer reload.
 *
 * Two independent breaks produced it, and BOTH have to be closed:
 *   (a) the delete handler never announced membership at all, and
 *   (b) `appendSessionPage` is additive — it appends and replaces by id, and
 *       has no removal branch — so even a full re-read of the first page leaves
 *       an id the server no longer returns sitting in the merged list.
 *
 * (b) cannot be fixed by inferring removals from a refetch: `loadPage(true)`
 * re-reads only the FIRST page, and an entry can drop out of that window
 * because other chats were touched, not because it was deleted. So the removal
 * is announced explicitly, by id.
 */
describe('a deleted chat leaves the sidebar without a reload', () => {
  it('drops the announced id, in this window and from a sibling window', async () => {
    const firstPage = Array.from({ length: 10 }, (_, index) => makeSummary(index));
    mocks.listSidebarSessions.mockResolvedValue({
      data: { sessions: firstPage, has_more: false, next_offset: null },
    });

    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(10));

    act(() => notifySessionListChanged({ removed: 'session-3' }));

    await waitFor(() => expect(result.current.sessions).toHaveLength(9));
    expect(result.current.sessions.map((session) => session.id)).not.toContain('session-3');
  });

  // The scrolled-in pages are the thing a naive prune-on-refresh would destroy;
  // `useSidebarSessions.test.ts`'s refresh case already pins that. Removing one
  // id must not cost the others.
  it('keeps every other loaded page', async () => {
    const firstPage = Array.from({ length: 10 }, (_, index) => makeSummary(index));
    const secondPage = Array.from({ length: 10 }, (_, index) => makeSummary(index + 10));
    mocks.listSidebarSessions
      .mockResolvedValueOnce({ data: { sessions: firstPage, has_more: true, next_offset: 10 } })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: false, next_offset: null },
      });

    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(10));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.sessions).toHaveLength(20));

    act(() => notifySessionListChanged({ removed: 'session-15' }));

    await waitFor(() => expect(result.current.sessions).toHaveLength(19));
    expect(result.current.sessions.map((session) => session.id)).toContain('session-19');
  });
});

/**
 * Item 11 of the 1.90.4 hold (2026-09-13). The daemon used to stamp
 * `updated_at` on a declassification, so the next head refresh carried the chat
 * — moved to the top, which was the bug. With the chat left where it belongs, a
 * refresh re-reads only the HEAD of the keyset and never reaches a months-old
 * row, so the row is re-marked by id from ANOTHER window's announcement.
 */
describe('a declassified chat is re-marked in the sidebar without a reload', () => {
  it('patches a scrolled-in row where it sits, from a sibling window', async () => {
    const firstPage = Array.from({ length: 10 }, (_, index) => ({
      ...makeSummary(index),
      privacy_tier: 'private' as const,
    }));
    const secondPage = Array.from({ length: 10 }, (_, index) => ({
      ...makeSummary(index + 10),
      privacy_tier: 'private' as const,
    }));
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: firstPage, has_more: true, next_cursor: 'cursor-page-2' },
      })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(10));
    act(() => result.current.loadMore());
    await waitFor(() => expect(result.current.sessions).toHaveLength(20));
    const order = result.current.sessions.map((session) => session.id);
    mocks.listSidebarSessions.mockClear();
    mocks.getSession.mockResolvedValue({
      data: { id: 'session-17', privacy_tier: 'public', privacy_reason: 'declassified_by_user' },
    });

    // Another window declassified it.
    const sibling = new BroadcastChannel('biorouter:session-row');
    try {
      sibling.postMessage({ sessionId: 'session-17' });
      await waitFor(() =>
        expect(result.current.sessions.find((s) => s.id === 'session-17')?.privacy_tier).toBe(
          'public'
        )
      );
    } finally {
      sibling.close();
    }

    expect(result.current.sessions.map((session) => session.id)).toEqual(order);
    expect(
      result.current.sessions
        .filter((s) => s.id !== 'session-17')
        .every((s) => s.privacy_tier === 'private')
    ).toBe(true);
    // No head refresh was needed — and one could not have reached this row.
    expect(mocks.listSidebarSessions).not.toHaveBeenCalled();
  });

  /**
   * Defect D4 of the 2026-09-13 repair round, the sidebar half. A head refresh
   * issued before a turn raised a chat, answered after the raise was patched
   * in, drew the row public again.
   */
  it('a page that raced a raise does not draw the row public again', async () => {
    const publicRow = { ...makeSummary(0), privacy_tier: 'public' as const };
    mocks.listSidebarSessions.mockResolvedValueOnce({
      data: { sessions: [publicRow], has_more: false, next_cursor: null },
    });
    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(1));

    let answerPage: ((value: unknown) => void) | undefined;
    mocks.listSidebarSessions.mockReturnValueOnce(
      new Promise((resolve) => {
        answerPage = resolve;
      })
    );
    // A membership nudge from anywhere schedules a head refresh.
    act(() => notifySessionListChanged());
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2));

    mocks.getSession.mockResolvedValue({
      data: { id: 'session-0', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
    });
    const sibling = new BroadcastChannel('biorouter:session-row');
    try {
      sibling.postMessage({ sessionId: 'session-0' });
      await waitFor(() => expect(result.current.sessions[0].privacy_tier).toBe('private'));
    } finally {
      sibling.close();
    }
    expect(mocks.getSession).toHaveBeenCalledTimes(1);

    // The read that settles the disagreement is held open, so the row's state
    // in the meantime is observable.
    let answerThirdRead: ((value: unknown) => void) | undefined;
    mocks.getSession.mockReturnValueOnce(
      new Promise((resolve) => {
        answerThirdRead = resolve;
      })
    );
    await act(async () => {
      answerPage!({ data: { sessions: [publicRow], has_more: false, next_cursor: null } });
    });

    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));
    expect(result.current.sessions[0].privacy_tier).toBe('private');
    await act(async () => {
      answerThirdRead!({
        data: { id: 'session-0', privacy_tier: 'private', privacy_reason: 'turn:versa_azure' },
      });
    });
    expect(result.current.sessions[0].privacy_tier).toBe('private');
  });
});

/**
 * Spec 3.4 "Paging". Any view but the default sorts or groups the WHOLE list,
 * so the hook reads it all: the first page, then the rest in one request of
 * everything the cap leaves room for (R-28), and every page carries the user's
 * proof: without it the daemon silently drops private rows, which would read as
 * chats vanishing on a view change.
 */
describe('loadAll pages through every chat', () => {
  it('asks for the rest of the list in one request after the first page (R-28)', async () => {
    const page = (from: number, count: number) =>
      Array.from({ length: count }, (_, index) => makeSummary(from + index));
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: page(0, 50), has_more: true, next_cursor: 'c2' },
      })
      .mockResolvedValueOnce({
        data: { sessions: page(50, 1_200), has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));

    await waitFor(() => expect(result.current.sessions).toHaveLength(1_250));
    await waitFor(() => expect(result.current.isLoadingAll).toBe(false));
    expect(mocks.listSidebarSessions.mock.calls.map(([request]) => request)).toEqual([
      { query: { limit: 50 }, headers: { 'X-User-Action': 'test-proof' }, throwOnError: true },
      {
        query: { limit: SIDEBAR_SESSION_LOAD_ALL_CAP - 50, cursor: 'c2' },
        headers: { 'X-User-Action': 'test-proof' },
        throwOnError: true,
      },
    ]);
  });

  it('follows an older daemon that answers 50 rows a page, with the proof, then stops', async () => {
    const page = (from: number, count: number) =>
      Array.from({ length: count }, (_, index) => makeSummary(from + index));
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: page(0, 50), has_more: true, next_cursor: 'c2' },
      })
      .mockResolvedValueOnce({
        data: { sessions: page(50, 50), has_more: true, next_cursor: 'c3' },
      })
      .mockResolvedValueOnce({
        data: { sessions: page(100, 7), has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));

    await waitFor(() => expect(result.current.sessions).toHaveLength(107));
    await waitFor(() => expect(result.current.isLoadingAll).toBe(false));
    expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(3);
    // Each request asks for all the room the cap leaves; the short answer is
    // what makes the loop ask again, from the cursor that answer carried.
    expect(mocks.listSidebarSessions.mock.calls.map(([request]) => request)).toEqual([
      { query: { limit: 50 }, headers: { 'X-User-Action': 'test-proof' }, throwOnError: true },
      {
        query: { limit: SIDEBAR_SESSION_LOAD_ALL_CAP - 50, cursor: 'c2' },
        headers: { 'X-User-Action': 'test-proof' },
        throwOnError: true,
      },
      {
        query: { limit: SIDEBAR_SESSION_LOAD_ALL_CAP - 100, cursor: 'c3' },
        headers: { 'X-User-Action': 'test-proof' },
        throwOnError: true,
      },
    ]);
  });

  it('keeps private rows a later page brings', async () => {
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: { sessions: [makeSummary(0)], has_more: true, next_cursor: 'c2' },
      })
      .mockResolvedValueOnce({
        data: {
          sessions: [{ ...makeSummary(1), privacy_tier: 'private' as const }],
          has_more: false,
          next_cursor: null,
        },
      });

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(result.current.sessions).toHaveLength(2));
    expect(result.current.sessions[1].privacy_tier).toBe('private');
  });

  it('turns on when the view changes, continuing from the pages already held', async () => {
    mocks.listSidebarSessions
      .mockResolvedValueOnce({
        data: {
          sessions: Array.from({ length: 10 }, (_, index) => makeSummary(index)),
          has_more: true,
          next_cursor: 'c2',
        },
      })
      .mockResolvedValueOnce({
        data: { sessions: [makeSummary(10)], has_more: false, next_cursor: null },
      });

    const { result, rerender } = renderHook(({ loadAll }) => useSidebarSessions({ loadAll }), {
      initialProps: { loadAll: false },
    });
    await waitFor(() => expect(result.current.sessions).toHaveLength(10));
    expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(1);

    rerender({ loadAll: true });
    await waitFor(() => expect(result.current.sessions).toHaveLength(11));
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(2, {
      query: { limit: SIDEBAR_SESSION_LOAD_ALL_CAP - 10, cursor: 'c2' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });
});

/**
 * Vision V1-H2. Published page by page, a name-sorted view reshuffled its top
 * rows with every arrival, under the pointer, for the minute and a half 2,000
 * chats took. The pages are read into a buffer now and the list changes once,
 * when the last one lands, carrying whatever happened to those rows meanwhile.
 */
describe('loadAll shows the whole list at once', () => {
  const page = (from: number, count: number) =>
    Array.from({ length: count }, (_, index) => makeSummary(from + index));

  function deferred<T>() {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((done) => {
      resolve = done;
    });
    return { promise, resolve };
  }

  it('keeps the first page on screen until the last page has landed', async () => {
    const second = deferred<unknown>();
    const third = deferred<unknown>();
    mocks.listSidebarSessions
      .mockResolvedValueOnce({ data: { sessions: page(0, 50), has_more: true, next_cursor: 'c2' } })
      .mockReturnValueOnce(second.promise)
      .mockReturnValueOnce(third.promise);

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2));
    expect(result.current.sessions).toHaveLength(50);
    expect(result.current.isLoadingAll).toBe(true);

    await act(async () => {
      second.resolve({ data: { sessions: page(50, 50), has_more: true, next_cursor: 'c3' } });
    });
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(3));
    // The second page is read, not shown: the rows on screen have not moved.
    expect(result.current.sessions).toHaveLength(50);
    expect(result.current.isLoadingAll).toBe(true);

    await act(async () => {
      third.resolve({ data: { sessions: page(100, 7), has_more: false, next_cursor: null } });
    });
    await waitFor(() => expect(result.current.sessions).toHaveLength(107));
    expect(result.current.isLoadingAll).toBe(false);
    expect(result.current.sessions.map((session) => session.id)).toEqual(
      page(0, 107).map((session) => session.id)
    );
  });

  it('carries a rename and a removal that land while the pages are being read', async () => {
    const second = deferred<unknown>();
    mocks.listSidebarSessions
      .mockResolvedValueOnce({ data: { sessions: page(0, 2), has_more: true, next_cursor: 'c2' } })
      .mockReturnValueOnce(second.promise)
      .mockResolvedValueOnce({
        data: { sessions: page(4, 2), has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2));
    await act(async () => {
      second.resolve({ data: { sessions: page(2, 2), has_more: true, next_cursor: 'c3' } });
    });
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(3));

    // Both rows of the second page are read and not yet shown when these land.
    act(() => {
      announceSessionName({
        sessionId: 'session-2',
        name: 'Renamed mid-read',
        userSetName: true,
        origin: 'user',
      });
      notifySessionListChanged({ removed: 'session-3' });
    });

    await waitFor(() => expect(result.current.isLoadingAll).toBe(false));
    expect(result.current.sessions.map((session) => session.id)).toEqual([
      'session-0',
      'session-1',
      'session-2',
      'session-4',
      'session-5',
    ]);
    expect(result.current.sessions[2].name).toBe('Renamed mid-read');
  });

  it('runs a head refresh asked for mid-read once the read is done', async () => {
    const second = deferred<unknown>();
    mocks.listSidebarSessions
      .mockResolvedValueOnce({ data: { sessions: page(0, 2), has_more: true, next_cursor: 'c2' } })
      .mockReturnValueOnce(second.promise)
      .mockResolvedValueOnce({
        data: { sessions: [makeSummary(99), ...page(0, 2)], has_more: false, next_cursor: null },
      });

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2));

    // A new chat elsewhere: the refresh is debounced 250ms and finds the read running.
    act(() => notifySessionListChanged());
    await act(() => new Promise((resolve) => setTimeout(resolve, 300)));
    expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2);

    await act(async () => {
      second.resolve({ data: { sessions: page(2, 2), has_more: false, next_cursor: null } });
    });
    await waitFor(() => expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(3));
    expect(mocks.listSidebarSessions).toHaveBeenLastCalledWith({
      query: { limit: 50 },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
    await waitFor(() =>
      expect(result.current.sessions.map((session) => session.id)).toContain('session-99')
    );
  });

  it('stops at the cap exactly, asking the last page for only the rows left (V1-L9)', async () => {
    let served = 0;
    mocks.listSidebarSessions.mockImplementation(
      async ({ query }: { query: { limit: number; cursor?: string } }) => {
        // An older daemon: at most 50 a page, whatever was asked. The head is a
        // short page, so pages of 50 would land 20 rows past the cap.
        const count = query.cursor ? Math.min(query.limit, 50) : 30;
        const sessions = page(served, count);
        served += count;
        return { data: { sessions, has_more: true, next_cursor: `c${served}` } };
      }
    );

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(result.current.isLoadingAll).toBe(false), { timeout: 5_000 });
    expect(result.current.sessions).toHaveLength(SIDEBAR_SESSION_LOAD_ALL_CAP);
    const limits = mocks.listSidebarSessions.mock.calls.map(
      ([request]) => (request as { query: { limit: number } }).query.limit
    );
    expect(limits[limits.length - 1]).toBe(20);
    // Every request after the head asks for exactly the room left under the cap.
    expect(limits.slice(1)).toEqual(
      Array.from(
        { length: limits.length - 1 },
        (_, index) => SIDEBAR_SESSION_LOAD_ALL_CAP - 30 - 50 * index
      )
    );
  });

  it('reads a full list from a current daemon in two requests, the cap exactly', async () => {
    mocks.listSidebarSessions.mockImplementation(
      async ({ query }: { query: { limit: number; cursor?: string } }) => {
        const from = query.cursor ? 50 : 0;
        return {
          data: {
            sessions: page(from, query.limit),
            has_more: true,
            next_cursor: `c${from + query.limit}`,
          },
        };
      }
    );

    const { result } = renderHook(() => useSidebarSessions({ loadAll: true }));
    await waitFor(() => expect(result.current.isLoadingAll).toBe(false));
    expect(result.current.sessions).toHaveLength(SIDEBAR_SESSION_LOAD_ALL_CAP);
    expect(mocks.listSidebarSessions).toHaveBeenCalledTimes(2);
  });
});
