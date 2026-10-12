/**
 * Vision V1-R3-2: History took about a second to show its rows on every visit,
 * because the one request it made was the whole listing, every chat on the
 * machine (5,542 on the store it was measured on), to draw sixteen rows.
 *
 * It now opens on the NEWEST page (`refreshSessionListHead`), laid over the
 * cache. The page is the only thing a row waits for; the rows after it are the
 * cache's, and a cold cache is filled by a full read that does not hold the first
 * paint up. The first fresh page still decides the order (principle 9): nothing
 * is painted from the cache and then re-sorted.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import {
  SESSION_LIST_HEAD_SIZE,
  clearSessionListCache,
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
/** Chat `n`: a larger `n` is OLDER, so chat 0 is the newest. */
const chat = (n: number, extra: Partial<Session> = {}): Session =>
  ({
    id: `chat-${String(n).padStart(3, '0')}`,
    name: `Chat ${n}`,
    working_dir: '/Users/someone/Desktop',
    created_at: new Date(NOW - (n + 1) * 60_000).toISOString(),
    updated_at: new Date(NOW - (n + 1) * 60_000).toISOString(),
    extension_data: {},
    message_count: 2,
    ...extra,
  }) as Session;

const range = (from: number, to: number) =>
  Array.from({ length: to - from }, (_, index) => chat(from + index));

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((res) => {
    resolve = res;
  });
  return { promise, resolve };
}

const contentLayer = () => document.querySelector('[data-history-layer="content"]') as HTMLElement;

const rowNames = () =>
  screen
    .queryAllByRole('button', { name: /^Open chat / })
    .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

const queryOf = (call: number) => mocks.listSessions.mock.calls[call][0].query;

function renderHistory() {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

describe('History opens on the newest page', () => {
  it('asks for one page, shows it over the cache, and does not read the whole listing', async () => {
    // Warm: 30 chats cached. The newest chat ran in another window, so the page
    // says it is now first and renamed.
    updateCachedSessionList(range(0, 30));
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [chat(0, { name: 'Chat 0, moved on' }), chat(1), chat(2)],
        has_more: true,
        next_cursor: 'next',
      },
    });
    renderHistory();

    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
    expect(queryOf(0)).toEqual({ include_subagents: false, limit: SESSION_LIST_HEAD_SIZE });

    // The page's rows first, then the cache's own, in one order; nothing re-sorts.
    const names = rowNames();
    expect(names.slice(0, 4)).toEqual(['Chat 0, moved on', 'Chat 1', 'Chat 2', 'Chat 3']);
    expect(names).toHaveLength(16);
    expect(mocks.listSessions).toHaveBeenCalledTimes(1);
  });

  it('builds no rows from the cache while the page is out', async () => {
    // The cached rows are never shown on open, so building them (sixteen rows of menus
    // and tooltips) only held the request back behind the first build: 120 ms in the
    // development build before it left. Nothing is drawn until the page lands.
    updateCachedSessionList(range(0, 30));
    const pageAnswer = deferred<{ data: { sessions: Session[]; has_more: boolean } }>();
    mocks.listSessions.mockReturnValue(pageAnswer.promise);
    renderHistory();

    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(1));
    expect(
      document.querySelectorAll('[data-history-layer="content"] [data-session-id]')
    ).toHaveLength(0);

    await act(async () => pageAnswer.resolve({ data: { sessions: range(0, 3), has_more: true } }));
    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(rowNames()).toHaveLength(16);
  });

  it('shows the page without waiting for the full read a cold cache starts', async () => {
    const full = deferred<{ data: { sessions: Session[] } }>();
    mocks.listSessions
      .mockResolvedValueOnce({
        data: { sessions: range(0, 3), has_more: true, next_cursor: 'next' },
      })
      .mockReturnValueOnce(full.promise);
    renderHistory();

    // The page is up while the full read is still out.
    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(rowNames()).toEqual(['Chat 0', 'Chat 1', 'Chat 2']);
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalledTimes(2));
    expect(queryOf(1)).toEqual({ include_subagents: false });

    // When the full read lands the older chats follow the ones already shown.
    await act(async () => full.resolve({ data: { sessions: range(0, 20) } }));
    await waitFor(() => expect(rowNames()).toHaveLength(16));
    expect(rowNames().slice(0, 3)).toEqual(['Chat 0', 'Chat 1', 'Chat 2']);
  });

  it('reads the whole listing when the sub-agent runs are asked for', async () => {
    updateCachedSessionList(range(0, 5));
    mocks.listSessions.mockResolvedValue({ data: { sessions: range(0, 5) } });
    renderHistory();
    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(queryOf(0)).toEqual({ include_subagents: false, limit: SESSION_LIST_HEAD_SIZE });

    // Sub-agent runs nest under a parent that can sit anywhere in the order, so a
    // page of the newest rows is not enough to place them.
    mocks.listSessions.mockClear();
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
    await waitFor(() => expect(mocks.listSessions).toHaveBeenCalled());
    expect(queryOf(0)).toEqual({ include_subagents: true });
  });
});
