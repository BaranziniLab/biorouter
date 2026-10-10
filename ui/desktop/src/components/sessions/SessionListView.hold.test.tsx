/**
 * Principle 9 in Chat history: the list is live, and a chat that arrives while
 * the pointer is over it must not shift the rows beneath. The order applies the
 * moment the pointer leaves. (The arithmetic is `historyHold.test.ts`'s; this
 * pins that the view wires it to the pointer.)
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
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

const first = session('s1', 'First chat', 5);
const second = session('s2', 'Second chat', 10);

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [first, second] } });
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

const rowNames = () =>
  screen
    .getAllByRole('button', { name: /^Open chat / })
    .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

async function renderHistory() {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('First chat');
  return document.querySelector('[data-history-layer="content"]') as HTMLElement;
}

describe('SessionListView holds its order under the pointer', () => {
  it('keeps a chat that arrives at the top waiting until the pointer leaves', async () => {
    const list = await renderHistory();
    expect(rowNames()).toEqual(['First chat', 'Second chat']);

    fireEvent.pointerEnter(list);
    act(() => updateCachedSessionList([session('s3', 'Brand new chat', 0), first, second]));

    // Nothing moved under the pointer.
    expect(rowNames()).toEqual(['First chat', 'Second chat']);

    fireEvent.pointerLeave(list);
    await waitFor(() =>
      expect(rowNames()).toEqual(['Brand new chat', 'First chat', 'Second chat'])
    );
  });

  it('applies a rename at once, because it moves nothing', async () => {
    const list = await renderHistory();
    fireEvent.pointerEnter(list);
    act(() => updateCachedSessionList([first, { ...second, name: 'Second, renamed' } as Session]));

    await waitFor(() => expect(rowNames()).toEqual(['First chat', 'Second, renamed']));
  });

  it('shows the live order when the pointer was never there', async () => {
    await renderHistory();
    act(() => updateCachedSessionList([session('s3', 'Brand new chat', 0), first, second]));
    await waitFor(() =>
      expect(rowNames()).toEqual(['Brand new chat', 'First chat', 'Second chat'])
    );
  });

  it("does not withhold the result of the person's own filter", async () => {
    const list = await renderHistory();
    const filter = screen.getByRole('searchbox', { name: 'Search history' });
    fireEvent.pointerEnter(list);

    fireEvent.change(filter, { target: { value: 'second' } });
    await waitFor(() => expect(rowNames()).toEqual(['Second chat']));

    // Widening it again shows the chats it brings back, pointer still inside.
    fireEvent.change(filter, { target: { value: '' } });
    await waitFor(() => expect(rowNames()).toEqual(['First chat', 'Second chat']));
  });
});
