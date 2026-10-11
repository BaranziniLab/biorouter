import { fireEvent, render, screen, within } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, { FOLDER_PREVIEW_COUNT } from './RecentChats';
import { sidebarCopy } from './copy';
import { SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY, type SidebarChatView } from './sidebarChatView';

/**
 * Group by Folder as the list draws it (spec 3.4, owner message 3): a header
 * per folder with its chevron, glyph and name; a `+` and a `⋯` that appear on
 * hover; a remembered collapse; five rows, then "Show more". The arranging
 * itself (labels, collisions, order) is pinned in `sidebarChatView.test.ts`.
 */

const FOLDER_VIEW: SidebarChatView = { groupBy: 'folder', sortBy: 'activity' };
const PROJECT = '/Users/x/project';
const NOTES = '/Users/x/notes';
const CREW_TASKS = '/Users/x/Library/Application Support/Biorouter/crew/tasks';

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
  window.localStorage.clear();
  Object.assign(window, { electron: { createChatWindow: vi.fn() } });
});

function chat(id: string, workingDir: string, minutesAgo: number): SessionSummary {
  const at = new Date(Date.now() - minutesAgo * 60_000).toISOString();
  return {
    id,
    name: `Chat ${id}`,
    created_at: at,
    updated_at: at,
    working_dir: workingDir,
    message_count: 2,
    user_set_name: true,
  };
}

/** Seven chats in the project (two past the preview), two in notes, one Crew task. */
const SESSIONS: SessionSummary[] = [
  ...Array.from({ length: 7 }, (_, index) => chat(`p${index}`, PROJECT, index + 1)),
  chat('n0', NOTES, 30),
  chat('n1', NOTES, 31),
  chat('c0', CREW_TASKS, 60),
];

function renderFolders(props: Partial<ComponentProps<typeof RecentChats>> = {}) {
  return render(
    <SidebarProvider>
      <RecentChats
        sessions={SESSIONS}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={FOLDER_VIEW}
        onViewChange={vi.fn()}
        homeDir="/Users/x"
        {...props}
      />
    </SidebarProvider>
  );
}

const folder = (path: string) => screen.getByTestId(`sidebar-folder-folder:${path}`);
const toggleOf = (path: string) => within(folder(path)).getByRole('button', { expanded: true });

