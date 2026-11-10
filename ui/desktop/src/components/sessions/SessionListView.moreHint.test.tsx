import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import { MORE_ACTIONS, moreActionsLabel } from './copy';

const mocks = vi.hoisted(() => ({ listSessions: vi.fn() }));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  divergeSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
  isPrivateCopyRefusal: () => false,
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const session = {
  id: '20261105_1',
  name: 'Cohort notes',
  working_dir: '/Users/x/project',
  created_at: '2026-11-05T12:00:00Z',
  updated_at: '2026-11-05T12:00:00Z',
  extension_data: {},
  message_count: 4,
} as Session;

/** Past the label tooltip's 500ms delay. */
const PAST_THE_DELAY_MS = 700;

const wait = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({ data: { sessions: [session] } });
});

/**
 * A History row draws its own `⋯` (its menu is the shared chat-row menu, not a `RowActions` item
 * list), so it did not get `RowActions`' fix: a plain `Tooltip` around the menu trigger, and a
 * pointer that left the open `⋯` and came back had "More actions" over its own open menu 500ms
 * later (H2; measured 522ms in the app). The pointer cases in Chromium for the shared triggers
 * are `ui/surfaceTriggerHint.browser.test.tsx`; this is the row's own.
 */
describe('History row ⋯', () => {
  async function renderRow() {
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Cohort notes');
    return screen.getByRole('button', { name: moreActionsLabel('Cohort notes') });
  }

  it('names itself on a hover, and never over its own open menu', async () => {
    const more = await renderRow();

    // A hover still shows the name: what this guards is ONLY the open menu.
    fireEvent.pointerMove(more, { pointerType: 'mouse' });
    expect(await screen.findByRole('tooltip', {}, { timeout: 2000 })).toHaveTextContent(
      MORE_ACTIONS
    );

    // Radix opens a dropdown on pointerdown; the press closes the hint.
    fireEvent.pointerDown(more, { button: 0, ctrlKey: false, pointerType: 'mouse' });
    await screen.findAllByRole('menuitem');
    await waitFor(() => expect(screen.queryByRole('tooltip')).toBeNull());
    expect(more).toHaveAttribute('aria-expanded', 'true');

    // Off the ⋯ and back onto it, resting, the menu still open.
    fireEvent.pointerUp(more, { pointerType: 'mouse' });
    fireEvent.pointerLeave(more, { pointerType: 'mouse' });
    fireEvent.pointerMove(more, { pointerType: 'mouse' });
    await wait(PAST_THE_DELAY_MS);
    expect(screen.getAllByRole('menuitem').length, 'the menu is still open').toBeGreaterThan(0);
    expect(screen.queryByRole('tooltip'), 'no hint over the open menu').toBeNull();
  });

  it('holds the kept step while its menu is open, as every trigger whose menu a click opened', async () => {
    const more = await renderRow();
    // `.br-surface-trigger[aria-expanded='true']` (main.css) paints the kept fill; the class is
    // what puts this ⋯ under that rule, and `aria-expanded` is what the rule reads.
    expect(more).toHaveClass('br-surface-trigger');
    expect(more).toHaveAttribute('aria-expanded', 'false');
    fireEvent.pointerDown(more, { button: 0, ctrlKey: false, pointerType: 'mouse' });
    await screen.findAllByRole('menuitem');
    expect(more).toHaveAttribute('aria-expanded', 'true');
  });
});
