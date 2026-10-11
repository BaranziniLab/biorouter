import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useEffect, useState } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import {
  laterActivity,
  RENAME_FAILED_TOAST_TITLE,
  subscribeSessionNameChanges,
} from '../../utils/sessionNameSync';
import { chatRowCopy } from '../chats/copy';
import { SIDEBAR_OVERLAY_BODY_CLASS, Sidebar, SidebarInset, SidebarProvider } from '../ui/sidebar';
import RecentChats, { scrollRowIntoView } from './RecentChats';
import { DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';

const mocks = vi.hoisted(() => ({
  updateSessionName: vi.fn(),
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../../api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../api')>()),
  updateSessionName: mocks.updateSessionName,
}));

vi.mock('../../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../../utils/userAction')>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

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
  mocks.updateSessionName.mockResolvedValue({ data: {} });
});

const SESSION: SessionSummary = {
  id: 's1',
  name: 'Cohort pull',
  created_at: new Date().toISOString(),
  updated_at: new Date().toISOString(),
  working_dir: '/Users/x/project',
  message_count: 4,
  user_set_name: false,
};

/** The list with its names kept live, the way `useSidebarSessions` keeps them. */
function LiveRecents() {
  const [sessions, setSessions] = useState([SESSION]);
  useEffect(
    () =>
      subscribeSessionNameChanges(({ sessionId, name, userSetName }) =>
        setSessions((current) =>
          current.map((session) =>
            session.id === sessionId ? { ...session, name, user_set_name: userSetName } : session
          )
        )
      ),
    []
  );
  return (
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
  );
}

function renderList() {
  return render(
    <SidebarProvider>
      <LiveRecents />
    </SidebarProvider>
  );
}

const row = () => screen.getByTestId('recent-chat-s1');
const input = () =>
  screen.getByRole('textbox', { name: chatRowCopy.rename.inputLabel('Cohort pull') });

async function renameFromMenu() {
  fireEvent.contextMenu(row());
  const rename = await screen.findByRole('menuitem', { name: chatRowCopy.menu.rename });
  fireEvent.click(rename);
  return screen.findByRole('textbox');
}

describe('renaming a chat from its sidebar row (owner message 4)', () => {
  it('offers Rename first in the row menu', async () => {
    renderList();
    fireEvent.contextMenu(row());
    const items = await screen.findAllByRole('menuitem');
    expect(items[0]).toHaveAccessibleName(chatRowCopy.menu.rename);
  });

  it('opens the editor in place, holding the title fully selected', async () => {
    renderList();
    const editor = (await renameFromMenu()) as HTMLInputElement;
    expect(editor).toHaveValue('Cohort pull');
    expect(editor).toHaveFocus();
    expect(editor.selectionStart).toBe(0);
    expect(editor.selectionEnd).toBe('Cohort pull'.length);
    expect(editor).toHaveAttribute('maxLength', '200');
    // The row keeps its box: it is the same row recipe, now holding an input.
    expect(editor.closest('.br-chat-row')).toHaveAttribute('data-editing', 'true');
  });

  it('commits on Enter with the user proof and shows no success toast', async () => {
    renderList();
    await renameFromMenu();
    fireEvent.change(input(), { target: { value: 'IRB cohort' } });
    fireEvent.keyDown(input(), { key: 'Enter' });

    await waitFor(() =>
      expect(mocks.updateSessionName).toHaveBeenCalledWith({
        path: { session_id: 's1' },
        body: { name: 'IRB cohort' },
        headers: { 'X-User-Action': 'test-proof' },
        throwOnError: true,
      })
    );
    expect(screen.getByTestId('recent-chat-s1')).toHaveTextContent('IRB cohort');
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('commits on blur', async () => {
    renderList();
    await renameFromMenu();
    fireEvent.change(input(), { target: { value: 'Blurred name' } });
    fireEvent.blur(input());
    await waitFor(() => expect(mocks.updateSessionName).toHaveBeenCalledTimes(1));
  });

  it('makes no call for an empty or unchanged name', async () => {
    renderList();
    await renameFromMenu();
    fireEvent.change(input(), { target: { value: '   ' } });
    fireEvent.keyDown(input(), { key: 'Enter' });
    await renameFromMenu();
    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Enter' });
    expect(mocks.updateSessionName).not.toHaveBeenCalled();
    expect(row()).toHaveTextContent('Cohort pull');
  });

  it('rolls a refused rename back and says why', async () => {
    const refusal = 'This chat is private. Rename it from a Biorouter window.';
    mocks.updateSessionName.mockRejectedValue(refusal);
    renderList();
    await renameFromMenu();
    fireEvent.change(input(), { target: { value: 'Refused name' } });
    fireEvent.keyDown(input(), { key: 'Enter' });

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: RENAME_FAILED_TOAST_TITLE,
        msg: refusal,
      })
    );
    expect(row()).toHaveTextContent('Cohort pull');
  });

  it('opens with F2 on a focused row and returns focus to the row afterwards', async () => {
    renderList();
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    expect(await screen.findByRole('textbox')).toHaveFocus();

    fireEvent.keyDown(screen.getByRole('textbox'), { key: 'Escape' });
    await waitFor(() => expect(row()).toHaveFocus());
    expect(mocks.updateSessionName).not.toHaveBeenCalled();
  });

  it('turns pasted newlines into spaces', async () => {
    renderList();
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    const editor = (await screen.findByRole('textbox')) as HTMLInputElement;
    editor.setSelectionRange(0, editor.value.length);
    fireEvent.paste(editor, { clipboardData: { getData: () => 'two\nlines' } });
    expect(editor).toHaveValue('two lines');
  });
});

