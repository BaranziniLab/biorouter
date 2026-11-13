/**
 * DR-33, owner decision 3: a chat a public decision model made public says so in
 * its row's hover card, and nowhere else. No lock, no new glyph: the row draws
 * exactly what an ordinary public chat draws, and the reason is a peek.
 */
import { fireEvent, render, screen } from '@testing-library/react';
import type { ComponentProps } from 'react';
import { beforeAll, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionSummary } from '../../api';
import { SidebarProvider } from '../ui/sidebar';
import RecentChats, { chatPublicNote } from './RecentChats';
import { sidebarCopy } from './copy';
import { DEFAULT_SIDEBAR_CHAT_VIEW } from './sidebarChatView';
import { withPrivacyTier } from './useSidebarSessions';

beforeEach(() => {
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

function makeSession(id: string, extra: Partial<SessionSummary> = {}): SessionSummary {
  const at = new Date().toISOString();
  return {
    id,
    name: `Chat ${id}`,
    created_at: at,
    updated_at: at,
    working_dir: `/workspace/${id}`,
    message_count: 2,
    user_set_name: false,
    privacy_tier: 'public',
    ...extra,
  };
}

const LOWERED: Partial<SessionSummary> = {
  public_because: { model: 'joey-1', place: 'Baseten' },
};

function renderRows(sessions: SessionSummary[]) {
  const props: ComponentProps<typeof RecentChats> = {
    sessions,
    runningSessionIds: new Set<string>(),
    hasMore: false,
    isLoadingMore: false,
    onLoadMore: vi.fn(),
    onOpen: vi.fn(),
    onViewAll: vi.fn(),
    view: DEFAULT_SIDEBAR_CHAT_VIEW,
    onViewChange: vi.fn(),
  };
  return render(
    <SidebarProvider>
      <RecentChats {...props} />
    </SidebarProvider>
  );
}

/** The shared tooltip opens on a Tab-moved focus, the way a person reaches it. */
function focusWithTab(element: HTMLElement) {
  fireEvent.keyDown(document.body, { key: 'Tab' });
  fireEvent.focus(element);
  fireEvent.keyUp(document.body, { key: 'Tab' });
}

describe('chatPublicNote', () => {
  it('names the decision model and where it runs', () => {
    expect(chatPublicNote(makeSession('a', LOWERED))).toBe(
      'Public: its decision model joey-1 runs on Baseten.'
    );
  });

  it('counts the other public decision models without naming them', () => {
    expect(
      chatPublicNote(
        makeSession('a', { public_because: { model: 'joey-1', place: 'Baseten', others: 1 } })
      )
    ).toBe(
      'Public: its decision model joey-1 runs on Baseten, and 1 other decision model is public too.'
    );
    expect(
      chatPublicNote(
        makeSession('a', { public_because: { model: 'joey-1', place: 'Baseten', others: 3 } })
      )
    ).toBe(
      'Public: its decision model joey-1 runs on Baseten, and 3 other decision models are public too.'
    );
  });

  it('says nothing for a chat the daemon gave no reason for', () => {
    expect(chatPublicNote(makeSession('a'))).toBeNull();
    expect(chatPublicNote(makeSession('a', { public_because: null }))).toBeNull();
    expect(
      chatPublicNote(makeSession('a', { public_because: { model: ' ', place: 'Baseten' } }))
    ).toBeNull();
  });

  it('never says "Public" on a chat the list holds as private', () => {
    // A row whose tier was patched in place after its page was read.
    expect(chatPublicNote(makeSession('a', { ...LOWERED, privacy_tier: 'private' }))).toBeNull();
  });

  it('uses the sidebar copy, so the words live in one place', () => {
    expect(sidebarCopy.chats.publicBecause('m', 'p')).toBe(
      'Public: its decision model m runs on p.'
    );
  });
});

describe('the row of a chat a decision model made public', () => {
  it('shows the reason in its hover card', async () => {
    renderRows([makeSession('lowered', LOWERED)]);
    focusWithTab(screen.getByTestId('recent-chat-lowered'));
    const [summary] = await screen.findAllByTestId('recent-chat-summary-lowered');
    expect(summary).toHaveTextContent('Public: its decision model joey-1 runs on Baseten.');
    // The usual line stays first.
    expect(summary).toHaveTextContent('/workspace/lowered');
  });

  it('draws the same glyph as an ordinary public chat: no lock, nothing new', () => {
    renderRows([makeSession('lowered', LOWERED), makeSession('plain')]);
    const lowered = screen.getByTestId('recent-chat-glyph-lowered');
    const plain = screen.getByTestId('recent-chat-glyph-plain');
    expect(lowered.innerHTML).toBe(plain.innerHTML);
    expect(lowered.getAttribute('class')).toBe(plain.getAttribute('class'));
    expect(lowered.querySelector('.br-icon-lock-badge')).toBeNull();
    // Nothing on the row itself names the reason; the hover card is the peek.
    expect(screen.getByTestId('recent-chat-lowered')).not.toHaveTextContent('Public');
  });

  it('adds no note to an ordinary chat or a private one', async () => {
    renderRows([
      makeSession('plain'),
      makeSession('private', { ...LOWERED, privacy_tier: 'private' }),
    ]);
    focusWithTab(screen.getByTestId('recent-chat-plain'));
    await screen.findAllByTestId('recent-chat-summary-plain');
    expect(screen.queryByTestId('recent-chat-public-note-plain')).toBeNull();

    focusWithTab(screen.getByTestId('recent-chat-private'));
    await screen.findAllByTestId('recent-chat-summary-private');
    expect(screen.queryByTestId('recent-chat-public-note-private')).toBeNull();
  });
});

describe('withPrivacyTier', () => {
  it('drops the reason with the classification it belonged to', () => {
    const patched = withPrivacyTier(makeSession('a', LOWERED), 'private');
    expect(patched.privacy_tier).toBe('private');
    expect('public_because' in patched).toBe(false);
  });

  it('leaves a row with no reason as it was, apart from the tier', () => {
    const row = makeSession('a');
    expect(withPrivacyTier(row, 'private')).toEqual({ ...row, privacy_tier: 'private' });
  });
});
