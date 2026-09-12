import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import {
  clearSessionListCache,
  subscribeSessionRemoved,
  updateCachedSessionList,
} from '../../utils/sessionListCache';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  deleteSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: mocks.deleteSession,
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: mocks.updateSessionName,
  declassifySession: mocks.declassifySession,
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

// The proof the desktop sends. Since issue #56's QA sweep (2026-09-10) the
// daemon answers a request without it as a public model — private chats and
// knowledge bases omitted or refused — so each call here must carry it.
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../ui/ConfirmationModal', () => ({
  ConfirmationModal: ({ isOpen, onConfirm }: { isOpen: boolean; onConfirm: () => void }) =>
    isOpen ? <button onClick={onConfirm}>Confirm deletion</button> : null,
}));

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
});

function row(overrides: Partial<Session> & { id: string; name: string }): Session {
  return {
    working_dir: '/tmp',
    created_at: '2026-07-14T12:00:00Z',
    updated_at: '2026-07-14T12:00:00Z',
    extension_data: {},
    message_count: 2,
    ...overrides,
  } as Session;
}

describe('SessionListView loading and cache', () => {
  it('shows a heatmap-inspired row animation while the first history request is pending', async () => {
    let finishRequest: ((value: { data: { sessions: never[] } }) => void) | undefined;
    mocks.listSessions.mockReturnValue(
      new Promise((resolve) => {
        finishRequest = resolve;
      })
    );

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    expect(screen.getByRole('status', { name: 'Loading chat history' })).toBeInTheDocument();
    expect(screen.getAllByTestId('history-loading-row')).toHaveLength(9);
    expect(
      screen
        .getAllByTestId('history-loading-row')[0]
        .querySelector('.biorouter-history-loading-cell')
    ).toBeInTheDocument();

    await act(async () => {
      finishRequest?.({ data: { sessions: [] } });
    });
    await waitFor(() =>
      expect(screen.queryByRole('status', { name: 'Loading chat history' })).not.toBeInTheDocument()
    );
  });

  it('renders cached history immediately while a return visit revalidates in the background', async () => {
    const session = {
      id: 'session-1',
      name: 'Cached conversation',
      created_at: '2026-07-14T12:00:00Z',
      updated_at: '2026-07-14T12:00:00Z',
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    };
    mocks.listSessions.mockResolvedValueOnce({ data: { sessions: [session] } });

    const firstVisit = render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Cached conversation');
    firstVisit.unmount();

    let finishRefresh: ((value: { data: { sessions: Array<typeof session> } }) => void) | undefined;
    mocks.listSessions.mockReturnValueOnce(
      new Promise((resolve) => {
        finishRefresh = resolve;
      })
    );

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    expect(screen.getByText('Cached conversation')).toBeInTheDocument();
    expect(screen.queryByRole('status', { name: 'Loading chat history' })).not.toBeInTheDocument();
    // The revalidation leaves one async hop after mount — it waits for the
    // user's proof, which it must carry — so it is awaited rather than assumed.
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'test-proof' } })
    );

    await act(async () => {
      finishRefresh?.({ data: { sessions: [session] } });
    });
  });

  it('limits the first history render to sixteen sessions', async () => {
    const sessions = Array.from({ length: 17 }, (_, index) => ({
      id: `session-${index}`,
      name: `Conversation ${index + 1}`,
      created_at: new Date(Date.UTC(2026, 6, 14 - index, 12)).toISOString(),
      updated_at: new Date(Date.UTC(2026, 6, 14 - index, 12)).toISOString(),
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    }));
    mocks.listSessions.mockResolvedValue({ data: { sessions } });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Conversation 1');
    expect(screen.getByText('Conversation 16')).toBeInTheDocument();
    expect(screen.queryByText('Conversation 17')).not.toBeInTheDocument();
    expect(screen.getByText('Loading more chats...')).toBeInTheDocument();
  });
});

describe('SessionListView empty state', () => {
  it('explains where chats appear and offers useful next steps', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const title = await screen.findByRole('heading', { name: 'No chats yet' });
    const emptyState = title.closest('section');

    expect(emptyState).toHaveAccessibleDescription(
      'Past chats will appear here after you start chatting. You can also import an existing chat.'
    );
    expect(screen.getByRole('button', { name: 'Start a chat' })).toBeInTheDocument();
    // Scoped: the page header carries an Import chat button too, and both now
    // share one name because they are one action. Before the rename they were
    // 'Import Session' and 'Import session', which only differed by case.
    expect(
      within(emptyState as HTMLElement).getByRole('button', { name: 'Import chat' })
    ).toBeInTheDocument();
  });
});