function StaticRecents({ sessions }: { sessions: SessionSummary[] }) {
  return (
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
  );
}

describe('the editor on an unusual row', () => {
  it('does not name an unnamed chat "Untitled chat" when Enter is pressed on the untouched draft', async () => {
    // A chat with no name is drawn "Untitled chat" and the editor opens on that
    // text: confirming it unchanged is not a rename.
    render(
      <SidebarProvider>
        <StaticRecents sessions={[{ ...SESSION, name: '' }]} />
      </SidebarProvider>
    );
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    const editor = await screen.findByRole('textbox');
    expect(editor).toHaveValue('Untitled chat');
    fireEvent.keyDown(editor, { key: 'Enter' });
    await waitFor(() => expect(screen.queryByRole('textbox')).toBeNull());
    expect(mocks.updateSessionName).not.toHaveBeenCalled();
  });

  it('still names an unnamed chat when the person types a name', async () => {
    render(
      <SidebarProvider>
        <StaticRecents sessions={[{ ...SESSION, name: '' }]} />
      </SidebarProvider>
    );
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    const editor = await screen.findByRole('textbox');
    fireEvent.change(editor, { target: { value: 'Named at last' } });
    fireEvent.keyDown(editor, { key: 'Enter' });
    await waitFor(() => expect(mocks.updateSessionName).toHaveBeenCalledTimes(1));
  });

  it('closes the editor when its chat is deleted while the name is being typed', async () => {
    const { rerender } = render(
      <SidebarProvider>
        <StaticRecents sessions={[SESSION, { ...SESSION, id: 's2', name: 'Another' }]} />
      </SidebarProvider>
    );
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    expect(await screen.findByRole('textbox')).toBeInTheDocument();

    rerender(
      <SidebarProvider>
        <StaticRecents sessions={[{ ...SESSION, id: 's2', name: 'Another' }]} />
      </SidebarProvider>
    );
    await waitFor(() => expect(screen.queryByRole('textbox')).toBeNull());
    expect(screen.getByTestId('recent-chat-s2')).toBeInTheDocument();
  });
});

