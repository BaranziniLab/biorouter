/**
 * History's search counter reads what the highlighter painted, and ⌘G steps
 * through those marks.
 *
 * It used to read `filteredSessions.length` — the chats the list filter kept —
 * over the marks the highlighter paints for every occurrence. `Desktop` read
 * 1/1039 with 996 marks, and each step called `setCurrentMatch(chatIndex)`, so
 * "5/1039" lit the fifth MARK, which could sit in any chat. The real SearchView
 * and SearchBar run here; only the highlighter is faked, because jsdom lays
 * nothing out and the real one would find no visible match at all.
 */
import { act, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  /** How many marks the fake highlighter paints — more than there are chats. */
  marks: 5,
  setCurrentMatch: vi.fn(),
}));

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

vi.mock('../../utils/searchHighlighter', () => ({
  SearchHighlighter: class {
    highlight() {
      return Array.from({ length: mocks.marks }, () => document.createElement('div'));
    }
    setCurrentMatch(index: number, shouldScroll?: boolean) {
      mocks.setCurrentMatch(index, shouldScroll);
    }
    clearHighlights() {}
    destroy() {}
  },
}));

const session = (id: string, name: string): Session =>
  ({
    id,
    name,
    working_dir: '/Users/someone/Desktop',
    created_at: '2026-09-12T12:00:00Z',
    updated_at: '2026-09-12T12:00:00Z',
    extension_data: {},
    message_count: 2,
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({
    data: { sessions: [session('a', 'Desktop cleanup'), session('b', 'Desktop icons')] },
  });
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

/** The "n/m" beside the search box, or null when it shows none. */
const counter = () => screen.queryByText(/^\d+\/\d+$/)?.textContent ?? null;

describe('SessionListView — the search counter is the highlighter’s', () => {
  it('counts the painted marks, not the chats the filter kept, and steps through them', async () => {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Desktop icons');

    fireEvent.keyDown(window, { key: 'f', metaKey: true });
    const input = await screen.findByPlaceholderText('Search history...');
    const user = userEvent.setup();
    await user.type(input, 'Desktop');

    await waitFor(() => expect(counter()).toBe('1/5'));
    // Past the list filter's own 300 ms debounce, which is when the old wiring
    // replaced the counter with the number of chats (1/2).
    await act(() => new Promise((resolve) => setTimeout(resolve, 600)));
    expect(counter()).toBe('1/5');

    mocks.setCurrentMatch.mockClear();
    await user.keyboard('{Enter}{Enter}{Enter}');
    expect(counter()).toBe('4/5');
    expect(mocks.setCurrentMatch).toHaveBeenLastCalledWith(3, true);

    // Past the last chat's index, which is where the old step wrapped.
    await user.keyboard('{Enter}');
    expect(counter()).toBe('5/5');
    expect(mocks.setCurrentMatch).toHaveBeenLastCalledWith(4, true);
  });
});
