import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import useSidebarSessions, {
  appendSessionPage,
  SIDEBAR_SESSION_PAGE_SIZE,
} from './useSidebarSessions';

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
