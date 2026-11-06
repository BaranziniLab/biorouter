/**
 * "Show subagent runs" must show the runs it asked for.
 *
 * Measured in the running app: History opened on the newest page (`include_subagents=false&limit=40`), the person pressed the
 * toggle, and `GET /sessions?include_subagents=true` came back with every run and its parent. The
 * view's rows held all of them (5,769, 225 of them runs), yet no run was drawn, nested or not, for
 * as long as the pointer rested on the list.
 *
 * The cause was the pointer hold (`historyHold.ts`, principle 9). The toggle starts a new hold, so
 * that the person's own change is not withheld, but it took that hold at the press, from the rows
 * the list held BEFORE the read the press started: every parent held with no runs. When the runs
 * arrived they were new runs under held parents, which a hold keeps waiting, so they never showed
 * until the pointer left. A keyless refresh from another surface (Home, the tab strip) changed
 * nothing: it kept the cache's identity and the hold kept withholding.
 *
 * These pin the sequence with the pointer inside, which the toggle tests elsewhere never had, and
 * that an answer to an earlier press never replaces the answer to the latest one.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import {
  SESSION_LIST_HEAD_SIZE,
  clearSessionListCache,
  refreshSessionList,
  updateCachedSessionList,
} from '../../utils/sessionListCache';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({ listSessions: vi.fn() }));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const NOW = Date.now();
const chat = (id: string, name: string, minutesAgo: number, over: Partial<Session> = {}) =>
  ({
    id,
    name,
    working_dir: '/Users/someone/cohort-study',
    created_at: new Date(NOW - minutesAgo * 60_000).toISOString(),
    updated_at: new Date(NOW - minutesAgo * 60_000).toISOString(),
    extension_data: {},
    message_count: 2,
    ...over,
  }) as Session;

const runOf = (id: string, name: string, minutesAgo: number) =>
  chat(id, `Subagent: ${name}`, minutesAgo, {
    session_type: 'sub_agent',
    parent_session_id: 'p1',
  });

const parent = chat('p1', 'Cohort review', 5, { session_type: 'user' });
const other = chat('c2', 'Literature notes', 20, { session_type: 'user' });
/** What a read without the runs answers: the head page and the full read alike. */
const CHATS = [parent, other];
/** What `include_subagents=true` answers: the runs, and the parent they nest under. */
const WITH_RUNS = [parent, runOf('r1', 'Run alpha', 6), runOf('r2', 'Run beta', 7), other];

type Query = { include_subagents?: boolean; limit?: number };
type Answer = { data: { sessions: Session[]; has_more?: boolean } };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

/** Each read answers as the daemon would for its query. */
function answerByQuery() {
  mocks.listSessions.mockImplementation(async ({ query }: { query: Query }) =>
    query.include_subagents
      ? { data: { sessions: WITH_RUNS } }
      : { data: { sessions: CHATS, has_more: query.limit !== undefined } }
  );
}

const queryOf = (call: number): Query => mocks.listSessions.mock.calls[call][0].query;
const contentLayer = () => document.querySelector('[data-history-layer="content"]') as HTMLElement;
const toggle = () => screen.getByLabelText(/show subagent runs/i);

/** The runs drawn, by title, each with whether it sits in a parent's block. */
const drawnRuns = () =>
  ['Run alpha', 'Run beta'].flatMap((title) => {
    const row = screen
      .queryByRole('button', { name: `Open chat ${title}` })
      ?.closest('[data-session-id]');
    return row ? [`${title}${row.closest('.br-history-children') ? ' (nested)' : ''}`] : [];
  });

const BOTH_NESTED = ['Run alpha (nested)', 'Run beta (nested)'];

/**
 * History opened warm on the newest page, as in the app: the sidebar's preload filled the cache
 * with a read that had no runs, and the view laid the head page over it.
 */
async function openHistory() {
  updateCachedSessionList(CHATS);
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('Cohort review');
  await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
  expect(queryOf(0)).toEqual({ include_subagents: false, limit: SESSION_LIST_HEAD_SIZE });
}

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