describe('SessionListView row actions', () => {
  it('shows three 32px row actions and keeps the destructive one in the overflow', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          {
            id: 'session-1',
            name: 'Example session',
            created_at: '2026-07-14T12:00:00Z',
            updated_at: '2026-07-14T12:00:00Z',
            extension_data: {},
            message_count: 3,
            working_dir: '/Users/wgu/Desktop',
          },
        ],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    // §3.10 — every visible row action is one 32px icon button. This assertion
    // is what ended the 28-vs-32px fork between the outlined trio and delete.
    const visibleActions = await Promise.all([
      screen.findByRole('button', { name: 'Edit Example session' }),
      screen.findByRole('button', { name: 'Export Example session' }),
      screen.findByRole('button', { name: 'More actions for Example session' }),
    ]);

    for (const action of visibleActions) {
      expect(action).toHaveClass('h-8', 'w-8', 'border');
      expect(action).not.toHaveAttribute('title');
    }

    // Destructive actions live only in the overflow, so Delete must NOT be one
    // of the buttons a stray click can reach.
    expect(screen.queryByRole('button', { name: 'Delete Example session' })).toBeNull();
  });

  it('offers Open in new tab above Open in new window, on the row-click path', async () => {
    const user = userEvent.setup();
    const onSelectSession = vi.fn();
    // A name of its own, so the query names the row this case rendered and
    // cannot also read as another case's.
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [row({ id: 'session-1', name: 'A launchable session' })] },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={onSelectSession} />
      </MemoryRouter>
    );

    await user.click(
      await screen.findByRole('button', { name: 'More actions for A launchable session' })
    );

    const items = (await screen.findAllByRole('menuitem')).map((el) => el.textContent);
    expect(items.indexOf('Open in new tab')).toBeGreaterThanOrEqual(0);
    expect(items.indexOf('Open in new tab')).toBeLessThan(items.indexOf('Open in new window'));

    await user.click(screen.getByRole('menuitem', { name: 'Open in new tab' }));
    expect(onSelectSession).toHaveBeenCalledWith('session-1');
  });

  it('uses the shared notification surface after deleting a session', async () => {
    const user = userEvent.setup();
    const session = {
      id: 'session-1',
      name: 'A session name long enough to exercise notification wrapping',
      created_at: '2026-07-14T12:00:00Z',
      updated_at: '2026-07-14T12:00:00Z',
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    };
    mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await user.click(
      await screen.findByRole('button', { name: `More actions for ${session.name}` })
    );
    await user.click(await screen.findByRole('menuitem', { name: `Delete ${session.name}` }));
    fireEvent.click(await screen.findByRole('button', { name: 'Confirm deletion' }));

    await waitFor(() =>
      expect(mocks.toastSuccess).toHaveBeenCalledWith({
        title: 'Chat deleted',
        msg: `"${session.name}" was removed from chat history.`,
      })
    );
    expect(mocks.deleteSession).toHaveBeenCalledWith({
      path: { session_id: session.id },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });

  /**
   * M11. The row left History, the tab strip and the database at once; only the
   * sidebar Recents entry survived, clearing on a renderer reload. Delete was
   * the one membership mutation that never announced itself on the list
   * channel — create, diverge and import all do — and Recents reads a different
   * endpoint and merges its re-reads, so it could not discover the removal on
   * its own.
   */
  it('announces the removal by id on the session-list channel', async () => {
    const user = userEvent.setup();
    const session = {
      id: 'session-1',
      name: 'Deleted chat',
      created_at: '2026-07-14T12:00:00Z',
      updated_at: '2026-07-14T12:00:00Z',
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    };
    mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });
    const removed: string[] = [];
    const unsubscribe = subscribeSessionRemoved((id) => removed.push(id));

    try {
      render(
        <MemoryRouter>
          <SessionListView onSelectSession={vi.fn()} />
        </MemoryRouter>
      );

      await user.click(
        await screen.findByRole('button', { name: `More actions for ${session.name}` })
      );
      await user.click(await screen.findByRole('menuitem', { name: `Delete ${session.name}` }));
      fireEvent.click(await screen.findByRole('button', { name: 'Confirm deletion' }));

      await waitFor(() => expect(removed).toEqual(['session-1']));
    } finally {
      unsubscribe();
    }
  });

  it('the Show-subagent-runs toggle refetches with include_subagents and nests children', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          {
            id: 'p1',
            session_type: 'user',
            name: 'Parent',
            working_dir: '/tmp',
            created_at: '2026-07-14T12:00:00Z',
            updated_at: '2026-07-14T12:00:00Z',
            extension_data: {},
            message_count: 3,
          },
          {
            id: 'c1',
            session_type: 'sub_agent',
            parent_session_id: 'p1',
            name: 'Subagent task',
            working_dir: '/tmp',
            created_at: '2026-07-14T12:00:00Z',
            updated_at: '2026-07-14T12:00:00Z',
            extension_data: {},
            message_count: 2,
          },
        ],
      },
    });
    // The component calls `useNavigate()`, so it MUST be inside a router, and
    // `onSelectSession` is a required prop. This is the exact shape every other
    // case in this file uses.
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    const toggle = await screen.findByLabelText(/show subagent runs/i);
    fireEvent.click(toggle);
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(
        expect.objectContaining({ query: { include_subagents: true } })
      )
    );
    // Nested, not merely present: the child sits inside the indented wrapper
    // and carries the badge, so the row reads as belonging to 'Parent'. The
    // wait is for the refetch the toggle kicked off; re-querying inside it
    // keeps the assertion reading the tree as it stands on each attempt.
    await waitFor(() => {
      const childRow = screen.getByText('Subagent task').closest('.ml-6');
      expect(childRow).not.toBeNull();
      // The badge belongs to the row, not above it: a bare inline span placed
      // before a block-level row inside a flex column renders on its own line.
      expect(
        screen
          .getByText('Subagent task')
          .closest('.biorouter-list-row')
          ?.querySelector('[data-testid="subagent-badge"]')
      ).not.toBeNull();
    });
  });

  // BR-71: `groupSessionsByDate` buckets on `updated_at`, and a parent's
  // `updated_at` advances every time the conversation is resumed. Grouping by
  // parent INSIDE each date bucket therefore drops any subagent that ran on an
  // earlier day back to top level — the confusing artifact the feature exists
  // to remove. Parent grouping has to run first.
  it('nests a subagent run under its parent across date buckets', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: 'p1',
            name: 'Parent',
            session_type: 'user',
            updated_at: '2026-07-14T12:00:00Z',
          }),
          row({
            id: 'c1',
            name: 'Subagent task',
            session_type: 'sub_agent',
            parent_session_id: 'p1',
            updated_at: '2026-07-10T12:00:00Z',
          }),
        ],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(
        expect.objectContaining({ query: { include_subagents: true } })
      )
    );

    await waitFor(() => expect(screen.getByText('Subagent task').closest('.ml-6')).not.toBeNull());
    // The child rides in its parent's bucket, so its own date never opens one.
    expect(screen.queryByText(/July 10/)).not.toBeInTheDocument();
  });

  it('badges a subagent run whose parent is not in the list', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: 'c9',
            name: 'Orphan run',
            session_type: 'sub_agent',
            parent_session_id: 'deleted-parent',
          }),
        ],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));

    // An orphan stays top-level so it is still reachable — but unbadged it is
    // an unexplained bare row, which is the same confusion in another form.
    await waitFor(() =>
      expect(
        screen
          .getByText('Orphan run')
          .closest('.biorouter-list-row')
          ?.querySelector('[data-testid="subagent-badge"]')
      ).not.toBeNull()
    );
  });

  // BR-71: `showSubagents` is per-component but the session cache it reads is
  // module-global, so a second History pane (or Home) can publish subagent rows
  // into a pane whose own toggle is off. The toggle governs what is FETCHED;
  // each pane still has to say what it will SHOW.
  it('never paints subagent runs from a warm shared cache while the toggle is off', () => {
    mocks.listSessions.mockReturnValue(new Promise(() => {}));
    updateCachedSessionList([
      row({ id: 'p1', name: 'Parent', session_type: 'user' }),
      row({ id: 'c1', name: 'Subagent task', session_type: 'sub_agent', parent_session_id: 'p1' }),
    ]);

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    expect(screen.getByText('Parent')).toBeInTheDocument();
    expect(screen.queryByText('Subagent task')).not.toBeInTheDocument();
  });

  it('does not adopt subagent runs another pane pushed into the shared cache', async () => {
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [row({ id: 'p1', name: 'Parent', session_type: 'user' })] },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Parent');

    // A sibling pane with its own toggle ON refetched and republished the list.
    act(() => {
      updateCachedSessionList([
        row({ id: 'p1', name: 'Parent', session_type: 'user' }),
        row({
          id: 'c1',
          name: 'Subagent task',
          session_type: 'sub_agent',
          parent_session_id: 'p1',
        }),
        row({ id: 'p2', name: 'Another chat', session_type: 'user' }),
      ]);
    });

    // Waiting on the sibling row proves the push landed, so the negative
    // assertion below cannot pass merely because the update had not flushed.
    await screen.findByText('Another chat');
    expect(screen.queryByText('Subagent task')).not.toBeInTheDocument();
  });

  it('uses the shared notification surface after editing a session', async () => {
    const session = {
      id: 'session-1',
      name: 'Original session name',
      created_at: '2026-07-14T12:00:00Z',
      updated_at: '2026-07-14T12:00:00Z',
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    };
    mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole('button', { name: `Edit ${session.name}` }));
    fireEvent.change(await screen.findByPlaceholderText('Enter chat description'), {
      target: { value: 'Updated session name' },
    });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(mocks.toastSuccess).toHaveBeenCalledWith({
        title: 'Chat updated',
        msg: 'Description saved.',
      })
    );
    expect(mocks.updateSessionName).toHaveBeenCalledWith({
      path: { session_id: session.id },
      body: { name: 'Updated session name' },
      headers: { 'X-User-Action': 'test-proof' },
      throwOnError: true,
    });
  });
});

