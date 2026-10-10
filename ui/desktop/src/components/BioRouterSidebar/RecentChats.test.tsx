import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatKindOf } from '../chats/chatKind';
import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, { chatHoverDetail, formatTimeSinceLastWorked } from './RecentChats';
import { sidebarCopy } from './copy';
import { DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';

const now = Date.parse('2026-07-15T12:00:00.000Z');

beforeEach(() => {
  // The disclosure persists to localStorage, which survives between tests.
  window.localStorage.clear();
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

function makeSession(index: number, referenceTime = now): SessionSummary {
  const updatedAt = new Date(referenceTime - index * 60_000).toISOString();
  return {
    id: `session-${index}`,
    name: `Chat ${index}`,
    created_at: updatedAt,
    updated_at: updatedAt,
    working_dir: `/workspace/project-${index}`,
    message_count: index,
    user_set_name: false,
  };
}

function renderRecentChats(props: Partial<ComponentProps<typeof RecentChats>> = {}) {
  const currentTime = Date.now();
  return render(
    <SidebarProvider>
      <RecentChats
        sessions={[
          makeSession(2, currentTime),
          makeSession(0, currentTime),
          makeSession(1, currentTime),
        ]}
        runningSessionIds={new Set(['session-1'])}
        hasMore={false}
        isLoadingMore={false}
        onLoadMore={vi.fn()}
        onOpen={vi.fn()}
        onViewAll={vi.fn()}
        view={DEFAULT_SIDEBAR_CHAT_VIEW}
        onViewChange={vi.fn()}
        {...props}
      />
    </SidebarProvider>
  );
}

describe('the date buckets', () => {
  /**
   * Coarse buckets, never a header per day (spec 3.4, F-07): the arranging
   * itself is tested in `sidebarChatView.test.ts`; this pins what the list
   * draws for it.
   */
  it('draws one quiet bucket label over the day, not a date per day', () => {
    renderRecentChats();
    const today = screen.getByText('Today');
    expect(today).toHaveClass('br-sidebar-bucket');
    expect(screen.queryByText(/^[A-Z][a-z]{2} \d{1,2}$/)).toBeNull();
  });
});

describe('sessionKind', () => {
  // `SessionSummary` exposes no kind/branch field, so the title is the only
  // signal available — see the note on `sessionKind`.
  it('reads the session kind off the title', () => {
    const kindOf = (name: string) => chatKindOf({ ...makeSession(0), name });

    expect(kindOf('Status check-in')).toBe('chat');
    expect(kindOf('Greeting 2 (branch 1)')).toBe('branch');
    expect(kindOf('Multiple sclerosis knowledge graph (branch 12)')).toBe('branch');
    expect(kindOf('app:spec-002-cohort-followup')).toBe('app');
    expect(kindOf('  app:padded  ')).toBe('app');
  });

  it('does not mistake prose that merely mentions a branch for a real branch', () => {
    expect(chatKindOf({ ...makeSession(0), name: 'Which git branch 2 use?' })).toBe('chat');
    expect(chatKindOf({ ...makeSession(0), name: 'Refactor the app: rename it' })).toBe('chat');
  });
});

describe('formatTimeSinceLastWorked', () => {
  it('renders concise elapsed time suitable for the sidebar summary', () => {
    expect(formatTimeSinceLastWorked(new Date(now - 36 * 60_000).toISOString(), now)).toBe(
      '36m ago'
    );
    expect(formatTimeSinceLastWorked(new Date(now - 3 * 24 * 60 * 60_000).toISOString(), now)).toBe(
      '3d ago'
    );
  });
});

/**
 * A keyboard user arriving on a row. The shared tooltip opens on focus only when
 * the Tab key moved it (Q2-56) — a focus a program restores opens nothing — so
 * the summary is reached the way a person reaches it: Tab down, focus, Tab up.
 */
function focusWithTab(element: HTMLElement) {
  fireEvent.keyDown(document.body, { key: 'Tab' });
  fireEvent.focus(element);
  fireEvent.keyUp(document.body, { key: 'Tab' });
}

describe('RecentChats', () => {
  it('opens individual chats, marks an ongoing chat, and exposes a compact summary on focus', async () => {
    const onOpen = vi.fn();
    renderRecentChats({ onOpen, activeSessionId: 'session-0' });

    const currentChat = screen.getByTestId('recent-chat-session-0');
    const ongoingChat = screen.getByTestId('recent-chat-session-1');
    expect(ongoingChat).toHaveAccessibleName('Open ongoing chat: Chat 1');
    // One authored row recipe (`sidebar.css`): 28px, 13px, muted at rest, the
    // accent rail on the current row. jsdom runs no CSS, so the class contract
    // is pinned here and the geometry in `sidebarGeometry.browser.test.ts`.
    expect(ongoingChat).toHaveClass('br-nav-row', 'br-chat-row');
    expect(ongoingChat.closest('ul')).toHaveClass('br-nav-list');
    expect(currentChat).toHaveAttribute('aria-current', 'page');
    expect(ongoingChat).not.toHaveAttribute('aria-current');
    expect(screen.getByTestId('running-chat-indicator-session-1')).toHaveClass('br-chat-row-ring');
    expect(ongoingChat).toHaveTextContent('Chat 1');
    expect(ongoingChat).not.toHaveTextContent('1 message');
    expect(screen.getByTestId('recents-disclosure')).toHaveTextContent(sidebarCopy.chats.header);

    fireEvent.click(currentChat);
    // One click, one real tab. The row hands over the NAME it is already
    // rendering, so the tab opens titled.
    expect(onOpen).toHaveBeenCalledWith('session-0', 'Chat 0', false);

    // Double click is not a distinct gesture: it is two opens of the same chat,
    // which the reducer collapses to an activate. Never a rename.
    onOpen.mockClear();
    fireEvent.doubleClick(currentChat);
    expect(onOpen.mock.calls.every((call) => call[0] === 'session-0')).toBe(true);
    expect(screen.queryByRole('textbox')).toBeNull();

    // Two lines at most: the title only when it is cut short (jsdom measures
    // nothing, so it is not), then `path · time · messages` in one line.
    focusWithTab(currentChat);
    const [summary] = await screen.findAllByTestId('recent-chat-summary-session-0');
    expect(summary).toHaveTextContent('/workspace/project-0 · Just now · 0 messages');
    expect(summary.querySelector('svg')).toBeNull();
  });

  it('preserves a user-chosen title that matches the legacy placeholder', () => {
    const onOpen = vi.fn();
    renderRecentChats({
      sessions: [{ ...makeSession(0), name: 'New Session', user_set_name: true }],
      onOpen,
    });

    fireEvent.click(screen.getByTestId('recent-chat-session-0'));

    expect(onOpen).toHaveBeenCalledWith('session-0', 'New Session', true);
  });

  it('truncates an overlong row title while preserving the full hover summary', async () => {
    const longTitle = 'app:ucsf-versa-gpt55-kb-visualizer-smoke-with-an-even-longer-suffix';
    const longSession = { ...makeSession(0), name: longTitle };
    renderRecentChats({ sessions: [longSession] });

    const row = screen.getByTestId('recent-chat-session-0');
    const title = screen.getByText(longTitle);
    expect(row).toHaveClass('br-nav-row');
    expect(title).toHaveClass('br-nav-row-label');

    // A cut title earns the card's first line.
    Object.defineProperties(title, {
      scrollWidth: { configurable: true, value: 400 },
      clientWidth: { configurable: true, value: 180 },
    });
    focusWithTab(row);
    const [summary] = await screen.findAllByTestId('recent-chat-summary-session-0');
    expect(summary).toHaveTextContent(longTitle);
  });

  it('keeps the full chat history one click away from the Chats header', () => {
    const onViewAll = vi.fn();
    renderRecentChats({ onViewAll });

    // A 24px icon button named "All chats", in place of the "See all" link.
    const viewAllButton = screen.getByTestId('view-all-chat-history');
    expect(viewAllButton).toHaveAccessibleName(sidebarCopy.chats.allChats);
    expect(viewAllButton).not.toHaveTextContent('See all');
    expect(screen.getByRole('button', { name: sidebarCopy.chats.viewOptions })).toHaveAttribute(
      'aria-haspopup',
      'menu'
    );

    fireEvent.click(viewAllButton);
    expect(onViewAll).toHaveBeenCalledOnce();
  });

  it('reads Chats in sentence case, with no caps label and no See all', () => {
    renderRecentChats();
    expect(screen.queryByText('Recents')).toBeNull();
    expect(screen.queryByText('RECENTS')).toBeNull();
    expect(screen.queryByText('See all')).toBeNull();
    expect(screen.getByText(sidebarCopy.chats.header)).toBeInTheDocument();
  });

  it('keeps All chats reachable while the list is retracted, so history is never stranded', () => {
    renderRecentChats();

    fireEvent.click(screen.getByTestId('recents-disclosure'));

    expect(screen.getByTestId('view-all-chat-history')).toBeVisible();
  });

  it('keeps All chats beside the header when the list is empty', () => {
    renderRecentChats({ sessions: [] });

    expect(screen.getByTestId('recent-chat-scroll')).toHaveClass('br-sidebar-chats-scroll');
    expect(screen.getByText(sidebarCopy.chats.empty)).toBeInTheDocument();
    expect(screen.getByTestId('view-all-chat-history')).toBeInTheDocument();
  });

  it('retracts the history behind the Chats header', () => {
    renderRecentChats();

    const disclosure = screen.getByTestId('recents-disclosure');
    const scrollWell = screen.getByTestId('recent-chat-scroll');
    expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    expect(disclosure).toHaveAttribute('aria-controls', scrollWell.id);
    expect(scrollWell).toBeVisible();
    expect(screen.getByTestId('recent-chat-session-0')).toBeVisible();

    fireEvent.click(disclosure);

    expect(disclosure).toHaveAttribute('aria-expanded', 'false');
    expect(scrollWell).not.toBeVisible();
    expect(screen.getByTestId('recent-chat-session-0')).not.toBeVisible();

    fireEvent.click(disclosure);

    expect(disclosure).toHaveAttribute('aria-expanded', 'true');
    expect(scrollWell).toBeVisible();
  });

  it('restores the retracted state from storage on the next mount', () => {
    const { unmount } = renderRecentChats();
    fireEvent.click(screen.getByTestId('recents-disclosure'));
    unmount();

    renderRecentChats();

    expect(screen.getByTestId('recents-disclosure')).toHaveAttribute('aria-expanded', 'false');
    expect(screen.getByTestId('recent-chat-scroll')).not.toBeVisible();
  });

  it('leads each row with a glyph for the session kind and accents the active one', () => {
    const sessions = [
      { ...makeSession(0), name: 'Status check-in' },
      { ...makeSession(1), name: 'Greeting 2 (branch 1)' },
      { ...makeSession(2), name: 'app:spec-002-cohort-followup' },
    ];
    renderRecentChats({ sessions, activeSessionId: 'session-1' });

    expect(screen.getByTestId('recent-chat-glyph-session-0')).toHaveAttribute(
      'data-chat-kind',
      'chat'
    );
    expect(screen.getByTestId('recent-chat-glyph-session-1')).toHaveAttribute(
      'data-chat-kind',
      'branch'
    );
    expect(screen.getByTestId('recent-chat-glyph-session-2')).toHaveAttribute(
      'data-chat-kind',
      'app'
    );

    // A 16px slot (`.br-chat-kind-icon`), the body muted, default ink on the
    // row the user is in; the lock badge alone wears the accent (spec 3.3).
    expect(screen.getByTestId('recent-chat-glyph-session-0')).toHaveClass(
      'br-chat-kind-icon',
      'text-text-muted'
    );
    expect(screen.getByTestId('recent-chat-glyph-session-1')).toHaveClass('text-text-default');
    expect(screen.getByTestId('recent-chat-glyph-session-1')).not.toHaveClass('text-accent-bar');
    // The icon library pins every glyph to one stroke weight (design.md §3.9).
    expect(screen.getByTestId('recent-chat-glyph-session-0')).toHaveAttribute(
      'stroke-width',
      '1.5'
    );
  });

  it('requests another page when the user scrolls near the end of the loaded chats', () => {
    const onLoadMore = vi.fn();
    renderRecentChats({ hasMore: true, onLoadMore });

    const scrollContainer = screen.getByTestId('recent-chat-scroll');
    Object.defineProperties(scrollContainer, {
      clientHeight: { configurable: true, value: 100 },
      scrollHeight: { configurable: true, value: 500 },
      scrollTop: { configurable: true, value: 350 },
    });

    fireEvent.scroll(scrollContainer);
    expect(onLoadMore).toHaveBeenCalledOnce();
  });

  // This block deliberately never names the badge component: Task 27's gate
  // greps src/components for that name and expects an exact file list.
  it('marks a private chat in the sidebar rail', () => {
    renderRecentChats({
      sessions: [{ ...makeSession(0), privacy_tier: 'private' }],
    });
    const glyph = screen.getByTestId('recent-chat-glyph-session-0');
    expect(glyph).toHaveAttribute('data-privacy', 'private');
    // ⚠ The tier is carried by the GLYPH's shape, not by a hue, so it survives
    // for anyone who cannot separate the two inks. A private plain chat is the
    // padlocked bubble; a public one is not.
    expect(glyph.getAttribute('aria-label')).toBe('Private chat');
  });

  it('leaves public and untiered chats unmarked on this 28px row', () => {
    renderRecentChats({
      sessions: [
        { ...makeSession(0), privacy_tier: 'public' },
        { ...makeSession(1) },
        { ...makeSession(2), privacy_tier: 'private' },
      ],
    });
    // Every row has a glyph now — the marker is which one, not whether one is
    // there. Exactly one row may claim the private tier, and an untiered row
    // must not: reading "no tier recorded" as private would mark half the
    // history, and reading it as private-looking is the same failure. Nor may
    // it claim Public, which is the same guess the other way round — it is
    // drawn as not yet known.
    const glyphs = [0, 1, 2].map((i) => screen.getByTestId(`recent-chat-glyph-session-${i}`));
    expect(glyphs.map((g) => g.getAttribute('data-privacy'))).toEqual([
      'public',
      'unknown',
      'private',
    ]);
    expect(glyphs.filter((g) => g.getAttribute('aria-label') === 'Private chat')).toHaveLength(1);
  });

  it('shows the privacy marker and the running indicator on the same row', () => {
    renderRecentChats({
      sessions: [{ ...makeSession(1), privacy_tier: 'private' }],
      runningSessionIds: new Set(['session-1']),
    });
    expect(screen.getByTestId('running-chat-indicator-session-1')).toBeInTheDocument();
    expect(screen.getByTestId('recent-chat-glyph-session-1')).toHaveAttribute(
      'data-privacy',
      'private'
    );
  });
});

describe('chatHoverDetail', () => {
  it('writes the folder with ~ for home, the time and the count on one line', () => {
    const session = { ...makeSession(3), working_dir: '/Users/me/lab/cohort' };
    expect(chatHoverDetail(session, '/Users/me', now)).toBe('~/lab/cohort · 3m ago · 3 messages');
  });
});
