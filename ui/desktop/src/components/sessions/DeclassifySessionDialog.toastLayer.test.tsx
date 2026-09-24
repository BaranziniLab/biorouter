import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { useEffect } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { ToastContainer, toast, type ToastTransitionProps } from 'react-toastify';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { DeclassifySessionDialog } from './DeclassifySessionDialog';
import type { Session } from '../../api';

/**
 * Defect D3a, second round (2026-09-13), measured by an independent tester on
 * the desktop and reproduced in the dev app before this file existed:
 *
 *  1. chat Y fails with the store write-locked → 503, "The chat store was busy";
 *  2. chat X fails the same way → still ONE toast on screen;
 *  3. X is retried and succeeds → "Chat marked public", and eight seconds later
 *     nothing at all. The database read Y private and X public: Y's failure
 *     report was gone although nothing about Y had been decided.
 *
 * The dialog kept its outstanding reports by CHAT, and `toastError` deduplicated
 * them by CONTENT. The busy sentence is identical for every chat, so both
 * reports were one toast, and X's success dismissed it.
 *
 * ⚠ Why these run against the REAL toast layer — `toasts.tsx` and
 * react-toastify's container — when `DeclassifySessionDialog.test.tsx` mocks
 * `toastError`: the defect lived in the seam between the two keys, and a mock of
 * either half encodes an assumption about the other. What is asserted here is
 * what the person sees: which reports are on screen.
 */

const mocks = vi.hoisted(() => ({
  declassifySession: vi.fn(),
  announceSessionRowChanged: vi.fn(),
  // What `readSessionRowFacts` reads back after a failure, per chat.
  rowTiers: new Map<string, 'public' | 'private'>(),
  rowListeners: new Set<(facts: { sessionId: string; privacy_tier: string }) => void>(),
}));