describe('Group by Folder', () => {
  it('draws one header per folder, named by its basename, Crew for the task folder', () => {
    renderFolders();
    expect(within(folder(PROJECT)).getByText('project')).toBeInTheDocument();
    expect(within(folder(NOTES)).getByText('notes')).toBeInTheDocument();
    expect(within(folder(CREW_TASKS)).getByText('Crew')).toBeInTheDocument();
    // Sentence-case 13px rows, never a date bucket in this view.
    expect(screen.queryByText('Today')).toBeNull();
  });

  it('offers a new chat window in the folder, and none in the Crew task folder', () => {
    renderFolders();
    fireEvent.click(
      within(folder(PROJECT)).getByRole('button', { name: sidebarCopy.folder.newChat('project') })
    );
    expect(window.electron.createChatWindow).toHaveBeenCalledWith(undefined, PROJECT);
    expect(
      within(folder(CREW_TASKS)).queryByRole('button', {
        name: sidebarCopy.folder.newChat('Crew'),
      })
    ).toBeNull();
    // Copy folder path stays offered on every folder that has one.
    expect(
      within(folder(CREW_TASKS)).getByRole('button', {
        name: sidebarCopy.folder.moreActions('Crew'),
      })
    ).toBeInTheDocument();
  });

  it('shows five rows, then a Show more that reveals the rest and does not count as a choice', () => {
    renderFolders();
    expect(screen.getByTestId('recent-chat-p4')).toBeInTheDocument();
    expect(screen.queryByTestId('recent-chat-p5')).toBeNull();
    expect(FOLDER_PREVIEW_COUNT).toBe(5);

    const more = screen.getByTestId(`sidebar-folder-more-folder:${PROJECT}`);
    expect(more).toHaveTextContent(sidebarCopy.chats.showMore);
    // `aria-expanded` is what keeps the overlay sidebar open on this click.
    expect(more).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(more);

    expect(screen.getByTestId('recent-chat-p6')).toBeInTheDocument();
    expect(more).toHaveTextContent(sidebarCopy.chats.showLess);
    expect(more).toHaveAttribute('aria-expanded', 'true');
    // A folder of two has nothing to show more of.
    expect(screen.queryByTestId(`sidebar-folder-more-folder:${NOTES}`)).toBeNull();
  });

  // Vision V1-R5-3: where focus goes is pinned here; that it is in view, in
  // `sidebarFill.browser.test.tsx` (jsdom has no layout).
  it('moves focus from Show more pressed by key to the first chat it shows, and no further', () => {
    renderFolders();
    const more = screen.getByTestId(`sidebar-folder-more-folder:${PROJECT}`);
    // Enter or Space on a button is a click with no pointer count (`detail` 0).
    more.focus();
    fireEvent.click(more, { detail: 0 });
    expect(screen.getByTestId('recent-chat-p5')).toHaveFocus();

    // Show less by key keeps focus on the toggle.
    more.focus();
    fireEvent.click(more, { detail: 0 });
    expect(more).toHaveTextContent(sidebarCopy.chats.showMore);
    expect(more).toHaveFocus();

    // A pointer's click moves focus to no row: a focused row would show its ⋯.
    fireEvent.click(more, { detail: 1 });
    expect(more).toHaveTextContent(sidebarCopy.chats.showLess);
    expect(more).toHaveFocus();
  });

  it('collapses a folder and remembers it', () => {
    const { unmount } = renderFolders();
    fireEvent.click(toggleOf(NOTES));
    expect(screen.queryByTestId('recent-chat-n0')).toBeNull();
    expect(
      JSON.parse(window.localStorage.getItem(SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY) ?? '[]')
    ).toContain(`folder:${NOTES}`);

    unmount();
    renderFolders();
    expect(screen.queryByTestId('recent-chat-n0')).toBeNull();
    expect(
      within(folder(NOTES)).getByRole('button', { expanded: false, name: 'notes' })
    ).toBeInTheDocument();
  });

  it('opens a collapsed folder that holds the current chat without changing the stored choice', () => {
    window.localStorage.setItem(
      SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY,
      JSON.stringify([`folder:${PROJECT}`])
    );
    renderFolders({ activeSessionId: 'p6' });
    // Open, and far enough that the current row (the seventh) is shown.
    expect(screen.getByTestId('recent-chat-p6')).toHaveAttribute('aria-current', 'page');
    expect(
      JSON.parse(window.localStorage.getItem(SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY) ?? '[]')
    ).toEqual([`folder:${PROJECT}`]);
  });

  it('closes the folder that holds the current chat when its header is clicked, and opens it again', () => {
    // It used to change only the stored choice, which the drawn folder ignores
    // while it holds the current chat: a click that did nothing on screen.
    renderFolders({ activeSessionId: 'p0' });
    expect(screen.getByTestId('recent-chat-p0')).toBeInTheDocument();

    fireEvent.click(toggleOf(PROJECT));
    expect(screen.queryByTestId('recent-chat-p0')).toBeNull();
    expect(
      JSON.parse(window.localStorage.getItem(SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY) ?? '[]')
    ).toContain(`folder:${PROJECT}`);

    fireEvent.click(
      within(folder(PROJECT)).getByRole('button', { expanded: false, name: 'project' })
    );
    expect(screen.getByTestId('recent-chat-p0')).toBeInTheDocument();
    expect(
      JSON.parse(window.localStorage.getItem(SIDEBAR_COLLAPSED_FOLDERS_STORAGE_KEY) ?? '[]')
    ).not.toContain(`folder:${PROJECT}`);
  });

  it('opens the folder again for the next chat that lives in it', () => {
    const { rerender } = renderFolders({ activeSessionId: 'p0' });
    fireEvent.click(toggleOf(PROJECT));
    expect(screen.queryByTestId('recent-chat-p0')).toBeNull();

    // Another chat becomes current, in another folder: this one stays closed.
    const props = {
      sessions: SESSIONS,
      runningSessionIds: new Set<string>(),
      hasMore: false,
      isLoadingMore: false,
      onLoadMore: vi.fn(),
      onOpen: vi.fn(),
      onViewAll: vi.fn(),
      view: FOLDER_VIEW,
      onViewChange: vi.fn(),
      homeDir: '/Users/x',
    };
    rerender(
      <SidebarProvider>
        <RecentChats {...props} activeSessionId="n0" />
      </SidebarProvider>
    );
    expect(screen.queryByTestId('recent-chat-p0')).toBeNull();
    expect(screen.getByTestId('recent-chat-n0')).toBeInTheDocument();

    // A chat in the closed folder becomes current: the folder opens for it.
    rerender(
      <SidebarProvider>
        <RecentChats {...props} activeSessionId="p1" />
      </SidebarProvider>
    );
    expect(screen.getByTestId('recent-chat-p1')).toBeInTheDocument();
  });

  it('puts the full path in the header tooltip, not on the row', () => {
    renderFolders();
    expect(within(folder(PROJECT)).queryByText('~/project')).toBeNull();
  });
});
