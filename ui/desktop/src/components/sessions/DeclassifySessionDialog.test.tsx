import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DeclassifySessionDialog, declassifyToastSubject } from './DeclassifySessionDialog';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({
  declassifySession: vi.fn(),
  // Returns the id the real `toastError` returns: its dedupe key, so two
  // identical failures on one chat share one id exactly as they share one toast.
  // The toast layer itself is exercised in `DeclassifySessionDialog.toastLayer.test.tsx`.
  toastError: vi.fn(
    ({ title, msg, dedupeScope }: { title: string; msg: string; dedupeScope?: string }) =>
      `error[${dedupeScope}]:${title}:${msg}`
  ),
  toastSuccess: vi.fn(),
  dismissToast: vi.fn(),
  announceSessionRowChanged: vi.fn(),
  readSessionRowFacts: vi.fn(),
  // Row reads delivered in this window, for the listener the dialog module
  // installs; `deliverRow` below plays one.
  rowListeners: new Set<(facts: { sessionId: string; privacy_tier: string }) => void>(),
}));

vi.mock('../../api', () => ({
  declassifySession: mocks.declassifySession,
}));

vi.mock('../../toasts', () => ({
  toastError: mocks.toastError,
  toastSuccess: mocks.toastSuccess,
  toastService: { dismiss: mocks.dismissToast },
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

vi.mock('../../utils/sessionRowSync', () => ({
  announceSessionRowChanged: mocks.announceSessionRowChanged,
  readSessionRowFacts: mocks.readSessionRowFacts,
  subscribeSessionRowChanges: (
    listener: (facts: { sessionId: string; privacy_tier: string }) => void
  ) => {
    mocks.rowListeners.add(listener);
    return () => mocks.rowListeners.delete(listener);
  },
}));

function deliverRow(sessionId: string, privacy_tier: 'public' | 'private') {
  for (const listener of [...mocks.rowListeners]) listener({ sessionId, privacy_tier });
}

/** The row as `readSessionRowFacts` would hand it back after a failure. */
function rowReads(privacy_tier: 'public' | 'private' | null) {
  mocks.readSessionRowFacts.mockResolvedValue(
    privacy_tier === null
      ? null
      : {
          sessionId: s.id,
          privacy_tier,
          privacy_reason: privacy_tier === 'public' ? 'declassified_by_user' : 'turn:versa_azure',
        }
  );
}

/**
 * Answer the way the generated client (`api/client/client.gen.ts`) really does,
 * for BOTH ways of calling it — which is what lets a test here fail on the code
 * it replaced rather than on a mock that only fits the new call.
 *
 * With `throwOnError` the client throws the parsed BODY and the Response is
 * gone; a plain-text body stays a string and an empty one becomes `{}`
 * (`finalError || {}`). Without it the same body comes back as `error`, beside
 * the Response.
 */
function answerLikeTheClient(status: number, body: string) {
  return async (options: { throwOnError?: boolean }) => {
    if (status === 200) {
      const data = { sessionId: s.id, privacyTier: 'public' };
      return { data, request: {}, response: { status } };
    }
    const error = body || {};
    if (options.throwOnError) throw error;
    return { data: undefined, error, request: {}, response: { status } };
  };
}

/** The daemon's 503 body, `privacy::declassify::DECLASSIFY_STORE_BUSY`. */
const STORE_BUSY =
  'Nothing was changed and this chat was not marked public, because other writes kept the chat ' +
  'store busy. Try again in a moment.';

const s = {
  id: 'abc123def456',
  name: 'Cohort of 4,102 patients',
  working_dir: '/tmp',
  created_at: '2026-07-14T12:00:00Z',
  updated_at: '2026-07-14T12:00:00Z',
  extension_data: {},
  message_count: 12,
  privacy_tier: 'private',
  privacy_reason: 'mcp:ucsfomopagent',
} as unknown as Session;

beforeEach(() => {
  vi.clearAllMocks();
  mocks.declassifySession.mockImplementation(answerLikeTheClient(200, ''));
  // Every failure re-reads the row before it says anything. Unless a test says
  // otherwise, the write did not land.
  rowReads('private');
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe('DeclassifySessionDialog', () => {
  it('the phrase gate is real, not decorative', async () => {
    const user = userEvent.setup();
    render(<DeclassifySessionDialog session={{ ...s, id: 'abc123def456' }} onClose={vi.fn()} />);

    const confirm = screen.getByRole('button', { name: /Make public/ });
    const field = () => screen.getByLabelText(/last 6 characters/i);

    await user.type(field(), 'ef45');
    expect(confirm).toBeDisabled();

    await user.clear(field());
    // The NAME, not the id. `is_default_session_name` shows "New Session",
    // "CLI Session" and "Session <N>" are live placeholders shared by dozens of
    // rows, so a name-typed phrase would confirm nothing about WHICH chat.
    await user.type(field(), s.name);
    expect(confirm).toBeDisabled();

    await user.clear(field());
    await user.type(field(), 'def456');
    expect(confirm).toBeEnabled();
  });

  it('a turn-only session gets the single-click path, an mcp session does not', () => {
    const { rerender } = render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
      />
    );
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.getByText(/undo/i)).toBeInTheDocument();

    rerender(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'mcp:ucsfomopagent' }}
        onClose={vi.fn()}
      />
    );
    expect(screen.getByRole('textbox')).toBeInTheDocument();
  });

  it('grades anything that is not a turn onto the typed confirmation', () => {
    // §12.4 asks for the strong control on a chat that reached a private data
    // source "or inherited from an `mcp:*` ancestor". A branch of an OMOP chat
    // carries `diverged:<parent>` and none of the `mcp:` spelling, so a
    // match-on-`mcp:` rule would hand the single-click control to a copy of
    // exactly the conversation the rule exists to protect. Mirrors
    // `requires_typed_confirmation` in `privacy/declassify.rs`.
    for (const reason of ['diverged:20260101_120000', 'inherited:x', 'imported', undefined]) {
      const view = render(
        <DeclassifySessionDialog session={{ ...s, privacy_reason: reason }} onClose={vi.fn()} />
      );
      expect(screen.getByRole('textbox')).toBeInTheDocument();
      view.unmount();
    }
  });

  it('tells each provenance the reason that is actually true of it', () => {
    // The dialog shipped saying "This chat reached a private data source" for
    // every chat on the strong control. That is false for `backfill:*` and
    // `imported`, and those are not an edge case: the one-time migration marks
    // a chat by the model it was last bound to, so on day one `backfill:*` is
    // most of the private rows. Asserted PER provenance, because a single
    // assertion on the `mcp:*` case is exactly what let the wrong string ship.
    const cases: Array<[string | undefined, string]> = [
      ['mcp:ucsfomopagent', 'This chat reached a private data source.'],
      ['inherited:20260101_120000', 'This chat was created inside a private chat.'],
      ['diverged:20260101_120000', 'This chat was branched out of a private chat.'],
      [
        'backfill:versa_azure',
        'This chat was marked private by the one-time migration, from the model it was last using rather than from anything it reached.',
      ],
      ['imported', 'This chat was imported already marked private.'],
      [
        'something_new',
        'This chat does not record an observed turn on a private model as the reason it is private.',
      ],
      [
        undefined,
        'This chat does not record an observed turn on a private model as the reason it is private.',
      ],
    ];

    for (const [reason, sentence] of cases) {
      const view = render(
        <DeclassifySessionDialog session={{ ...s, privacy_reason: reason }} onClose={vi.fn()} />
      );
      const description = screen.getByRole('dialog').textContent ?? '';
      expect(description, `the copy shown for ${String(reason)}`).toContain(sentence);
      if (reason !== 'mcp:ucsfomopagent') {
        expect(
          description,
          `a ${String(reason)} chat was told it reached a data source`
        ).not.toContain('reached a private data source');
      }
      view.unmount();
    }
  });

  it('sends the typed phrase and the proof-of-user, and reports the new tier', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    render(
      <DeclassifySessionDialog session={s} onClose={vi.fn()} onDeclassified={onDeclassified} />
    );

    await user.type(screen.getByLabelText(/last 6 characters/i), 'def456');
    await user.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() =>
      expect(mocks.declassifySession).toHaveBeenCalledWith({
        path: { session_id: 'abc123def456' },
        body: { confirmation: 'def456' },
        headers: { 'X-User-Action': 'test-key' },
      })
    );
    await waitFor(() => expect(onDeclassified).toHaveBeenCalledWith('abc123def456'));
    // Item 11: the change announces itself, so a second window's lists re-read
    // the row instead of waiting for a re-sort that no longer happens.
    expect(mocks.announceSessionRowChanged).toHaveBeenCalledWith('abc123def456');
  });

  it('the single-click path holds the request open for the undo window', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={60_000}
      />
    );

    await user.click(screen.getByRole('button', { name: /Make public/ }));

    // The undo is only real if NOTHING has been sent yet: declassification is
    // one-way at the daemon (a re-raise would write a second ledger row under a
    // different provenance), so an "undo" that fires after the request would be
    // a lie in the audit trail.
    const undo = await screen.findByRole('button', { name: /Undo/ });
    expect(mocks.declassifySession).not.toHaveBeenCalled();

    await user.click(undo);
    expect(mocks.declassifySession).not.toHaveBeenCalled();
    expect(onDeclassified).not.toHaveBeenCalled();
  });

  it('the single-click path sends once the undo window closes', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={10}
      />
    );

    await user.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() =>
      expect(mocks.declassifySession).toHaveBeenCalledWith({
        path: { session_id: 'abc123def456' },
        body: { confirmation: null },
        headers: { 'X-User-Action': 'test-key' },
      })
    );
    await waitFor(() => expect(onDeclassified).toHaveBeenCalledWith('abc123def456'));
  });

  it('the undo window is a deadline, not a countdown a re-render restarts', async () => {
    // Both entry points hand this component INLINE arrows —
    // `SessionListView`: `onClose={() => setDeclassifyTarget(null)}`,
    // `SessionHistoryView`: both props — so it is given fresh callback
    // identities on every parent render, and the session list re-renders on its
    // own change subscription. A `send` that closes over them directly changes
    // identity each time; an effect keyed on it then clears the pending timeout
    // and arms a fresh FULL-LENGTH one. One re-render silently doubles the
    // advertised window, and any re-render source faster than it means the
    // request is never sent at all — the user sits in front of "becomes public
    // in 5 seconds" forever. That is precisely the "an action that reads as an
    // action that did nothing" failure this dialog exists to avoid.
    vi.useFakeTimers();
    const view = render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={() => {}}
        onDeclassified={() => {}}
        undoMs={5000}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    expect(screen.getByRole('button', { name: /Undo/ })).toBeInTheDocument();

    for (let elapsed = 0; elapsed < 5000; elapsed += 1000) {
      // A new pair of arrows each time, exactly as a parent re-render supplies.
      view.rerender(
        <DeclassifySessionDialog
          session={{ ...s, privacy_reason: 'turn:versa_azure' }}
          onClose={() => {}}
          onDeclassified={() => {}}
          undoMs={5000}
        />
      );
      await act(async () => {
        await vi.advanceTimersByTimeAsync(1000);
      });
    }

    expect(mocks.declassifySession).toHaveBeenCalledTimes(1);
  });

  it('abandoning the undo window sends nothing at all', () => {
    // Pins the direction this fails in, which is the safe one: the request has
    // not been made while the window is open, so unmounting — navigating away,
    // closing History — leaves the chat private. The alternative, firing an
    // irreversible action after the user has left the surface that announced
    // it, would be worse.
    vi.useFakeTimers();
    const { unmount } = render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        undoMs={5000}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    unmount();
    act(() => {
      vi.advanceTimersByTime(60_000);
    });
    expect(mocks.declassifySession).not.toHaveBeenCalled();
  });

  it('a refused single-click escalates to the typed control instead of looping', async () => {
    // The row this dialog is handed comes from the session list's cache. If it
    // still says `turn:*` but the daemon has since recorded `mcp:*`, the
    // single-click control is the wrong one: the request goes out with no
    // confirmation and the daemon refuses it over a phrase the user was never
    // shown. Dropping back to the SAME control re-renders it from the SAME
    // stale prop, so clicking again fails identically — an unrecoverable
    // dialog. The strong control is always an acceptable answer to a refusal,
    // and it is the only one that can recover from a stale grade.
    mocks.declassifySession.mockImplementation(
      answerLikeTheClient(
        400,
        "The confirmation did not match the last six characters of this chat's id. Nothing was changed."
      )
    );

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    expect(screen.getByLabelText(/last 6 characters/i)).toBeInTheDocument();
    // And it says why the field appeared, rather than borrowing the copy for a
    // chat that genuinely reached a private data source — which would be a
    // claim about this conversation that nothing here established.
    expect(screen.queryByText(/reached a private data source/i)).toBeNull();
  });

  it('surfaces a refusal instead of claiming the chat is now public', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    const refusal =
      'This chat does not record an observed turn on a private model as the reason it is private, ' +
      'so marking it public needs your operating system to confirm it is you. That did not happen, ' +
      'and nothing was changed.';
    mocks.declassifySession.mockImplementation(answerLikeTheClient(403, refusal));

    render(
      <DeclassifySessionDialog session={s} onClose={vi.fn()} onDeclassified={onDeclassified} />
    );
    await user.type(screen.getByLabelText(/last 6 characters/i), 'def456');
    await user.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: 'Could not mark this chat public — “Cohort of 4,102 patients”',
        msg: refusal,
        dedupeScope: 'declassify:abc123def456',
      })
    );
    expect(onDeclassified).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(mocks.announceSessionRowChanged).not.toHaveBeenCalled();
  });

  it('a thrown request (the daemon unreachable) is a failure, not a success', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    mocks.declassifySession.mockRejectedValue(new TypeError('Failed to fetch'));

    render(
      <DeclassifySessionDialog session={s} onClose={vi.fn()} onDeclassified={onDeclassified} />
    );
    await user.type(screen.getByLabelText(/last 6 characters/i), 'def456');
    await user.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    expect(onDeclassified).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  // Item 8 of the 1.90.4 hold (2026-09-13). Reproduced in the running app by
  // holding `sessions.db`'s write lock across one single-click declassification:
  // the daemon answered a bodyless 500 after its five-second busy wait, the toast
  // read "Could not mark this chat public / [object Object]", and the dialog
  // swapped the single click for the typed phrase under "That request was
  // refused. This chat's record has changed since this list was loaded" — none
  // of which was true.
  it('a busy store says so in the daemon’s words and keeps the single click', async () => {
    mocks.declassifySession.mockImplementation(answerLikeTheClient(503, STORE_BUSY));
    const onDeclassified = vi.fn();

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: 'The chat store was busy — “Cohort of 4,102 patients”',
        msg: STORE_BUSY,
        dedupeScope: 'declassify:abc123def456',
      })
    );
    // Not escalated: a busy store says nothing about this chat's grade, so the
    // control it was offered is still the right one to try again with.
    await waitFor(() => expect(screen.getByRole('button', { name: /Make public/ })).toBeEnabled());
    expect(screen.queryByRole('textbox')).toBeNull();
    expect(screen.queryByText(/record has changed/i)).toBeNull();
    // And never reported public.
    expect(onDeclassified).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
    expect(mocks.announceSessionRowChanged).not.toHaveBeenCalled();
  });

  it('a failure with no body is described, never shown as [object Object]', async () => {
    mocks.declassifySession.mockImplementation(answerLikeTheClient(500, ''));

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    const { title, msg } = mocks.toastError.mock.calls[0][0] as { title: string; msg: string };
    expect(msg).not.toContain('[object Object]');
    // Stated as what the row read back, which is all this dialog knows.
    expect(msg).toMatch(/still private/);
    expect(title).toBe('Could not mark this chat public — “Cohort of 4,102 patients”');
    expect(screen.queryByRole('textbox')).toBeNull();
  });
});

