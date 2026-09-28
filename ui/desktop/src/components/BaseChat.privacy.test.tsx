import { readFileSync } from 'node:fs';
import path from 'node:path';
import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { SessionNamePill } from './SessionNamePill';

// See sessions/SessionItem.test.tsx: this file deliberately never names the
// badge component, because Task 27's gate greps src/components for that name
// and expects an exact file list.

/** vitest runs with `ui/desktop` as its root — the same idiom the badge's own suite uses. */
const read = (...p: string[]) => readFileSync(path.join(process.cwd(), ...p), 'utf8');

describe('the chat header — the privacy marker', () => {
  it('names the tier in words beside the conversation title', () => {
    render(<SessionNamePill name="Cohort query" onRename={vi.fn()} privacyTier="private" />);
    const badge = screen.getByTestId('privacy-badge');
    expect(badge).toHaveAttribute('data-privacy', 'private');
    expect(badge).toHaveTextContent('Private');
  });

  it('names Public too — this is the one surface with room to say it', () => {
    render(<SessionNamePill name="Cohort query" onRename={vi.fn()} privacyTier="public" />);
    expect(screen.getByTestId('privacy-badge')).toHaveAttribute('data-privacy', 'public');
  });

  it('says nothing when the tier is unknown, rather than guessing Public', () => {
    render(<SessionNamePill name="Cohort query" onRename={vi.fn()} />);
    expect(screen.queryByTestId('privacy-badge')).toBeNull();
  });

  it('keeps the marker out of the title editor, where it would sit beside an input', () => {
    render(<SessionNamePill name="Cohort query" onRename={vi.fn()} privacyTier="private" />);
    // The pill is in its resting (non-editing) state here; the badge is part of
    // the resting row, not chrome that survives into the rename input.
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByTestId('privacy-badge')).toBeInTheDocument();
  });

  /**
   * BaseChat itself cannot be mounted in jsdom — every one of its six existing
   * suites is a `.test.ts` of extracted pure functions for exactly that reason.
   * What it owes this task is one thing and it is checkable: the header it
   * renders must hand the session's tier to the pill above. A pill that can
   * render a badge nobody feeds is the same defect the task's own gate exists
   * to catch, one level down.
   */
  it('BaseChat hands the session tier to the header pill', () => {
    const source = read('src', 'components', 'BaseChat.tsx');
    const pill = /<SessionNamePill\b[\s\S]*?\/>/.exec(source);
    expect(pill, 'BaseChat no longer renders SessionNamePill').not.toBeNull();
    expect(pill![0]).toMatch(/privacyTier=\{session\?\.privacy_tier\}/);
  });

  /**
   * Task 30A (issue #56, DR-17 requirement 3). Same constraint as above —
   * BaseChat cannot be mounted in jsdom — and the same checkable obligation: the
   * chat surface must mount the disclosure gate, and must hand it the chat's
   * BOUND PROVIDER rather than the chat's classification.
   *
   * ⚠ `session?.provider_name`, never `session?.privacy_tier`. The tier is the
   * chat's ratcheted classification; a fresh chat on a private model is
   * classified `public`, and a gate keyed on that would show a "this model is
   * not hosted by your institution" dialog over Versa. The gate's own suite
   * proves the predicate; this proves the argument.
   */
  it('BaseChat mounts the non-private-model disclosure gate on the bound provider', () => {
    const source = read('src', 'components', 'BaseChat.tsx');
    const gate = /<NonPrivateModelDisclosureGate\b[\s\S]*?\/>/.exec(source);
    expect(gate, 'BaseChat does not mount the disclosure gate').not.toBeNull();
    // The bound provider first; before the chat has a session, the model held
    // for it (T3-SH-1), which is bound in the same click that sends.
    expect(gate![0]).toMatch(
      /providerName=\{session\?\.provider_name \?\? \(sessionId \? null : pendingChatModel\?\.provider\)\}/
    );
    expect(gate![0]).not.toMatch(/privacy_tier/);
  });
});
