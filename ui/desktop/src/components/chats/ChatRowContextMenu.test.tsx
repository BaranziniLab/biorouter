import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { ChatRowContextMenu, chatRowMenuEntries } from './ChatRowContextMenu';

const mocks = vi.hoisted(() => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../toasts', () => ({
  toastSuccess: mocks.toastSuccess,
  toastError: mocks.toastError,
}));

beforeEach(() => {
  vi.clearAllMocks();
  Object.assign(navigator, { clipboard: { writeText: vi.fn(() => Promise.resolve()) } });
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
});

function openMenu(openInNewTab = vi.fn()) {
  render(
    <ChatRowContextMenu target={{ sessionId: '20260823_2', workingDir: '/tmp/p', openInNewTab }}>
      <div data-testid="row">Chat about volcano plots</div>
    </ChatRowContextMenu>
  );
  fireEvent.contextMenu(screen.getByTestId('row'));
  return { openInNewTab };
}

describe('ChatRowContextMenu', () => {
  /**
   * `asChild`: the trigger must attach to the row the surface rendered, not wrap
   * it. History's rows sit directly inside `.biorouter-list-shell`, which draws
   * their separators — an extra element between the two takes the separators
   * with it, and the sidebar's `space-y-0.5` rhythm breaks the same way.
   */
  it('attaches to the row element instead of wrapping it', () => {
    openMenu();
    const row = screen.getByTestId('row');
    expect(row.tagName).toBe('DIV');
    expect(row.textContent).toBe('Chat about volcano plots');
  });

  it('opens on right-click with the three actions, in order', async () => {
    openMenu();
    const items = await screen.findAllByRole('menuitem');
    expect(items.map((item) => item.textContent)).toEqual([
      'Open in new tab',
      'Open in new window',
      'Copy chat ID',
    ]);
  });

  /**
   * The Menu key and Shift+F10 both dispatch `contextmenu` on the focused
   * element, which is the keyboard equivalent the issue asks for — and
   * deliberately the SAME trigger rather than an app-specific shortcut. jsdom
   * does not synthesise that event from the keypress, so the assertion is that
   * a `contextmenu` event with no pointer coordinates (what a keyboard produces)
   * still opens the menu.
   */
  it('opens from a keyboard-originated contextmenu event', async () => {
    render(
      <ChatRowContextMenu target={{ sessionId: '20260823_2', openInNewTab: vi.fn() }}>
        <button data-testid="row">Chat</button>
      </ChatRowContextMenu>
    );
    const row = screen.getByTestId('row');
    row.focus();
    fireEvent.contextMenu(row, { button: 0, buttons: 0, clientX: 0, clientY: 0 });

    expect(await screen.findAllByRole('menuitem')).toHaveLength(3);
  });

  it('runs the surface’s own opener for "Open in new tab"', async () => {
    const { openInNewTab } = openMenu();
    fireEvent.click(await screen.findByText('Open in new tab'));
    await waitFor(() => expect(openInNewTab).toHaveBeenCalledTimes(1));
  });

  it('copies the raw conversation id', async () => {
    openMenu();
    fireEvent.click(await screen.findByText('Copy chat ID'));
    await waitFor(() => expect(navigator.clipboard.writeText).toHaveBeenCalledWith('20260823_2'));
    expect(mocks.toastSuccess).toHaveBeenCalled();
  });

  /**
   * The tab strip opens this menu inside the titlebar band, where App.tsx paints
   * a `-webkit-app-region: drag` rect. Electron folds those in DOM order, so
   * without a later `no-drag` the menu's top edge would look present and be
   * dead (issue #74). jsdom cannot evaluate app-region, so the class on the
   * content is the only thing a unit test can hold.
   */
  it('marks the menu surface no-drag so it survives the titlebar band', async () => {
    openMenu();
    const menu = await screen.findByRole('menu');
    expect(menu.className).toContain('no-drag');
  });

  /**
   * Owner message 4: Rename and "all the other actions possible there". The
   * order is fixed (spec 3.4) and text only; the separators are not menu items.
   */
  it('lists every action a surface supplies, in the fixed order', async () => {
    render(
      <ChatRowContextMenu
        target={{ sessionId: '20260823_2', openInNewTab: vi.fn() }}
        onRename={vi.fn()}
        onDiverge={vi.fn()}
        onExport={vi.fn()}
        onDelete={vi.fn()}
      >
        <div data-testid="row">Chat</div>
      </ChatRowContextMenu>
    );
    fireEvent.contextMenu(screen.getByTestId('row'));
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
    expect(screen.getByRole('menuitem', { name: 'Rename' })).toHaveTextContent('F2');
    expect(screen.getByRole('menuitem', { name: 'Delete chat…' })).toHaveAttribute(
      'data-variant',
      'destructive'
    );
    expect(screen.getAllByRole('separator')).toHaveLength(3);
    expect(items.every((item) => item.querySelector('svg') === null)).toBe(true);
  });

  it('runs the handler of the item that was chosen', async () => {
    const onRename = vi.fn();
    const onExport = vi.fn();
    render(
      <ChatRowContextMenu
        target={{ sessionId: '20260823_2', openInNewTab: vi.fn() }}
        onRename={onRename}
        onExport={onExport}
      >
        <div data-testid="row">Chat</div>
      </ChatRowContextMenu>
    );
    fireEvent.contextMenu(screen.getByTestId('row'));
    fireEvent.click(await screen.findByRole('menuitem', { name: 'Export…' }));
    await waitFor(() => expect(onExport).toHaveBeenCalledTimes(1));
    expect(onRename).not.toHaveBeenCalled();
  });
});

describe('chatRowMenuEntries', () => {
  const target = { sessionId: 's1', openInNewTab: () => {} };

  it('places a surface’s extra items right after Copy chat ID', () => {
    const entries = chatRowMenuEntries(target, {
      onDelete: () => {},
      extraItems: [{ key: 'make-public', label: 'Make this chat public', onSelect: () => {} }],
    });
    expect(entries.map((entry) => entry.key)).toEqual([
      'open-tab',
      'open-window',
      'after-open',
      'copy-id',
      'make-public',
      'before-delete',
      'delete',
    ]);
  });

  it('draws an extra item’s note above it, visibly, in the menu', async () => {
    render(
      <ChatRowContextMenu
        target={{ sessionId: 's1', openInNewTab: vi.fn() }}
        extraItems={[
          {
            key: 'make-public',
            label: 'Make this chat public',
            onSelect: vi.fn(),
            disabled: true,
            note: 'Only the host can make a chat public.',
            noteTestId: 'declassify-note',
          },
        ]}
      >
        <div data-testid="row">Chat</div>
      </ChatRowContextMenu>
    );
    fireEvent.contextMenu(screen.getByTestId('row'));
    const note = await screen.findByTestId('declassify-note');
    expect(note).toHaveTextContent('Only the host can make a chat public.');
    const item = screen.getByRole('menuitem', { name: 'Make this chat public' });
    expect(note.compareDocumentPosition(item)).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(item).toHaveAttribute('data-disabled');
  });

  it('hides the rename hint on a surface without the key', () => {
    const [rename] = chatRowMenuEntries(target, { onRename: () => {}, renameShortcut: null });
    expect(rename).toMatchObject({ key: 'rename' });
    expect(rename).not.toHaveProperty('shortcut');
  });
});