/**
 * Defect D2 of the 2026-09-13 repair round. The daemon wrote, and its answer
 * never reached the renderer: measured by failing the POST's RESPONSE in the
 * running app (CDP `Fetch.failRequest` at the response stage) with the daemon's
 * 200 already sent. The database read `public` with one ledger row; the toast
 * read "Biorouter could not be reached, so this chat was not marked public.",
 * the dialog stayed open, and both windows' rows stayed private. A missing
 * answer is not a "no", so the row is asked before a word is said.
 */
describe('an answer that never arrived', () => {
  const lostAnswer = () =>
    mocks.declassifySession.mockRejectedValue(new TypeError('Failed to fetch'));

  it('is a success when the row reads public — the write landed', async () => {
    const user = userEvent.setup();
    const onDeclassified = vi.fn();
    const onClose = vi.fn();
    lostAnswer();
    rowReads('public');

    render(
      <DeclassifySessionDialog session={s} onClose={onClose} onDeclassified={onDeclassified} />
    );
    await user.type(screen.getByLabelText(/last 6 characters/i), 'def456');
    await user.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(onDeclassified).toHaveBeenCalledWith('abc123def456'));
    expect(mocks.readSessionRowFacts).toHaveBeenCalledWith('abc123def456');
    expect(mocks.toastSuccess).toHaveBeenCalled();
    expect(mocks.toastError).not.toHaveBeenCalled();
    // Every window's rows re-read it, as after any success.
    expect(mocks.announceSessionRowChanged).toHaveBeenCalledWith('abc123def456');
    expect(onClose).toHaveBeenCalled();
  });

  it('says the chat is still private when the row says so, and not more', async () => {
    const onDeclassified = vi.fn();
    lostAnswer();
    rowReads('private');

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    const { msg } = mocks.toastError.mock.calls[0][0] as { msg: string };
    expect(msg).toMatch(/still private/);
    expect(onDeclassified).not.toHaveBeenCalled();
    expect(mocks.announceSessionRowChanged).not.toHaveBeenCalled();
    // A lost answer says nothing about the grade.
    expect(screen.queryByRole('textbox')).toBeNull();
  });

  it('claims neither outcome when the row cannot be read either', async () => {
    const onDeclassified = vi.fn();
    lostAnswer();
    rowReads(null);

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(mocks.toastError).toHaveBeenCalled());
    const { msg } = mocks.toastError.mock.calls[0][0] as { msg: string };
    expect(msg).toMatch(/could not be asked whether this chat is now public/);
    expect(msg).not.toMatch(/not marked public/);
    expect(msg).not.toMatch(/still private/);
    expect(onDeclassified).not.toHaveBeenCalled();
    expect(mocks.toastSuccess).not.toHaveBeenCalled();
  });

  it('keeps the daemon’s own sentence when it gave one and the row is unreadable', async () => {
    mocks.declassifySession.mockImplementation(answerLikeTheClient(503, STORE_BUSY));
    rowReads(null);

    render(
      <DeclassifySessionDialog
        session={{ ...s, privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() =>
      expect(mocks.toastError).toHaveBeenCalledWith({
        title: 'The chat store was busy — “Cohort of 4,102 patients”',
        msg: STORE_BUSY,
        dedupeScope: 'declassify:abc123def456',
      })
    );
  });
});