// This block deliberately never names the badge component: Task 27's gate greps
// src/components for that name and expects an exact file list.
describe('SessionListView privacy markers', () => {
  it('marks a private conversation in the list', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [row({ id: 'session-1', name: 'Patient cohort', privacy_tier: 'private' })],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Patient cohort');
    expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-privacy', 'private');
  });

  it('leaves public conversations unmarked, so the marker keeps meaning something', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({ id: 'session-1', name: 'Patient cohort', privacy_tier: 'private' }),
          row({ id: 'session-2', name: 'Public notes', privacy_tier: 'public' }),
          row({ id: 'session-3', name: 'Untiered chat' }),
        ],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Patient cohort');
    // All three rows carry a glyph; exactly one carries the private tier. The
    // marker is WHICH glyph, not whether one is present — and the untiered row
    // must read as unmarked rather than inherit the private one.
    const tiers = screen
      .getAllByTestId('chat-kind-icon')
      .map((g) => g.getAttribute('data-privacy'));
    expect(tiers).toHaveLength(3);
    expect(tiers.filter((t) => t === 'private')).toHaveLength(1);
  });
});

// v1.89.0 visual review, D1. jsdom has no layout engine and never runs
// Tailwind, so nothing here can measure the overlap that made these figures
// illegible — `29,988,671` is 72px wide and the box was a fixed 48px, so the
// last digits painted over the puzzle icon on every row carrying an estimate.
// What IS testable is the property that caused it: a hard `width` on a span
// whose content the app does not control. These pin that no count in the
// cluster is given one, which is what a "just make the box bigger" fix would
// re-introduce with a new cliff a few digits further out.
describe('SessionListView stat column sizing', () => {
  const statSpan = (text: string) =>
    screen.getAllByText(text).find((el) => el.tagName === 'SPAN' && el.className.includes('w-'));

  it('floors the count boxes instead of clipping them', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: 'session-1',
            name: 'Long-running analysis',
            message_count: 12345,
            // Five extensions and a 29.9M-token history: the row from the
            // review, and the shape every fixed width in this cluster failed on.
            extension_data: {
              'enabled_extensions.v0': {
                extensions: [
                  { name: 'developer' },
                  { name: 'memory' },
                  { name: 'knowledge' },
                  { name: 'computercontroller' },
                  { name: 'autovisualiser' },
                ],
              },
            },
            accumulated_total_tokens: 29_988_671,
          } as never),
        ],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Long-running analysis');

    for (const text of ['12345', '29,988,671', '5']) {
      const span = statSpan(text);
      expect(span, `no stat span rendered for ${text}`).toBeDefined();
      // A floor keeps the column; a fixed width is the clip.
      expect(span!.className).toMatch(/\bmin-w-\d/);
      expect(span!.className).not.toMatch(/(^|\s)w-\d/);
    }
  });
});

