import { act, fireEvent, render, screen } from '@testing-library/react';
import { createElement, type ComponentProps } from 'react';
import { afterEach, beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';

import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats from './RecentChats';
import {
  SIDEBAR_RENDER_WINDOW,
  sidebarRenderWindow,
  type SidebarChatView,
} from './sidebarChatView';

/**
 * Vision V1-R2-2 (R-28): a switch of Group by or Sort by once the list is held
 * took 300 to 370 ms in the development build against a 100 ms target. What it
 * paid for: every drawn row mounted twice (the view's generation and window
 * were set from an effect, one commit late), each row built two closed menus
 * of ten items, every list re-render re-rendered every row, and 60 lines were
 * drawn to show about 21.
 */

const counts = vi.hoisted(() => ({
  glyphs: [] as string[],
  contextMenus: [] as string[],
  dropdowns: [] as string[],
}));

// Each row draws one glyph, keyed by its chat, so a glyph render is a row render.
vi.mock('../chats/ChatKindIcon', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../chats/ChatKindIcon')>();
  return {
    ...actual,
    ChatKindIcon: (props: ComponentProps<typeof actual.ChatKindIcon>) => {
      counts.glyphs.push((props.testId ?? '').replace('recent-chat-glyph-', ''));
      return createElement(actual.ChatKindIcon, props);
    },
  };
});

vi.mock('../chats/ChatRowContextMenu', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../chats/ChatRowContextMenu')>();
  return {
    ...actual,
    ChatRowContextMenuContent: (props: ComponentProps<typeof actual.ChatRowContextMenuContent>) => {
      counts.contextMenus.push(props.target.sessionId);
      return createElement(actual.ChatRowContextMenuContent, props);
    },
    ChatRowDropdownMenuContent: (
      props: ComponentProps<typeof actual.ChatRowDropdownMenuContent>
    ) => {
      counts.dropdowns.push(props.target.sessionId);
      return createElement(actual.ChatRowDropdownMenuContent, props);
    },
  };
});

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
  counts.glyphs.length = 0;
  counts.contextMenus.length = 0;
  counts.dropdowns.length = 0;
});

const BY_NAME: SidebarChatView = { groupBy: 'none', sortBy: 'name' };
const BY_CREATED: SidebarChatView = { groupBy: 'none', sortBy: 'created' };

function chat(index: number): SessionSummary {
  const at = new Date(Date.now() - index * 60_000).toISOString();
  return {
    id: `s${index}`,
    // Name order is the reverse of creation order, so the two views draw different rows.
    name: `Chat ${String(999 - index).padStart(3, '0')}`,
    created_at: at,
    updated_at: at,
    working_dir: '/Users/x/project',
    message_count: 2,
    user_set_name: true,
  };
}

const MANY = Array.from({ length: 500 }, (_, index) => chat(index));

function list(view: SidebarChatView, onOpen = vi.fn()) {
  return (
    <SidebarProvider>
      <RecentChats
        sessions={MANY}
        runningSessionIds={new Set()}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        // A new closure on every render, as the sidebar passes it.
        onOpen={(...args) => onOpen(...args)}
        onViewAll={vi.fn()}
        view={view}
        onViewChange={vi.fn()}
        homeDir="/Users/x"
      />
    </SidebarProvider>
  );
}

const drawnRows = () => document.querySelectorAll('li.br-chat-row-item').length;

/** Rows put into the document while `change` runs. */
function rowsInsertedBy(change: () => void): number {
  const scroll = screen.getByTestId('recent-chat-scroll');
  const observer = new MutationObserver(() => {});
  observer.observe(scroll, { childList: true, subtree: true });
  change();
  const inserted = observer
    .takeRecords()
    .flatMap((record) => [...record.addedNodes])
    .reduce(
      (total, node) =>
        node instanceof Element
          ? total +
            (node.matches('li.br-chat-row-item') ? 1 : 0) +
            node.querySelectorAll('li.br-chat-row-item').length
          : total,
      0
    );
  observer.disconnect();
  return inserted;
}

/** jsdom has no layout: give the list a height and a scroll position near its end. */
function scrollNearEnd(container: HTMLElement) {
  Object.defineProperty(container, 'clientHeight', { configurable: true, value: 600 });
  Object.defineProperty(container, 'scrollHeight', { configurable: true, value: 1_000 });
  Object.defineProperty(container, 'scrollTop', { configurable: true, writable: true, value: 400 });
  fireEvent.scroll(container);
}

