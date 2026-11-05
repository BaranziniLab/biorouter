import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { parseSessionSubagents, type SessionSubagents } from '../../utils/sessionSubagents';
import { wirePhase, wireRow, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { SummarySubagents, type SummarySubagentsData } from './SummarySubagents';
import { summaryCopy } from './copy';

const copy = summaryCopy.agents;

function data(rows: Record<string, unknown>[], plan: Record<string, unknown> | null = null) {
  return parseSessionSubagents(wireSubagents(rows, plan)) as SessionSubagents;
}

function draw(value: SessionSubagents | null, overrides: Partial<SummarySubagentsData> = {}) {
  const onOpen = vi.fn();
  const refresh = vi.fn();
  const utils = render(
    <SummarySubagents
      data={value}
      loading={false}
      error={false}
      refresh={refresh}
      onOpen={onOpen}
      {...overrides}
    />
  );
  return { ...utils, onOpen, refresh };
}

/** A focus the person moved with Tab, the only programmatic-looking focus a tooltip accepts. */
function focusWithTab(element: HTMLElement) {
  fireEvent.keyDown(document.body, { key: 'Tab' });
  fireEvent.focus(element);
  fireEvent.keyUp(document.body, { key: 'Tab' });
}

const many = (count: number, status: string, prefix: string) =>
  Array.from({ length: count }, (_, i) =>
    wireRow({ sessionId: `${prefix}-${i}`, name: `${prefix} task ${i + 1}`, status })
  );

describe('nothing to list', () => {
  it('draws nothing without data, or with no subagents and no plan', () => {
    const { container, rerender } = draw(null);
    expect(container).toBeEmptyDOMElement();
    rerender(
      <SummarySubagents
        data={data([])}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
      />
    );
    expect(container).toBeEmptyDOMElement();
  });
});

describe('up to four subagents: comfortable two-line rows', () => {
  it('leads with the status shape and the role or name, then model, setup and run time', () => {
    draw(
      data([
        wireRow({ sessionId: 'a', role: 'Reviewer' }),
        wireRow({ sessionId: 'b', status: 'completed', elapsedSeconds: 3900, quietSeconds: null }),
      ])
    );
    const section = screen.getByRole('region', { name: copy.subagents });
    expect(within(section).getByText(copy.subagents)).toBeInTheDocument();
    // Heading counts: running first, then done of total.
    expect(within(section).getByText(`${copy.running(1)} · 1 of 2`)).toBeInTheDocument();
    expect(within(section).getByText(`1 of 2${copy.doneSuffix}`)).toHaveAttribute(
      'aria-live',
      'polite'
    );

    const first = screen.getByTestId('subagent-row-a');
    expect(first).toHaveAttribute('data-layout', 'comfortable');
    expect(first).toHaveAttribute('data-status', 'running');
    expect(first).toHaveAccessibleName(
      'Running, Reviewer, claude-sonnet-4-5, 2 ext, 1 skill, 1 KB, 1m'
    );
    expect(first).toHaveTextContent('claude-sonnet-4-5 · 2 ext · 1 skill · 1 KB · 1m');
    const second = screen.getByTestId('subagent-row-b');
    expect(second).toHaveAccessibleName(/^Completed, Search PubMed for trials, /);
    expect(second).toHaveTextContent('1h 5m');
    // No state groups in a short list.
    expect(screen.queryByRole('button', { name: /Needs attention/ })).toBeNull();
  });

  it('says a long-silent active child is quiet, never stalled', () => {
    draw(data([wireRow({ sessionId: 'a', quietSeconds: 240 })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row).toHaveAttribute('data-quiet', '');
    expect(row).toHaveTextContent(copy.quietFor('4m'));
    expect(row).not.toHaveTextContent(/stall|dead|stuck/i);
  });

  it('opens the subagent as a tab on click, with its name', () => {
    const { onOpen } = draw(data([wireRow({ sessionId: 'a', role: 'Reviewer' })]));
    fireEvent.click(screen.getByTestId('subagent-row-a'));
    expect(onOpen).toHaveBeenCalledWith('a', 'Search PubMed for trials');
  });

  it('marks a private child with the lock, read as words', () => {
    draw(data([wireRow({ sessionId: 'a', privacyTier: 'private' })]));
    expect(screen.getByTestId('subagent-row-a')).toHaveAccessibleName(
      /^Running, Search PubMed for trials, .*, Private$/
    );
  });
});

describe('five or more: dense rows grouped by state', () => {
  const rows = [
    ...many(3, 'running', 'run'),
    wireRow({ sessionId: 'blocked', status: 'blocked', name: 'Asks a question' }),
    wireRow({ sessionId: 'failed', status: 'failed', name: 'Broke' }),
    ...many(15, 'completed', 'done'),
  ];

  it('groups Running, Needs attention and Done, in that order, with counts', () => {
    draw(data(rows));
    const toggles = screen
      .getAllByRole('button', { expanded: true })
      .map((button) => button.getAttribute('aria-label'));
    expect(toggles).toEqual([
      `${copy.groups.running}, 3`,
      `${copy.groups.attention}, 2`,
      `${copy.groups.done}, 15`,
    ]);
    expect(screen.getByTestId('subagent-row-run-0')).toHaveAttribute('data-layout', 'dense');
    // A dense row is one line: the run time trails, the setup line is gone.
    expect(screen.getByTestId('subagent-row-run-0')).not.toHaveTextContent('ext');
    expect(
      within(screen.getByRole('list', { name: copy.groups.attention })).getAllByRole('button')
    ).toHaveLength(2);
  });

  it('shows at most 12 rows of a group, then Show all N, and folds a group', () => {
    draw(data(rows));
    const done = screen.getByRole('list', { name: copy.groups.done });
    expect(within(done).getAllByTestId(/^subagent-row-/)).toHaveLength(12);
    fireEvent.click(within(done).getByRole('button', { name: copy.showAll(15) }));
    expect(within(done).getAllByTestId(/^subagent-row-/)).toHaveLength(15);
    fireEvent.click(within(done).getByRole('button', { name: copy.showFewer }));
    expect(within(done).getAllByTestId(/^subagent-row-/)).toHaveLength(12);

    const toggle = screen.getByRole('button', { name: `${copy.groups.done}, 15` });
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByRole('list', { name: copy.groups.done })).toBeNull();
    expect(screen.getByTestId('subagent-row-run-0')).toBeInTheDocument();
  });
});

describe('with a phase plan', () => {
  const plan = {
    title: 'Survey the trials',
    goal: null,
    revision: 2,
    phases: [
      wirePhase({
        id: 'plan',
        title: 'Plan the search',
        kind: 'plan',
        status: 'completed',
        agentIds: ['p1'],
        counts: { total: 1, done: 1, running: 0, blocked: 0, failed: 0, quiet: 0 },
      }),
      wirePhase({
        id: 'research',
        title: 'Read the trials',
        kind: 'research',
        status: 'in_progress',
        agentIds: ['r1', 'r2', 'r3'],
        counts: { total: 3, done: 1, running: 1, blocked: 0, failed: 1, quiet: 1 },
      }),
      wirePhase({ id: 'verify', title: 'Check the numbers', kind: 'verification' }),
      wirePhase({ id: 'meet', title: 'Reconcile', kind: 'meeting' }),
    ],
  };
  const rows = [
    wireRow({ sessionId: 'p1', status: 'completed', phaseId: 'plan', quietSeconds: null }),
    wireRow({ sessionId: 'r1', status: 'completed', phaseId: 'research', role: 'Reader 1' }),
    wireRow({ sessionId: 'r2', status: 'running', phaseId: 'research', quietSeconds: 400 }),
    wireRow({ sessionId: 'r3', status: 'failed', phaseId: 'research' }),
    wireRow({ sessionId: 'loose', status: 'running', name: 'A one-off check' }),
  ];

  it('draws the plan’s title and each phase with its kind, progress and counts', () => {
    draw(data(rows, plan));
    const phases = screen.getByRole('region', { name: copy.phases });
    expect(within(phases).getByText('Survey the trials')).toBeInTheDocument();
    expect(within(phases).getByText(`1 of 4${copy.phasesCountSuffix}`)).toBeInTheDocument();
    const list = within(phases).getByRole('list', { name: copy.phaseListLabel });
    const items = within(list)
      .getAllByRole('listitem')
      .filter((li) => li.dataset.phaseId);
    expect(items.map((li) => li.dataset.phaseId)).toEqual(['plan', 'research', 'verify', 'meet']);

    const research = items[1];
    expect(
      within(research).getByRole('progressbar', { name: 'Read the trials progress' })
    ).toHaveAttribute('aria-valuenow', '1');
    expect(within(research).getByText('1 running · 1 quiet · 1 failed')).toBeInTheDocument();
    const toggle = within(research).getByRole('button', { name: /^Read the trials/ });
    expect(toggle).toHaveAccessibleName(
      'Read the trials, Research, In progress, 1 of 3 subagents done'
    );
  });

  it('opens the running phase and folds the finished one in a plan of more than three', () => {
    draw(data(rows, plan));
    const planToggle = screen.getByRole('button', { name: /^Plan the search/ });
    const researchToggle = screen.getByRole('button', { name: /^Read the trials/ });
    expect(planToggle).toHaveAttribute('aria-expanded', 'false');
    expect(researchToggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.queryByTestId('subagent-row-p1')).toBeNull();
    expect(screen.getByTestId('subagent-row-r2')).toBeInTheDocument();
    fireEvent.click(planToggle);
    expect(screen.getByTestId('subagent-row-p1')).toBeInTheDocument();
    // A phase with no subagents is a plain line, not a control.
    expect(screen.queryByRole('button', { name: /^Check the numbers/ })).toBeNull();
    expect(
      screen.getByRole('group', { name: /^Check the numbers, Verification, Pending/ })
    ).toBeInTheDocument();
  });

  it('lists a subagent in no phase under Other subagents', () => {
    draw(data(rows, plan));
    const other = screen.getByRole('region', { name: copy.otherSubagents });
    expect(within(other).getByTestId('subagent-row-loose')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: copy.subagents })).toBeNull();
  });

  it('draws the phases of a plan that has no subagents yet', () => {
    draw(data([], plan));
    expect(screen.getByRole('region', { name: copy.phases })).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: copy.otherSubagents })).toBeNull();
  });
});

