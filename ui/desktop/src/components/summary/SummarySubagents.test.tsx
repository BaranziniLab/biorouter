import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import {
  inSpawnOrder,
  parseSessionSubagents,
  type SessionSubagents,
} from '../../utils/sessionSubagents';
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
  it('leads with the square and the role or name, the run time, then the model and what it is doing', () => {
    draw(
      data([
        wireRow({ sessionId: 'a', role: 'Reviewer' }),
        wireRow({ sessionId: 'b', status: 'completed', elapsedSeconds: 3900, quietSeconds: null }),
      ])
    );
    const section = screen.getByRole('region', { name: copy.agents });
    expect(within(section).getByText(copy.agents)).toBeInTheDocument();
    // Heading counts: running first, then done of total.
    expect(within(section).getByText(`${copy.running(1)} · 1 of 2`)).toBeInTheDocument();
    expect(within(section).getByText(`1 of 2${copy.doneSuffix}`)).toHaveAttribute(
      'aria-live',
      'polite'
    );
    // Four or fewer: rows only, no grid.
    expect(section.querySelector('.br-agent-grid')).toBeNull();

    const first = screen.getByTestId('subagent-row-a');
    expect(first).toHaveAttribute('data-layout', 'comfortable');
    expect(first).toHaveAttribute('data-status', 'running');
    expect(first).toHaveAttribute('data-mark', 'running');
    // The square: a language model's mark, its running core breathing.
    const mark = first.querySelector('.br-agent-mark')!;
    expect(mark).toHaveAttribute('data-shape', 'square');
    expect(mark).toHaveAttribute('data-kind', 'running');
    expect(mark).toHaveAttribute('data-pulse', '');
    expect(mark).toHaveAttribute('width', '12');
    expect(first).toHaveAccessibleName('Running, Reviewer, claude-sonnet-4-5, 1m');
    expect(first).toHaveTextContent(/^Reviewer1mclaude-sonnet-4-5 · Working…$/);
    const second = screen.getByTestId('subagent-row-b');
    // An ended child with no result line says its status word.
    expect(second).toHaveAccessibleName(
      'Completed, Search PubMed for trials, claude-sonnet-4-5, 1h 5m'
    );
    expect(second.querySelector('.br-agents__meta')).toHaveTextContent(
      'claude-sonnet-4-5 · Completed'
    );
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
    expect(line('q')).toBe('claude-sonnet-4-5 · Waiting to start');
    expect(line('c')).toBe('claude-sonnet-4-5 · Stopping…');
    expect(line('r')).toBe('claude-sonnet-4-5 · Found 12 trials');
    // A child that ended early reads Stopped, never Incomplete, and waits for a look.
    expect(line('i')).toBe('claude-sonnet-4-5 · Stopped');
    expect(screen.getByTestId('subagent-row-i')).toHaveAccessibleName(
      'Stopped, Search PubMed for trials, claude-sonnet-4-5, 1m'
    );
    expect(screen.getByTestId('subagent-row-i')).toHaveAttribute('data-mark', 'waiting');
  });

  it('names the model and the run time on the row (owner, 2026-11-11), never the setup', () => {
    draw(data([wireRow({ sessionId: 'a' })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row.querySelector('[data-count]')).toBeNull();
    expect(row).toHaveTextContent('claude-sonnet-4-5');
    expect(row.querySelector('.br-agents__trailing')).toHaveTextContent('1m');
    expect(row).not.toHaveTextContent(/anthropic|developer|20250929/);
  });

  it('says a long-silent active child is quiet, never stalled, and marks it waiting', () => {
    draw(data([wireRow({ sessionId: 'a', quietSeconds: 240 })]));
    const row = screen.getByTestId('subagent-row-a');
    expect(row).toHaveAttribute('data-quiet', '');
    expect(row).toHaveAttribute('data-mark', 'waiting');
    expect(row).toHaveTextContent(copy.quietFor('4m'));
    expect(row).not.toHaveTextContent(/stall|dead|stuck/i);
    expect(row).toHaveAccessibleName(
      'Quiet for 4m, Search PubMed for trials, claude-sonnet-4-5, 1m'
    );
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
    expect(row).toHaveAccessibleName(
      'Running, Search PubMed for trials, claude-sonnet-4-5, 1m, private'
    );
    expect(row.querySelector('.br-agents__lock')).not.toBeNull();
  });

  it('draws a subagent on a decision model as a circle, and says so', () => {
    draw(
      data([
        wireRow({
          sessionId: 'd',
          role: 'Dose check',
          providerName: 'ollama_decision',
          modelName: 'tev1:0.8b',
          status: 'completed',
          quietSeconds: null,
        }),
      ])
    );
    const row = screen.getByTestId('subagent-row-d');
    expect(row.querySelector('.br-agent-mark')).toHaveAttribute('data-shape', 'circle');
    expect(row.querySelector('.br-agent-mark')).toHaveAttribute('data-kind', 'done');
    expect(row).toHaveAccessibleName(
      'Completed, Dose check, tev1:0.8b · this computer, 1m, decision model'
    );
  });
});

