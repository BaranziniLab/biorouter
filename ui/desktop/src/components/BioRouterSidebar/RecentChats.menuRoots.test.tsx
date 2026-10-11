import { fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import { SIDEBAR_RENDER_WINDOW, type SidebarChatView } from './sidebarChatView';

/**
 * Vision V1-R2-2, round 3 (R-28): a mounted Radix menu root adds a capture
 * `keydown` listener to the document, and on every key press each of those
 * adds a `pointerdown` and a `pointermove` listener. A ContextMenu and a
 * DropdownMenu root in every drawn row made every key press and every view
 * switch pay for every row: with 4,233 rows drawn after a scroll to the end,
 * a production build took 0.59 s to switch views and 0.28 s to open the view
 * menu, most of it in `addEventListener` and `removeEventListener`. The list
 * now has one right-click menu, and a row's ⋯ mounts its menu when it is used.
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

afterEach(() => {
  vi.restoreAllMocks();
});

const BY_NAME: SidebarChatView = { groupBy: 'none', sortBy: 'name' };
const BY_DATE: SidebarChatView = { groupBy: 'date', sortBy: 'activity' };

function chat(index: number): SessionSummary {
  const at = new Date(Date.now() - index * 60_000).toISOString();
  return {
    id: `s${index}`,
    name: `Chat ${String(999 - index).padStart(3, '0')}`,
    created_at: at,
    updated_at: at,
    working_dir: '/Users/x/project',
    message_count: 2,
    user_set_name: true,
  };
}

const MANY = Array.from({ length: 200 }, (_, index) => chat(index));

function list(view: SidebarChatView) {
  return (
    <SidebarProvider>
      <RecentChats
        sessions={MANY}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={view}
        onViewChange={vi.fn()}
        homeDir="/Users/x"
      />
    </SidebarProvider>
  );
}

/** Document listeners of `type` added while `run` runs. */
function documentListenersAddedBy(type: string, run: () => void): number {
  const add = vi.spyOn(document, 'addEventListener');
  run();
  const added = add.mock.calls.filter(([name]) => name === type).length;
  add.mockRestore();
  return added;
}

describe('the list mounts no menu root per row (V1-R2-2)', () => {
  it('draws its rows without a document keydown listener for each', () => {
    let listeners = 0;
    listeners = documentListenersAddedBy('keydown', () => {
      render(list(BY_NAME));
    });
    expect(document.querySelectorAll('li.br-chat-row-item')).toHaveLength(SIDEBAR_RENDER_WINDOW);
    // The view menu's own root and the list's right-click menu: not one per row.
    expect(listeners).toBeLessThan(5);
  });

  it('adds no listener per row on a key press', () => {
    render(list(BY_NAME));
    const added = documentListenersAddedBy('pointerdown', () => {
      fireEvent.keyDown(document.body, { key: 'ArrowDown' });
    });
    expect(added).toBeLessThan(5);
  });

  it('keeps focus on a ⋯ its focus armed, and opens its menu from the keyboard', async () => {
    render(list(BY_NAME));
    const more = screen.getByTestId('recent-chat-more-s199');
    expect(more).toHaveAttribute('aria-haspopup', 'menu');
    expect(more).toHaveAttribute('aria-expanded', 'false');

    more.focus();
    const armed = screen.getByTestId('recent-chat-more-s199');
    expect(armed).toHaveFocus();
    fireEvent.keyDown(armed, { key: 'Enter' });

    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    expect(screen.getByTestId('recent-chat-more-s199')).toHaveAttribute('aria-expanded', 'true');
  });

  it('opens the right-click menu on the row it came from, and on nothing else', async () => {
    render(list(BY_DATE));
    fireEvent.contextMenu(document.querySelector('.br-sidebar-bucket')!);
    expect(screen.queryByRole('menu')).toBeNull();

    fireEvent.contextMenu(screen.getByTestId('recent-chat-s3'));
    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    const lit = [...document.querySelectorAll('li.br-chat-row-item[data-menu-open]')];
    expect(lit.map((li) => li.querySelector('[data-row]')?.getAttribute('data-row'))).toEqual([
      'chat:s3',
    ]);
  });
});