/// A working directory is a PATH, and this list is one click from
/// SessionHistoryView, whose header draws the same value beside the same
/// Folder glyph at the same size and colour — in `font-mono`. Body font here
/// made a path change typeface purely by being navigated to. The sidebar's
/// RecentChats tooltip, SharedSessionView and SessionItem all use mono too.
///
/// jsdom never runs Tailwind, so asserting a computed font would pass whatever
/// the class says. This asserts the CLASS, and walks the ancestors because
/// `font-mono` on a parent is inherited — the way this would regress without
/// the element itself being touched.
describe('SessionListView — the working directory is a path, so it is monospace', () => {
  it('sets the row working directory in monospace, not the body font', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [row({ id: 'session-1', name: 'Analysis', working_dir: '/Users/wgu/data' })],
      },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Analysis');
    const dir = screen.getByText('/Users/wgu/data');
    expect(dir.className).toMatch(/font-mono/);
  });
});

// v1.89.0 visual review, D4. The bare `<input type="checkbox">` this replaces
// computed `appearance: auto` / `accent-color: auto`, so it painted as the macOS
// system blue in light mode and a bare white square in dark — the only unstyled
// control found anywhere in the sweep. jsdom cannot see either rendering; what
// it can see is whether the app draws its own box, which is the thing that
// stopped the OS from drawing one.
describe('SessionListView subagent toggle', () => {
  it('uses the design-system checkbox rather than the OS control', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const input = await screen.findByLabelText(/show subagent runs/i);
    // `appearance-none opacity-0` is what hands the painting to the app's own
    // box. A revert to the native control fails here, and the label association
    // below keeps the accessible control from being replaced by a decorative div.
    // (It read `sr-only` until the primitive stopped hiding its input in a 1px
    // corner — see `Checkbox.tsx`: that shape made the visible square a picture
    // unless a call site remembered to wrap it in a label.)
    expect(input).toHaveClass('appearance-none', 'opacity-0');
    expect(input).toHaveAttribute('type', 'checkbox');

    fireEvent.click(input);
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(
        expect.objectContaining({ query: { include_subagents: true } })
      )
    );
  });
});

