import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import {
  SIDEBAR_RENDER_WINDOW,
  SIDEBAR_RENDER_WINDOW_STEP,
  type SidebarChatView,
} from './sidebarChatView';

/**
 * R-28: a view draws the first lines of the whole arranged list and more as the
 * list is scrolled near its end. Every row was drawn before, and mounting 2,000
 * of them took 2 to 4 s per switch of Group by or Sort by.
 */

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
});

const NAME_VIEW: SidebarChatView = { groupBy: 'none', sortBy: 'name' };

function chat(index: number): SessionSummary {
  const at = new Date(Date.now() - index * 60_000).toISOString();
  return {
    id: `s${index}`,
    // Zero-padded so name order is index order: the LAST chat by activity sorts first.
    name: `Chat ${String(999 - index).padStart(3, '0')}`,
    created_at: at,
    updated_at: at,
    working_dir: '/Users/x/project',
    message_count: 2,
    user_set_name: true,
  };
}

const MANY = Array.from({ length: 500 }, (_, index) => chat(index));

function renderList(props: Partial<ComponentProps<typeof RecentChats>> = {}) {
  return render(
    <SidebarProvider>
      <RecentChats
        sessions={MANY}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={NAME_VIEW}
        onViewChange={vi.fn()}
        homeDir="/Users/x"
        {...props}
      />
    </SidebarProvider>
  );
}

const drawnRows = () => document.querySelectorAll('li.br-chat-row-item').length;

/** jsdom has no layout: give the list a height and a scroll position. */
function scrollNearEnd(container: HTMLElement) {
  Object.defineProperty(container, 'clientHeight', { configurable: true, value: 600 });
  Object.defineProperty(container, 'scrollHeight', { configurable: true, value: 1_000 });
  Object.defineProperty(container, 'scrollTop', { configurable: true, writable: true, value: 400 });
  fireEvent.scroll(container);
}

describe('the chat list draws what can be seen (R-28)', () => {
  it('draws the first lines of the WHOLE arranged list, not of the list it was given', () => {
    renderList();
    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW);
    // Sorted over all 500 before the cut: the first drawn row is the one name order puts first.
    expect(screen.getAllByText(/^Chat \d{3}$/)[0]).toHaveTextContent('Chat 500');
  });

  it('draws more rows of what it holds before it asks the daemon for a page', () => {
    const onLoadMore = vi.fn();
    renderList({ hasMore: true, onLoadMore });
    scrollNearEnd(screen.getByTestId('recent-chat-scroll'));
    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW + SIDEBAR_RENDER_WINDOW_STEP);
    expect(onLoadMore).not.toHaveBeenCalled();
  });

  it('asks for the next page once everything it holds is drawn', () => {
    const onLoadMore = vi.fn();
    renderList({ sessions: MANY.slice(0, 20), hasMore: true, onLoadMore });
    expect(drawnRows()).toBe(20);
    scrollNearEnd(screen.getByTestId('recent-chat-scroll'));
    expect(onLoadMore).toHaveBeenCalled();
  });

  it('starts again from the first lines when the view changes', () => {
    const { rerender } = renderList();
    scrollNearEnd(screen.getByTestId('recent-chat-scroll'));
    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW + SIDEBAR_RENDER_WINDOW_STEP);
    rerender(
      <SidebarProvider>
        <RecentChats
          sessions={MANY}
          runningSessionIds={new Set()}
          hasMore={false}
          isLoadingMore={false}
          onLoadMore={vi.fn()}
          onOpen={vi.fn()}
          onViewAll={vi.fn()}
          view={{ groupBy: 'none', sortBy: 'created' }}
          onViewChange={vi.fn()}
          homeDir="/Users/x"
        />
      </SidebarProvider>
    );
    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW);
  });
});

describe('a rename needs its row drawn (R-28)', () => {
  afterEach(() => {
    Object.assign(window, { electron: undefined });
  });

  it('draws the current chat when ⌘⌥R names it, though name order put it past the window', async () => {
    Object.assign(window, { electron: { platform: 'darwin' } });
    // s0 is "Chat 999": the last of 500 by name, far past the first 60 lines.
    renderList({ activeSessionId: 's0' });
    expect(screen.queryByTestId('recent-chat-s0')).toBeNull();
    fireEvent.keyDown(window, { code: 'KeyR', metaKey: true, altKey: true });
    const editor = await screen.findByRole('textbox');
    expect(editor).toHaveValue('Chat 999');
    await waitFor(() => expect(editor).toHaveFocus());
  });
});
