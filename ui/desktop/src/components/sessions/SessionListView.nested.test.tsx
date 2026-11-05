/**
 * Subagent runs read as subordinate to the chat that started them (the person's ask: in History
 * a run looked as wide and as heavy as its parent).
 *
 * - A run nested under its parent is a `data-depth="1"` row inside the parent's group; an orphan
 *   (its parent is not in the list) keeps the top-level shape, as the grouping rule says.
 * - A nested row is one line: the title, then the time and the model it ran on. No folder (it
 *   is the parent's), and its third stat cell is how the run ended (`subagent_outcome.v1`), never
 *   its extensions. Nothing says "Subagent" in words (V1-R2-10): the glyph does.
 * - A parent with more than three runs shows the newest three and "Show N more runs", which
 *   unfolds the rest in place and hands focus to the first run it revealed. A filter shows every
 *   matching run.
 *
 * jsdom lays nothing out; the geometry (the inset block, the 40px line, the 13px title, the rail
 * and its elbows) is measured in Chromium by `historyRow.browser.test.ts`.
 */
import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import type { ReactNode } from 'react';
import { MemoryRouter } from 'react-router-dom';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import SessionListView from './SessionListView';
import { clearSessionListCache, updateCachedSessionList } from '../../utils/sessionListCache';
import type { Session } from '../../api';
import {
  EXTENSIONS_STAT,
  RUN_STATUS_STAT,
  RUN_STATUS_WORDS,
  runsFromLabel,
  showMoreRuns,
} from './copy';

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

const session = (id: string, name: string, over: Partial<Session> = {}) =>
  ({
    id,
    name,
    working_dir: '/Users/someone/cohort-study',
    created_at: '2026-09-05T17:00:00Z',
    updated_at: '2026-09-05T17:00:00Z',
    extension_data: {},
    message_count: 2,
    ...over,
  }) as Session;

/** A run of `p1`, `minute` minutes past five (later is newer). */
const run = (id: string, minute: number, over: Partial<Session> = {}) =>
  session(id, `Subagent: Run ${id}`, {
    session_type: 'sub_agent',
    parent_session_id: 'p1',
    updated_at: `2026-09-05T17:${String(minute).padStart(2, '0')}:00Z`,
    ...over,
  });

const parent = session('p1', 'Cohort review', {
  session_type: 'user',
  updated_at: '2026-09-05T18:00:00Z',
});

beforeEach(() => {
  vi.clearAllMocks();
  clearSessionListCache();
  (window as unknown as { electron: unknown }).electron = {
    platform: 'darwin',
    on: () => () => {},
  };
});

/** History with the runs toggle on, once the refetch it starts has landed. */
async function renderRuns(sessions: Session[], waitFor_ = 'Cohort review') {
  mocks.listSessions.mockResolvedValue({ data: { sessions } });
  render(
    <MemoryRouter>
      <SessionListView onSelectSession={vi.fn()} />
    </MemoryRouter>
  );
  await screen.findByText(waitFor_);
  fireEvent.click(await screen.findByLabelText(/show subagent runs/i));
  await waitFor(() =>
    expect(mocks.listSessions).toHaveBeenLastCalledWith(
      expect.objectContaining({ query: { include_subagents: true } })
    )
  );
}

const rowOf = (title: string) =>
  screen
    .getByRole('button', { name: `Open chat ${title}` })
    .closest('[data-session-id]') as HTMLElement | null;

