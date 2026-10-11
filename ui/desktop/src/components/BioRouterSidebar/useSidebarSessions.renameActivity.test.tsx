/**
 * R-30, coordinator ruling 16: under "Last activity" a chat moves when
 * something happens IN it. Renaming it is that; opening it is not.
 *
 * The daemon raises a renamed chat's `updated_at` when it saves the name, but
 * the rename route answers with no body and nothing re-read the list, so the
 * sidebar kept the chat where it was until some unrelated refresh. Usually that
 * was the person opening ANOTHER chat (the open re-reads the head), which read
 * as "opening a chat moved it". These drive the sidebar's own list hook and the
 * real row list against a fake daemon that stamps a rename and never an open.
 */
import { act, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { renameSessionOptimistically } from '../../utils/sessionNameSync';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import { arrangeSidebarChats, DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';
import useSidebarSessions from './useSidebarSessions';

const daemon = vi.hoisted(() => ({
  rows: [] as SessionSummary[],
  listSidebarSessions: vi.fn(),
  updateSessionName: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSidebarSessions: daemon.listSidebarSessions,
  updateSessionName: daemon.updateSessionName,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

beforeAll(() => {
  Object.defineProperty(window, 'matchMedia', {
    writable: true,
    value: vi.fn().mockImplementation((query: string) => ({
      matches: false,
      media: query,
      onchange: null,
      addEventListener: vi.fn(),
      removeEventListener: vi.fn(),
      addListener: vi.fn(),
      removeListener: vi.fn(),
      dispatchEvent: vi.fn(),
    })),
  });
});

/** The daemon's order: `updated_at`, newest first. */
function headOf(limit = 10) {
  const sorted = [...daemon.rows].sort(
    (left, right) => Date.parse(right.updated_at) - Date.parse(left.updated_at)
  );
  return {
    data: {
      sessions: sorted.slice(0, limit).map((row) => ({ ...row })),
      has_more: sorted.length > limit,
      next_cursor: sorted.length > limit ? 'next' : null,
    },
  };
}

function chat(letter: string, minutesAgo: number): SessionSummary {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return {
    id: `session-${letter}`,
    name: `Chat ${letter}`,
    working_dir: '/workspace',
    created_at: at,
    updated_at: at,
    message_count: 2,
    user_set_name: false,
  };
}

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon.rows = [chat('a', 10), chat('b', 20), chat('c', 30)];
  daemon.listSidebarSessions.mockImplementation(async ({ query }: { query: { limit: number } }) =>
    headOf(query.limit)
  );
  // The daemon saves the name and stamps `updated_at` (a rename is activity),
  // and answers with no body, as `PUT /sessions/{id}/name` does.
  daemon.updateSessionName.mockImplementation(
    async ({ path, body }: { path: { session_id: string }; body: { name: string } }) => {
      const row = daemon.rows.find((candidate) => candidate.id === path.session_id);
      if (row) {
        row.name = body.name;
        row.user_set_name = true;
        row.updated_at = new Date().toISOString();
      }
      return { data: undefined };
    }
  );
});

afterEach(() => {
  vi.useRealTimers();
});

function SidebarList({ openSessionId }: { openSessionId: string | null }) {
  const { sessions, hasMore, isLoading, isLoadingAll, loadMore } = useSidebarSessions({
    openSessionId,
  });
  return (
    <SidebarProvider>
      <RecentChats
        sessions={sessions}
        activeSessionId={openSessionId}
        runningSessionIds={new Set()}
        hasMore={hasMore}
        isLoadingMore={isLoading}
        isLoadingAll={isLoadingAll}
        onLoadMore={loadMore}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={DEFAULT_SIDEBAR_CHAT_VIEW}
        onViewChange={vi.fn()}
      />
    </SidebarProvider>
  );
}

const order = () =>
  Array.from(document.querySelectorAll('[data-row^="chat:"]')).map((row) =>
    row.getAttribute('data-row')
  );

describe('a rename is activity, and the sidebar shows it at once', () => {
  it('moves a renamed chat to the top under Last activity without opening another chat', async () => {
    const view = render(<SidebarList openSessionId={null} />);
    await waitFor(() =>
      expect(order()).toEqual(['chat:session-a', 'chat:session-b', 'chat:session-c'])
    );
    const readsBefore = daemon.listSidebarSessions.mock.calls.length;

    // The oldest chat is renamed from its chat title, while no other chat is
    // opened and the window keeps focus.
    await act(async () => {
      await renameSessionOptimistically('session-c', 'Renamed oldest', {
        name: 'Chat c',
        userSetName: false,
      });
    });

    expect(daemon.updateSessionName).toHaveBeenCalledTimes(1);
    expect(order()).toEqual(['chat:session-c', 'chat:session-a', 'chat:session-b']);
    expect(view.getByTestId('recent-chat-session-c')).toHaveTextContent('Renamed oldest');
    // Moved from the saved rename itself, not from a re-read of the list.
    expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(readsBefore);
  });

  it('moves no row when a chat is opened, before a rename or after one', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const view = render(<SidebarList openSessionId={null} />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    const asLoaded = ['chat:session-a', 'chat:session-b', 'chat:session-c'];
    expect(order()).toEqual(asLoaded);

    // Opening the middle chat: the head is read again (250 ms, then 2 s), and
    // the daemon, which stamps nothing on an open, lists the same order.
    view.rerender(<SidebarList openSessionId="session-b" />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_500);
    });
    expect(daemon.listSidebarSessions.mock.calls.length).toBeGreaterThanOrEqual(3);
    expect(order()).toEqual(asLoaded);

    await act(async () => {
      await renameSessionOptimistically('session-c', 'Renamed oldest', {
        name: 'Chat c',
        userSetName: false,
      });
    });
    const renamedFirst = ['chat:session-c', 'chat:session-a', 'chat:session-b'];
    expect(order()).toEqual(renamedFirst);

    // Opening another chat after the rename leaves every row where it is.
    view.rerender(<SidebarList openSessionId="session-a" />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_500);
    });
    expect(order()).toEqual(renamedFirst);
  });
});