describe('⌘⌥R renames the chat you are in', () => {
  afterEach(() => {
    Object.assign(window, { electron: undefined });
  });

  const renderActive = (defaultOpen = true, sessions: SessionSummary[] = [SESSION]) =>
    render(
      <SidebarProvider defaultOpen={defaultOpen}>
        <Sidebar variant="inset" collapsible="offcanvas">
          <RecentChats
            sessions={sessions}
            activeSessionId="s1"
            runningSessionIds={new Set()}
            hasMore={false}
            isLoadingMore={false}
            onLoadMore={vi.fn()}
            onOpen={vi.fn()}
            onViewAll={vi.fn()}
            view={DEFAULT_SIDEBAR_CHAT_VIEW}
            onViewChange={vi.fn()}
          />
        </Sidebar>
        <SidebarInset>
          <main />
        </SidebarInset>
      </SidebarProvider>
    );

  it('opens the editor on the active row with ⌘⌥R on macOS', async () => {
    Object.assign(window, { electron: { platform: 'darwin' } });
    renderActive();
    // `code`, not `key`: Option changes the character the key types.
    fireEvent.keyDown(window, { key: '®', code: 'KeyR', metaKey: true, altKey: true });
    expect(await screen.findByRole('textbox')).toHaveValue('Cohort pull');
  });

  it('uses Ctrl+Alt+R elsewhere, and ignores a plain ⌘R and a Shift chord', async () => {
    Object.assign(window, { electron: { platform: 'linux' } });
    renderActive();
    fireEvent.keyDown(window, { code: 'KeyR', metaKey: true, altKey: true });
    fireEvent.keyDown(window, { code: 'KeyR', ctrlKey: true });
    fireEvent.keyDown(window, { code: 'KeyR', ctrlKey: true, altKey: true, shiftKey: true });
    expect(screen.queryByRole('textbox')).toBeNull();

    fireEvent.keyDown(window, { code: 'KeyR', ctrlKey: true, altKey: true });
    expect(await screen.findByRole('textbox')).toBeInTheDocument();
  });

  it('does nothing when the active chat is not in the list', () => {
    Object.assign(window, { electron: { platform: 'darwin' } });
    renderActive(true, [{ ...SESSION, id: 'other' }]);
    fireEvent.keyDown(window, { code: 'KeyR', metaKey: true, altKey: true });
    expect(screen.queryByRole('textbox')).toBeNull();
  });

  it('opens a closed sidebar first, so the editor is not left inert and unseen', async () => {
    Object.assign(window, { electron: { platform: 'darwin' } });
    renderActive(false);
    expect(document.querySelector('[data-slot="sidebar"]')).toHaveAttribute(
      'data-state',
      'collapsed'
    );
    fireEvent.keyDown(window, { code: 'KeyR', metaKey: true, altKey: true });
    const editor = await screen.findByRole('textbox');
    expect(document.querySelector('[data-slot="sidebar"]')).toHaveAttribute(
      'data-state',
      'expanded'
    );
    await waitFor(() => expect(editor).toHaveFocus());
  });
});

describe('Escape in the editor inside the overlay sidebar', () => {
  beforeEach(() => document.body.classList.add(SIDEBAR_OVERLAY_BODY_CLASS));
  afterEach(() => document.body.classList.remove(SIDEBAR_OVERLAY_BODY_CLASS));

  it('cancels the rename without closing the overlay', async () => {
    render(
      <SidebarProvider defaultOpen>
        <Sidebar variant="inset" collapsible="offcanvas">
          <LiveRecents />
        </Sidebar>
        <SidebarInset>
          <main />
        </SidebarInset>
      </SidebarProvider>
    );
    row().focus();
    fireEvent.keyDown(row(), { key: 'F2' });
    const editor = await screen.findByRole('textbox');
    fireEvent.change(editor, { target: { value: 'Not this' } });

    await act(async () => {
      fireEvent.keyDown(editor, { key: 'Escape' });
    });

    expect(document.querySelector('[data-slot="sidebar"]')).toHaveAttribute(
      'data-state',
      'expanded'
    );
    expect(row()).toHaveTextContent('Cohort pull');
    expect(mocks.updateSessionName).not.toHaveBeenCalled();
  });
});

/**
 * Vision V1-R2-4 and ruling 16: a rename is activity, and the renderer shows its move at once. The
 * usual rename (row menu, type, Enter) leaves the pointer on the list, where the hold of principle
 * 9 kept the row in place until the pointer left; History had moved it within a second. The
 * person's own rename is the move they asked for, so it is not held.
 */
