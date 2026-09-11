import { afterEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { SubagentComposerSlot } from './SubagentComposerSlot';
import {
  SUBAGENT_TAB_READ_ONLY_REASON,
  isReadOnlySubagentChat,
  subagentTabReadOnlyReason,
} from './subagentReadOnly';

/**
 * SD-8 for a delegated subagent's tab: on a `biorouter serve` page the daemon
 * refuses every write to a subagent's chat (SD-7, SD-11), so the composer's
 * place says why instead of offering a composer that fails on click.
 *
 * A stand-in composer rather than `ChatInput`: what is asserted is the slot's
 * one decision — mount the children, or mount the reason in their place — and
 * `ChatInput` needs a dozen providers to render at all. That BaseChat puts the
 * real composer inside this slot is pinned against its source in
 * `BaseChat.subagentReadOnly.test.ts`.
 */
function Composer() {
  return (
    <form aria-label="composer">
      <textarea data-testid="chat-input" />
      <button type="submit" aria-label="Send message" />
    </form>
  );
}

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('SubagentComposerSlot', () => {
  it('leaves the desktop composer alone on a subagent tab', () => {
    // The desktop holds the user-action key, so every one of these controls
    // works there and nothing may be taken away.
    render(
      <SubagentComposerSlot isSubagentChat>
        <Composer />
      </SubagentComposerSlot>
    );
    expect(screen.getByTestId('chat-input')).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Send message' })).toBeInTheDocument();
    expect(screen.queryByTestId('subagent-read-only-note')).toBeNull();
  });

  it('explains, in a browser, instead of mounting the composer at all', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(
      <SubagentComposerSlot isSubagentChat>
        <Composer />
      </SubagentComposerSlot>
    );

    const note = screen.getByTestId('subagent-read-only-note');
    expect(note).toHaveAttribute('role', 'status');
    expect(note).toHaveTextContent(SUBAGENT_TAB_READ_ONLY_REASON);
    // Not disabled — ABSENT. A greyed-out Send beside a live Stop, extension
    // picker and continuation banner would still be a row of refusals.
    expect(screen.queryByTestId('chat-input')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Send message' })).toBeNull();
    expect(screen.queryByRole('form', { name: 'composer' })).toBeNull();
  });

  it('keeps the composer for every other chat in a browser', () => {
    // An ordinary chat on a serve daemon can send, steer and (since SD-11)
    // stop. Taking its composer away would be a regression, not caution.
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    render(
      <SubagentComposerSlot isSubagentChat={false}>
        <Composer />
      </SubagentComposerSlot>
    );
    expect(screen.getByTestId('chat-input')).toBeInTheDocument();
    expect(screen.queryByTestId('subagent-read-only-note')).toBeNull();
  });
});

describe('the read-only predicate the store and the tab share', () => {
  it('answers null and false on the desktop', () => {
    expect(subagentTabReadOnlyReason()).toBeNull();
    expect(isReadOnlySubagentChat('sub_agent')).toBe(false);
  });

  it('holds in a browser for a subagent chat, and for no other kind', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    expect(subagentTabReadOnlyReason()).toBe(SUBAGENT_TAB_READ_ONLY_REASON);
    expect(isReadOnlySubagentChat('sub_agent')).toBe(true);
    for (const other of ['user', 'scheduled', 'hidden', 'terminal', undefined, null] as const) {
      expect(isReadOnlySubagentChat(other)).toBe(false);
    }
  });

  it('asks the page each time rather than remembering an early answer', () => {
    // `utils/surface.ts`'s own warning: the marker is stamped after modules
    // load, so a value captured at import time would say "desktop" in a
    // browser. Flipping the marker between calls is the check.
    expect(isReadOnlySubagentChat('sub_agent')).toBe(false);
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    expect(isReadOnlySubagentChat('sub_agent')).toBe(true);
  });

  it('says what cannot be done, why, and where it can — for a person', () => {
    // Not the daemon's sentence, which is written for whatever reads an error.
    expect(SUBAGENT_TAB_READ_ONLY_REASON).toMatch(/sending to, steering or stopping/i);
    expect(SUBAGENT_TAB_READ_ONLY_REASON).toMatch(/prove a request came from you/i);
    expect(SUBAGENT_TAB_READ_ONLY_REASON).toMatch(/desktop app/i);
    expect(SUBAGENT_TAB_READ_ONLY_REASON).not.toMatch(/daemon|user-action key/i);
  });
});