describe('the hover card', () => {
  it('shows the full setup on a Tab focus', async () => {
    draw(
      data([
        wireRow({
          sessionId: 'a',
          role: 'Reviewer',
          phaseId: 'research',
          resultLine: 'Found 12 trials',
          status: 'completed',
          quietSeconds: null,
          privacyTier: 'private',
          extensions: ['developer', 'memory'],
          skills: [],
          knowledgeBases: ['soul', 'trials'],
        }),
      ])
    );
    focusWithTab(screen.getByTestId('subagent-row-a'));
    const [card] = await screen.findAllByTestId('subagent-card-a');
    expect(card).toHaveTextContent('Search PubMed for trials');
    expect(card).toHaveTextContent(`${copy.card.role}: Reviewer`);
    expect(card).toHaveTextContent(`${copy.status.completed}: Found 12 trials`);
    expect(card).toHaveTextContent('claude-sonnet-4-5-20250929 (anthropic)');
    expect(card).toHaveTextContent('developer, memory');
    expect(card).toHaveTextContent(`${copy.card.skills}${copy.card.none}`);
    expect(card).toHaveTextContent('soul, trials');
    expect(card).toHaveTextContent('12.3K tokens · 3 tool calls · 7 messages · 1m');
    expect(card).toHaveTextContent(copy.card.private);
  });

  it('names the phase the child works in, by the plan’s title for it', async () => {
    const plan = {
      title: 'Plan',
      goal: null,
      revision: 1,
      phases: [wirePhase({ id: 'research', title: 'Read the trials', status: 'in_progress' })],
    };
    draw(data([wireRow({ sessionId: 'a', phaseId: 'research' })], plan));
    focusWithTab(screen.getByTestId('subagent-row-a'));
    const [card] = await screen.findAllByTestId('subagent-card-a');
    expect(card).toHaveTextContent(`${copy.card.phase}: Read the trials`);
  });

  it('is not built until it first opens', () => {
    draw(data([wireRow({ sessionId: 'a' })]));
    expect(screen.queryByTestId('subagent-card-a')).toBeNull();
  });
});

