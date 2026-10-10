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
import { chatRowCopy } from '../chats/copy';
import {
  CHAT_DELETED,
  HISTORY_EMPTY,
  HISTORY_TITLE,
  IMPORT_CHAT,
  LOADING_MORE_CHATS,
  RENAME_PLACEHOLDER,
  RENAME_TITLE,
  SHOW_SUBAGENT_RUNS,
  START_A_CHAT,
} from './copy';

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
    expect(screen.getByText(LOADING_MORE_CHATS)).toBeInTheDocument();
  });
});

describe('SessionListView empty state', () => {
  /**
   * One sentence and one way forward. Import lives in the band, where it is on
   * every state of the page, so the empty state does not offer it twice.
   */
  it('says where chats will appear and offers one next step', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const title = await screen.findByRole('heading', { name: 'No chats yet' });
    const emptyState = title.closest('section');

    expect(emptyState).toHaveAccessibleDescription(HISTORY_EMPTY);
    expect(
      within(emptyState as HTMLElement).getByRole('button', { name: START_A_CHAT })
    ).toBeInTheDocument();
    expect(
      within(emptyState as HTMLElement).queryByRole('button', { name: IMPORT_CHAT })
    ).toBeNull();
    expect(screen.getByRole('button', { name: IMPORT_CHAT })).toBeInTheDocument();
  });
});

describe('SessionListView row actions', () => {
  it('shows one row action and the overflow, and keeps the destructive one in the overflow', async () => {
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

    // Principle 6: one icon plus `⋯`, both ghost round 32px buttons, revealed
    // together on hover or focus (the reveal is authored CSS on their cluster).
    const visibleActions = await Promise.all([
      screen.findByRole('button', { name: 'Rename Example session' }),
      screen.findByRole('button', { name: 'More actions for Example session' }),
    ]);

    for (const action of visibleActions) {
      expect(action).toHaveClass('h-control-md', 'w-control-md');
      expect(action).not.toHaveAttribute('title');
      expect(action.closest('.br-history-row-actions')).not.toBeNull();
    }

    // Export moved into the menu with the rest, and Delete must NOT be a button
    // a stray click can reach.
    expect(screen.queryByRole('button', { name: 'Export Example session' })).toBeNull();
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
    await user.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.delete }));
    fireEvent.click(await screen.findByRole('button', { name: 'Confirm deletion' }));

    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalledWith({ title: CHAT_DELETED }));
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
      await user.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.delete }));
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

  /**
   * Rename goes through the one optimistic path every surface shares: the PUT
   * carries the person's proof, and there is no success toast, because the row
   * already shows the new name.
   */
  it('renames a chat through the shared helper, with no success toast', async () => {
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
    mocks.updateSessionName.mockResolvedValue({ data: undefined });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole('button', { name: `Rename ${session.name}` }));
    expect(await screen.findByRole('dialog', { name: RENAME_TITLE })).toBeInTheDocument();
    const field = screen.getByPlaceholderText(RENAME_PLACEHOLDER);
    expect(field).toHaveValue(session.name);
    fireEvent.change(field, { target: { value: 'Updated session name' } });
    fireEvent.click(screen.getByRole('button', { name: 'Save' }));

    await waitFor(() =>
      expect(mocks.updateSessionName).toHaveBeenCalledWith({
        path: { session_id: session.id },
        body: { name: 'Updated session name' },
        headers: { 'X-User-Action': 'test-proof' },
        throwOnError: true,
      })
    );
    expect(await screen.findByText('Updated session name')).toBeInTheDocument();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(screen.queryByRole('dialog', { name: RENAME_TITLE })).toBeNull();
  });

  it('makes no call for an unchanged name', async () => {
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [row({ id: 'session-1', name: 'Same name' })] },
    });

    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    fireEvent.click(await screen.findByRole('button', { name: 'Rename Same name' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Save' }));
    await waitFor(() => expect(screen.queryByRole('dialog', { name: RENAME_TITLE })).toBeNull());
    expect(mocks.updateSessionName).not.toHaveBeenCalled();
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

/// A row shows names, not paths (principle 10). The folder a chat ran in is
/// named by its last segment, in the body face; the full path is a machine
/// string and lives in the row's tooltip (mono) and in the filter, never on the
/// row itself.
describe('SessionListView — a row names its folder, not its path', () => {
  it('shows the folder name in the body face and keeps the path off the row', async () => {
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
    const folder = screen.getByText('data');
    expect(folder).toHaveAttribute('data-working-dir', '/Users/wgu/data');
    for (let el: HTMLElement | null = folder; el; el = el.parentElement) {
      expect(el.className).not.toMatch(/font-mono/);
    }
    expect(screen.queryByText('/Users/wgu/data')).toBeNull();
  });
});

// The subagent filter is a view toggle in the band: a button that says whether
// it is on (`aria-pressed`) and is named by what it does whatever its state.
describe('SessionListView subagent toggle', () => {
  it('is a pressed-state button in the band that refetches with include_subagents', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const toggle = await screen.findByRole('button', { name: SHOW_SUBAGENT_RUNS });
    expect(toggle).toHaveAttribute('aria-pressed', 'false');
    expect(toggle.closest('[data-band]')).not.toBeNull();

    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-pressed', 'true');
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
 * a row click opens the live chat, which is already on this measure.
 *
 * ⚠ **jsdom can see the ATTRIBUTE and the COUNT, and nothing else.** There is
 * no layout engine and Tailwind never runs here. The case this file cannot see,
 * a `<ReadableContent` written with no `size`, is closed at the source by
 * `styles/measures.test.ts`.
 */
describe('SessionListView sits on the chat measure under a band', () => {
  it('renders one reading column, at the chat size, below the band', async () => {
    const { container } = render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    await screen.findByText(HISTORY_TITLE);

    // The band is full width and holds no column; only the body is measured.
    const columns = [...container.querySelectorAll('.biorouter-readable-content')];
    expect(columns).toHaveLength(1);
    expect((columns[0] as HTMLElement).dataset.size).toBe('chat');
    expect(columns[0].closest('[data-band]')).toBeNull();
  });

  /**
   * The 44px band (spec 3.4 and 3.10): the title, its help in an InfoTip, and
   * the filter and two icon actions at the trailing edge. No paragraph under
   * the title.
   */
  it('puts the title, the filter and the actions on one band', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );

    const heading = await screen.findByRole('heading', { level: 1, name: HISTORY_TITLE });
    const band = heading.closest('[data-band]') as HTMLElement;
    expect(band).not.toBeNull();
    expect(within(band).getByRole('searchbox', { name: 'Search history' })).toHaveAttribute(
      'placeholder',
      expect.stringMatching(/^Search history… (⌘F|Ctrl\+F)$/)
    );
    expect(within(band).getByRole('button', { name: IMPORT_CHAT })).toBeInTheDocument();
    expect(within(band).getByRole('button', { name: SHOW_SUBAGENT_RUNS })).toBeInTheDocument();
    expect(band.querySelector('p')).toBeNull();
  });
});
