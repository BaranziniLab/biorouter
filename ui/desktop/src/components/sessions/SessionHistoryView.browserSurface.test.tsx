import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import SessionHistoryView from './SessionHistoryView';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import type { Session } from '../../api';

/**
 * SD-8 at issue #56 §12.1's SECOND entry point — the saved-chat page's action
 * bar. History's row menu is the one QA measured, but the two mount the same
 * dialog, and a fix that closed only the menu would leave the identical 403 one
 * click away on the page the menu opens.
 */

const mocks = vi.hoisted(() => ({ declassifySession: vi.fn() }));

vi.mock('../../api', () => ({ declassifySession: mocks.declassifySession }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-Caller-Provider': 'versa_azure' }),
}));

vi.mock('../ProgressiveMessageList', () => ({
  default: () => <div data-testid="messages" />,
}));

vi.mock('../conversation/SearchView', () => ({
  SearchView: ({ children }: { children: ReactNode }) => <>{children}</>,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

function renderPrivatePage() {
  return render(
    <MemoryRouter>
      <SessionHistoryView
        session={
          {
            id: '20260905_8',
            name: 'Biorouter workflow listing',
            created_at: '2026-07-14T12:00:00Z',
            updated_at: '2026-07-14T12:00:00Z',
            working_dir: '/tmp',
            message_count: 0,
            extension_data: {},
            conversation: [],
            privacy_tier: 'private',
            privacy_reason: 'turn:versa_azure',
          } as Session
        }
        isLoading={false}
        error={null}
        onBack={vi.fn()}
        onRetry={vi.fn()}
        showActionButtons
      />
    </MemoryRouter>
  );
}

beforeEach(() => {
  vi.clearAllMocks();
  mocks.declassifySession.mockResolvedValue({});
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
});

describe('SessionHistoryView declassification on a browser-served surface', () => {
  /**
   * ⚠ **Fails against `origin/main`**, which renders a live "Make public"
   * button here and nothing that explains anything.
   *
   * The note carries the short line visibly and the full reason on `title` — a
   * tooltip would be unreachable, because `buttonVariants` sets
   * `disabled:pointer-events-none` and the Share button two elements away has
   * had a tooltip nobody can trigger for exactly that reason.
   */
  it('replaces the button with a line saying where the chat can be marked public', () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    renderPrivatePage();

    expect(screen.queryByRole('button', { name: 'Make public' })).toBeNull();
    const note = screen.getByTestId('declassify-browser-note');
    expect(note).toHaveTextContent(/needs the host/i);
    expect(note.getAttribute('title')).toMatch(/biorouter serve/);
    expect(mocks.declassifySession).not.toHaveBeenCalled();
  });

  /**
   * ⚠ **The control.** Passes before and after — the desktop keeps the button
   * and, behind it, §12.4's typed-confirmation friction.
   */
  it('leaves the desktop action bar fully usable', () => {
    renderPrivatePage();

    expect(screen.getByRole('button', { name: 'Make public' })).toBeInTheDocument();
    expect(screen.queryByTestId('declassify-browser-note')).toBeNull();
  });
});