describe('the list hook keeps a saved rename where it moved it', () => {
  const firstUnderLastActivity = (sessions: SessionSummary[]) =>
    arrangeSidebarChats(sessions, DEFAULT_SIDEBAR_CHAT_VIEW)[0]?.sessions[0]?.id;

  it('brings in a renamed chat past the pages read so far', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    daemon.rows = Array.from({ length: 12 }, (_, index) =>
      chat(String.fromCharCode(97 + index), (index + 1) * 10)
    );
    const { result } = renderHook(() => useSidebarSessions());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.sessions.map((session) => session.id)).not.toContain('session-l');

    await act(async () => {
      await renameSessionOptimistically('session-l', 'Renamed far down', {
        name: 'Chat l',
        userSetName: false,
      });
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });

    expect(firstUnderLastActivity(result.current.sessions)).toBe('session-l');
    expect(result.current.sessions.find((session) => session.id === 'session-l')?.name).toBe(
      'Renamed far down'
    );
  });

  it('does not let a page read before the rename put the chat back', async () => {
    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(3));

    // A head refresh is in flight with the order from before the rename.
    let answer: (value: unknown) => void = () => {};
    const stale = headOf();
    daemon.listSidebarSessions.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          answer = resolve;
        })
    );
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await waitFor(() => expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(2), {
      timeout: 1_000,
    });

    await act(async () => {
      await renameSessionOptimistically('session-c', 'Renamed oldest', {
        name: 'Chat c',
        userSetName: false,
      });
    });
    expect(firstUnderLastActivity(result.current.sessions)).toBe('session-c');

    await act(async () => {
      answer(stale);
    });
    await waitFor(() => expect(result.current.isLoading).toBe(false));
    expect(firstUnderLastActivity(result.current.sessions)).toBe('session-c');
    expect(result.current.sessions.find((session) => session.id === 'session-c')?.name).toBe(
      'Renamed oldest'
    );
  });

  it('moves nothing for a rename the daemon refused', async () => {
    daemon.updateSessionName.mockRejectedValueOnce('This chat is private.');
    const { result } = renderHook(() => useSidebarSessions());
    await waitFor(() => expect(result.current.sessions).toHaveLength(3));

    await act(async () => {
      await renameSessionOptimistically('session-c', 'Refused', {
        name: 'Chat c',
        userSetName: false,
      });
    });

    expect(firstUnderLastActivity(result.current.sessions)).toBe('session-a');
    expect(result.current.sessions.find((session) => session.id === 'session-c')?.name).toBe(
      'Chat c'
    );
  });
});
