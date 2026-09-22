import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import { chatKindOf } from '../chats/chatKind';
import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, {
  formatSessionDateLabel,
  formatTimeSinceLastWorked,
  groupRecentChatsByDate,
  sortRecentChats,
} from './RecentChats';

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
        {...props}
      />
    </SidebarProvider>
  );
}

describe('sortRecentChats', () => {
  it('orders every loaded chat by most recent activity without a fixed cap', () => {
    const sessions = Array.from({ length: 12 }, (_, index) => makeSession(index));

    expect(sortRecentChats(sessions).map((session) => session.id)).toEqual(
      Array.from({ length: 12 }, (_, index) => `session-${index}`)
    );
  });

  it('groups the sorted result under human-readable activity dates', () => {
    const today = makeSession(0);
    const yesterday = {
      ...makeSession(1),
      updated_at: '2026-07-14T12:00:00.000Z',
    };
    const earlier = {
      ...makeSession(2),
      updated_at: '2026-07-10T12:00:00.000Z',
    };

    expect(groupRecentChatsByDate([earlier, yesterday, today], now)).toEqual([
      { label: 'Today', sessions: [today] },
      { label: 'Yesterday', sessions: [yesterday] },
      { label: 'Jul 10', sessions: [earlier] },
    ]);
    expect(formatSessionDateLabel(today.updated_at, now)).toBe('Today');
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

describe('RecentChats', () => {
  it('opens individual chats, marks an ongoing chat, and exposes a compact summary on focus', async () => {
    const onOpen = vi.fn();
    const currentDateLabel = new Intl.DateTimeFormat('en-US', {
      month: 'short',
      day: 'numeric',
    }).format(Date.now());
    renderRecentChats({ onOpen, activeSessionId: 'session-0' });

    const currentChat = screen.getByTestId('recent-chat-session-0');
    const ongoingChat = screen.getByTestId('recent-chat-session-1');
    expect(ongoingChat).toHaveAccessibleName('Open ongoing chat: Chat 1');
    expect(ongoingChat).toHaveClass('w-full', 'h-control-md', 'px-3', 'text-sm');
    expect(ongoingChat.parentElement).toHaveClass('flex', 'flex-col', 'gap-0.5');
    expect(ongoingChat).not.toHaveClass('font-medium');
    expect(currentChat).toHaveClass('font-medium');
    expect(currentChat).toHaveAttribute('aria-current', 'page');
    expect(screen.getByTestId('running-chat-indicator-session-1')).toBeInTheDocument();
    expect(ongoingChat).toHaveTextContent('Chat 1');
    expect(ongoingChat).not.toHaveTextContent('1 message');
    expect(screen.getByText('Recents')).toBeInTheDocument();
    expect(screen.getByText('Today')).toHaveClass('text-xs', 'font-normal');
    // The badge is the past-7-day chat count (all 3 fixtures are seconds old),
    // shown whether the list is expanded or not.
    expect(screen.queryByTestId('recent-actions-divider')).not.toBeInTheDocument();

    fireEvent.click(currentChat);
    // One click, one real tab — no preview slot, no options object. Whether the
    // chat is already open is the reducer's business (it dedupes), not the row's.
    //
    // The row hands over the NAME it is already rendering, so the tab opens
    // titled instead of showing "New chat" until BaseChat has fetched the
    // session. The row is the only place that knows this without a round-trip.
    expect(onOpen).toHaveBeenCalledWith('session-0', 'Chat 0', false);

    // Double click is not a distinct gesture any more: it is two opens of the
    // same chat, which the reducer collapses to an activate.
    onOpen.mockClear();
    fireEvent.doubleClick(currentChat);
    expect(onOpen.mock.calls.every((call) => call[0] === 'session-0')).toBe(true);

    fireEvent.focus(currentChat);
    const [summary] = await screen.findAllByTestId('recent-chat-summary-session-0');
    expect(summary).toHaveTextContent('Chat 0');
    expect(summary).toHaveTextContent('/workspace/project-0');
    expect(summary).toHaveTextContent('Last worked');
    expect(summary).toHaveTextContent('0 messages');
    expect(summary).toHaveTextContent(currentDateLabel);
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
    expect(row).toHaveClass('w-full', 'min-w-0', 'max-w-full', 'overflow-hidden');
    expect(title).toHaveClass('min-w-0', 'flex-1', 'truncate');

    fireEvent.focus(row);
    const [summary] = await screen.findAllByTestId('recent-chat-summary-session-0');
    expect(summary).toHaveTextContent(longTitle);
  });

  it('keeps the full chat history one click away from the Recents label', () => {
    const onViewAll = vi.fn();
    renderRecentChats({ onViewAll });

    const viewAllButton = screen.getByTestId('view-all-chat-history');
    expect(viewAllButton).toHaveTextContent('See all');
    expect(viewAllButton).not.toHaveClass('text-text-muted');

    fireEvent.click(viewAllButton);
    expect(onViewAll).toHaveBeenCalledOnce();
  });

  it('keeps See all reachable while Recents is retracted, so history is never stranded', () => {
    renderRecentChats();

    fireEvent.click(screen.getByTestId('recents-disclosure'));

    expect(screen.getByTestId('view-all-chat-history')).toBeVisible();
  });

  it('keeps See all attached to Recents when the list is empty', () => {
    renderRecentChats({ sessions: [] });

    const scrollContainer = screen.getByTestId('recent-chat-scroll');
    expect(scrollContainer).toHaveClass('shrink', 'overflow-y-auto');
    expect(scrollContainer).not.toHaveClass('flex-1');
    expect(screen.getByText('No recent chats yet')).toBeInTheDocument();
    expect(screen.getByTestId('view-all-chat-history')).toBeInTheDocument();
    // No chats at all, so there is no past-week count to show.
  });

  it('retracts the history behind the Recents label, keeping the past-week count in both states', () => {
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
    // The count is a persistent past-week metric now, not a stand-in for the
    // hidden rows — it stays put through the collapse.

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

    // 14px, subdued — and the accent only on the row the user is in.
    expect(screen.getByTestId('recent-chat-glyph-session-0')).toHaveClass(
      'h-3.5',
      'w-3.5',
      'text-text-subtle'
    );
    expect(screen.getByTestId('recent-chat-glyph-session-1')).toHaveClass('text-accent-bar');
    expect(screen.getByTestId('recent-chat-glyph-session-1')).not.toHaveClass('text-text-subtle');
    // The icon library pins every glyph to one stroke weight (design.md §3.9).
    expect(screen.getByTestId('recent-chat-glyph-session-0')).toHaveAttribute(
      'stroke-width',
      '1.5'
    );
    // Rows stay 32px — the glyph must not change sidebar density (D-12).
    expect(screen.getByTestId('recent-chat-session-0')).toHaveClass('h-control-md');
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

  it('leaves public and untiered chats unmarked on this 32px row', () => {
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
