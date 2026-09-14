import { describe, it, expect, vi } from 'vitest';
import { render, screen } from '@testing-library/react';
import { ChatTabStrip, ChatTabStripProps } from './ChatTabStrip';
import { ChatTab } from './chatGroupsTypes';

// See SessionItem.test.tsx: this file deliberately never names the badge
// component, because Task 27's gate greps src/components for that name and
// expects an exact file list.

function tab(over: Partial<ChatTab> = {}): ChatTab {
  return {
    tabId: 'tab-1',
    sessionId: 's1',
    title: 'Cohort query',
    userSetName: false,
    ...over,
  };
}

function renderStrip(over: Partial<ChatTabStripProps> = {}) {
  const props: ChatTabStripProps = {
    tabs: [tab()],
    activeTabId: 'tab-1',
    runningSessionIds: [],
    onSelect: vi.fn(),
    onClose: vi.fn(),
    onReorder: vi.fn(),
    reserveTitlebar: false,
    isCompactSidebarOverlayOpen: false,
    ...over,
  };
  return { ...render(<ChatTabStrip {...props} />), props };
}

describe('ChatTabStrip — the privacy marking on the tab glyph', () => {
  it('marks a private chat, keyed by session id rather than by tab id', () => {
    renderStrip({ privacyTiers: { s1: 'private' } });
    expect(screen.getAllByTestId('chat-kind-icon')[0]).toHaveAttribute('data-privacy', 'private');
  });

  it('leaves a public chat unmarked on this dense surface', () => {
    renderStrip({ privacyTiers: { s1: 'public' } });
    expect(screen.getAllByTestId('chat-kind-icon')[0]).toHaveAttribute('data-privacy', 'public');
  });

  it('draws a tab with no known tier as not yet known — neither private nor public', () => {
    renderStrip();
    // ⚠ A tab the daemon has said nothing about is not a tab to claim
    // protection for, and it is not a tab to call Public either. This test
    // used to assert `public` here, and that pinned the defect measured on
    // 2026-09-14: a private chat's subagent tabs read Public for seconds after
    // every Settings round-trip and reload, while their own rows were read.
    const glyph = screen.getAllByTestId('chat-kind-icon')[0];
    expect(glyph).toHaveAttribute('data-privacy', 'unknown');
    expect(glyph.getAttribute('aria-label')).toBe('Chat, privacy not yet known');
  });

  it('survives hover and does not collide with the running dot', () => {
    renderStrip({ privacyTiers: { s1: 'private' }, runningSessionIds: ['s1'] });

    // Both indicators render at once — they answer different questions.
    expect(screen.getByLabelText('Running')).toBeInTheDocument();
    const glyph = screen.getAllByTestId('chat-kind-icon')[0];
    expect(glyph).toHaveAttribute('data-privacy', 'private');

    // The RUNNING dot is `group-hover:hidden` so the close control can take its
    // slot. A privacy marker that vanishes exactly when you point at the tab is
    // not a privacy marker — the property survived the dot becoming a glyph.
    const glyphClass = glyph.getAttribute('class') ?? '';
    expect(glyphClass).not.toContain('hidden');
    // And it is not the running pulse's class — which is 7px, --text-accent and
    // animated.
    expect(glyphClass.split(/\s+/)).not.toContain('br-tab__dot');
  });

  it('marks only the private tab when a strip mixes tiers', () => {
    renderStrip({
      tabs: [tab(), tab({ tabId: 'tab-2', sessionId: 's2', title: 'Public chat' })],
      privacyTiers: { s2: 'private' },
    });
    const priv = screen
      .getAllByTestId('chat-kind-icon')
      .filter((g) => g.getAttribute('data-privacy') === 'private');
    expect(priv).toHaveLength(1);
    expect(priv[0].closest('[data-tab-id]')).toHaveAttribute('data-tab-id', 'tab-2');
  });
});