describe('a run nested under its parent reads as subordinate', () => {
  it('marks a nested run data-depth="1", and neither the parent nor an orphan', async () => {
    await renderRuns([
      parent,
      run('a', 10),
      session('o1', 'Subagent: Orphan run', {
        session_type: 'sub_agent',
        parent_session_id: 'deleted-parent',
      }),
    ]);
    await screen.findByText('Run a');

    const nested = rowOf('Run a')!;
    expect(nested.getAttribute('data-depth')).toBe('1');
    expect(nested.closest('.br-history-children')).not.toBeNull();

    expect(rowOf('Cohort review')!.hasAttribute('data-depth')).toBe(false);
    const orphan = rowOf('Orphan run')!;
    expect(orphan.hasAttribute('data-depth')).toBe(false);
    expect(orphan.closest('.br-history-children')).toBeNull();
    // The orphan keeps the top-level shape: two lines, its folder named.
    expect(within(orphan).getByText('cohort-study')).toBeInTheDocument();
  });

  it("names the group after its parent, on a filled block, with no 'Subagent' in words", async () => {
    await renderRuns([parent, run('a', 10)]);
    await screen.findByText('Run a');

    const group = screen.getByRole('group', { name: runsFromLabel('Cohort review') });
    expect(group).toHaveClass('br-history-children', 'br-fill-surface');
    expect(within(group).getByText('Run a')).toBeInTheDocument();
    // V1-R2-10: the glyph says the kind, once.
    const nested = rowOf('Run a')!;
    expect(nested.textContent).not.toMatch(/Subagent/);
    expect(nested.querySelector('[data-chat-kind]')?.getAttribute('data-chat-kind')).toBe(
      'subagent'
    );
  });

  it('puts the time and the model on the title line, and drops the folder', async () => {
    await renderRuns([
      parent,
      run('a', 10, {
        provider_name: 'versa_azure',
        model_config: { model_name: 'gpt-5.5', toolshim: false },
      }),
      run('b', 11),
    ]);
    await screen.findByText('Run a');

    const withModel = rowOf('Run a')!;
    // One line: the 13px title class (not the chat's `text-label`), then the meta.
    const title = within(withModel).getByText('Run a');
    expect(title).toHaveClass('br-history-row-title');
    expect(title).not.toHaveClass('text-label');
    const meta = withModel.querySelector('.br-history-row-meta')!;
    expect(meta.querySelector('time')).not.toBeNull();
    expect(within(meta as HTMLElement).getByText('gpt-5.5')).toBeInTheDocument();
    expect(withModel.querySelector('[data-working-dir]')).toBeNull();
    expect(withModel.textContent).not.toMatch(/cohort-study/);

    // A run that recorded no model draws no chip rather than an empty one.
    expect(rowOf('Run b')!.querySelector('[data-run-model]')).toBeNull();
  });

  it('leads a run with the role its parent gave it, keeping the task name in its tooltip and accessible name', async () => {
    await renderRuns([
      parent,
      run('a', 10, {
        extension_data: { 'subagent_spawn.v1': { role: 'celsius', phase_id: 'research' } },
        model_config: { model_name: 'gpt-5.5-2026-04-24', toolshim: false },
      }),
      run('b', 11),
    ]);
    const opener = await screen.findByRole('button', { name: 'Open chat celsius: Run a' });
    const row = opener.closest('[data-session-id]') as HTMLElement;
    const title = within(row).getByText('celsius');
    expect(title).toHaveClass('br-history-row-title');
    expect(title).toHaveAttribute('title', 'Run a');
    // The chip carries the short label; the full id is its tooltip's.
    expect(within(row).getByText('gpt-5.5')).toBeInTheDocument();
    // A run with no role keeps its task-derived title and no tooltip.
    expect(within(rowOf('Run b')!).getByText('Run b')).not.toHaveAttribute('title');
  });

  it("shows the run's status from its outcome record in the third cell, never its extensions", async () => {
    const extensions = {
      'enabled_extensions.v0': { extensions: [{ type: 'builtin', name: 'developer' }] },
    };
    await renderRuns([
      { ...parent, extension_data: extensions },
      run('done', 14, {
        extension_data: {
          ...extensions,
          'subagent_outcome.v1': {
            status: 'completed',
            ended_at: '2026-09-05T17:14:00Z',
            result_line: 'Wrote the summary to notes.md',
          },
        },
      }),
      run('asked', 13, {
        extension_data: { 'subagent_outcome.v1': { status: 'blocked', ended_at: 'x' } },
      }),
      run('broke', 12, {
        extension_data: { 'subagent_outcome.v1': { status: 'failed', ended_at: 'x' } },
      }),
      run('cut', 11, {
        extension_data: { 'subagent_outcome.v1': { status: 'incomplete', ended_at: 'x' } },
      }),
    ]);
    await screen.findByText('Run done');

    for (const [title, status] of [
      ['Run done', 'completed'],
      ['Run asked', 'blocked'],
      ['Run broke', 'failed'],
    ] as const) {
      const cell = rowOf(title)!.querySelector('.br-history-run-status')!;
      expect(cell.getAttribute('data-run-status'), title).toBe(status);
      // The word is spoken; the glyph is decoration.
      expect(cell.textContent, title).toContain(`${RUN_STATUS_STAT}: ${RUN_STATUS_WORDS[status]}`);
      expect(cell.querySelector('svg')?.getAttribute('aria-hidden')).toBe('true');
    }
    // The run's own line is spoken after the word.
    expect(rowOf('Run done')!.querySelector('.br-history-run-status')!.textContent).toContain(
      'Wrote the summary to notes.md'
    );

    // A nested row has no extensions cell, even with extensions recorded; the parent keeps its own.
    for (const title of ['Run done', 'Run asked', 'Run broke']) {
      expect(rowOf(title)!.textContent, title).not.toContain(`${EXTENSIONS_STAT}:`);
    }
    expect(rowOf('Cohort review')!.textContent).toContain(`${EXTENSIONS_STAT}:`);
  });

  it('draws nothing in the status cell for a run with no outcome record, or one still going', async () => {
    await renderRuns([
      parent,
      run('fresh', 10),
      run('busy', 11, {
        extension_data: { 'subagent_outcome.v1': { status: 'running' } },
      }),
    ]);
    await screen.findByText('Run fresh');

    for (const title of ['Run fresh', 'Run busy']) {
      const row = rowOf(title)!;
      expect(row.querySelector('.br-history-run-status'), title).toBeNull();
      // The track stays, empty, so the run's columns do not slide.
      expect(row.querySelector('.br-history-row-stats')!.children).toHaveLength(3);
    }
  });
});

