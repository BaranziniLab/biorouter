/**
 * R-30, coordinator ruling 16, in Chat history: renaming a chat is activity in
 * it, so it moves to the top at once; opening a chat is not, so it moves no row.
 *
 * The daemon raises a renamed chat's `updated_at`, but the rename route answers
 * with no body and History only patched the name, so the chat stayed in place
 * until something unrelated re-read the list. These run History against a fake
 * daemon that stamps a rename and never an open.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import { renameSessionOptimistically } from '../../utils/sessionNameSync';
import type { Session } from '../../api';

const daemon = vi.hoisted(() => ({
  rows: [] as Session[],
  listSessions: vi.fn(),
  updateSessionName: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: daemon.listSessions,
  updateSessionName: daemon.updateSessionName,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const session = (id: string, name: string, minutesAgo: number): Session =>
  ({
    id,
    name,
    working_dir: '/Users/someone/Desktop',
    created_at: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
    updated_at: new Date(Date.now() - minutesAgo * 60_000).toISOString(),
    extension_data: {},
    message_count: 2,
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  daemon.rows = [
    session('s1', 'Newest chat', 5),
    session('s2', 'Middle chat', 10),
    session('s3', 'Oldest chat', 15),
  ];
  // `GET /sessions`: the daemon's order, `updated_at` newest first.
  daemon.listSessions.mockImplementation(async () => ({
    data: {
      sessions: [...daemon.rows]
        .sort((left, right) => Date.parse(right.updated_at) - Date.parse(left.updated_at))
        .map((row) => ({ ...row })),
    },
  }));
  // `PUT /sessions/{id}/name`: saves the name, stamps the chat, no body.
  daemon.updateSessionName.mockImplementation(
    async ({ path, body }: { path: { session_id: string }; body: { name: string } }) => {
      const row = daemon.rows.find((candidate) => candidate.id === path.session_id);
      if (row) {
        row.name = body.name;
        row.updated_at = new Date().toISOString();
      }
      return { data: undefined };
    }
  );
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

const rowNames = () =>
  screen
    .getAllByRole('button', { name: /^Open chat / })
    .map((button) => button.getAttribute('aria-label')?.replace('Open chat ', ''));

async function renderHistory(onSelectSession = vi.fn()) {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={onSelectSession} />
    </MemoryRouter>
  );
  await waitFor(() => expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Oldest chat']));
  return document.querySelector('[data-history-layer="content"]') as HTMLElement;
}

const renameOldest = () =>
  act(async () => {
    await renameSessionOptimistically('s3', 'Renamed oldest', {
      name: 'Oldest chat',
      userSetName: false,
    });
  });

describe('Chat history shows a rename as activity', () => {
  it('moves a renamed chat to the top at once, without opening another chat', async () => {
    await renderHistory();
    const readsBefore = daemon.listSessions.mock.calls.length;

    await renameOldest();

    expect(daemon.updateSessionName).toHaveBeenCalledTimes(1);
    expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat']);
    // Moved from the saved rename itself, not from a re-read of the list.
    expect(daemon.listSessions).toHaveBeenCalledTimes(readsBefore);
  });

  it('moves no row when a chat is opened, before a rename or after one', async () => {
    const onSelectSession = vi.fn();
    await renderHistory(onSelectSession);

    fireEvent.click(screen.getByRole('button', { name: 'Open chat Middle chat' }));
    expect(onSelectSession).toHaveBeenCalledWith('s2');
    expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Oldest chat']);

    await renameOldest();
    expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat']);

    fireEvent.click(screen.getByRole('button', { name: 'Open chat Middle chat' }));
    expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat']);
  });

  it('waits for the pointer to leave before the renamed chat moves (principle 9)', async () => {
    const list = await renderHistory();
    fireEvent.pointerEnter(list);

    await renameOldest();
    // The name changes in place; the row does not move under the pointer.
    expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Renamed oldest']);

    fireEvent.pointerLeave(list);
    await waitFor(() =>
      expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat'])
    );
  });

  it('moves nothing for a rename the daemon refused', async () => {
    daemon.updateSessionName.mockRejectedValueOnce('This chat is private.');
    await renderHistory();

    await renameOldest();

    expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Oldest chat']);
  });
});