describe('keyboard', () => {
  const rows = [
    wireRow({ sessionId: 'a', name: 'First' }),
    wireRow({ sessionId: 'b', name: 'Second' }),
    wireRow({ sessionId: 'c', name: 'Third', status: 'completed' }),
  ];

  it('is one tab stop, on the first running row, and the arrows move between rows', async () => {
    draw(data(rows));
    const [a, b, c] = ['a', 'b', 'c'].map((id) => screen.getByTestId(`subagent-row-${id}`));
    expect([a, b, c].map((row) => row.tabIndex)).toEqual([0, -1, -1]);
    act(() => a.focus());
    fireEvent.keyDown(a, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(b);
    // A move the person made with the arrows opens that row's card.
    expect((await screen.findAllByTestId('subagent-card-b')).length).toBeGreaterThan(0);
    fireEvent.keyDown(b, { key: 'End' });
    expect(document.activeElement).toBe(c);
    fireEvent.keyDown(c, { key: 'Home' });
    expect(document.activeElement).toBe(a);
    expect([a, b, c].map((row) => row.tabIndex)).toEqual([0, -1, -1]);
    fireEvent.keyDown(a, { key: 'Enter' });
    fireEvent.click(a);
  });

  /** Redraw with new data, as a poll does. */
  function redraw(rerender: ReturnType<typeof render>['rerender'], value: SessionSubagents) {
    rerender(
      <SummarySubagents
        data={value}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
      />
    );
  }

  it('keeps focus and the card on a row that a poll moves to another group', async () => {
    const running = many(6, 'running', 'run');
    const { rerender } = draw(data(running));
    const first = screen.getByTestId('subagent-row-run-0');
    act(() => first.focus());
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'ArrowDown' });
    const before = screen.getByTestId('subagent-row-run-2');
    expect(document.activeElement).toBe(before);
    expect((await screen.findAllByTestId('subagent-card-run-2')).length).toBeGreaterThan(0);

    // The child finishes: its row is drawn under Done now, a different parent.
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    const after = screen.getByTestId('subagent-row-run-2');
    expect(after).not.toBe(before);
    expect(after.closest('[data-group]')).toHaveAttribute('data-group', 'done');
    expect(document.activeElement).toBe(after);
    expect((await screen.findAllByTestId('subagent-card-run-2')).length).toBeGreaterThan(0);
    // Still the list's one tab stop, so the arrows go on from it.
    expect(after.tabIndex).toBe(0);
    fireEvent.keyDown(after, { key: 'ArrowUp' });
    expect(document.activeElement).not.toBe(document.body);
  });

  it('keeps focus on a row when a fifth spawn turns the flat list into groups', () => {
    const four = many(4, 'running', 'run');
    const { rerender } = draw(data(four));
    const before = screen.getByTestId('subagent-row-run-1');
    act(() => before.focus());
    act(() => redraw(rerender, data([...four, ...many(1, 'running', 'late')])));
    const after = screen.getByTestId('subagent-row-run-1');
    expect(after).toHaveAttribute('data-layout', 'dense');
    expect(document.activeElement).toBe(after);
  });

  it('leaves focus alone when the person moved it out of the list', () => {
    const running = many(6, 'running', 'run');
    const outside = document.createElement('button');
    document.body.appendChild(outside);
    const { rerender } = draw(data(running));
    act(() => screen.getByTestId('subagent-row-run-2').focus());
    act(() => outside.focus());
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    expect(document.activeElement).toBe(outside);
    outside.remove();
  });

  it('reaches group headers and Show all from the arrows too', () => {
    draw(data([...many(3, 'running', 'run'), ...many(13, 'completed', 'done')]));
    const first = screen.getByRole('button', { name: `${copy.groups.running}, 3` });
    act(() => first.focus());
    fireEvent.keyDown(first, { key: 'End' });
    expect(document.activeElement).toHaveTextContent(copy.showAll(13));
  });
});

describe('a failed refresh', () => {
  it('keeps the rows and offers Retry', () => {
    const { refresh } = draw(data([wireRow({ sessionId: 'a' })]), { error: true });
    expect(screen.getByTestId('subagent-row-a')).toBeInTheDocument();
    const alert = screen.getByRole('alert');
    expect(alert).toHaveTextContent(copy.refreshFailed);
    fireEvent.click(within(alert).getByRole('button', { name: summaryCopy.retry }));
    expect(refresh).toHaveBeenCalled();
  });
});
