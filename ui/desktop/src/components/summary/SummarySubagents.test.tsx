import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { parseSessionSubagents, type SessionSubagents } from '../../utils/sessionSubagents';
import {
  olderPhase,
  wirePhase,
  wireRow,
  wireSubagents,
} from '../../utils/sessionSubagents.fixture';
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
  it('leads with the status shape and the role or name, then one line of what it is doing', () => {
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
    expect(first).toHaveAccessibleName('Running, Reviewer, Working…');
    expect(first).toHaveTextContent(/^ReviewerWorking…$/);
    const second = screen.getByTestId('subagent-row-b');
    // An ended child with no result line says its status word.
    expect(second).toHaveAccessibleName('Completed, Search PubMed for trials, Completed');
    // No state groups in a short list.
    expect(screen.queryByRole('button', { name: /Needs attention/ })).toBeNull();
  });

  it('follows section 4.1 for the second line, status by status', () => {
    draw(
      data([
        wireRow({ sessionId: 'q', status: 'queued', quietSeconds: null }),
        wireRow({ sessionId: 'c', status: 'cancelling', quietSeconds: null }),
        wireRow({
          sessionId: 'r',
          status: 'completed',
          quietSeconds: null,
          resultLine: 'Found 12 trials',
        }),
        wireRow({ sessionId: 'i', status: 'incomplete', quietSeconds: null }),
      ])
    );
    const line = (id: string) =>
      screen.getByTestId(`subagent-row-${id}`).querySelector('.br-agents__meta')?.textContent;
    expect(line('q')).toBe('Waiting to start');
    expect(line('c')).toBe('Stopping…');
    expect(line('r')).toBe('Found 12 trials');
    // A child that ended early reads Stopped, never Incomplete.
    expect(line('i')).toBe('Stopped');
    expect(screen.getByTestId('subagent-row-i')).toHaveAccessibleName(
      'Stopped, Search PubMed for trials, Stopped'
    );
  });

  it('draws no model, setup counts or run time on the row (C-9: they are in the peek)', () => {
    draw(data([wireRow({ sessionId: 'a' })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row.querySelector('[data-count]')).toBeNull();
    expect(row).not.toHaveTextContent(/claude|anthropic|1m|developer/);
  });

  it('says a long-silent active child is quiet, never stalled', () => {
    draw(data([wireRow({ sessionId: 'a', quietSeconds: 240 })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row).toHaveAttribute('data-quiet', '');
    expect(row).toHaveTextContent(copy.quietFor('4m'));
    expect(row).not.toHaveTextContent(/stall|dead|stuck/i);
    expect(row).toHaveAccessibleName('Running, Search PubMed for trials, Quiet for 4m');
  });

  it('keeps the peek on click, and does not open the tab: Open in a tab does, with its role', async () => {
    const { onOpen } = draw(data([wireRow({ sessionId: 'a', role: 'Reviewer' })]));
    const row = screen.getByTestId('subagent-row-a');
    fireEvent.click(row);
    expect(onOpen).not.toHaveBeenCalled();
    expect(row).toHaveAttribute('data-pinned', 'true');
    expect(row).toHaveAttribute('aria-expanded', 'true');
    const card = await screen.findByTestId('subagent-card-a');
    fireEvent.click(within(card).getByRole('button', { name: copy.peek.openInTab }));
    expect(onOpen).toHaveBeenCalledWith('a', 'Reviewer');
    // A second click on a kept peek closes it.
    fireEvent.click(row);
    fireEvent.click(row);
    expect(row).not.toHaveAttribute('data-pinned');
  });

  it('marks a private child with a muted lock, read as words', () => {
    draw(data([wireRow({ sessionId: 'a', privacyTier: 'private' })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row).toHaveAccessibleName('Running, Search PubMed for trials, Working…, private');
    expect(row.querySelector('.br-agents__lock')).not.toBeNull();
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
    expect(screen.getByTestId('subagent-row-run-0').querySelector('[data-count]')).toBeNull();
    expect(screen.getByTestId('subagent-row-run-0')).toHaveAccessibleName(
      'Running, run task 1, 1m'
    );
    expect(
      within(screen.getByRole('list', { name: copy.groups.attention })).getAllByRole('button')
    ).toHaveLength(2);
  });

  it('orders each group by spawn, never by when its children finished', () => {
    const fanOut = (n: number, status: string, second: number) =>
      wireRow({
        sessionId: `r${n}`,
        name: `ribosome ${n}`,
        status,
        quietSeconds: null,
        createdAt: `2026-11-04T10:00:0${second}Z`,
      });
    // One fan-out, all but the first spawned in the same second, so the roster's tie-break is
    // a race; the finished ones arrive here in the order they finished.
    const finished = [
      fanOut(10, 'completed', 1),
      fanOut(1, 'completed', 0),
      fanOut(3, 'completed', 1),
    ];
    const { rerender } = draw(
      data([
        fanOut(5, 'running', 1),
        fanOut(4, 'running', 1),
        ...finished,
        fanOut(2, 'completed', 1),
      ])
    );
    const order = (group: string) =>
      within(screen.getByRole('list', { name: group }))
        .getAllByTestId(/^subagent-row-/)
        .map((row) => row.getAttribute('data-testid')!.replace('subagent-row-', ''));
    expect(order(copy.groups.done)).toEqual(['r1', 'r2', 'r3', 'r10']);
    expect(order(copy.groups.running)).toEqual(['r4', 'r5']);

    // r4 finishes last, and takes its sibling place in Done rather than the end.
    rerender(
      <SummarySubagents
        data={data([
          fanOut(5, 'running', 1),
          ...finished,
          fanOut(2, 'completed', 1),
          fanOut(4, 'completed', 1),
        ])}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
      />
    );
    expect(order(copy.groups.done)).toEqual(['r1', 'r2', 'r3', 'r4', 'r10']);
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

  it('draws each phase as one line: status glyph, title, count, and no bar', () => {
    draw(data(rows, plan));
    const phases = screen.getByRole('region', { name: copy.phasesLabel('Survey the trials') });
    // The plan's title is the section's name, not a line of its own.
    expect(within(phases).queryByText('Survey the trials')).toBeNull();
    expect(within(phases).getByText(`1 of 4${copy.phasesCountSuffix}`)).toBeInTheDocument();
    const list = within(phases).getByRole('list', { name: copy.phaseListLabel });
    const items = within(list)
      .getAllByRole('listitem')
      .filter((li) => li.dataset.phaseId);
    expect(items.map((li) => li.dataset.phaseId)).toEqual(['plan', 'research', 'verify', 'meet']);
    expect(within(phases).queryByRole('progressbar')).toBeNull();

    const research = items[1];
    // Only what needs acting on, under the line: the running and quiet counts are on the rows.
    expect(within(research).getByText('1 failed')).toHaveClass('br-agents__phase-counts');
    const toggle = within(research).getByRole('button', { name: /^Read the trials/ });
    expect(toggle).toHaveAccessibleName(
      'Read the trials, research phase, in progress, 1 of 3 subagents done'
    );
    expect(toggle).toHaveTextContent('1 of 3');
    expect(toggle.closest('.br-agents__phase-line')).not.toBeNull();
  });

  it('shows no "0 of 0" for a phase that used no subagents (H-12)', () => {
    draw(data(rows, plan));
    const verify = screen.getByRole('group', { name: /^Check the numbers/ });
    expect(verify).toHaveAccessibleName('Check the numbers, verification phase, pending');
    expect(verify).not.toHaveTextContent(/0 of 0/);
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
  });

  it('lists a subagent in no phase under Other subagents', () => {
    draw(data(rows, plan));
    const other = screen.getByRole('region', { name: copy.otherSubagents });
    expect(within(other).getByTestId('subagent-row-loose')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: copy.subagents })).toBeNull();
  });

  it('draws the phases of a plan that has no subagents yet', () => {
    draw(data([], plan));
    expect(
      screen.getByRole('region', { name: copy.phasesLabel('Survey the trials') })
    ).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: copy.otherSubagents })).toBeNull();
  });
});

describe('the peek', () => {
  const child = () =>
    wireRow({
      sessionId: 'a',
      role: 'Reviewer',
      resultLine: 'Found 12 trials',
      status: 'completed',
      quietSeconds: null,
      elapsedSeconds: 3,
      privacyTier: 'private',
      providerName: 'versa_azure',
      modelName: 'gpt-5.5-2026-04-24',
      extensions: ['developer', 'computercontroller', 'memory'],
      skills: [],
      knowledgeBases: ['soul', 'trial-notes'],
    });

  it('previews on a Tab focus, leaving the row at rest', async () => {
    draw(data([child()]));
    const row = screen.getByTestId('subagent-row-a');
    focusWithTab(row);
    const card = await screen.findByTestId('subagent-card-a');
    expect(card).toHaveAccessibleName('Reviewer, subagent');
    expect(row).not.toHaveAttribute('data-pinned');
    expect(row).toHaveAttribute('aria-expanded', 'true');
  });

  it('holds the task, the result and Open in a tab, and the setup by name under Details', async () => {
    draw(data([child()]));
    fireEvent.click(screen.getByTestId('subagent-row-a'));
    const card = await screen.findByTestId('subagent-card-a');
    expect(card).toHaveTextContent(copy.peek.task);
    expect(card).toHaveTextContent('Search PubMed for trials');
    expect(card).toHaveTextContent('Completed · 3s');
    expect(card).toHaveTextContent('Found 12 trials');
    expect(within(card).getByRole('button', { name: copy.peek.openInTab })).toBeInTheDocument();
    const details = within(card).getByRole('button', { name: copy.peek.details });
    // Closed, with the model's short name as its summary.
    expect(details).toHaveAccessibleDescription('gpt-5.5');
    fireEvent.click(details);
    expect(card).toHaveTextContent('Developer');
    expect(card).toHaveTextContent('Biorouter Copilot');
    expect(card).toHaveTextContent(`${copy.peek.skills}${copy.peek.none}`);
    // A knowledge base id the list does not know reads as words, never raw.
    expect(card).toHaveTextContent('Soul, Trial notes');
    expect(card).toHaveTextContent('12.3K tokens · 3 tool calls · 7 messages');
  });

  it('shows no provider key, dated slug, raw extension key or knowledge base id (C-9)', async () => {
    draw(data([child()]));
    fireEvent.click(screen.getByTestId('subagent-row-a'));
    const card = await screen.findByTestId('subagent-card-a');
    fireEvent.click(within(card).getByRole('button', { name: copy.peek.details }));
    const text = card.textContent ?? '';
    expect(text).not.toMatch(/versa_azure|2026-04-24|computercontroller|\bsoul\b|trial-notes/);
    // The role and phase lines and the old hint are gone.
    expect(text).not.toMatch(/Role:|Phase:|Click to open/);
  });

  it('names a decision model by its alias and where it runs', async () => {
    draw(
      data([wireRow({ sessionId: 'd', providerName: 'ollama_decision', modelName: 'tev1:0.8b' })])
    );
    fireEvent.click(screen.getByTestId('subagent-row-d'));
    const card = await screen.findByTestId('subagent-card-d');
    const details = within(card).getByRole('button', { name: copy.peek.details });
    expect(details).toHaveAccessibleDescription('tev1:0.8b · this computer');
    expect(card.textContent).not.toMatch(/ollama_decision/);
  });

  it('says Working… while the child is active, and that one that ended left nothing', async () => {
    draw(
      data([
        wireRow({ sessionId: 'w' }),
        wireRow({ sessionId: 'e', status: 'failed', quietSeconds: null }),
      ])
    );
    fireEvent.click(screen.getByTestId('subagent-row-w'));
    expect(await screen.findByTestId('subagent-card-w')).toHaveTextContent(copy.working);
    fireEvent.click(screen.getByTestId('subagent-row-e'));
    expect(await screen.findByTestId('subagent-card-e')).toHaveTextContent(copy.peek.noResult);
    // One open at a time.
    expect(screen.queryByTestId('subagent-card-w')).toBeNull();
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

  it('is one tab stop, on the first running row, and the arrows move between rows, opening nothing', () => {
    draw(data(rows));
    const [a, b, c] = ['a', 'b', 'c'].map((id) => screen.getByTestId(`subagent-row-${id}`));
    expect([a, b, c].map((row) => row.tabIndex)).toEqual([0, -1, -1]);
    act(() => a.focus());
    fireEvent.keyDown(a, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(b);
    // A move with the arrows opens no peek.
    expect(screen.queryByTestId('subagent-card-b')).toBeNull();
    fireEvent.keyDown(b, { key: 'End' });
    expect(document.activeElement).toBe(c);
    fireEvent.keyDown(c, { key: 'Home' });
    expect(document.activeElement).toBe(a);
    expect([a, b, c].map((row) => row.tabIndex)).toEqual([0, -1, -1]);
  });

  it('keeps the peek on Enter and moves focus into it; Escape returns focus to the row', async () => {
    draw(data(rows));
    const a = screen.getByTestId('subagent-row-a');
    act(() => a.focus());
    // Enter on a button is a click with no pointer detail.
    fireEvent.click(a, { detail: 0 });
    const card = await screen.findByTestId('subagent-card-a');
    expect(a).toHaveAttribute('data-pinned', 'true');
    expect(card.contains(document.activeElement)).toBe(true);
    fireEvent.keyDown(document.activeElement as HTMLElement, { key: 'Escape' });
    expect(screen.queryByTestId('subagent-card-a')).toBeNull();
    expect(document.activeElement).toBe(a);
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

  it('keeps a kept peek, and its row, through a poll that finishes the child', async () => {
    const running = many(6, 'running', 'run');
    const { rerender } = draw(data(running));
    const before = screen.getByTestId('subagent-row-run-2');
    fireEvent.click(before);
    expect(await screen.findByTestId('subagent-card-run-2')).toBeInTheDocument();

    // The child finishes. While its peek is kept the row stays where it was kept,
    // so the poll does not remount it and the peek stays open.
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    const after = screen.getByTestId('subagent-row-run-2');
    expect(after).toBe(before);
    expect(after.closest('[data-group]')).toHaveAttribute('data-group', 'running');
    expect(screen.getByTestId('subagent-card-run-2')).toBeInTheDocument();
    expect(after).toHaveAttribute('data-status', 'completed');

    // Let go, it moves to Done.
    fireEvent.click(after);
    expect(screen.getByTestId('subagent-row-run-2').closest('[data-group]')).toHaveAttribute(
      'data-group',
      'done'
    );
  });

  it('keeps focus on a row that a poll moves to another group', () => {
    const running = many(6, 'running', 'run');
    const { rerender } = draw(data(running));
    const before = screen.getByTestId('subagent-row-run-2');
    act(() => before.focus());
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    const after = screen.getByTestId('subagent-row-run-2');
    expect(after).not.toBe(before);
    expect(after.closest('[data-group]')).toHaveAttribute('data-group', 'done');
    expect(document.activeElement).toBe(after);
    expect(after.tabIndex).toBe(0);
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

/**
 * Item C: the person's gate controls on the plan. A gate's chip opens its
 * dialog (`decisions/PhaseGateDialog.tsx`, mounted by BaseChat), says when the
 * gate waits for the person and when the person passed or skipped the phase; a
 * phase a gate holds says what it waits for; an open phase with no gate offers
 * "Add gate".
 */
describe('phase gates and the person', () => {
  const GATE = {
    threshold: 0.9,
    questionCount: 1,
    attempts: 1,
    maxAttempts: 1,
    lastPassed: false,
    lastMinP: 0.02,
    blocked: true,
    owner: 'agent',
    questions: { y2031: 'Does the hand-off cite a paper published in 2031?' },
    needsPerson: true,
    holds: ['write', 'review'],
  };
  const gatedPlan = (gate: Record<string, unknown>, research: Record<string, unknown> = {}) => ({
    title: 'Moon facts',
    goal: null,
    revision: 1,
    phases: [
      wirePhase({ id: 'research', title: 'Research', status: 'in_progress', gate, ...research }),
      wirePhase({ id: 'write', title: 'Write', kind: 'synthesis', heldBy: 'research' }),
      wirePhase({ id: 'review', title: 'Review', kind: 'review', heldBy: 'research' }),
    ],
  });
  const phaseItem = (id: string) =>
    document.querySelector<HTMLElement>(`[data-phase-id="${id}"]`) as HTMLElement;

  it('says a gate that failed every attempt needs the person, and the chip opens its dialog', () => {
    const onOpenGate = vi.fn();
    draw(data([], gatedPlan(GATE)), { onOpenGate });
    const chip = screen.getByTestId('phase-gate-research');
    expect(chip).toHaveTextContent('Needs you');
    expect(chip).toHaveAttribute('data-state', 'blocked');
    expect(chip).toHaveAttribute('data-needs-person');
    expect(chip).toHaveAccessibleName('Gate needs you: blocked after 1 attempts, Research');
    // On the phase's own line, never a row of its own.
    expect(chip.closest('.br-agents__phase-line')).not.toBeNull();
    fireEvent.click(chip);
    expect(onOpenGate).toHaveBeenCalledWith('research');
  });

  it('says what a held phase waits for, by the gated phase’s title', () => {
    draw(data([], gatedPlan(GATE)), { onOpenGate: () => {} });
    for (const id of ['write', 'review']) {
      const item = phaseItem(id);
      expect(within(item).getByText("Waits for Research's gate")).toHaveClass(
        'br-agents__phase-counts'
      );
      expect(item).toHaveAttribute('data-held-by', 'research');
    }
    expect(
      within(phaseItem('write')).getByRole('group', { name: /Waits for Research's gate$/ })
    ).toBeInTheDocument();
    expect(within(phaseItem('research')).queryByText(/Waits for/)).toBeNull();
  });

  it('says the person passed or skipped a phase they closed, and that a decision stands while it waits', () => {
    const { rerender } = draw(
      data(
        [],
        gatedPlan(
          { ...GATE, blocked: false, needsPerson: false, personDecision: 'pass', holds: [] },
          { status: 'completed' }
        )
      ),
      { onOpenGate: () => {} }
    );
    const redraw = (gate: Record<string, unknown>, status: string) =>
      rerender(
        <SummarySubagents
          data={data([], gatedPlan(gate, { status }))}
          loading={false}
          error={false}
          refresh={() => {}}
          onOpen={() => {}}
          onOpenGate={() => {}}
        />
      );
    expect(screen.getByTestId('phase-gate-research')).toHaveTextContent('Passed');
    expect(screen.getByTestId('phase-gate-research')).toHaveAccessibleName(
      'Passed by you, Research'
    );
    expect(screen.getByTestId('phase-gate-research')).toHaveAttribute('data-state', 'passed');
    redraw({ ...GATE, blocked: false, needsPerson: false, personDecision: 'skip' }, 'skipped');
    expect(screen.getByTestId('phase-gate-research')).toHaveTextContent('Skipped');
    expect(screen.getByTestId('phase-gate-research')).toHaveAccessibleName(
      'Skipped by you, Research'
    );
    expect(screen.getByTestId('phase-gate-research')).toHaveAttribute('data-state', 'skipped');
    // Recorded while its subagents still work: the pass stands, it has not closed the phase.
    redraw({ ...GATE, personDecision: 'pass' }, 'in_progress');
    expect(screen.getByTestId('phase-gate-research')).toHaveAccessibleName(
      'Your pass stands until its subagents finish, Research'
    );
    expect(screen.getByTestId('phase-gate-research')).not.toHaveAttribute('data-needs-person');
  });

  it('still draws a gate an older daemon describes, which names no owner, holder or decision', () => {
    draw(
      data([], {
        title: 'Plan',
        goal: null,
        revision: 1,
        phases: [
          olderPhase({
            id: 'verify',
            title: 'Check the numbers',
            status: 'in_progress',
            gate: {
              threshold: 0.8,
              questionCount: 2,
              attempts: 0,
              maxAttempts: 3,
              lastPassed: null,
              lastMinP: null,
              blocked: false,
            },
          }),
          olderPhase({ id: 'report', title: 'Report' }),
        ],
      }),
      { onOpenGate: () => {} }
    );
    expect(screen.getByTestId('phase-gate-verify')).toHaveTextContent('Gate');
    expect(screen.getByTestId('phase-gate-verify')).toHaveAccessibleName(
      'Gate not asked yet · threshold 0.80, Check the numbers'
    );
    expect(screen.getByTestId('phase-gate-verify')).toHaveAttribute('data-state', 'pending');
    // No holder named: the next phase waits for nothing it can say.
    expect(within(phaseItem('report')).queryByText(/Waits for/)).toBeNull();
    expect(phaseItem('report')).not.toHaveAttribute('data-held-by');
  });

  it('offers a quiet Add gate on an open phase with no gate, shown on hover and on focus', () => {
    const onOpenGate = vi.fn();
    draw(
      data([], {
        title: 'Plan',
        goal: null,
        revision: 1,
        phases: [
          wirePhase({ id: 'done', title: 'Done already', status: 'completed' }),
          wirePhase({ id: 'draft', title: 'Draft', status: 'in_progress' }),
          wirePhase({ id: 'ship', title: 'Ship' }),
        ],
      }),
      { onOpenGate }
    );
    // A closed phase has nothing to gate.
    expect(screen.queryByTestId('phase-add-gate-done')).toBeNull();
    const add = screen.getByTestId('phase-add-gate-draft');
    expect(add).toHaveTextContent('Add gate');
    expect(add).toHaveAccessibleName('Add gate, Draft');
    // In the chip's slot on the phase's line, with no inline style.
    expect(add.closest('.br-agents__phase-line')).not.toBeNull();
    expect(add).not.toHaveAttribute('style');
    // Quiet at rest; the phase's hover shows it, and leaving hides it again.
    expect(add).toHaveStyle({ opacity: '0' });
    fireEvent.mouseEnter(phaseItem('draft'));
    expect(add).toHaveStyle({ opacity: '1' });
    fireEvent.mouseLeave(phaseItem('draft'));
    expect(add).toHaveStyle({ opacity: '0' });
    // Its own focus shows it too: the keyboard finds it where the pointer does. In
    // a plan with nothing else to reach, it holds the area's one Tab stop.
    const ship = screen.getByTestId('phase-add-gate-ship');
    expect([add.tabIndex, ship.tabIndex]).toEqual([0, -1]);
    act(() => add.focus());
    expect(add).toHaveStyle({ opacity: '1' });
    fireEvent.keyDown(add, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(ship);
    expect(ship).toHaveStyle({ opacity: '1' });
    expect(add).toHaveStyle({ opacity: '0' });
    fireEvent.click(ship);
    expect(onOpenGate).toHaveBeenCalledWith('ship');
  });

  it('offers no Add gate where the chip is not a control', () => {
    draw(
      data([], { title: 'Plan', goal: null, revision: 1, phases: [wirePhase({ id: 'draft' })] })
    );
    expect(screen.queryByTestId('phase-add-gate-draft')).toBeNull();
  });
});
