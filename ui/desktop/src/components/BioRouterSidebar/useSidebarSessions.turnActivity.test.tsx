/**
 * Under "Last activity" a running chat moves to the top (sidebar-chat-list.md, "Rows hold
 * still under the pointer"; R-30: running a turn IS activity).
 *
 * The final check (2026-10-12) sent a turn in the third chat and watched it for 40 s: the daemon
 * had already raised the chat's `updated_at`, the pointer was off the list, no menu or rename was
 * open, and the row stayed third with its spinner until the turn FINISHED. The list re-read on a
 * finished turn, a new chat, a session-meta change, focus and visibility, and the running set only
 * drew the spinner. While opening a chat bumped it (before R-30), the open hid this; now it shows.
 *
 * These drive the sidebar's own list hook and the real row list against a fake daemon.
 */
import { act, fireEvent, render, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { ChatState } from '../../types/chatState';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import { arrangeSidebarChats, DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';
import useSidebarSessions, { sessionsInTurn } from './useSidebarSessions';

const daemon = vi.hoisted(() => ({
  rows: [] as SessionSummary[],
  listSidebarSessions: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSidebarSessions: daemon.listSidebarSessions,
  updateSessionName: vi.fn(),
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

/** The daemon stores a message in the chat, which raises its `updated_at`. */
function daemonStamps(sessionId: string) {
  const row = daemon.rows.find((candidate) => candidate.id === sessionId);
  if (row) row.updated_at = new Date().toISOString();
}

const NO_TURNS: ReadonlySet<string> = new Set();

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  daemon.rows = [chat('a', 10), chat('b', 20), chat('c', 30)];
  daemon.listSidebarSessions.mockImplementation(async ({ query }: { query: { limit: number } }) =>
    headOf(query.limit)
  );
});

afterEach(() => {
  vi.useRealTimers();
});

function SidebarList({ turns = NO_TURNS }: { turns?: ReadonlySet<string> }) {
  const { sessions, hasMore, isLoading, isLoadingAll, loadMore } = useSidebarSessions({
    turnSessionIds: turns,
  });
  return (
    <SidebarProvider>
      <RecentChats
        sessions={sessions}
        activeSessionId={null}
        runningSessionIds={turns}
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

const asLoaded = ['chat:session-a', 'chat:session-b', 'chat:session-c'];
const cFirst = ['chat:session-c', 'chat:session-a', 'chat:session-b'];

describe('a running chat moves to the top while its turn runs', () => {
  it('moves the chat the moment its turn starts, not when the turn finishes', async () => {
    const view = render(<SidebarList />);
    await waitFor(() => expect(order()).toEqual(asLoaded));
    const readsBefore = daemon.listSidebarSessions.mock.calls.length;

    // The person sends in the oldest chat. The turn is running; nothing has finished.
    view.rerender(<SidebarList turns={new Set(['session-c'])} />);

    expect(order()).toEqual(cFirst);
    // Moved from the turn starting, without waiting on a re-read of the list.
    expect(daemon.listSidebarSessions).toHaveBeenCalledTimes(readsBefore);

    // A later render with an equal set (the turn went from thinking to streaming) is not a
    // second start, and the chat stays where it is.
    view.rerender(<SidebarList turns={new Set(['session-c'])} />);
    expect(order()).toEqual(cFirst);
  });

  it('waits while the pointer is over the list, and moves when it leaves', async () => {
    const view = render(<SidebarList />);
    await waitFor(() => expect(order()).toEqual(asLoaded));
    const list = view.getByTestId('recent-chat-scroll');

    fireEvent.pointerEnter(list);
    view.rerender(<SidebarList turns={new Set(['session-c'])} />);
    expect(order()).toEqual(asLoaded);

    fireEvent.pointerLeave(list);
    expect(order()).toEqual(cFirst);
  });

  it('keeps the chat on top over a head read the daemon answered before it stamped the turn', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const view = render(<SidebarList />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(order()).toEqual(asLoaded);

    view.rerender(<SidebarList turns={new Set(['session-c'])} />);
    expect(order()).toEqual(cFirst);

    // The window comes back to the front mid-turn, and the head page answers with the order
    // from before the daemon stored the person's message.
    const readsBefore = daemon.listSidebarSessions.mock.calls.length;
    act(() => {
      window.dispatchEvent(new Event('focus'));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(daemon.listSidebarSessions.mock.calls.length).toBeGreaterThan(readsBefore);
    expect(order()).toEqual(cFirst);
  });

  it('lets go when the turn ends, and the daemon’s order stands', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const view = render(<SidebarList />);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });

    // A turn in the second chat is refused before the daemon stores anything.
    view.rerender(<SidebarList turns={new Set(['session-b'])} />);
    expect(order()).toEqual(['chat:session-b', 'chat:session-a', 'chat:session-c']);
    view.rerender(<SidebarList />);
    act(() => {
      window.dispatchEvent(new Event('message-stream-finished'));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(order()).toEqual(asLoaded);

    // A turn in the oldest chat that the daemon stores stays on top after it ends.
    view.rerender(<SidebarList turns={new Set(['session-c'])} />);
    daemonStamps('session-c');
    view.rerender(<SidebarList />);
    act(() => {
      window.dispatchEvent(new Event('message-stream-finished'));
    });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    expect(order()).toEqual(cFirst);
  });
});

describe('the list hook and a turn in a chat it has not read yet', () => {
  const firstUnderLastActivity = (sessions: SessionSummary[]) =>
    arrangeSidebarChats(sessions, DEFAULT_SIDEBAR_CHAT_VIEW)[0]?.sessions[0]?.id;

  it('reads the head, which brings the running chat in at the top', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    daemon.rows = Array.from({ length: 12 }, (_, index) =>
      chat(String.fromCharCode(97 + index), (index + 1) * 10)
    );
    const { result, rerender } = renderHook(
      ({ turns }: { turns: ReadonlySet<string> }) => useSidebarSessions({ turnSessionIds: turns }),
      { initialProps: { turns: NO_TURNS } }
    );
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(result.current.sessions.map((session) => session.id)).not.toContain('session-l');

    // A turn starts in a chat past the first page (opened from History, say), and the daemon
    // stores the message a moment later than the first head read.
    rerender({ turns: new Set(['session-l']) });
    await act(async () => {
      await vi.advanceTimersByTimeAsync(300);
    });
    daemonStamps('session-l');
    await act(async () => {
      await vi.advanceTimersByTimeAsync(2_500);
    });

    expect(firstUnderLastActivity(result.current.sessions)).toBe('session-l');
  });
});

describe('sessionsInTurn', () => {
  const entry = (sessionId: string, chatState: ChatState, completedAt?: number) => ({
    sessionId,
    title: sessionId,
    chatState,
    startedAt: 1,
    completedAt,
  });

  it('counts a running turn, and not a finished one or an agent restart', () => {
    const turns = sessionsInTurn([
      entry('thinking', ChatState.Thinking),
      entry('streaming', ChatState.Streaming),
      entry('asking', ChatState.WaitingForUserInput),
      entry('compacting', ChatState.Compacting),
      entry('finished', ChatState.Idle, 2),
      // A folder change restarts the agent: the spinner shows, but it is not a turn, and
      // whether it stamps the chat is the daemon's to decide (R-30).
      entry('restarting', ChatState.RestartingAgent),
    ]);
    expect([...turns].sort()).toEqual(['asking', 'compacting', 'streaming', 'thinking']);
  });
});
