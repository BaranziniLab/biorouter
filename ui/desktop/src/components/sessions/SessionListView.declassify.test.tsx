import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';

/**
 * Issue #56 §12.1 — declassification is reached from the History ROW.
 *
 * Its own file for topic, not for isolation. It used to be for isolation: while
 * `SessionItem` was declared inside `SessionListView`'s body it was a fresh
 * component TYPE on every parent render, so React discarded and rebuilt every
 * row's DOM on each one — closing any open Radix menu and detaching any node a
 * test had already queried. These cases were split out to dodge that, and the
 * helpers below retried around it. `SessionItem` is at module scope now, so a
 * parent render reconciles the rows instead of replacing them, and neither the
 * split nor a retry is load-bearing any more.
 *
 * The paired absence assertion — that this action is NOT in the chat title menu,
 * which is the obvious slot and the one §12.1 forbids — lives in
 * `SessionNamePill.test.tsx`.
 */

const mocks = vi.hoisted(() => ({
  listSessions: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: mocks.declassifySession,
}));

vi.mock('../../toasts', () => ({
  toastSuccess: vi.fn(),
  toastError: vi.fn(),
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [] } });
  // The shape the generated client resolves a 200 with. The dialog reads the
  // status off the Response, so a bare `{}` — no Response at all — is a failed
  // request, exactly as it is at runtime.
  mocks.declassifySession.mockResolvedValue({
    data: { sessionId: 'x', privacyTier: 'public' },
    response: { status: 200 },
  });
});

function row(overrides: Partial<Session> & { id: string; name: string }): Session {
  return {
    working_dir: '/tmp',
    created_at: '2026-07-14T12:00:00Z',
    updated_at: '2026-07-14T12:00:00Z',
    extension_data: {},
    message_count: 2,
    ...overrides,
  } as Session;
}

function renderList() {
  return render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
}

/**
 * Open a row's overflow menu, and return once its one item is on screen.
 *
 * `fireEvent.pointerDown`, not `userEvent.click`: Radix's menu opens on
 * pointerdown, and `userEvent` yields to the event loop between pointerdown and
 * pointerup. Every passing interaction in the sibling file uses `fireEvent` for
 * the same reason.
 *
 * One pointerdown, deliberately. This used to retry inside a `waitFor` because
 * a parent render could shut the menu the instant it opened; that is fixed at
 * the source (see the file header), and a retry here would now hide a real
 * regression — a menu that needs two clicks to stay open is a bug, and this is
 * the test that should say so.
 */
async function openRowMenu(trigger: RegExp | string) {
  fireEvent.pointerDown(screen.getByLabelText(trigger), { button: 0, ctrlKey: false });
  await screen.findByText(/Make this chat public/);
}

/**
 * A public row must not be OFFERED declassification. It asserts the absence of
 * the item, not of the `⋯` trigger.
 *
 * Those were the same assertion once, and are not any more. When this file was
 * written the tier gated the whole button, because a "More actions" trigger
 * opening a one-item menu — empty on every public row — was worse than no
 * trigger. §3.10 then folded Open-in-new-tab, Open-in-new-window and Delete
 * behind that same `⋯`, so the trigger is now on every row and only the ITEM is
 * gated. Asserting `queryByLabelText(/More actions for/)` is null would now fail
 * for a reason that has nothing to do with privacy, and — worse — would have
 * gone on passing if the item had leaked onto public rows while the trigger
 * happened to be hidden. The item is the invariant; the trigger never was.
 */
async function expectNoDeclassifyItem() {
  // Open it for real: an item cannot be absent from a menu that never rendered,
  // so a check that skips this passes for the wrong reason.
  fireEvent.pointerDown(screen.getByLabelText(/More actions for/), {
    button: 0,
    ctrlKey: false,
  });
  await screen.findByText(/Open in new tab/);
  expect(screen.queryByText(/Make this chat public/)).toBeNull();
}

describe('SessionListView declassification entry point', () => {
  it('offers "Make this chat public" from the row overflow menu of a private chat', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: 'session-1',
            name: 'Patient cohort',
            privacy_tier: 'private',
            privacy_reason: 'mcp:ucsfomopagent',
          }),
        ],
      },
    });

    renderList();

    await screen.findByText('Patient cohort');
    await openRowMenu(/More actions for/);
    expect(screen.getByText(/Make this chat public/)).toBeInTheDocument();
  });

  it('opens the shared dialog on the right row, showing that row and no other', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: '20260714_120000',
            name: 'Patient cohort',
            privacy_tier: 'private',
            privacy_reason: 'mcp:ucsfomopagent',
          }),
          row({
            id: '20260714_130000',
            name: 'Another private chat',
            privacy_tier: 'private',
            privacy_reason: 'mcp:cdwagent',
          }),
        ],
      },
    });

    renderList();

    await screen.findByText('Patient cohort');
    await openRowMenu('More actions for Another private chat');
    fireEvent.click(screen.getByText(/Make this chat public/));

    // The dialog previews the row it will act on — the ResetPanel precedent —
    // and the phrase is that row's, not the first row's. Opening the menu on the
    // wrong row is the mistake this preview exists to catch, so a dialog that
    // showed the first row would defeat the whole confirmation.
    const dialog = await screen.findByRole('dialog');
    expect(dialog).toHaveTextContent('Another private chat');
    expect(dialog).toHaveTextContent('130000');
    expect(dialog).not.toHaveTextContent('Patient cohort');
  });

  it('drops the private marker from the row once the chat is public', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          row({
            id: '20260714_120000',
            name: 'Patient cohort',
            privacy_tier: 'private',
            // `mcp:*`, so §12.4 grades this onto the typed confirmation and the
            // request goes out on confirm. The `turn:*` path is the same wiring
            // behind a five-second hold, and its window is exercised where it can
            // be shortened (`DeclassifySessionDialog.test.tsx`) rather than by
            // making this suite wait five real seconds.
            privacy_reason: 'mcp:ucsfomopagent',
          }),
        ],
      },
    });

    renderList();
    await screen.findByText('Patient cohort');
    expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-privacy', 'private');

    await openRowMenu(/More actions for/);
    fireEvent.click(screen.getByText(/Make this chat public/));
    fireEvent.change(await screen.findByLabelText(/last 6 characters/i), {
      target: { value: '120000' },
    });
    fireEvent.click(screen.getByRole('button', { name: /Make public/ }));

    // The row stops claiming to be private without waiting for a refetch — an
    // action whose only visible effect arrives on the next page load reads as one
    // that did nothing. The control goes with it: a public row has nothing left
    // to declassify.
    await waitFor(() => expect(mocks.declassifySession).toHaveBeenCalled());
    // The glyph stays — every row has one — but it stops claiming the private
    // tier. Asserting its ABSENCE would now pass for a row that vanished.
    await waitFor(() =>
      expect(screen.getByTestId('chat-kind-icon')).toHaveAttribute('data-privacy', 'public')
    );
    await expectNoDeclassifyItem();
  });

  it('leaves a public chat without the control at all', async () => {
    mocks.listSessions.mockResolvedValue({
      data: { sessions: [row({ id: 'session-2', name: 'Public notes' })] },
    });

    renderList();

    await screen.findByText('Public notes');
    await expectNoDeclassifyItem();
  });
});