vi.mock('../../api', () => ({
  declassifySession: mocks.declassifySession,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

vi.mock('../../utils/sessionRowSync', () => ({
  announceSessionRowChanged: mocks.announceSessionRowChanged,
  readSessionRowFacts: async (sessionId: string) => ({
    sessionId,
    privacy_tier: mocks.rowTiers.get(sessionId) ?? 'private',
    privacy_reason: 'turn:versa_azure',
  }),
  subscribeSessionRowChanges: (
    listener: (facts: { sessionId: string; privacy_tier: string }) => void
  ) => {
    mocks.rowListeners.add(listener);
    return () => mocks.rowListeners.delete(listener);
  },
}));

/** The daemon's 503 body, `privacy::declassify::DECLASSIFY_STORE_BUSY`. */
const STORE_BUSY =
  'Nothing was changed and this chat was not marked public, because other writes kept the chat ' +
  'store busy. Try again in a moment.';

/**
 * The daemon's 500 body, `routes::session::DECLASSIFY_FAILED`. Not a 400: that
 * one escalates the dialog to the typed phrase, which is not what these test.
 */
const FAILED =
  'Nothing was changed and this chat was not marked public, because Biorouter hit an error. The ' +
  'daemon log records it.';

type Answer = { status: number; body?: string };

/** Answers each chat's requests in order, the way the generated client returns them. */
const answers = new Map<string, Answer[]>();
function answer(sessionId: string, ...queue: Answer[]) {
  answers.set(sessionId, [...(answers.get(sessionId) ?? []), ...queue]);
}

/**
 * react-toastify removes a dismissed toast when its exit ANIMATION ends, and
 * jsdom runs no animations — so without this every dismissed toast would stay in
 * the document and "is it on screen" could never be answered. Only the animation
 * is replaced; dismissal, dedup and the container's store are the real ones.
 */
function NoAnimation({ children, isIn, done }: ToastTransitionProps) {
  useEffect(() => {
    if (!isIn) done();
  }, [isIn, done]);
  return <>{children}</>;
}

/** Chat Y: a placeholder name, as `20260809_21` has ("New Session"). */
function chatY(suffix: string): Session {
  return {
    id: `20260809_21${suffix}`,
    name: 'New Session',
    working_dir: '/tmp',
    created_at: '2026-08-09T12:00:00Z',
    updated_at: '2026-08-09T12:00:00Z',
    extension_data: {},
    message_count: 4,
    privacy_tier: 'private',
    privacy_reason: 'turn:versa_azure',
  } as unknown as Session;
}

/** Chat X: a named chat, as `20260809_23` is. */
function chatX(suffix: string): Session {
  return { ...chatY(suffix), id: `20260809_23${suffix}`, name: 'Subagent delegation request' };
}

/**
 * Open the dialog on `session`, the way both entry points mount it. `press`
 * presses Make public in THIS dialog (the single click, so after the undo
 * window) and waits for the answer: a failure hands the button back, a success
 * calls `onClose`. `close` unmounts it, which is what both entry points do on
 * close. One dialog is open at a time, as in the app.
 */
function openDialog(session: Session) {
  const onClose = vi.fn();
  const view = render(
    <MemoryRouter>
      <DeclassifySessionDialog session={session} onClose={onClose} undoMs={5} />
    </MemoryRouter>
  );
  const makePublic = () => screen.getByRole('button', { name: /Make public/ });
  return {
    press: async () => {
      const closedBefore = onClose.mock.calls.length;
      await waitFor(() => expect(makePublic()).toBeEnabled());
      fireEvent.click(makePublic());
      await waitFor(() => expect(mocks.declassifySession).toHaveBeenCalledTimes(1));
      await waitFor(() => {
        if (onClose.mock.calls.length === closedBefore) expect(makePublic()).toBeEnabled();
      });
      mocks.declassifySession.mockClear();
    },
    close: view.unmount,
  };
}

/** Fail (or succeed) once on `session` and close the dialog. */
async function attemptOnce(session: Session) {
  const dialog = openDialog(session);
  await dialog.press();
  dialog.close();
}

const readable = (toast: HTMLElement) => (toast.textContent ?? '').replace(/\s+/g, ' ').trim();

/**
 * Every report on screen, as the text a person reads. A failure is an `alert`
 * and a confirmation a polite `status` (`toasts.tsx`, T-57), and a person sees
 * both, so both count.
 */
function reports(): string[] {
  return [...screen.queryAllByRole('alert'), ...screen.queryAllByRole('status')].map(readable);
}
// Read from the ALERTS only: a failure that stopped interrupting would vanish
// from this list and fail the counts below.
const busyReports = () =>
  screen
    .queryAllByRole('alert')
    .map(readable)
    .filter((text) => text.includes('Try again in a moment'));

beforeEach(() => {
  vi.clearAllMocks();
  answers.clear();
  mocks.rowTiers.clear();
  mocks.declassifySession.mockImplementation(async ({ path }: { path: { session_id: string } }) => {
    const next = answers.get(path.session_id)?.shift() ?? { status: 200 };
    if (next.status === 200) {
      mocks.rowTiers.set(path.session_id, 'public');
      return {
        data: { sessionId: path.session_id, privacyTier: 'public' },
        request: {},
        response: { status: 200 },
      };
    }
    return { data: undefined, error: next.body, request: {}, response: { status: next.status } };
  });
  render(
    <MemoryRouter>
      <ToastContainer transition={NoAnimation} />
    </MemoryRouter>
  );
});

afterEach(async () => {
  await act(async () => {
    toast.dismiss();
  });
  cleanup();
});

describe('a failure report belongs to its chat', () => {
  it('a success on one chat leaves another chat’s failure on screen', async () => {
    const y = chatY('a');
    const x = chatX('a');
    answer(y.id, { status: 503, body: STORE_BUSY });
    answer(x.id, { status: 503, body: STORE_BUSY }, { status: 200 });

    await attemptOnce(y);
    const onX = openDialog(x);
    await onX.press();
    await onX.press();
    onX.close();

    await waitFor(() =>
      expect(reports().some((t) => t.startsWith('Chat marked public'))).toBe(true)
    );
    // Nothing about Y was decided, so Y's report is still there — and it is the
    // only failure left, because X's own success retracted X's.
    expect(busyReports()).toHaveLength(1);
    expect(busyReports()[0]).toContain(`chat ${y.id}`);
    expect(reports().some((t) => t.includes('Subagent delegation request'))).toBe(false);
  });

  it('two chats failing at once are two reports, each naming its chat', async () => {
    const y = chatY('b');
    const x = chatX('b');
    answer(y.id, { status: 503, body: STORE_BUSY });
    answer(x.id, { status: 503, body: STORE_BUSY });

    await attemptOnce(y);
    await attemptOnce(x);

    // The same sentence twice is only useful if each says which chat it is
    // about. A placeholder name is shared by dozens of rows, so that chat is
    // named by its id.
    const busy = busyReports();
    expect(busy).toHaveLength(2);
    expect(busy.filter((t) => t.includes(`chat ${y.id}`))).toHaveLength(1);
    expect(busy.filter((t) => t.includes('“Subagent delegation request”'))).toHaveLength(1);
  });

  it('two chats with the same name still keep their own reports', async () => {
    // Naming the chat in the title is for the person; it is not what keeps the
    // reports apart. Two chats can share a name, and then only the chat's id in
    // the dedup scope stops one's success taking down the other's report.
    const first = { ...chatX('f'), id: '20260809_31f', name: 'Weekly cohort' };
    const second = { ...chatX('f'), id: '20260809_32f', name: 'Weekly cohort' };
    answer(first.id, { status: 503, body: STORE_BUSY });
    answer(second.id, { status: 503, body: STORE_BUSY }, { status: 200 });

    await attemptOnce(first);
    const onSecond = openDialog(second);
    await onSecond.press();
    expect(busyReports()).toHaveLength(2);
    await onSecond.press();
    onSecond.close();

    await waitFor(() =>
      expect(reports().some((t) => t.startsWith('Chat marked public'))).toBe(true)
    );
    expect(busyReports()).toHaveLength(1);
  });

  it('a same-chat retry replaces its own report rather than stacking', async () => {
    const x = chatX('c');
    answer(x.id, { status: 503, body: STORE_BUSY }, { status: 503, body: STORE_BUSY });

    const onX = openDialog(x);
    await onX.press();
    await onX.press();
    onX.close();

    expect(busyReports()).toHaveLength(1);
  });

  it('a different failure on one chat does not take down another chat’s report', async () => {
    const y = chatY('d');
    const x = chatX('d');
    answer(y.id, { status: 503, body: STORE_BUSY });
    answer(x.id, { status: 503, body: STORE_BUSY }, { status: 500, body: FAILED });

    await attemptOnce(y);
    const onX = openDialog(x);
    await onX.press();
    await onX.press();
    onX.close();

    const busy = busyReports();
    expect(busy).toHaveLength(1);
    expect(busy[0]).toContain(`chat ${y.id}`);
    // X's busy report was replaced by its new failure, not left beside it.
    const faults = reports().filter((t) => t.includes('Biorouter hit an error'));
    expect(faults).toHaveLength(1);
    expect(faults[0]).toContain('“Subagent delegation request”');
  });

  it('a public row read about one chat retracts only that chat’s report', async () => {
    const y = chatY('e');
    const x = chatX('e');
    answer(y.id, { status: 503, body: STORE_BUSY });
    answer(x.id, { status: 503, body: STORE_BUSY });

    await attemptOnce(y);
    await attemptOnce(x);
    expect(busyReports()).toHaveLength(2);

    // X declassified from another window: this window hears it as a row read.
    await act(async () => {
      for (const listener of [...mocks.rowListeners]) {
        listener({ sessionId: x.id, privacy_tier: 'public' });
      }
    });

    await waitFor(() => expect(busyReports()).toHaveLength(1));
    expect(busyReports()[0]).toContain(`chat ${y.id}`);
  });
});