describe('five or more: a grid of squares, then the first five rows', () => {
  const rows = [
    ...many(3, 'running', 'run'),
    wireRow({ sessionId: 'blocked', status: 'blocked', name: 'Asks a question' }),
    wireRow({ sessionId: 'failed', status: 'failed', name: 'Broke' }),
    ...many(15, 'completed', 'done'),
  ];
  const cells = () =>
    within(screen.getByRole('list', { name: copy.gridOfChat }))
      .getAllByRole('button')
      .map((cell) => cell.getAttribute('data-testid')!.replace('subagent-cell-', ''));
  const listed = () =>
    within(screen.getByRole('list', { name: copy.rowsOfChat }))
      .getAllByTestId(/^subagent-row-/)
      .map((row) => row.getAttribute('data-testid')!.replace('subagent-row-', ''));

  it('draws one 10px square per agent in spawn order, each named in words', () => {
    draw(data(rows));
    expect(cells()).toEqual(inSpawnOrder(data(rows).subagents).map((row) => row.sessionId));
    expect(cells()).toHaveLength(20);
    const square = screen.getByTestId('subagent-cell-failed');
    expect(square).toHaveAccessibleName('Failed, Broke, claude-sonnet-4-5, 1m');
    expect(square.querySelector('.br-agent-mark')).toHaveAttribute('width', '10');
    expect(square.querySelector('.br-agent-mark')).toHaveAttribute('data-kind', 'failed');
    expect(
      screen.getByTestId('subagent-cell-blocked').querySelector('.br-agent-mark')
    ).toHaveAttribute('data-kind', 'waiting');
  });

  it('lists who needs a look first: waiting, failed, running, then the newest', () => {
    draw(data(rows));
    expect(listed()).toEqual(['blocked', 'failed', 'run-0', 'run-1', 'run-2']);
    expect(screen.getByTestId('subagent-row-run-0')).toHaveAttribute('data-layout', 'dense');
    // A dense row is one line: the run time trails, no second line.
    expect(screen.getByTestId('subagent-row-run-0').querySelector('.br-agents__meta')).toBeNull();
    expect(screen.getByTestId('subagent-row-run-0')).toHaveAccessibleName(
      'Running, run task 1, claude-sonnet-4-5, 1m'
    );
  });

  it('keeps a square where it is when its child finishes', () => {
    const { rerender } = draw(data(rows));
    const before = cells();
    rerender(
      <SummarySubagents
        data={data(
          rows.map((row) =>
            row.sessionId === 'run-1' ? { ...row, status: 'completed', quietSeconds: null } : row
          )
        )}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
      />
    );
    expect(cells()).toEqual(before);
    expect(listed()).not.toContain('run-1');
  });

  it('shows five rows, then Show N more, then Show fewer, and folds the section', () => {
    draw(data(rows));
    expect(listed()).toHaveLength(5);
    fireEvent.click(screen.getByRole('button', { name: copy.showMore(15) }));
    expect(listed()).toHaveLength(20);
    fireEvent.click(screen.getByRole('button', { name: copy.showFewer }));
    expect(listed()).toHaveLength(5);

    const toggle = screen.getByTestId('summary-agents-toggle');
    expect(toggle).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(toggle);
    expect(toggle).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('subagent-cell-run-0')).toBeNull();
    // The heading keeps its count while folded.
    expect(toggle).toHaveTextContent(`${copy.running(3)} · 15 of 20`);
  });

  it('keeps the card of a square as a row’s: hover peeks, click keeps', async () => {
    draw(data(rows));
    const square = screen.getByTestId('subagent-cell-failed');
    fireEvent.click(square);
    expect(square).toHaveAttribute('data-pinned', 'true');
    const card = await screen.findByTestId('subagent-card-failed');
    expect(card).toHaveTextContent('Broke');
    expect(card).toHaveTextContent(copy.peek.openInTab);
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

  it('draws each phase as one band: its title, its count, and no bar', () => {
    draw(data(rows, plan));
    const section = screen.getByRole('region', { name: copy.planLabel('Survey the trials') });
    // The plan's title is the section's name, not a line of its own.
    expect(within(section).queryByText('Survey the trials')).toBeNull();
    expect(within(section).getByText(copy.plan)).toBeInTheDocument();
    expect(within(section).getByText(`1 of 4${copy.phasesCountSuffix}`)).toBeInTheDocument();
    const list = within(section).getByRole('list', { name: copy.phaseListLabel });
    const items = within(list)
      .getAllByRole('listitem')
      .filter((li) => li.dataset.phaseId);
    expect(items.map((li) => li.dataset.phaseId)).toEqual(['plan', 'research', 'verify', 'meet']);
    expect(within(section).queryByRole('progressbar')).toBeNull();

    const research = items[1];
    // Its failures are its grid's crosses and words in its name, never a line under it.
    expect(within(research).queryByText('1 failed')).toBeNull();
    const toggle = within(research).getByRole('button', { name: /^Read the trials/ });
    expect(toggle).toHaveAccessibleName(
      'Read the trials, research phase, in progress, 1 of 3 subagents done, 1 failed'
    );
    expect(toggle).toHaveTextContent('1 of 3');
    expect(toggle.closest('.br-agents__phase-line')).not.toBeNull();
    // Five agents in the chat: the open phase draws its grid, in spawn order.
    const grid = within(research).getByRole('list', { name: copy.gridLabel('Read the trials') });
    expect(within(grid).getAllByRole('button')).toHaveLength(3);
  });

  it('shows no "0 of 0" for a phase that used no subagents (H-12), but where it stands', () => {
    draw(data(rows, plan));
    const verify = screen.getByRole('group', { name: /^Check the numbers/ });
    expect(verify).toHaveAccessibleName('Check the numbers, verification phase, pending');
    expect(verify).not.toHaveTextContent(/0 of 0/);
    expect(verify).toHaveTextContent(copy.phaseStatus.pending);
  });

  it('opens the running phase and folds the finished one in a plan of more than three', () => {
    draw(data(rows, plan));
    const planToggle = screen.getByRole('button', { name: /^Plan the search/ });
    const researchToggle = screen.getByRole('button', { name: /^Read the trials/ });
    expect(planToggle).toHaveAttribute('aria-expanded', 'false');
    expect(researchToggle).toHaveAttribute('aria-expanded', 'true');
    expect(screen.queryByTestId('subagent-row-p1')).toBeNull();
    // A folded phase whose every agent finished draws its line alone.
    expect(screen.queryByTestId('subagent-cell-p1')).toBeNull();
    expect(screen.getByTestId('subagent-row-r2')).toBeInTheDocument();
    fireEvent.click(planToggle);
    expect(screen.getByTestId('subagent-row-p1')).toBeInTheDocument();
    // A phase with no subagents is a plain line, not a control.
    expect(screen.queryByRole('button', { name: /^Check the numbers/ })).toBeNull();
  });

  it('keeps a folded phase’s grid while something in it is not finished', () => {
    draw(data(rows, plan));
    const researchToggle = screen.getByRole('button', { name: /^Read the trials/ });
    fireEvent.click(researchToggle);
    expect(researchToggle).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('subagent-row-r2')).toBeNull();
    expect(screen.getByTestId('subagent-cell-r2')).toBeInTheDocument();
    expect(screen.getByTestId('subagent-cell-r3')).toBeInTheDocument();
  });

  it('lists a subagent in no phase under Other agents, inside the Plan section', () => {
    draw(data(rows, plan));
    const other = document.querySelector<HTMLElement>('[data-group="other"]')!;
    expect(within(other).getByText(copy.otherAgents)).toBeInTheDocument();
    expect(within(other).getByTestId('subagent-row-loose')).toBeInTheDocument();
    expect(screen.queryByRole('region', { name: copy.agents })).toBeNull();
  });

  it('draws the phases of a plan that has no subagents yet', () => {
    draw(data([], plan));
    expect(
      screen.getByRole('region', { name: copy.planLabel('Survey the trials') })
    ).toBeInTheDocument();
    expect(document.querySelector('[data-group="other"]')).toBeNull();
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

  it('keeps a kept peek, and its row, through a poll that ranks the child out of the five', async () => {
    const running = many(6, 'running', 'run');
    const { rerender } = draw(data(running));
    const before = screen.getByTestId('subagent-row-run-2');
    // A pointer's click (detail 1): it keeps the card and leaves focus where it is.
    fireEvent.click(before, { detail: 1 });
    expect(await screen.findByTestId('subagent-card-run-2')).toBeInTheDocument();

    // The child finishes, which ranks it after the five still running. While its
    // peek is kept the row stays drawn, so the poll does not remount it and the
    // peek stays open.
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    const after = screen.getByTestId('subagent-row-run-2');
    expect(after).toBe(before);
    expect(screen.getByTestId('subagent-card-run-2')).toBeInTheDocument();
    expect(after).toHaveAttribute('data-status', 'completed');

    // Let go, it leaves the first five for Show more.
    fireEvent.click(after, { detail: 1 });
    expect(screen.queryByTestId('subagent-row-run-2')).toBeNull();
    expect(screen.getByRole('button', { name: copy.showMore(1) })).toBeInTheDocument();
  });

  it('keeps focus on a row whose child finishes while it holds focus', () => {
    const running = many(6, 'running', 'run');
    const { rerender } = draw(data(running));
    const before = screen.getByTestId('subagent-row-run-2');
    act(() => before.focus());
    const finished = running.map((row, i) =>
      i === 2 ? { ...row, status: 'completed', quietSeconds: null } : row
    );
    act(() => redraw(rerender, data(finished)));
    const after = screen.getByTestId('subagent-row-run-2');
    expect(after).toHaveAttribute('data-status', 'completed');
    expect(document.activeElement).toBe(after);
    expect(after.tabIndex).toBe(0);
  });

  it('keeps focus on a row when a fifth spawn turns the rows into a grid', () => {
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

  it('reaches the grid and Show more from the arrows, and Left and Right walk the squares', () => {
    draw(data([...many(3, 'running', 'run'), ...many(13, 'completed', 'done')]));
    const first = screen.getByTestId('subagent-row-run-0');
    // The tab stop starts on the first running row.
    expect(first.tabIndex).toBe(0);
    act(() => first.focus());
    // Up leaves the rows for the grid: ONE stop, on its current square.
    fireEvent.keyDown(first, { key: 'ArrowUp' });
    const cells = screen.getAllByTestId(/^subagent-cell-/);
    expect(document.activeElement).toBe(cells[0]);
    expect(cells.filter((cell) => cell.hasAttribute('data-row'))).toEqual([cells[0]]);
    fireEvent.keyDown(cells[0], { key: 'ArrowRight' });
    expect(document.activeElement).toBe(cells[1]);
    expect(cells[1]).toHaveAttribute('data-row');
    expect(cells[0]).not.toHaveAttribute('data-row');
    fireEvent.keyDown(cells[1], { key: 'ArrowLeft' });
    expect(document.activeElement).toBe(cells[0]);
    // A move with the arrows opens no card.
    expect(screen.queryByTestId(/^subagent-card-/)).toBeNull();
    fireEvent.keyDown(cells[0], { key: 'ArrowDown' });
    expect(document.activeElement).toBe(first);
    fireEvent.keyDown(first, { key: 'End' });
    expect(document.activeElement).toHaveTextContent(copy.showMore(11));
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
    // Decision-model work: a circle, half full while it waits for the person.
    expect(chip.querySelector('.br-agent-mark')).toHaveAttribute('data-shape', 'circle');
    expect(chip.querySelector('.br-agent-mark')).toHaveAttribute('data-kind', 'waiting');
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
    // A gate that wants no look is its circle alone; the words are its name.
    expect(screen.getByTestId('phase-gate-research')).toHaveTextContent(/^$/);
    expect(
      screen.getByTestId('phase-gate-research').querySelector('.br-agent-mark')
    ).toHaveAttribute('data-kind', 'done');
    expect(screen.getByTestId('phase-gate-research')).toHaveAccessibleName(
      'Passed by you, Research'
    );
    expect(screen.getByTestId('phase-gate-research')).toHaveAttribute('data-state', 'passed');
    redraw({ ...GATE, blocked: false, needsPerson: false, personDecision: 'skip' }, 'skipped');
    expect(
      screen.getByTestId('phase-gate-research').querySelector('.br-agent-mark')
    ).toHaveAttribute('data-kind', 'idle');
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
    expect(screen.getByTestId('phase-gate-verify').querySelector('.br-agent-mark')).toHaveAttribute(
      'data-kind',
      'queued'
    );
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
    // A compact plus: its words are its name (and its tooltip), never the title's room.
    expect(add).toHaveTextContent(/^$/);
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