describe('a rename moves its chat at once, under the pointer too', () => {
  const minutesAgo = (minutes: number) => new Date(Date.now() - minutes * 60_000).toISOString();
  const chat = (id: string, name: string, updatedAt: string): SessionSummary => ({
    ...SESSION,
    id,
    name,
    created_at: updatedAt,
    updated_at: updatedAt,
  });
  const START = [
    chat('a', 'Atlas', minutesAgo(1)),
    chat('b', 'Biopsy', minutesAgo(2)),
    // Ten days old: under an older bucket than Today.
    chat('c', 'Cohort', minutesAgo(10 * 24 * 60)),
  ];
  const order = () =>
    Array.from(document.querySelectorAll('[data-row^="chat:"]')).map((node) =>
      node.getAttribute('data-row')
    );

  /** The list with names AND a saved rename's activity kept live, as `useSidebarSessions` does. */
  function ActiveRecents({ raise }: { raise?: { id: string; at: string } }) {
    const [sessions, setSessions] = useState(START);
    useEffect(
      () =>
        subscribeSessionNameChanges(({ sessionId, name, userSetName, activityAt }) =>
          setSessions((current) =>
            current.map((session) =>
              session.id === sessionId
                ? {
                    ...session,
                    name,
                    user_set_name: userSetName,
                    updated_at: laterActivity(session.updated_at, activityAt),
                  }
                : session
            )
          )
        ),
      []
    );
    const shown = raise
      ? sessions.map((session) =>
          session.id === raise.id ? { ...session, updated_at: raise.at } : session
        )
      : sessions;
    return (
      <RecentChats
        sessions={shown}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={DEFAULT_SIDEBAR_CHAT_VIEW}
        onViewChange={vi.fn()}
      />
    );
  }

  it('moves the renamed chat to the top while the pointer rests on the list, and keeps focus on it', async () => {
    const { rerender } = render(
      <SidebarProvider>
        <ActiveRecents />
      </SidebarProvider>
    );
    expect(order()).toEqual(['chat:a', 'chat:b', 'chat:c']);
    const list = screen.getByTestId('recent-chat-scroll');
    fireEvent.pointerEnter(list);

    fireEvent.contextMenu(screen.getByTestId('recent-chat-c'));
    fireEvent.click(await screen.findByRole('menuitem', { name: chatRowCopy.menu.rename }));
    const editor = await screen.findByRole('textbox');
    fireEvent.change(editor, { target: { value: 'IRB cohort' } });
    fireEvent.keyDown(editor, { key: 'Enter' });

    // Saved: the chat moves from its old bucket to the top under Today, with the pointer still in.
    await waitFor(() => expect(order()).toEqual(['chat:c', 'chat:a', 'chat:b']));
    // The move remounted the row under Today; focus went with it, not to the page.
    expect(screen.getByTestId('recent-chat-c')).toHaveFocus();

    // Only the person's own rename goes through: a turn elsewhere still waits for the pointer.
    rerender(
      <SidebarProvider>
        <ActiveRecents raise={{ id: 'b', at: new Date(Date.now() + 60_000).toISOString() }} />
      </SidebarProvider>
    );
    expect(order()).toEqual(['chat:c', 'chat:a', 'chat:b']);
    fireEvent.pointerLeave(list);
    expect(order()).toEqual(['chat:b', 'chat:c', 'chat:a']);
  });

  it('scrolls the list just enough to show a row, with its bucket when it is the first one', () => {
    const container = document.createElement('div');
    const section = document.createElement('section');
    const first = document.createElement('button');
    first.setAttribute('data-row', 'chat:first');
    const later = document.createElement('button');
    later.setAttribute('data-row', 'chat:later');
    section.append(first, later);
    container.append(section);
    const rect = (top: number, height: number) => ({ top, bottom: top + height }) as DOMRect;
    container.getBoundingClientRect = () => rect(100, 300);
    section.getBoundingClientRect = () => rect(20, 80);
    first.getBoundingClientRect = () => rect(44, 28);
    later.getBoundingClientRect = () => rect(420, 28);

    container.scrollTop = 500;
    // Above the list: up to the section's top, so "Today" shows over the row.
    scrollRowIntoView(container, first);
    expect(container.scrollTop).toBe(420);

    container.scrollTop = 0;
    // Below it: down just far enough for the row's bottom edge.
    scrollRowIntoView(container, later);
    expect(container.scrollTop).toBe(48);
  });
});
