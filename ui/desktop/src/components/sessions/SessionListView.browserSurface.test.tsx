import { fireEvent, render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import type { Session } from '../../api';

/**
 * SD-8 at History's row menu — the surface where a browser user meets
 * declassification, and the one measured failing on a real `biorouter serve` on
 * 2026-09-12: the item was offered with no `aria-disabled` and no note, the
 * destructive-confirm dialog asked for the last six characters of the chat id,
 * and `POST /sessions/{id}/declassify` then answered 403 with a sentence
 * written for an AI agent telling the reader to go and mark it public from the
 * chat history — which is where they were standing.
 *
 * Its own file, beside `SessionListView.declassify.test.tsx` rather than inside
 * it, because that file mocks `userActionHeaders` to hand back a key. A suite
 * that pretends the surface has a proof cannot also be the suite that asserts
 * what happens where there is none.
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

// ⚠ Deliberately NOT mocked to return a key. On this surface `userActionHeaders`
// answers with the caller-provider header and no proof, which is the whole
// reason the request cannot succeed.
vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-Caller-Provider': 'versa_azure' }),
}));

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

/** Radix opens its menu on pointerdown, so `fireEvent` and not `userEvent`. */
function openRowMenu() {
  fireEvent.pointerDown(screen.getByLabelText(/More actions for/), { button: 0, ctrlKey: false });
}

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  // The shape the generated client resolves a 200 with. The dialog reads the
  // status off the Response, so a bare `{}` — no Response at all — is a failed
  // request, exactly as it is at runtime.
  mocks.declassifySession.mockResolvedValue({
    data: { sessionId: 'x', privacyTier: 'public' },
    response: { status: 200 },
  });
  mocks.listSessions.mockResolvedValue({
    data: {
      sessions: [
        row({
          id: '20260905_8',
          name: 'Biorouter workflow listing',
          privacy_tier: 'private',
          // `turn:*` — the single-click grade, which is the one the QA run hit
          // and the one with no typed phrase standing between the click and the
          // request.
          privacy_reason: 'turn:versa_azure',
        }),
      ],
    },
  });
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('SessionListView declassification on a browser-served surface', () => {
  /**
   * ⚠ **Fails against `origin/main`**, where the item carries no `disabled` and
   * so renders no `aria-disabled`, and there is no note for `findByTestId` to
   * resolve. Both halves were measured absent in the browser on 2026-09-12
   * (`aria-disabled=null`, `title=null`, `data-disabled=null`).
   */
  it('greys the item out and says where the chat can be marked public', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderList();

    await screen.findByText('Biorouter workflow listing');
    openRowMenu();

    const note = await screen.findByTestId('declassify-browser-note');
    expect(note.textContent).toMatch(/biorouter serve/);
    expect(note.textContent).toMatch(/biorouter session declassify/);

    expect(screen.getByRole('menuitem', { name: /Make this chat public/ })).toHaveAttribute(
      'aria-disabled',
      'true'
    );
  });

  /**
   * `aria-disabled` is a claim about the markup; this is the claim that matters.
   * The defect was not a live-looking item — it was the request it produced and
   * the refusal that came back, so the assertion is that NOTHING is sent.
   *
   * ⚠ Fails against `origin/main`, where the click opens the confirm dialog.
   */
  it('opens no confirmation dialog and issues no request when the item is clicked', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderList();

    await screen.findByText('Biorouter workflow listing');
    openRowMenu();
    await screen.findByTestId('declassify-browser-note');

    fireEvent.click(screen.getByRole('menuitem', { name: /Make this chat public/ }));

    expect(screen.queryByRole('dialog')).toBeNull();
    expect(mocks.declassifySession).not.toHaveBeenCalled();
  });

  /**
   * ⚠ **The control.** Passes before and after. Taking declassification away
   * from the desktop — where the daemon holds the key and the whole flow works —
   * would be a far worse regression than the 403 this change replaces, so the
   * desktop path is asserted positively rather than left to the absence of a
   * failure. The typed-confirmation friction it guards is deliberate (§12.4) and
   * is untouched here; `DeclassifySessionDialog.test.tsx` pins that.
   */
  it('leaves the desktop row menu fully usable, dialog and all', async () => {
    renderList();

    await screen.findByText('Biorouter workflow listing');
    openRowMenu();

    const item = await screen.findByRole('menuitem', { name: /Make this chat public/ });
    expect(item).not.toHaveAttribute('aria-disabled', 'true');
    expect(screen.queryByTestId('declassify-browser-note')).toBeNull();

    fireEvent.click(item);
    expect(await screen.findByRole('dialog')).toHaveTextContent('Make this chat public?');
  });
});