/**
 * Chat history sits on the CHAT measure, not the fluid page measure (operator
 * decision, 2026-09-07): a row is a title on the left and a stats cluster on
 * the right, so page width landed between the two rather than showing more, and
 * a row click opens the live chat — which is already on this measure.
 *
 * ⚠ **jsdom can see the ATTRIBUTE and the COUNT, and nothing else.** There is
 * no layout engine and Tailwind never runs here, so `max-w-measure-chat`
 * computes to nothing and a column's `getBoundingClientRect()` is zero whatever
 * the size says; the widths themselves were measured in a browser against the
 * built stylesheet (760px column, 704px rows, at 1048 / 1280 / 1680). The case
 * this file cannot see at all — a `<ReadableContent` written with no `size`
 * prop, which silently DEFAULTS to the page measure — is closed at the source
 * by `styles/measures.test.ts`.
 */
describe('SessionListView sits on the chat measure', () => {
  it('renders both reading columns at the chat size, neither left on the default', async () => {
    const { container } = render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Chat history');

    const columns = [...container.querySelectorAll('.biorouter-readable-content')];
    // The count is pinned as well as the size: the header and the scrolling
    // body meet at one left edge under a full-bleed hairline, so a third column
    // added on the default size would be a visible step in that edge rather
    // than an invisible inconsistency.
    expect(columns).toHaveLength(2);
    for (const column of columns) {
      expect((column as HTMLElement).dataset.size).toBe('chat');
    }
  });

  /**
   * Import chat moved OFF the title row (operator decision, 2026-09-07): the
   * page's actions go on their own line under the description, which is what
   * §4.2 now says and what Workflows / Extensions / Skills / Built apps already
   * did. Pinned at this call site as well as in `PageHeader.test.tsx`, because
   * the primitive can be correct while a view has grown a second header of its
   * own beside it.
   *
   * The title's `min-w-0 truncate` went with the button: it existed only so the
   * heading would give way to the control sharing its row, and nothing shares
   * that row now.
   */
  it('puts Import chat on its own line under the description', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const heading = await screen.findByRole('heading', { level: 1, name: 'Chat history' });
    expect(heading.parentElement?.querySelector('button')).toBeNull();
    expect(heading.className).not.toMatch(/\btruncate\b/);

    const strip = screen
      .getAllByRole('button', { name: 'Import chat' })[0]
      .closest('.biorouter-settings-control-strip');
    expect(strip).not.toBeNull();
  });

  /**
   * The subagent filter is `PageHeader`'s `children`, not a second `actions`
   * entry: a control that changes what the list SHOWS is not an action the page
   * offers, and sharing the strip with Import chat would give the two the same
   * standing.
   */
  it('keeps the subagent filter out of the action strip', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText('Chat history');
    const checkbox = screen.getByRole('checkbox', { name: /Show subagent runs/i });
    expect(checkbox.closest('.biorouter-settings-control-strip')).toBeNull();
  });
});