describe('a parent with more than three runs', () => {
  const fiveRuns = () => [
    parent,
    run('r1', 1),
    run('r2', 2),
    run('r3', 3),
    run('r4', 4),
    run('r5', 5),
  ];
  const shownRuns = () =>
    [
      ...screen
        .getByRole('group', { name: runsFromLabel('Cohort review') })
        .querySelectorAll('[data-depth="1"]'),
    ].map((row) => row.getAttribute('data-session-id'));

  it('shows the newest three, then "Show 2 more runs"', async () => {
    await renderRuns(fiveRuns());
    await screen.findByText('Run r5');

    expect(shownRuns()).toEqual(['r5', 'r4', 'r3']);
    const more = screen.getByRole('button', { name: showMoreRuns(2) });
    // Inside the parent's group, after its runs.
    expect(more.closest('.br-history-children')).toBe(
      screen.getByRole('group', { name: runsFromLabel('Cohort review') })
    );
    expect(screen.queryByText('Run r2')).toBeNull();
  });

  it('shows all three of three, with nothing folded', async () => {
    await renderRuns(fiveRuns().slice(0, 4));
    await screen.findByText('Run r3');
    expect(shownRuns()).toEqual(['r3', 'r2', 'r1']);
    expect(screen.queryByRole('button', { name: /more run/ })).toBeNull();
  });

  it('says "1 more run" for one', async () => {
    await renderRuns(fiveRuns().slice(0, 5));
    await screen.findByText('Run r4');
    expect(screen.getByRole('button', { name: 'Show 1 more run' })).toBeInTheDocument();
  });

  it('unfolds in place from the keyboard and hands focus to the first run it revealed', async () => {
    const user = userEvent.setup();
    await renderRuns(fiveRuns());
    await screen.findByText('Run r5');

    // Reachable by Tab: the next stop after the third run's actions.
    screen.getByRole('button', { name: 'More actions for Run r3' }).focus();
    await user.tab();
    const more = screen.getByRole('button', { name: showMoreRuns(2) });
    expect(more).toHaveFocus();

    await user.keyboard('{Enter}');
    await waitFor(() => expect(shownRuns()).toEqual(['r5', 'r4', 'r3', 'r2', 'r1']));
    expect(screen.queryByRole('button', { name: /more run/ })).toBeNull();
    // Focus lands where the row was, on the first run it revealed, not on the page.
    expect(screen.getByRole('button', { name: 'Open chat Run r2' })).toHaveFocus();
  });

  it('keeps a focused run drawn, and focused, when a live update pushes it past the newest three', async () => {
    await renderRuns(fiveRuns());
    await screen.findByText('Run r5');
    const open = screen.getByRole('button', { name: 'Open chat Run r3' });
    open.focus();
    expect(open).toHaveFocus();

    // A new spawn lands at the top: r3 is now the fourth newest run.
    act(() => updateCachedSessionList((current) => [...current, run('r6', 6)]));
    await waitFor(() => expect(shownRuns()).toEqual(['r6', 'r5', 'r4', 'r3']));
    expect(screen.getByRole('button', { name: 'Open chat Run r3' })).toBe(open);
    expect(open).toHaveFocus();
    // The rest still fold, and unfolding reveals the first one not drawn.
    expect(screen.getByRole('button', { name: showMoreRuns(2) })).toBeInTheDocument();
    expect(screen.queryByText('Run r2')).toBeNull();
  });

  it('folds a run again once focus has left it', async () => {
    await renderRuns(fiveRuns());
    await screen.findByText('Run r5');
    screen.getByRole('button', { name: 'Open chat Run r3' }).focus();
    act(() => (document.activeElement as HTMLElement).blur());

    act(() => updateCachedSessionList((current) => [...current, run('r6', 6)]));
    await waitFor(() => expect(shownRuns()).toEqual(['r6', 'r5', 'r4']));
  });

  it('shows every matching run under a filter, with nothing folded', async () => {
    const user = userEvent.setup();
    await renderRuns(fiveRuns());
    await screen.findByText('Run r5');

    await user.type(screen.getByRole('searchbox', { name: 'Search history' }), '/cohort-study');
    await waitFor(() => expect(shownRuns()).toEqual(['r5', 'r4', 'r3', 'r2', 'r1']));
    expect(screen.queryByRole('button', { name: /more run/ })).toBeNull();
  });
});