describe('a view switch draws each row once (V1-R2-2)', () => {
  it('mounts the rows it draws once, not once under the old view and again under the new', () => {
    const { rerender } = render(list(BY_NAME));
    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW);

    const inserted = rowsInsertedBy(() => rerender(list(BY_CREATED)));

    expect(drawnRows()).toBe(SIDEBAR_RENDER_WINDOW);
    expect(inserted).toBe(SIDEBAR_RENDER_WINDOW);
  });

  it('never draws the lines scrolled into view under the old view first', () => {
    const { rerender } = render(list(BY_NAME));
    scrollNearEnd(screen.getByTestId('recent-chat-scroll'));
    expect(drawnRows()).toBeGreaterThan(SIDEBAR_RENDER_WINDOW);

    const inserted = rowsInsertedBy(() => rerender(list(BY_CREATED)));

    expect(inserted).toBe(SIDEBAR_RENDER_WINDOW);
  });
});

describe('a row builds what it shows on request only when asked (V1-R2-2)', () => {
  it('builds no menu for a closed row, and only the opened row’s menu on a right-click', async () => {
    render(list(BY_NAME));
    expect(counts.contextMenus).toEqual([]);
    expect(counts.dropdowns).toEqual([]);

    fireEvent.contextMenu(screen.getByTestId('recent-chat-s499'));

    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    expect(new Set(counts.contextMenus)).toEqual(new Set(['s499']));
    expect(counts.dropdowns).toEqual([]);
  });

  it('builds the ⋯ menu when the ⋯ button opens it', async () => {
    render(list(BY_NAME));
    fireEvent.pointerDown(screen.getByTestId('recent-chat-more-s498'), {
      button: 0,
      ctrlKey: false,
    });

    expect(await screen.findByRole('menuitem', { name: 'Rename' })).toBeInTheDocument();
    expect(new Set(counts.dropdowns)).toEqual(new Set(['s498']));
    expect(counts.contextMenus).toEqual([]);
  });
});

describe('a list re-render leaves the rows alone (V1-R2-2)', () => {
  it('re-renders no row when the pointer comes onto the list', () => {
    const onOpen = vi.fn();
    render(list(BY_NAME, onOpen));
    counts.glyphs.length = 0;

    fireEvent.pointerEnter(screen.getByTestId('recent-chat-scroll'));

    expect(counts.glyphs).toEqual([]);
    // The stable opener still reaches the latest one the sidebar passed.
    fireEvent.click(screen.getByTestId('recent-chat-s499'));
    expect(onOpen).toHaveBeenCalledWith('s499', 'Chat 500', true);
  });

  it('re-renders only the row being renamed while its name is typed', () => {
    render(list(BY_NAME));
    const row = screen.getByTestId('recent-chat-s499');
    row.focus();
    fireEvent.keyDown(row, { key: 'F2' });
    const editor = screen.getByRole('textbox');
    counts.glyphs.length = 0;

    fireEvent.change(editor, { target: { value: 'Chat 500 renamed' } });

    expect(editor).toHaveValue('Chat 500 renamed');
    expect(new Set(counts.glyphs)).toEqual(new Set(['s499']));
  });
});

describe('a view draws what the list shows (V1-R2-2)', () => {
  type ObserverCallback = ConstructorParameters<typeof ResizeObserver>[0];
  type Observed = { callback: ObserverCallback; targets: Element[] };
  let observers: Observed[] = [];

  beforeEach(() => {
    observers = [];
    vi.stubGlobal(
      'ResizeObserver',
      class {
        private entry: Observed;
        constructor(callback: ObserverCallback) {
          this.entry = { callback, targets: [] };
          observers.push(this.entry);
        }
        observe(target: Element) {
          this.entry.targets.push(target);
        }
        unobserve() {}
        disconnect() {
          this.entry.targets = [];
        }
      }
    );
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('draws the list’s height in lines plus the overscan after a switch', () => {
    const { rerender } = render(list(BY_NAME));
    const container = screen.getByTestId('recent-chat-scroll');
    // The list of a 1440 x 900 window: 613px, 21 rows.
    Object.defineProperty(container, 'clientHeight', { configurable: true, value: 613 });
    Object.defineProperty(container, 'scrollHeight', { configurable: true, value: 1_800 });
    act(() => {
      for (const { callback, targets } of observers) {
        if (targets.includes(container)) callback([], {} as ResizeObserver);
      }
    });

    rerender(list(BY_CREATED));

    expect(drawnRows()).toBe(sidebarRenderWindow(613));
    expect(drawnRows()).toBeLessThan(SIDEBAR_RENDER_WINDOW);
  });
});
