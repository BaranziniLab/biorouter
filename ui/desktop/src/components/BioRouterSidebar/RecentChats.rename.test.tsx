import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useEffect, useState } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import {
  RENAME_FAILED_TOAST_TITLE,
  subscribeSessionNameChanges,
} from '../../utils/sessionNameSync';
import { chatRowCopy } from '../chats/copy';
import { SIDEBAR_OVERLAY_BODY_CLASS, Sidebar, SidebarInset, SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
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
