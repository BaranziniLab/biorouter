import { afterEach, describe, expect, it } from 'vitest';
import { render, screen } from '@testing-library/react';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { SubagentComposerSlot } from './SubagentComposerSlot';
import {
  SUBAGENT_TAB_READ_ONLY_REASON,
  composerSlotMode,
  isReadOnlySubagentChat,
  subagentComposerKind,
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
      <SubagentComposerSlot kind="subagent">
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
      <SubagentComposerSlot kind="subagent">
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
      <SubagentComposerSlot kind="other">
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

describe('the third state: a browser tab that does not know yet', () => {
  /**
   * The review's finding 3, and the reason this component stopped taking a
   * boolean. SD-8 promises a control that can never work here says so BEFORE it
   * is touched; a composer mounted while the answer is still in flight breaks
   * that promise for the seconds in which the child is actually running.
   */
  it('withholds the composer rather than mounting one it may have to take back', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    const { container } = render(
      <SubagentComposerSlot kind="unknown">
        <Composer />
      </SubagentComposerSlot>
    );
    expect(screen.queryByTestId('chat-input')).toBeNull();
    expect(screen.queryByRole('button', { name: 'Send message' })).toBeNull();
    // Nothing at all, not an explanation that might turn out to be wrong.
    expect(screen.queryByTestId('subagent-read-only-note')).toBeNull();
    expect(container).toBeEmptyDOMElement();
  });

  it('leaves the desktop alone in that state, where every control works', () => {
    render(
      <SubagentComposerSlot kind="unknown">
        <Composer />
      </SubagentComposerSlot>
    );
    expect(screen.getByTestId('chat-input')).toBeInTheDocument();
  });
});

describe('subagentComposerKind', () => {
  const base = { sessionId: 'chat-1' };

  it('takes the badge at mount, which is the only source that is synchronous', () => {
    expect(subagentComposerKind({ ...base, badge: 'subagent' })).toBe('subagent');
  });

  it("is unknown while the store has not reported THIS tab's session", () => {
    // The fail-open the review found: every source below answers falsey here,
    // and a boolean made that indistinguishable from "not a subagent".
    expect(subagentComposerKind(base)).toBe('unknown');
    // A row for a DIFFERENT session is not an answer about this one: a chat is
    // keyed by tab id and the session behind a tab is rebindable.
    expect(
      subagentComposerKind({ ...base, loadedSessionId: 'chat-0', loadedSessionType: 'user' })
    ).toBe('unknown');
  });

  it('resolves negative only on a loaded row for this session', () => {
    expect(
      subagentComposerKind({ ...base, loadedSessionId: 'chat-1', loadedSessionType: 'user' })
    ).toBe('other');
    expect(
      subagentComposerKind({ ...base, loadedSessionId: 'chat-1', loadedSessionType: 'sub_agent' })
    ).toBe('subagent');
  });

  it('never withholds a composer there is no chat to withhold it from', () => {
    // The empty tab before the first message mints a session. Withholding here
    // would leave a browser unable to start a chat at all.
    expect(subagentComposerKind({ sessionId: '' })).toBe('other');
  });

  it('resolves a failed load rather than withholding forever', () => {
    // A load failure is not evidence of a subagent, and the tab is already
    // saying it could not be read. Withholding would be a lockout, not a gate.
    expect(subagentComposerKind({ ...base, loadFailed: true })).toBe('other');
  });

  it('still believes the hook, whose answer is positive-only', () => {
    expect(subagentComposerKind({ ...base, hookSaysSubagent: true })).toBe('subagent');
  });
});

describe('composerSlotMode', () => {
  it('is the composer on the desktop whatever the tab knows', () => {
    for (const kind of ['subagent', 'other', 'unknown'] as const) {
      expect(composerSlotMode(kind)).toBe('composer');
    }
  });

  it('separates the three in a browser', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    expect(composerSlotMode('subagent')).toBe('read-only');
    expect(composerSlotMode('unknown')).toBe('withheld');
    expect(composerSlotMode('other')).toBe('composer');
  });
});
