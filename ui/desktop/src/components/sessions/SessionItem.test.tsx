import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import SessionItem from './SessionItem';
import type { Session } from '../../api';

/**
 * NOTE: this file must never name the badge component in source.
 *
 * Task 27's own gate is `grep -rl <the badge's component name> src/components`
 * with an exact expected file list — a test file that spells it joins that list
 * and turns a passing gate into a failing one. Everything here goes through the
 * rendered contract (`data-testid="privacy-badge"`, `data-privacy`) instead,
 * which is what the surface actually owes its user.
 */
function session(over: Partial<Session> = {}): Session {
  return {
    id: 'session-1',
    name: 'Cohort query',
    created_at: '2026-07-14T12:00:00Z',
    updated_at: '2026-07-14T12:00:00Z',
    working_dir: '/tmp',
    message_count: 3,
    extension_data: {},
    ...over,
  } as Session;
}

describe('SessionItem — the privacy marker', () => {
  it('marks a private session', () => {
    render(<SessionItem session={session({ privacy_tier: 'private' })} />);
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).toHaveAttribute('data-privacy', 'private');
    expect(glyph.getAttribute('aria-label')).toBe('Private chat');
  });

  it('leaves a public session unmarked on this dense row', () => {
    render(<SessionItem session={session({ privacy_tier: 'public' })} />);
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).toHaveAttribute('data-privacy', 'public');
    expect(glyph.getAttribute('aria-label')).toBe('Chat');
  });

  it('draws a session with no tier at all as not yet known rather than guessing', () => {
    render(<SessionItem session={session()} />);
    // ⚠ "No tier recorded" is neither the private glyph — a row the daemon has
    // said nothing about is not a row to claim protection for — nor Public,
    // which is the same guess in the other direction.
    const glyph = screen.getByTestId('chat-kind-icon');
    expect(glyph).toHaveAttribute('data-privacy', 'unknown');
    expect(glyph.getAttribute('aria-label')).toBe('Chat, privacy not yet known');
  });

  /**
   * The kind axis, which this row could not express at all before: every
   * session drew the same bubble, so a branch, a sub-agent and a chat were
   * indistinguishable in History.
   */
  it('distinguishes a branch, a sub-agent and a plain chat', () => {
    const { unmount } = render(<SessionItem session={session({ diverged_from: 'session-0' })} />);
    expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-chat-kind', 'branch');
    unmount();

    const second = render(<SessionItem session={session({ session_type: 'sub_agent' })} />);
    expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-chat-kind', 'subagent');
    second.unmount();

    render(<SessionItem session={session()} />);
    expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-chat-kind', 'chat');
  });
});