describe('the subagent toggle under a resting pointer', () => {
  it('shows the runs it read, keeps them through a keyless refresh, and hides them again', async () => {
    answerByQuery();
    await openHistory();

    // The pointer rests on the list, and the toggle is pressed without it moving (a keyboard
    // press, or a click while the cursor's hold stands).
    fireEvent.pointerEnter(contentLayer());
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-pressed', 'true');

    // The whole listing with the runs, asked for with the person's proof.
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(
        expect.objectContaining({
          headers: { 'X-User-Action': 'test-proof' },
          query: { include_subagents: true },
        })
      )
    );
    await waitFor(() => expect(drawnRuns()).toEqual(BOTH_NESTED));

    // Another surface re-reads the list with no opinion of its own (Home's recents, the tab
    // strip): the cache keeps its identity, and the runs stay.
    const calls = mocks.listSessions.mock.calls.length;
    await act(async () => {
      await refreshSessionList();
    });
    expect(queryOf(calls)).toEqual({ include_subagents: true });
    expect(drawnRuns()).toEqual(BOTH_NESTED);

    // A turn's end patching a row emits too; nothing moves and nothing drops out.
    act(() =>
      updateCachedSessionList((rows) =>
        rows.map((row) => (row.id === 'c2' ? { ...row, message_count: 4 } : row))
      )
    );
    expect(drawnRuns()).toEqual(BOTH_NESTED);

    // Pressed again, pointer still inside: the runs go at once and stay gone.
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-pressed', 'false');
    expect(drawnRuns()).toEqual([]);
    await waitFor(() =>
      expect(mocks.listSessions).toHaveBeenLastCalledWith(
        expect.objectContaining({
          headers: { 'X-User-Action': 'test-proof' },
          query: { include_subagents: false },
        })
      )
    );
    await act(async () => {});
    expect(drawnRuns()).toEqual([]);
    expect(screen.getByText('Cohort review')).toBeInTheDocument();
  });

  it('lets the runs in when a hold was taken while their read was out', async () => {
    answerByQuery();
    await openHistory();

    const read = deferred<Answer>();
    mocks.listSessions.mockImplementationOnce(() => read.promise);
    fireEvent.click(toggle());
    await waitFor(() => expect(queryOf(1)).toEqual({ include_subagents: true }));

    // The pointer comes in while the read is out: the hold it takes has no runs to hold.
    fireEvent.pointerEnter(contentLayer());
    expect(drawnRuns()).toEqual([]);

    await act(async () => read.resolve({ data: { sessions: WITH_RUNS } }));
    await waitFor(() => expect(drawnRuns()).toEqual(BOTH_NESTED));
  });
});

describe('the subagent toggle pressed in quick succession', () => {
  it('never lets the answer to an earlier press replace the answer to the latest', async () => {
    answerByQuery();
    await openHistory();

    // Every read from here on waits until the test answers it.
    const reads: Array<{ query: Query; answer: ReturnType<typeof deferred<Answer>> }> = [];
    mocks.listSessions.mockImplementation(({ query }: { query: Query }) => {
      const answer = deferred<Answer>();
      reads.push({ query, answer });
      return answer.promise;
    });

    // On, off, on: three reads, the first two superseded by the third.
    fireEvent.click(toggle());
    fireEvent.click(toggle());
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-pressed', 'true');
    await waitFor(() => expect(reads).toHaveLength(3));
    expect(reads.map((read) => read.query)).toEqual([
      { include_subagents: true },
      { include_subagents: false },
      { include_subagents: true },
    ]);

    // They land out of order: the first, then the latest, then the "off" read last.
    await act(async () => reads[0].answer.resolve({ data: { sessions: WITH_RUNS } }));
    await act(async () => reads[2].answer.resolve({ data: { sessions: WITH_RUNS } }));
    await waitFor(() => expect(drawnRuns()).toEqual(BOTH_NESTED));

    await act(async () => reads[1].answer.resolve({ data: { sessions: CHATS } }));
    await act(async () => {});
    expect(drawnRuns()).toEqual(BOTH_NESTED);
  });
});
