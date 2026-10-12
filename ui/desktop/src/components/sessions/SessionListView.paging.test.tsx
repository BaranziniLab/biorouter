import { act, fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SessionListView from './SessionListView';
import { clearSessionListCache, updateCachedSessionList } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import { LOADING_MORE_CHATS } from './copy';

const mocks = vi.hoisted(() => ({ listSessions: vi.fn() }));

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

// A scroll area that reports its scrolls, which is what pages the list. jsdom
// measures every box as 0, so any scroll event reads as "near the bottom".
vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({
    children,
    handleScroll,
  }: {
    children: ReactNode;
    handleScroll?: (target: HTMLDivElement) => void;
  }) => (
    <div data-testid="scroller" onScroll={(event) => handleScroll?.(event.currentTarget)}>
      {children}
    </div>
  ),
}));

const FIRST_PAGE = 16;
const PAGE = 20;

function chats(count: number): Session[] {
  return Array.from({ length: count }, (_, index) => {
    const when = new Date(Date.now() - (index + 1) * 60_000).toISOString();
    return {
      id: `session-${index}`,
      name: `Chat ${index + 1}`,
      created_at: when,
      updated_at: when,
      extension_data: {},
      message_count: 3,
      working_dir: '/Users/wgu/Desktop',
    } as Session;
  });
}

function renderedRows(): number {
  return document.querySelectorAll('.session-item').length;
}

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
});

describe('SessionListView paging', () => {
  /**
   * Principle 9: nothing moves under the reader for a change that touched none of
   * their rows. The page count used to reset with the ROW count, so deleting one
   * chat, or one arriving from another window, sent a list scrolled several pages
   * down back to its first sixteen.
   */
  it('keeps the pages already revealed when the number of chats changes', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: chats(60) } });
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Chat 1');
    expect(renderedRows()).toBe(FIRST_PAGE);

    fireEvent.scroll(screen.getByTestId('scroller'));
    expect(renderedRows()).toBe(FIRST_PAGE + PAGE);

    // A chat deleted elsewhere: the cache announces it and the list follows.
    await act(async () => {
      updateCachedSessionList((current) => current.filter((row) => row.id !== 'session-3'));
    });

    expect(screen.queryByText('Chat 4')).not.toBeInTheDocument();
    expect(renderedRows()).toBe(FIRST_PAGE + PAGE);
    expect(screen.getByText(`Chat ${FIRST_PAGE + PAGE}`)).toBeInTheDocument();
  });

  it('stops offering more once every chat is showing', async () => {
    mocks.listSessions.mockResolvedValue({ data: { sessions: chats(30) } });
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Chat 1');
    expect(screen.getByText(LOADING_MORE_CHATS)).toBeInTheDocument();

    fireEvent.scroll(screen.getByTestId('scroller'));

    expect(renderedRows()).toBe(30);
    expect(screen.queryByText(LOADING_MORE_CHATS)).not.toBeInTheDocument();
  });
});
