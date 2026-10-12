import { fireEvent, render, screen } from '@testing-library/react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, { scrollRowIntoView } from './RecentChats';
import { SIDEBAR_RENDER_WINDOW, type SidebarChatView } from './sidebarChatView';

/**
 * Vision V1-R2-2, round 4 (R-28). The list grew as it was scrolled and never
 * shrank, so after a scroll to its end every chat was a mounted row: with
 * 5,542 drawn, a right-click took about 2.2 s before its menu showed and a
 * view switch 0.5 s. The list now draws only the lines near the viewport (the
 * rest are placeholders of their exact height, measured in Chromium by
 * `sidebarFill.browser.test.tsx`), and its right-click menu is not modal: a
 * modal menu sets `pointer-events` on <body>, which restyled every element in
 * the window when it opened and again when it closed.
 *
 * jsdom has no layout, so here the list cannot be measured and draws every
 * line it reaches, as before; the drawn range itself is tested in Chromium.
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

const BY_ACTIVITY: SidebarChatView = { groupBy: 'none', sortBy: 'activity' };

function chat(index: number): SessionSummary {
  const at = new Date(Date.now() - index * 60_000).toISOString();
  return {
    id: `s${index}`,
    name: `Chat ${index}`,
    created_at: at,
    updated_at: at,
    working_dir: '/Users/x/project',
    message_count: 2,
    user_set_name: true,
  };
}

function renderList(count: number) {
  return render(
    <SidebarProvider>
      <RecentChats
        sessions={Array.from({ length: count }, (_, index) => chat(index))}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={BY_ACTIVITY}
        onViewChange={vi.fn()}
        homeDir="/Users/x"
      />
    </SidebarProvider>
  );
}

describe('the chat list draws the rows near the viewport (V1-R2-2, round 4)', () => {
  it('leaves <body> alone when its right-click menu opens: the menu is not modal', async () => {
    renderList(5);
    fireEvent.contextMenu(screen.getByTestId('recent-chat-s2'));
    expect(await screen.findAllByRole('menuitem')).not.toHaveLength(0);
    // A modal menu sets `pointer-events: none` on <body> and locks its scroll.
    expect(document.body.style.pointerEvents).toBe('');
    expect(document.body.hasAttribute('data-scroll-locked')).toBe(false);
    // The row the menu is about stays lit while it is open.
    expect(screen.getByTestId('recent-chat-s2').closest('li')?.hasAttribute('data-menu-open')).toBe(
      true
    );
  });

  it('draws every line it reaches when the list cannot be measured, with no placeholder', () => {
    const { container } = renderList(100);
    expect(container.querySelectorAll('li.br-chat-row-item')).toHaveLength(SIDEBAR_RENDER_WINDOW);
    expect(container.querySelector('.br-chat-row-gap')).toBeNull();
    // Each group says where its rows start, for the line math.
    expect(container.querySelector('section')?.getAttribute('data-rows-line')).toBe('0');
  });

  it('shows a row’s header only when the row is its group’s first line, not its first drawn row', () => {
    const container = document.createElement('div');
    const section = document.createElement('section');
    const list = document.createElement('ul');
    const gap = document.createElement('li');
    gap.className = 'br-chat-row-gap';
    const item = document.createElement('li');
    const row = document.createElement('button');
    row.setAttribute('data-row', 'chat:deep');
    item.append(row);
    list.append(gap, item);
    section.append(list);
    container.append(section);
    const rect = (top: number, height: number) => ({ top, bottom: top + height }) as DOMRect;
    container.getBoundingClientRect = () => rect(100, 300);
    // The group's header is 3,000px up; the rows between are a placeholder.
    section.getBoundingClientRect = () => rect(-2_900, 3_100);
    row.getBoundingClientRect = () => rect(60, 28);
    container.scrollTop = 5_000;
    scrollRowIntoView(container, row);
    // Just far enough for the row, not up to its group's header past every row above it.
    expect(container.scrollTop).toBe(4_960);
  });
});
