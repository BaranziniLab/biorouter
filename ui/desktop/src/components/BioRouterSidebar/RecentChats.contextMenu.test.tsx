import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { chatRowCopy } from '../chats/copy';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, { canDivergeChat } from './RecentChats';
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
describe('the sidebar chat row menu', () => {
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

  /**
   * History's menu is the reference for what a chat offers (spec 3.4), and the
   * two share one menu, so they share its rule: a subagent run is machinery, and
   * an empty chat has nothing to branch from. A scheduled run, a terminal chat
   * and a one-message chat still offer it; the daemon answers for the rest.
   */
  it('offers Diverge by History’s rule', () => {
    expect(canDivergeChat(session)).toBe(true);
    expect(canDivergeChat({ ...session, message_count: 0 })).toBe(false);
    expect(canDivergeChat({ ...session, message_count: 1 })).toBe(true);
    expect(canDivergeChat({ ...session, session_type: 'sub_agent' })).toBe(false);
    expect(canDivergeChat({ ...session, parent_session_id: 'parent' })).toBe(false);
    expect(canDivergeChat({ ...session, session_type: 'scheduled' })).toBe(true);
  });

  it('leaves Diverge out of a subagent run’s menu and keeps every other item', async () => {
    render(
      <SidebarProvider>
        <RecentChats
          sessions={[{ ...session, session_type: 'sub_agent' }]}
          runningSessionIds={new Set()}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={vi.fn()}
          onOpen={vi.fn()}
          onViewAll={vi.fn()}
          view={DEFAULT_SIDEBAR_CHAT_VIEW}
          onViewChange={vi.fn()}
        />
      </SidebarProvider>
    );
    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.getAttribute('data-chat-row-action'))).toEqual([
      'rename',
      'open-tab',
      'open-window',
      'export',
      'copy-id',
      'delete',
    ]);
  });

  it('leaves focus off the row after a choice made with the pointer, and returns it after Escape', async () => {
    // Handing focus back to a row the person clicked with a mouse rings it and
    // pins its ⋯ open (`:focus-within`); a keyboard user needs it back.
    renderRecents();
    const row = screen.getByTestId('recent-chat-20260823_2');

    row.focus();
    fireEvent.pointerDown(row, { button: 2 });
    fireEvent.contextMenu(row);
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Copy chat ID' }));
    await waitFor(() => expect(screen.queryByRole('menuitem')).toBeNull());
    expect(row).not.toHaveFocus();

    row.focus();
    fireEvent.keyDown(row, { key: 'F10', shiftKey: true });
    const menu = await screen.findByRole('menu');
    fireEvent.keyDown(menu, { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
    await waitFor(() => expect(row).toHaveFocus());
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

/**
 * Vision V1-R5-2. Neither row menu is modal (a modal menu restyles the whole window, R-28), so the
 * list is modal itself while one is open: it does not scroll, its rows take no pointer
 * (`data-row-menu-open`, sidebar.css; measured in `sidebarFill.browser.test.tsx`) and its rows
 * hold their order wherever the pointer is. Without it the list scrolled 599px under an open menu
 * whose Delete still named the chat that had gone off screen.
 */
describe('the list under an open row menu (vision V1-R5-2)', () => {
  const list = () => screen.getByTestId('recent-chat-scroll');
  /** Whether the list kept a wheel turn from scrolling it. */
  const wheelKept = () => {
    const event = new WheelEvent('wheel', { deltaY: 300, bubbles: true, cancelable: true });
    list().dispatchEvent(event);
    return event.defaultPrevented;
  };
  const closeMenu = async () => {
    fireEvent.keyDown(screen.getByRole('menu'), { key: 'Escape' });
    await waitFor(() => expect(screen.queryByRole('menu')).toBeNull());
  };

  it('stops scrolling and marks itself while the right-click menu is open', async () => {
    renderRecents();
    expect(list()).not.toHaveAttribute('data-row-menu-open');
    expect(wheelKept(), 'at rest the wheel scrolls the list').toBe(false);

    fireEvent.contextMenu(screen.getByTestId('recent-chat-20260823_2'));
    await screen.findByRole('menuitem', { name: 'Rename' });
    expect(list()).toHaveAttribute('data-row-menu-open', 'true');
    expect(wheelKept(), 'the wheel scrolls nothing under the menu').toBe(true);

    await closeMenu();
    expect(list()).not.toHaveAttribute('data-row-menu-open');
    expect(wheelKept()).toBe(false);
  });

  it('does the same for the row’s ⋯ menu', async () => {
    renderRecents();
    fireEvent.pointerDown(screen.getByTestId('recent-chat-more-20260823_2'), {
      button: 0,
      ctrlKey: false,
    });
    await screen.findByRole('menuitem', { name: 'Rename' });
    expect(list()).toHaveAttribute('data-row-menu-open', 'true');
    expect(wheelKept()).toBe(true);

    await closeMenu();
    expect(list()).not.toHaveAttribute('data-row-menu-open');
    expect(wheelKept()).toBe(false);
  });

  it('holds its rows in their order while a row menu is open, wherever the pointer is', async () => {
    const at = (minutes: number) => new Date(Date.UTC(2026, 7, 23, 12, minutes)).toISOString();
    const first = { ...session, id: 'a', name: 'Alpha', updated_at: at(10) };
    const second = { ...session, id: 'b', name: 'Beta', updated_at: at(5) };
    const draw = (sessions: SessionSummary[]) => (
      <SidebarProvider>
        <RecentChats
          sessions={sessions}
          runningSessionIds={new Set()}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={vi.fn()}
          onOpen={vi.fn()}
          onViewAll={vi.fn()}
          view={DEFAULT_SIDEBAR_CHAT_VIEW}
          onViewChange={vi.fn()}
        />
      </SidebarProvider>
    );
    const order = () =>
      [...list().querySelectorAll('button[data-row^="chat:"]')].map((row) =>
        row.getAttribute('data-row')
      );
    const { rerender } = render(draw([first, second]));
    expect(order()).toEqual(['chat:a', 'chat:b']);

    // The menu opens on Beta, and the pointer leaves the list for the menu (or anywhere else).
    fireEvent.pointerEnter(list());
    fireEvent.contextMenu(screen.getByTestId('recent-chat-b'));
    await screen.findByRole('menuitem', { name: 'Rename' });
    fireEvent.pointerLeave(list());

    // A turn in Beta raises it: with its menu open, nothing moves.
    rerender(draw([first, { ...second, updated_at: at(20) }]));
    expect(order()).toEqual(['chat:a', 'chat:b']);

    // The menu closes with the pointer away: the live order applies.
    await closeMenu();
    expect(order()).toEqual(['chat:b', 'chat:a']);
  });
});

describe('deleting a chat from the sidebar', () => {
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
    // History's toast, word for word: the title and no sentence under it.
    await waitFor(() =>
      expect(mocks.toastSuccess).toHaveBeenCalledWith({ title: chatRowCopy.deleteDialog.deleted })
    );
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
