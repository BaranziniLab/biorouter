import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import { chatRowCopy } from '../chats/copy';
import { MAKE_CHAT_PUBLIC, RENAME_TITLE } from './copy';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  exportSession: vi.fn(),
  divergeSession: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: mocks.exportSession,
  divergeSession: mocks.divergeSession,
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
  isPrivateCopyRefusal: () => false,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const session = {
  id: '20260823_2',
  name: 'Excel research',
  working_dir: '/Users/x/project',
  created_at: '2026-08-23T12:00:00Z',
  updated_at: '2026-08-23T12:00:00Z',
  extension_data: {},
  message_count: 4,
} as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
});

async function renderHistory(onSelectSession = vi.fn()) {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={onSelectSession} />
    </MemoryRouter>
  );
  await screen.findByText('Excel research');
  return { onSelectSession };
}

/** The keys of the open menu's items, in order. */
function menuKeys(): (string | null)[] {
  return screen.getAllByRole('menuitem').map((item) => item.getAttribute('data-chat-row-action'));
}

/** The menu a public chat with messages offers: the sidebar's order (owner message 4). */
const FULL_MENU = ['rename', 'open-tab', 'open-window', 'diverge', 'export', 'copy-id', 'delete'];

describe('History row menus', () => {
  /**
   * History, the sidebar and the tab strip share one menu: the order and the
   * words come from `chatRowMenuEntries`, so a Rename on one surface is a
   * Rename on all of them.
   */
  it('offers the shared menu on a right-click, in its fixed order', async () => {
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));

    await screen.findAllByRole('menuitem');
    expect(menuKeys()).toEqual(FULL_MENU);
    expect(screen.getByRole('menuitem', { name: chatRowCopy.menu.rename })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: chatRowCopy.menu.copyId })).toBeInTheDocument();
    expect(screen.getByRole('menuitem', { name: chatRowCopy.menu.delete })).toBeInTheDocument();
  });

  /**
   * The `⋯` overflow is the pointer-free path to the same menu, so it must carry
   * the SAME list, or the two menus on one row drift apart.
   */
  it('offers the identical list from the keyboard-reachable overflow', async () => {
    await renderHistory();
    // Radix opens a dropdown on pointerdown, not click.
    fireEvent.pointerDown(screen.getByLabelText('More actions for Excel research'), {
      button: 0,
      ctrlKey: false,
    });

    await screen.findAllByRole('menuitem');
    expect(menuKeys()).toEqual(FULL_MENU);
  });

  /**
   * macOS sends no `contextmenu` for Shift+F10 or the Menu key, so the row has
   * to open its own (ui/keyboardContextMenu.ts). Without it the menu was
   * reachable by pointer alone on the platform the app ships first.
   */
  it('opens the menu from the keyboard with Shift+F10', async () => {
    await renderHistory();
    const open = screen.getByRole('button', { name: 'Open chat Excel research' });
    open.focus();
    fireEvent.keyDown(open, { key: 'F10', shiftKey: true });

    await screen.findAllByRole('menuitem');
    expect(menuKeys()).toEqual(FULL_MENU);
  });

  it('opens Rename with F2 on a focused row', async () => {
    await renderHistory();
    const open = screen.getByRole('button', { name: 'Open chat Excel research' });
    open.focus();
    fireEvent.keyDown(open, { key: 'F2' });

    expect(await screen.findByRole('dialog', { name: RENAME_TITLE })).toBeInTheDocument();
  });

  it('copies the raw chat id from the right-click menu', async () => {
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));
    fireEvent.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.copyId }));

    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2'));
  });

  it('opens a tab through the same handler the row click uses', async () => {
    const { onSelectSession } = await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));
    fireEvent.click(await screen.findByText('Open in new tab'));

    await waitFor(() => expect(onSelectSession).toHaveBeenCalledWith('20260823_2'));
  });

  /**
   * The window path was History's before it was shared; this pins that sharing
   * it did not change the five arguments the window is opened with.
   */
  it('opens a window with the arguments History always used', async () => {
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));
    fireEvent.click(await screen.findByText('Open in new window'));

    await waitFor(() =>
      expect(window.electron.createChatWindow).toHaveBeenCalledWith(
        undefined,
        '/Users/x/project',
        undefined,
        '20260823_2',
        'pair'
      )
    );
  });

  it('exports through the shared export path, with the proof', async () => {
    mocks.exportSession.mockRejectedValue('refused');
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));
    fireEvent.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.export }));

    await waitFor(() =>
      expect(mocks.exportSession).toHaveBeenCalledWith(
        expect.objectContaining({
          path: { session_id: '20260823_2' },
          headers: { 'X-User-Action': 'test-proof' },
        })
      )
    );
  });

  it('diverges through the shared diverge path, with the proof', async () => {
    mocks.divergeSession.mockResolvedValue({ data: { sessionId: 'branch-1', workingDir: '/x' } });
    Object.assign(window.electron, { createDivergedChatWindow: vi.fn() });
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));
    fireEvent.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.diverge }));

    await waitFor(() =>
      expect(mocks.divergeSession).toHaveBeenCalledWith(
        expect.objectContaining({
          path: { session_id: '20260823_2' },
          headers: { 'X-User-Action': 'test-proof' },
        })
      )
    );
  });

  /**
   * "Works for user, scheduled, diverged, and subagent rows." Every kind keeps
   * the menu and copies its own id. A subagent run is machinery rather than a
   * chat to branch, so it is the one row without Diverge.
   */
  it('offers the menu on every kind of row, copying each row’s own id', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          session,
          { ...session, id: '20260823_3', name: 'Nightly QC', session_type: 'scheduled' },
          { ...session, id: '20260823_4', name: 'Branch of Excel', diverged_from: '20260823_2' },
          {
            ...session,
            id: '20260823_5',
            name: 'Subagent: Word research',
            session_type: 'sub_agent',
            parent_session_id: '20260823_2',
          },
        ],
      },
    });
    await renderHistory();
    // The subagent row is only fetched and shown once the toggle is on.
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
    await screen.findByText('Subagent: Word research');

    for (const [label, id, keys] of [
      ['Excel research', '20260823_2', FULL_MENU],
      ['Nightly QC', '20260823_3', FULL_MENU],
      ['Branch of Excel', '20260823_4', FULL_MENU],
      ['Subagent: Word research', '20260823_5', FULL_MENU.filter((key) => key !== 'diverge')],
    ] as const) {
      fireEvent.contextMenu(screen.getByText(label));
      await screen.findAllByRole('menuitem');
      expect(menuKeys()).toEqual(keys);
      fireEvent.click(screen.getByRole('menuitem', { name: chatRowCopy.menu.copyId }));
      await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenLastCalledWith(id));
    }
  });

  it('adds Make this chat public after Copy chat ID on a private row only', async () => {
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [{ ...session, privacy_tier: 'private' }] },
    });
    await renderHistory();
    fireEvent.contextMenu(screen.getByText('Excel research'));

    await screen.findAllByRole('menuitem');
    expect(menuKeys()).toEqual([
      'rename',
      'open-tab',
      'open-window',
      'diverge',
      'export',
      'copy-id',
      'declassify',
      'delete',
    ]);
    expect(screen.getByRole('menuitem', { name: MAKE_CHAT_PUBLIC })).toBeInTheDocument();
  });

  /**
   * The trigger is `asChild` on the row itself. A wrapper element here would sit
   * between `.biorouter-list-shell` and its rows and take the list separators
   * with it — invisible to jsdom, so the structural assertion is the guard.
   */
  it('keeps the menu trigger on the row, adding no wrapper', async () => {
    await renderHistory();
    const row = document.querySelector('.session-item');
    expect(row).not.toBeNull();
    expect(row?.parentElement?.className).toContain('session-grid');
  });
});
