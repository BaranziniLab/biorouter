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
import { clearSessionListCache, updateCachedSessionList } from '../../utils/sessionListCache';
import { renameSession, renameSessionOptimistically } from '../../utils/sessionNameSync';
import { RENAME_PLACEHOLDER, RENAME_TITLE } from './copy';
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

/** A timestamp later than any rename these tests make, as an ISO string. */
const nextSecond = () => new Date(Date.now() + 1_000).toISOString();

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

  it("moves a person's own rename at once, with the pointer on the list (V1-R6-1)", async () => {
    const list = await renderHistory();
    fireEvent.pointerEnter(list);

    await renameOldest();

    // Ruling 16: the row they renamed is where its new activity puts it, not where it was.
    expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat']);
  });

  it("still holds every other move while it lets the person's own through", async () => {
    const list = await renderHistory();
    fireEvent.pointerEnter(list);
    await renameOldest();

    // A chat that arrives from elsewhere, newer than the rename, waits for the pointer.
    const arrival = { ...session('s4', 'Arrived elsewhere', 0), updated_at: nextSecond() };
    act(() => updateCachedSessionList((current) => [arrival, ...current]));
    expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat']);

    fireEvent.pointerLeave(list);
    await waitFor(() =>
      expect(rowNames()).toEqual([
        'Arrived elsewhere',
        'Renamed oldest',
        'Newest chat',
        'Middle chat',
      ])
    );
  });

  it("does not lift a move that is not the person's: the model's rename waits for the pointer", async () => {
    const list = await renderHistory();
    fireEvent.pointerEnter(list);

    await act(async () => {
      await renameSession('s3', 'Named by the model', 'llm');
    });
    expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Named by the model']);

    fireEvent.pointerLeave(list);
    await waitFor(() =>
      expect(rowNames()).toEqual(['Named by the model', 'Newest chat', 'Middle chat'])
    );
  });

  describe('renamed in the dialog, with the pointer resting on the list', () => {
    /**
     * The sequence the vision pass measured: the dialog is modal, so the list sees the pointer
     * LEAVE when it opens and ENTER again when it closes, and that second enter takes a hold
     * from the order the list had.
     */
    async function openDialogAndSave(list: HTMLElement, name: string) {
      fireEvent.pointerEnter(list);
      fireEvent.click(screen.getByRole('button', { name: 'Rename Oldest chat' }));
      fireEvent.pointerLeave(list);
      const field = await screen.findByPlaceholderText(RENAME_PLACEHOLDER);
      fireEvent.change(field, { target: { value: name } });
      fireEvent.keyDown(field, { key: 'Enter' });
      await waitFor(() => expect(screen.queryByRole('dialog', { name: RENAME_TITLE })).toBeNull());
    }

    /** A daemon that answers the rename when told to. */
    function slowDaemon() {
      let release: () => void = () => undefined;
      const gate = new Promise<void>((resolve) => {
        release = resolve;
      });
      daemon.updateSessionName.mockImplementationOnce(
        async ({ path, body }: { path: { session_id: string }; body: { name: string } }) => {
          await gate;
          const row = daemon.rows.find((candidate) => candidate.id === path.session_id);
          if (row) {
            row.name = body.name;
            row.updated_at = new Date().toISOString();
          }
          return { data: undefined };
        }
      );
      return release;
    }

    it('moves the row at once when the dialog closes before the daemon has answered', async () => {
      const release = slowDaemon();
      const list = await renderHistory();
      await openDialogAndSave(list, 'Renamed oldest');

      // The dialog is gone and the pointer is over the list again: the hold is taken now.
      fireEvent.pointerEnter(list);
      expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Renamed oldest']);

      await act(async () => release());
      await waitFor(() =>
        expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat'])
      );
    });

    it('moves the row at once when the pointer comes back before the list has caught up', async () => {
      const release = slowDaemon();
      const list = await renderHistory();
      await openDialogAndSave(list, 'Renamed oldest');

      // The daemon answers and the pointer enters in the same breath, so the hold is taken from
      // the list as it was a moment ago: the order from before the move.
      await act(async () => {
        release();
        await new Promise((resolve) => setTimeout(resolve, 0));
        fireEvent.pointerEnter(list);
      });
      await waitFor(() =>
        expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat'])
      );
    });

    it('gives the moved row its focus back', async () => {
      const release = slowDaemon();
      const list = await renderHistory();
      await openDialogAndSave(list, 'Renamed oldest');
      fireEvent.pointerEnter(list);
      // The dialog has handed focus back to where it came from; the move then takes it away.
      await new Promise((resolve) => setTimeout(resolve, 30));
      act(() => (document.activeElement as HTMLElement | null)?.blur());
      expect(document.activeElement).toBe(document.body);

      await act(async () => release());
      await waitFor(() =>
        expect(rowNames()).toEqual(['Renamed oldest', 'Newest chat', 'Middle chat'])
      );
      const moved = screen.getByRole('button', { name: 'Open chat Renamed oldest' });
      expect(document.activeElement).toBe(moved);
    });
  });

  it('moves nothing for a rename the daemon refused', async () => {
    daemon.updateSessionName.mockRejectedValueOnce('This chat is private.');
    await renderHistory();

    await renameOldest();

    expect(rowNames()).toEqual(['Newest chat', 'Middle chat', 'Oldest chat']);
  });
});
