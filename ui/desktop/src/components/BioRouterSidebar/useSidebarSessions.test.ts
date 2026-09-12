import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import useSidebarSessions, {
  appendSessionPage,
  SIDEBAR_SESSION_PAGE_SIZE,
} from './useSidebarSessions';
import { notifySessionListChanged } from '../../utils/sessionListCache';

const mocks = vi.hoisted(() => ({
  listSidebarSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSidebarSessions: mocks.listSidebarSessions,
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
        data: { sessions: firstPage, has_more: true, next_offset: 10 },
      })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: false, next_offset: null },
      });

    const { result } = renderHook(() => useSidebarSessions());

    await waitFor(() => expect(result.current.sessions).toHaveLength(10));
    expect(result.current.hasMore).toBe(true);
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(1, {
      query: { limit: 10, offset: 0 },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });

    act(() => result.current.loadMore());

    await waitFor(() => expect(result.current.sessions).toHaveLength(12));
    expect(result.current.hasMore).toBe(false);
    expect(mocks.listSidebarSessions).toHaveBeenNthCalledWith(2, {
      query: { limit: 10, offset: 10 },
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
        data: { sessions: firstPage, has_more: true, next_offset: 10 },
      })
      .mockResolvedValueOnce({
        data: { sessions: secondPage, has_more: true, next_offset: 20 },
      })
      .mockResolvedValueOnce({
        data: { sessions: refreshedFirstPage, has_more: true, next_offset: 10 },
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
      query: { limit: 10, offset: 0 },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });
});

/**
 * M11. A deleted chat left History, the tab strip and the database immediately
 * and stayed in the sidebar Recents until a renderer reload.
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
describe('a deleted chat leaves Recents without a reload', () => {
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
