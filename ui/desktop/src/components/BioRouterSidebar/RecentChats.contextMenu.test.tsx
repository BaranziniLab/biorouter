import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import { DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';

const mocks = vi.hoisted(() => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
  deleteConversation: vi.fn(),
}));

vi.mock('../../utils/deleteConversation', () => ({ deleteConversation: mocks.deleteConversation }));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

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

beforeEach(() => {
  vi.clearAllMocks();
  window.localStorage.clear();
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
});

const session: SessionSummary = {
  id: '20260823_2',
  name: 'Excel research',
  created_at: '2026-08-23T12:00:00.000Z',
  updated_at: '2026-08-23T12:00:00.000Z',
  working_dir: '/Users/x/project',
  message_count: 4,
  user_set_name: true,
};

function renderRecents(onOpen = vi.fn()) {
  render(
    <SidebarProvider>
      <RecentChats
        sessions={[session]}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={onOpen}
        onViewAll={vi.fn()}
        view={DEFAULT_SIDEBAR_CHAT_VIEW}
        onViewChange={vi.fn()}
      />
    </SidebarProvider>
  );
  return { onOpen };
}

/**
 * Before #114 these rows were click-only, so the conversation id — the handle
 * Chat Recall's exact load and every `workspace_*` id argument already take —
 * could not be got out of the sidebar at all.
 */
describe('sidebar Recents right-click menu', () => {
  /**
   * Owner message 4: right-click a chat to rename it, "along with all the other
   * actions possible there", in the spec's fixed order (3.4), text only.
   */
  it('offers rename, the openers, diverge, export, copy and delete on a right-click', async () => {
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));

    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.getAttribute('aria-label') ?? item.textContent)).toEqual([
      'RenameF2',
      'Open in new tab',
      'Open in new window',
      'Diverge',
      'Export…',
      'Copy chat ID',
      'Delete chat…',
    ]);
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
  });

  it('opens the same menu from Shift+F10 on the focused row', async () => {
    renderRecents();
    const row = screen.getByTestId('recent-chat-20260823_2');
    row.focus();
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true });
    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
  });

  it('opens the same menu from the row’s ⋯ button', async () => {
    renderRecents();
    const more = screen.getByTestId('recent-chat-more-20260823_2');
    expect(more).toHaveAccessibleName('More actions for Excel research');
    fireEvent.pointerDown(more, { button: 0, ctrlKey: false });
    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.getAttribute('data-chat-row-action'))).toEqual([
      'rename',
      'open-tab',
      'open-window',
      'diverge',
      'export',
      'copy-id',
      'delete',
    ]);
  });

  /**
   * The row's own click passes the name and the `user_set_name` flag along so
   * the new tab is born titled instead of showing a placeholder while BaseChat
   * refetches a session the sidebar already listed. The menu must open the tab
   * the same way — asserted on all three arguments, because a call that dropped
   * the last two would still open the right conversation and lose the title.
   */
  it('opens a tab through the row’s own opener, name and all', async () => {
    const { onOpen } = renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    fireEvent.click(await screen.findByText('Open in new tab'));

    await waitFor(() => expect(onOpen).toHaveBeenCalledWith('20260823_2', 'Excel research', true));
  });

  it('copies the raw conversation id', async () => {
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    fireEvent.click(await screen.findByText('Copy chat ID'));

    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2'));
  });

  it('opens a window on the session’s own directory', async () => {
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
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

  /**
   * The row carries two `asChild` triggers on one button — the tooltip's and the
   * menu's. If either stopped merging onto the same element the row would either
   * lose its tooltip or gain a wrapper box inside the sidebar's 2px rhythm, so
   * both are asserted together on the one node.
   */
  it('keeps the row a single button carrying both the tooltip and the menu', () => {
    renderRecents();
    const row = screen.getByTestId('recent-chat-20260823_2');
    expect(row.tagName).toBe('BUTTON');
    expect(row.getAttribute('aria-label')).toBe('Open chat: Excel research');
  });
});

describe('permanent Recents deletion', () => {
  it('requires confirmation and cancel leaves the session intact', async () => {
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Delete chat…' }));
    expect(await screen.findByText(/This can't be undone/)).toBeInTheDocument();
    expect(mocks.deleteConversation).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(mocks.deleteConversation).not.toHaveBeenCalled();
  });
  it('deletes exactly the confirmed conversation', async () => {
    mocks.deleteConversation.mockResolvedValue(undefined);
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Delete chat…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    await waitFor(() => expect(mocks.deleteConversation).toHaveBeenCalledWith('20260823_2'));
    expect(mocks.deleteConversation).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(mocks.toastSuccess).toHaveBeenCalled());
  });
  it('keeps the dialog open and shows an actionable API error', async () => {
    mocks.deleteConversation.mockRejectedValue(new Error('Server unavailable'));
    renderRecents();
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Delete chat…' }));
    fireEvent.click(await screen.findByRole('button', { name: 'Delete' }));
    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith(
        expect.objectContaining({ msg: 'Server unavailable' })
      )
    );
    expect(screen.getByRole('button', { name: 'Delete' })).toBeInTheDocument();
  });
});
