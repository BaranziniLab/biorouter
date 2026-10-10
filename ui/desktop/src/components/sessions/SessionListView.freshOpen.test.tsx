/**
 * Vision V1-M8, principle 9: Chat history must not paint the cached order and
 * then re-sort it in front of the reader.
 *
 * The cache is only as fresh as the last membership change, and opening a chat
 * in a tab moves its `updated_at` without one. Painting it on open showed one
 * chat under Today, then the first fresh page moved three more in above the
 * rows and changed the row under the pointer. Every open now waits for its
 * first fresh page; the cache is only the fallback when that fetch fails.
 */
import { act, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView, { HISTORY_SKELETON_DELAY_MS } from './SessionListView';
import { clearSessionListCache, updateCachedSessionList } from '../../utils/sessionListCache';
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
const session = (id: string, name: string, minutesAgo: number): Session =>
  ({
    id,
    name,
    working_dir: '/Users/someone/Desktop',
    created_at: new Date(NOW - minutesAgo * 60_000).toISOString(),
    updated_at: new Date(NOW - minutesAgo * 60_000).toISOString(),
    extension_data: {},
    message_count: 2,
  }) as Session;

// Cached: "Older chat" was last touched two days ago. Fresh: it was opened in a
// tab a minute ago, so the daemon now lists it first.
const recent = session('s1', 'Recent chat', 30);
const olderStale = session('s2', 'Older chat', 2 * 24 * 60);
const olderFresh = session('s2', 'Older chat', 1);

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const contentLayer = () => document.querySelector('[data-history-layer="content"]') as HTMLElement;

const rowNames = () =>
  screen
    .queryAllByRole('button', { name: /^Open chat / })
    .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

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
  // A warm cache holding the order from before the tab opened the older chat.
  updateCachedSessionList([recent, olderStale]);
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

describe('Chat history opens on its first fresh page', () => {
  it('keeps the cached order hidden and reveals the fresh one at rest', async () => {
    const page = deferred<{ data: { sessions: Session[] } }>();
    mocks.listSessions.mockReturnValue(page.promise);
    renderHistory();

    // The cached rows are not shown, and take no pointer while hidden.
    expect(contentLayer().className).toContain('opacity-0');
    expect(contentLayer().className).toContain('pointer-events-none');
    // A warm open shows no skeleton straight away: a fast answer lands with
    // no skeleton flash at all.
    expect(screen.queryAllByTestId('history-loading-row')).toHaveLength(0);

    await act(async () => page.resolve({ data: { sessions: [olderFresh, recent] } }));

    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    // The first order on screen is the fresh one; nothing re-sorts after it.
    expect(rowNames()).toEqual(['Older chat', 'Recent chat']);
    expect(screen.getByRole('region', { name: 'Today' })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: 'Previous 7 days' })).toBeNull();
  });

  it('shows the skeleton when the fresh page is slow', async () => {
    const page = deferred<{ data: { sessions: Session[] } }>();
    mocks.listSessions.mockReturnValue(page.promise);
    renderHistory();

    await waitFor(
      () => expect(screen.getAllByTestId('history-loading-row').length).toBeGreaterThan(0),
      { timeout: HISTORY_SKELETON_DELAY_MS * 10 }
    );
    expect(contentLayer().className).toContain('opacity-0');

    await act(async () => page.resolve({ data: { sessions: [olderFresh, recent] } }));
    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(rowNames()).toEqual(['Older chat', 'Recent chat']);
  });

  it('falls back to the cached rows, not an error, when the fetch fails', async () => {
    mocks.listSessions.mockRejectedValue(new Error('daemon unreachable'));
    vi.spyOn(console, 'error').mockImplementation(() => {});
    renderHistory();

    await waitFor(() => expect(contentLayer().className).toContain('opacity-100'));
    expect(rowNames()).toEqual(['Recent chat', 'Older chat']);
    expect(screen.queryByRole('button', { name: 'Try again' })).toBeNull();
  });
});