/**
 * Defect D3a of the 2026-09-13 repair round. Error toasts do not expire, so a
 * failure that a later attempt overturned stayed beside "Chat marked public":
 * measured still on screen 60 s after the success toast had closed — after two
 * busy answers, after a lost answer, and after the escalation's refusal.
 */
describe('a failure report is retracted by the next outcome', () => {
  it('a later success dismisses the failure it overturned', async () => {
    mocks.declassifySession
      .mockImplementationOnce(answerLikeTheClient(503, STORE_BUSY))
      .mockImplementationOnce(answerLikeTheClient(200, ''));
    const onDeclassified = vi.fn();

    render(
      <DeclassifySessionDialog
        // Its own id: the outstanding-failure map is module-level, and an id
        // an earlier test failed on would already hold an entry.
        session={{ ...s, id: 'overturned01', privacy_reason: 'turn:versa_azure' }}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    const failureId = mocks.toastError.mock.results[0].value;
    expect(mocks.dismissToast).not.toHaveBeenCalled();

    await waitFor(() => expect(screen.getByRole('button', { name: /Make public/ })).toBeEnabled());
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    await waitFor(() => expect(onDeclassified).toHaveBeenCalled());
    expect(mocks.dismissToast).toHaveBeenCalledWith(failureId);
    expect(mocks.toastSuccess).toHaveBeenCalled();
  });

  it('reaches a failure raised before the dialog was closed and reopened', async () => {
    mocks.declassifySession
      .mockImplementationOnce(answerLikeTheClient(503, STORE_BUSY))
      .mockImplementationOnce(answerLikeTheClient(200, ''));
    const session = { ...s, id: 'reopened0001', privacy_reason: 'turn:versa_azure' };

    const first = render(
      <DeclassifySessionDialog session={session} onClose={vi.fn()} undoMs={5} />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    const failureId = mocks.toastError.mock.results[0].value;
    // Both entry points unmount the dialog when it closes.
    first.unmount();

    const onDeclassified = vi.fn();
    render(
      <DeclassifySessionDialog
        session={session}
        onClose={vi.fn()}
        onDeclassified={onDeclassified}
        undoMs={5}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    await waitFor(() => expect(onDeclassified).toHaveBeenCalled());
    expect(mocks.dismissToast).toHaveBeenCalledWith(failureId);
  });

  it('a declassification made elsewhere retracts the failure too', async () => {
    // Fail here, close the dialog, and mark the chat public from another
    // window: this window hears it as a row read, and its "still private" toast
    // is then a report about a chat that is not.
    mocks.declassifySession.mockImplementation(answerLikeTheClient(503, STORE_BUSY));
    const session = { ...s, id: 'elsewhere001', privacy_reason: 'turn:versa_azure' };
    const view = render(<DeclassifySessionDialog session={session} onClose={vi.fn()} undoMs={5} />);
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
    await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(1));
    const failureId = mocks.toastError.mock.results[0].value;
    view.unmount();

    deliverRow('elsewhere001', 'private');
    expect(mocks.dismissToast).not.toHaveBeenCalled();

    deliverRow('elsewhere001', 'public');
    expect(mocks.dismissToast).toHaveBeenCalledWith(failureId);
  });

  it('a different failure replaces the earlier one; the same failure keeps its toast', async () => {
    const refusal =
      "The confirmation did not match the last six characters of this chat's id. Nothing was changed.";
    mocks.declassifySession
      .mockImplementationOnce(answerLikeTheClient(503, STORE_BUSY))
      .mockImplementationOnce(answerLikeTheClient(503, STORE_BUSY))
      .mockImplementationOnce(answerLikeTheClient(400, refusal));
    const session = { ...s, id: 'replaced0001', privacy_reason: 'turn:versa_azure' };

    render(<DeclassifySessionDialog session={session} onClose={vi.fn()} undoMs={5} />);
    const press = async (times: number) => {
      await waitFor(() =>
        expect(screen.getByRole('button', { name: /Make public/ })).toBeEnabled()
      );
      fireEvent.click(screen.getByRole('button', { name: /Make public/ }));
      await waitFor(() => expect(mocks.toastError).toHaveBeenCalledTimes(times));
    };

    await press(1);
    const busyId = mocks.toastError.mock.results[0].value;
    await press(2);
    // Deduplicated onto the same toast: dismissing it would take away the
    // report of the attempt just made.
    expect(mocks.dismissToast).not.toHaveBeenCalled();

    await press(3);
    expect(mocks.dismissToast).toHaveBeenCalledWith(busyId);
  });
});

describe('declassifyToastSubject', () => {
  it('names a placeholder-named chat by its id, since dozens of rows share the name', () => {
    expect(declassifyToastSubject('New Session', '20260809_21')).toBe('chat 20260809_21');
    expect(declassifyToastSubject('New chat', '20260809_21')).toBe('chat 20260809_21');
    expect(declassifyToastSubject('  ', '20260809_21')).toBe('chat 20260809_21');
    expect(declassifyToastSubject(undefined, '20260809_21')).toBe('chat 20260809_21');
  });

  it('quotes any other name, and cuts a long one short', () => {
    expect(declassifyToastSubject(' Subagent delegation request ', 'x')).toBe(
      '“Subagent delegation request”'
    );
    const long = declassifyToastSubject('a'.repeat(59) + '😀😀', 'x');
    expect(long).toBe(`“${'a'.repeat(59)}…”`);
    // Characters, not UTF-16 units: an emoji at the cut is kept whole or dropped.
    expect([...declassifyToastSubject('b'.repeat(58) + '😀😀😀', 'x')].slice(-3).join('')).toBe(
      '😀…”'
    );
  });
});
