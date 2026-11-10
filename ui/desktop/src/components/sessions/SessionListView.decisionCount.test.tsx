/**
 * A chat's decision-model calls in History (owner rule A1.5): ONE count on the
 * chat's own row, read from the row's `decision_count`, never child rows, and
 * never on a nested subagent run.
 */
import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import { decisionCountOf } from './historyRow';

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

const session = (id: string, name: string, over: Record<string, unknown> = {}) =>
  ({
    id,
    name,
    working_dir: '/Users/someone/cohort-study',
    created_at: '2026-09-05T17:00:00Z',
    updated_at: '2026-09-05T18:00:00Z',
    extension_data: {},
    message_count: 2,
    ...over,
  }) as unknown as Session;

const rowOf = (title: string) =>
  screen
    .getByRole('button', { name: `Open chat ${title}` })
    .closest('[data-session-id]') as HTMLElement;

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

describe('decisionCountOf', () => {
  it('reads a positive count, and anything else as none', () => {
    expect(decisionCountOf({ decision_count: 1204 })).toBe(1204);
    expect(decisionCountOf({ decisionCount: 3 })).toBe(3);
    expect(decisionCountOf({ decision_count: 0 })).toBe(0);
    expect(decisionCountOf({ decision_count: '12' })).toBe(0);
    expect(decisionCountOf({})).toBe(0);
  });
});

describe("a chat's decision count on its History row", () => {
  it('shows one count on a top-level row that has decision calls, and nothing on one without', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          session('a', 'Cohort review', { decision_count: 1204 }),
          session('b', 'Plain chat', { decision_count: 0, updated_at: '2026-09-05T17:30:00Z' }),
        ],
      },
    });
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Cohort review');
    const chip = rowOf('Cohort review').querySelector('.br-history-decisions');
    expect(chip).not.toBeNull();
    expect(chip).toHaveAttribute('data-decision-count', '1204');
    expect(chip).toHaveTextContent('1,204');
    expect(chip).toHaveTextContent('1,204 decision-model calls');
    expect(rowOf('Plain chat').querySelector('.br-history-decisions')).toBeNull();
    // Never rows of their own: the list holds the two chats and nothing else.
    expect(document.querySelectorAll('[data-session-id]')).toHaveLength(2);
  });

  it('never shows a count on a nested subagent run', async () => {
    mocks.listSessions.mockResolvedValue({
      data: {
        sessions: [
          session('p1', 'Cohort review', { session_type: 'user', decision_count: 5 }),
          session('r1', 'Subagent: Run one', {
            session_type: 'sub_agent',
            parent_session_id: 'p1',
            decision_count: 40,
            updated_at: '2026-09-05T17:10:00Z',
          }),
        ],
      },
    });
    render(
      <MemoryRouter>
        <SessionListView onSelectSession={vi.fn()} />
      </MemoryRouter>
    );
    await screen.findByText('Cohort review');
    fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
    await waitFor(() => expect(document.querySelector('[data-session-id="r1"]')).not.toBeNull());
    const child = document.querySelector('[data-session-id="r1"]') as HTMLElement;
    expect(child).toHaveAttribute('data-depth', '1');
    expect(child.querySelector('.br-history-decisions')).toBeNull();
    expect(rowOf('Cohort review').querySelector('.br-history-decisions')).toHaveAttribute(
      'data-decision-count',
      '5'
    );
  });
});
