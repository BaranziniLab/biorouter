/**
 * A History row's muted line (vision V1-L3, V1-L6): one date format, and one
 * branch glyph per row.
 *
 * - "When" reads the time for today, "Sep 5" this year and "Sep 5, 2025"
 *   before, never "09/05/2026 10:49 AM" under a "September" heading; the full
 *   stamp is the row's tooltip and the `<time>` element's `dateTime`.
 * - A branch says what it branched from in words. The row's kind glyph already
 *   draws the branch, so the line carries no second, different branch glyph.
 */
import { render, screen } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
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

const session = (id: string, name: string, updated: Date, over: Partial<Session> = {}) =>
  ({
    id,
    name,
    working_dir: '/Users/someone/Desktop',
    created_at: updated.toISOString(),
    updated_at: updated.toISOString(),
    extension_data: {},
    message_count: 2,
    ...over,
  }) as Session;

const today = session('s1', 'Today chat', new Date(2026, 9, 9, 14, 39));
const september = session('s2', 'September chat', new Date(2026, 8, 5, 10, 49));
const lastYear = session('s3', 'Last year chat', new Date(2025, 8, 5, 10, 49));
const branch = session('s4', 'Report folders (branch 1)', new Date(2026, 8, 5, 10, 40), {
  diverged_from: 's2',
});

/** ICU may put a narrow no-break space before AM/PM; the words are what is pinned. */
const plain = (text: string | null | undefined) => (text ?? '').replace(/\s/g, ' ');

const rowOf = (name: string) =>
  screen.getByRole('button', { name: `Open chat ${name}` }).closest('[data-session-id]')!;

beforeEach(() => {
  // Only the clock is faked; timers stay real so the list loads as it does live.
  vi.useFakeTimers({ toFake: ['Date'] });
  vi.setSystemTime(new Date(2026, 9, 9, 15, 0));
  vi.clearAllMocks();
  clearSessionListCache();
  mocks.listSessions.mockResolvedValue({
    data: { sessions: [today, september, branch, lastYear] },
  });
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

afterEach(() => {
  vi.useRealTimers();
});

async function renderHistory() {
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText('Today chat');
}

describe("a History row's muted line", () => {
  it('reads one date format: the time today, the day this year, the year before that', async () => {
    await renderHistory();
    const when = (name: string) => rowOf(name).querySelector('time');

    expect(plain(when('Today chat')?.textContent)).toBe('2:39 PM');
    expect(plain(when('September chat')?.textContent)).toBe('Sep 5');
    expect(plain(when('Last year chat')?.textContent)).toBe('Sep 5, 2025');
    // The machine-readable stamp travels with it.
    expect(when('September chat')).toHaveAttribute('datetime', september.updated_at);
    // No numeric US date anywhere on the list.
    for (const time of document.querySelectorAll('[data-history-layer="content"] time')) {
      expect(time.textContent).not.toMatch(/\//);
    }
  });

  it('names what a branch came from in words, with no second branch glyph', async () => {
    await renderHistory();
    const row = rowOf('Report folders (branch 1)');

    expect(row.querySelector('.br-history-row-meta')).toHaveTextContent(
      'Branched from September chat'
    );
    expect(row.querySelector('.br-history-row-meta svg')).toBeNull();
    // The row's one branch glyph is its kind glyph.
    expect(row.querySelectorAll('[data-chat-kind="branch"]')).toHaveLength(1);
  });
});
