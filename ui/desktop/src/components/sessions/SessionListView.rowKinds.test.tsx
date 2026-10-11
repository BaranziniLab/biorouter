/**
 * Vision round 2, History's rows (V1-R2-6, V1-R2-8, V1-R2-10).
 *
 * - V1-R2-8: one list order for the same data. The daemon's `GET /sessions` orders by
 *   `updated_at` alone, so chats sharing a second came back in scan order and History drew them
 *   differently from the sidebar. History now sorts by last activity, `id` ascending among ties.
 * - V1-R2-10: a row says its kind once, in the glyph. A sub-agent run has no badge and no
 *   "Subagent:" in its title, and an app's chat shows the app's name rather than `app:<id>`.
 * - V1-R2-6: a row whose `⋯` menu is open says so on the row (`data-menu-open`), which
 *   `history.css` turns into the hover ground. The right-click menu's own `data-state` is
 *   Radix's, so only the `⋯` half is the component's to set.
 */
import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';

const mocks = vi.hoisted(() => ({ listSessions: vi.fn() }));

vi.mock('../../api', () => ({
  listSessions: mocks.listSessions,
  deleteSession: vi.fn(),
  exportSession: vi.fn(),
  importSession: vi.fn(),
  updateSessionName: vi.fn(),
  declassifySession: vi.fn(),
}));

vi.mock('../../toasts', () => ({ toastSuccess: vi.fn(), toastError: vi.fn() }));

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'test-proof' }),
}));

vi.mock('../ui/scroll-area', () => ({
  ScrollArea: ({ children }: { children: ReactNode }) => <div>{children}</div>,
}));

const SECOND = '2026-09-05T17:49:17Z';

const session = (id: string, name: string, over: Partial<Session> = {}) =>
  ({
    id,
    name,
    working_dir: '/Users/someone/Desktop',
    created_at: SECOND,
    updated_at: SECOND,
    extension_data: {},
    message_count: 2,
    ...over,
  }) as Session;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

async function renderHistory(sessions: Session[]) {
  mocks.listSessions.mockResolvedValue({ data: { sessions } });
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
}

const rowOrder = () =>
  [...document.querySelectorAll('[data-session-id]')].map((el) =>
    el.getAttribute('data-session-id')
  );

describe('History orders chats that share a second the way the sidebar does (V1-R2-8)', () => {
  it('breaks a tie by id ascending, whatever order the daemon returned', async () => {
    await renderHistory([
      session('s-c', 'Third'),
      session('s-a', 'First'),
      session('s-b', 'Second'),
    ]);
    await screen.findByText('First');
    expect(rowOrder()).toEqual(['s-a', 's-b', 's-c']);
  });

  it('still puts newer activity first, whatever the ids say', async () => {
    await renderHistory([
      session('s-a', 'Older', { updated_at: '2026-09-05T10:00:00Z' }),
      session('s-z', 'Newer', { updated_at: '2026-09-05T12:00:00Z' }),
      session('s-m', 'Middle', { updated_at: '2026-09-05T11:00:00Z' }),
    ]);
    await screen.findByText('Newer');
    expect(rowOrder()).toEqual(['s-z', 's-m', 's-a']);
  });
});

describe("a History row says its kind once, in the glyph's name (V1-R2-10)", () => {
  it('shows a sub-agent run by its label, with no badge and no "Subagent:" prefix', async () => {
    await renderHistory([
      session('p1', 'Parent chat'),
      session('c1', 'Subagent: Task: find pictures of dogs', {
        session_type: 'sub_agent',
        parent_session_id: 'p1',
      }),
    ]);
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));

    const open = await screen.findByRole('button', {
      name: 'Open chat Task: find pictures of dogs',
    });
    const row = open.closest('[data-session-id]')!;
    expect(within(row as HTMLElement).getByText('Task: find pictures of dogs')).toBeInTheDocument();
    expect(row.textContent).not.toMatch(/Subagent/);
    expect(screen.queryByTestId('subagent-badge')).toBeNull();
    // The glyph carries it, in its accessible name and as the row's kind.
    const glyph = row.querySelector('[data-chat-kind]');
    expect(glyph?.getAttribute('data-chat-kind')).toBe('subagent');
    expect(glyph?.getAttribute('aria-label')).toMatch(/sub-agent/i);
  });

  it('shows an app chat by the app name, not the machine string', async () => {
    await renderHistory([
      session('a1', 'app:living-news-router'),
      session('a2', 'app:researcher-impact-dashboard:analyst'),
    ]);
    const first = await screen.findByRole('button', { name: 'Open chat Living news router' });
    expect(first.closest('[data-session-id]')!.textContent).not.toMatch(/app:/);
    expect(
      first
        .closest('[data-session-id]')!
        .querySelector('[data-chat-kind]')!
        .getAttribute('data-chat-kind')
    ).toBe('app');
    // A worker profile's own chat says which profile.
    expect(
      screen.getByRole('button', { name: 'Open chat Researcher impact dashboard (analyst)' })
    ).toBeInTheDocument();
  });

  it('keeps the stored name searchable: the filter finds a row by what it was called', async () => {
    const user = userEvent.setup();
    await renderHistory([
      session('a1', 'app:living-news-router'),
      session('c1', 'Subagent: audit the cohort', {
        session_type: 'sub_agent',
        parent_session_id: 'gone',
      }),
      session('u1', 'Unrelated chat'),
    ]);
    await screen.findByText('Unrelated chat');
    // The sub-agent run only shows once the toggle fetches it.
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
    await screen.findByText('audit the cohort');

    const filter = screen.getByRole('searchbox', { name: 'Search history' });
    // By the stored spelling (what the person may remember from the sidebar or the CLI)…
    await user.type(filter, 'app:living');
    expect(await screen.findByText('Living news router')).toBeInTheDocument();
    await vi.waitFor(() => expect(screen.queryByText('Unrelated chat')).toBeNull());
    // …and by the title the row shows.
    await user.clear(filter);
    await user.type(filter, 'living news');
    expect(await screen.findByText('Living news router')).toBeInTheDocument();
    await vi.waitFor(() => expect(screen.queryByText('Unrelated chat')).toBeNull());
    await user.clear(filter);
    await user.type(filter, 'subagent');
    expect(await screen.findByText('audit the cohort')).toBeInTheDocument();
  });
});

describe('a row marks itself while its ⋯ menu is open (V1-R2-6)', () => {
  it('sets data-menu-open on the row for as long as the menu is open', async () => {
    const user = userEvent.setup();
    await renderHistory([session('s1', 'Some chat')]);
    const row = (await screen.findByText('Some chat')).closest('[data-session-id]')!;
    expect(row.hasAttribute('data-menu-open')).toBe(false);

    await user.click(screen.getByRole('button', { name: 'More actions for Some chat' }));
    await screen.findByRole('menu');
    expect(row.hasAttribute('data-menu-open')).toBe(true);

    await user.keyboard('{Escape}');
    await vi.waitFor(() => expect(row.hasAttribute('data-menu-open')).toBe(false));
  });
});
