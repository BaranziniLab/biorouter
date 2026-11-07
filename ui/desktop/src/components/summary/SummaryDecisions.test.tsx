import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import {
  parseDecisionSettings,
  parsePhaseGateSnapshot,
  parseSessionDecisionSummary,
  type DecisionSettings,
  type SessionDecisionSummary,
} from '../../utils/sessionDecisions';
import { parseSessionSubagents } from '../../utils/sessionSubagents';
import { wirePhase, wireRow, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { SummaryDecisions, type SummaryDecisionsData } from './SummaryDecisions';
import { SummarySubagents } from './SummarySubagents';
import { PhaseGateDialog } from '../decisions/PhaseGateDialog';

const settings = (monitors: Record<string, unknown>[] = []): DecisionSettings =>
  parseDecisionSettings({ defaultModel: null, monitors })!;

const monitor = (overrides: Record<string, unknown> = {}) => ({
  id: 'form',
  name: 'Form reached',
  question: { instructions: 'Has the agent reached the consent form?', kind: { binary: {} } },
  threshold: 0.85,
  action: 'stop',
  ...overrides,
});

/** 5,000 calls: 4,800 over 50 decide calls, 180 from one monitor, 20 from a phase gate. */
function heavySummary(): SessionDecisionSummary {
  return parseSessionDecisionSummary({
    total: 5000,
    aggregates: [
      ...Array.from({ length: 50 }, (_, i) => ({
        origin: 'tool',
        originId: `call-${i}`,
        label: 'decide',
        count: 96,
        passed: 40,
        failed: 56,
        errors: i === 49 ? 2 : 0,
        lastAt: `2026-11-04T10:${String(i).padStart(2, '0')}:00Z`,
        lastAnswer: 'yes',
        lastPassProbability: 0.93,
      })),
      {
        origin: 'monitor',
        originId: 'form',
        label: 'Form reached',
        count: 180,
        passed: 0,
        failed: 180,
        errors: 0,
        lastAt: '2026-11-04T11:00:00Z',
        lastAnswer: 'no',
        lastPassProbability: 0.31,
      },
      {
        origin: 'gate',
        originId: 'verify',
        label: 'Check the numbers',
        count: 20,
        passed: 1,
        failed: 19,
        errors: 0,
        lastAt: '2026-11-04T11:05:00Z',
        lastAnswer: 'no',
        lastPassProbability: 0.62,
      },
    ],
    recentTurns: [],
  })!;
}

function draw(overrides: Partial<SummaryDecisionsData> = {}) {
  const onOpenHistory = vi.fn();
  const onOpenMonitors = vi.fn();
  const refresh = vi.fn();
  const utils = render(
    <SummaryDecisions
      sessionId="s1"
      data={{ summary: heavySummary(), settings: settings([monitor()]) }}
      loading={false}
      error={false}
      refresh={refresh}
      onOpenHistory={onOpenHistory}
      onOpenMonitors={onOpenMonitors}
      {...overrides}
    />
  );
  return { ...utils, onOpenHistory, onOpenMonitors, refresh };
}

describe('the Decisions section', () => {
  it('draws nothing while the chat has no decision calls and no monitors', () => {
    const { container } = draw({
      data: { summary: parseSessionDecisionSummary({ total: 0 })!, settings: settings() },
    });
    expect(container).toBeEmptyDOMElement();
    const { container: none } = draw({ data: null });
    expect(none).toBeEmptyDOMElement();
  });

  it('shows a chat whose only decision activity is a monitor nothing has checked yet', () => {
    draw({
      data: {
        summary: parseSessionDecisionSummary({ total: 0 })!,
        settings: settings([monitor()]),
      },
    });
    const row = screen.getByTestId('decision-source-monitor:form');
    expect(row).toHaveAttribute('data-state', 'armed');
    expect(row).toHaveTextContent('Not checked yet');
  });

  it('aggregates 5,000 calls into one row per source, never one per call', () => {
    draw({
      gates: [
        {
          phaseId: 'verify',
          title: 'Check the numbers',
          gate: parsePhaseGateSnapshot({
            threshold: 0.8,
            attempts: 2,
            maxAttempts: 3,
            lastPassed: false,
            lastMinP: 0.62,
            blocked: false,
          })!,
        },
      ],
    });
    const section = screen.getByTestId('summary-decisions');
    const rows = within(section).getAllByRole('listitem');
    expect(rows).toHaveLength(3);
    expect(section.querySelectorAll('.br-decisions__row')).toHaveLength(3);
    expect(within(section).getByText('5,000')).toBeInTheDocument();
    const decide = screen.getByTestId('decision-source-tool');
    expect(decide).toHaveTextContent('decide');
    expect(decide).toHaveTextContent('4,800');
    expect(decide).toHaveTextContent('last yes · P 0.93 · 2 errors');
    const gate = screen.getByTestId('decision-source-gate:verify');
    expect(gate).toHaveAttribute('data-state', 'failed');
    expect(gate).toHaveTextContent('0.62 < 0.80 · attempt 2 of 3');
    // The monitor's state is a shape and a word, its bar marks its threshold.
    const form = screen.getByTestId('decision-source-monitor:form');
    expect(form).toHaveAttribute('data-state', 'armed');
    expect(form.getAttribute('aria-label')).toMatch(/^Watching, Form reached, 180 calls/);
    const mark = form.querySelector<HTMLElement>('.br-pbar__mark');
    expect(mark?.style.left).toBe('85%');
    expect(form.querySelector<HTMLElement>('.br-pbar__fill')?.style.width).toBe('31%');
  });

  it('says a gate blocked by evaluation errors could not be evaluated, not that it ran out of attempts', () => {
    const gate = (phaseId: string, attempts: number, lastPassed: boolean | null) => ({
      phaseId,
      title: phaseId,
      gate: parsePhaseGateSnapshot({
        threshold: 0.8,
        attempts,
        maxAttempts: 3,
        lastPassed,
        lastMinP: lastPassed === null ? null : 0.4,
        blocked: true,
      })!,
    });
    draw({ gates: [gate('errors', 0, null), gate('one', 1, false), gate('spent', 3, false)] });
    // Two errors in a row block a gate and use no attempt.
    const errors = screen.getByTestId('decision-source-gate:errors');
    expect(errors).toHaveAttribute('data-state', 'blocked');
    expect(errors).toHaveTextContent('blocked: could not be evaluated');
    expect(errors).not.toHaveTextContent('attempts');
    expect(screen.getByTestId('decision-source-gate:one')).toHaveTextContent(
      'blocked: could not be evaluated'
    );
    // A gate that used every attempt says so.
    expect(screen.getByTestId('decision-source-gate:spent')).toHaveTextContent(
      'blocked after 3 attempts'
    );
  });

  it('opens the decision history filtered to the row it came from', () => {
    const { onOpenHistory } = draw();
    fireEvent.click(screen.getByTestId('decision-source-monitor:form'));
    expect(onOpenHistory).toHaveBeenLastCalledWith({ origin: 'monitor', originId: 'form' });
    fireEvent.click(screen.getByTestId('decision-source-tool'));
    expect(onOpenHistory).toHaveBeenLastCalledWith({ origin: 'tool', originId: null });
  });

  it('opens the Monitors dialog from a plain button, and says how many are on', () => {
    const { onOpenMonitors } = draw({
      data: {
        summary: heavySummary(),
        settings: settings([monitor(), monitor({ id: 'idle', name: 'Idle', enabled: false })]),
      },
    });
    const button = screen.getByTestId('summary-decisions-monitors');
    expect(button.tagName).toBe('BUTTON');
    expect(button).toHaveTextContent('1 on');
    fireEvent.click(button);
    expect(onOpenMonitors).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('decision-source-monitor:idle')).toHaveAttribute('data-state', 'off');
  });

  it('marks a monitor the newest turn marker says fired, and offers Retry after a failed read', () => {
    const { refresh } = draw({ fired: new Set(['form']), error: true });
    expect(screen.getByTestId('decision-source-monitor:form')).toHaveAttribute(
      'data-state',
      'fired'
    );
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(refresh).toHaveBeenCalled();
  });
});

describe('a gate row and the person', () => {
  const personGate = (
    personDecision: 'pass' | 'skip' | null,
    status: 'completed' | 'skipped' | 'in_progress'
  ) => ({
    phaseId: 'verify',
    title: 'Check the numbers',
    status,
    gate: parsePhaseGateSnapshot({
      threshold: 0.8,
      attempts: 3,
      maxAttempts: 3,
      lastPassed: false,
      lastMinP: 0.62,
      blocked: false,
      personDecision,
    })!,
  });

  it('says the person passed or skipped the phase, before what the gate itself last said', () => {
    const { rerender } = draw({ gates: [personGate('pass', 'completed')] });
    const row = () => screen.getByTestId('decision-source-gate:verify');
    expect(row()).toHaveTextContent('passed by you · 0.62 < 0.80 · attempt 3 of 3');
    expect(row().getAttribute('aria-label')).toMatch(/, passed by you · 0\.62 < 0\.80/);
    const redraw = (gates: ReturnType<typeof personGate>[]) =>
      rerender(
        <SummaryDecisions
          sessionId="s1"
          data={{ summary: heavySummary(), settings: settings([monitor()]) }}
          loading={false}
          error={false}
          refresh={() => {}}
          onOpenHistory={() => {}}
          onOpenMonitors={() => {}}
          gates={gates}
        />
      );
    redraw([personGate('skip', 'skipped')]);
    expect(row()).toHaveTextContent('skipped by you');
    // Recorded while the phase's subagents still work: it stands.
    redraw([personGate('pass', 'in_progress')]);
    expect(row()).toHaveTextContent('your pass stands');
    redraw([personGate(null, 'in_progress')]);
    expect(row()).not.toHaveTextContent(/by you|stands/);
    expect(row()).toHaveTextContent('0.62 < 0.80 · attempt 3 of 3');
  });
});

describe('a phase gate on its phase row', () => {
  it('draws a chip that opens the gate’s dialog, in words and by shape', () => {
    const onOpenGate = vi.fn();
    const data = parseSessionSubagents(
      wireSubagents([wireRow({ sessionId: 'c1', phaseId: 'verify' })], {
        title: 'Plan',
        goal: null,
        revision: 1,
        phases: [
          wirePhase({
            id: 'verify',
            title: 'Check the numbers',
            kind: 'verification',
            status: 'in_progress',
            agentIds: ['c1'],
            gate: {
              threshold: 0.8,
              questionCount: 2,
              attempts: 2,
              maxAttempts: 3,
              lastPassed: false,
              lastMinP: 0.62,
              blocked: false,
            },
          }),
        ],
      })
    );
    render(
      <SummarySubagents
        data={data}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
        onOpenGate={onOpenGate}
      />
    );
    const chip = screen.getByTestId('phase-gate-verify');
    expect(chip).toHaveTextContent('Gate 0.62 < 0.80, attempt 2 of 3');
    expect(chip).toHaveAttribute('data-state', 'failed');
    fireEvent.click(chip);
    expect(onOpenGate).toHaveBeenCalledWith('verify');
  });

  it('tolerates a phase with no gate (a daemon before decision gates)', () => {
    const data = parseSessionSubagents(
      wireSubagents([wireRow({ sessionId: 'c1', phaseId: 'research' })], {
        title: 'Plan',
        goal: null,
        revision: 1,
        phases: [wirePhase({ agentIds: ['c1'], status: 'in_progress' })],
      })
    );
    expect(data!.plan!.phases[0].gate).toBeNull();
    render(
      <SummarySubagents
        data={data}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpen={() => {}}
        onOpenGate={() => {}}
      />
    );
    // No chip; the open phase offers the person "Add gate" in its place.
    expect(document.querySelector('.br-agents__gate:not([data-add-gate])')).toBeNull();
    expect(screen.queryByTestId('phase-gate-research')).toBeNull();
    expect(screen.getByTestId('phase-add-gate-research')).toHaveAttribute('data-add-gate');
  });
});

/**
 * The chip and "Add gate" open the gate's dialog, wired as BaseChat wires them
 * (`openPhaseGate` sets the phase, the dialog reads it from the roster). The
 * dialog holds the gate's history behind Show history.
 */
describe('the chip opens the gate dialog', () => {
  const BASECHAT = readFileSync(join(__dirname, '..', 'BaseChat.tsx'), 'utf8');

  function Harness({ value }: { value: ReturnType<typeof parseSessionSubagents> }) {
    const [phaseId, setPhaseId] = useState<string | null>(null);
    const phase = value?.plan?.phases.find((candidate) => candidate.id === phaseId) ?? null;
    return (
      <>
        <SummarySubagents
          data={value}
          loading={false}
          error={false}
          refresh={() => {}}
          onOpen={() => {}}
          onOpenGate={setPhaseId}
        />
        <PhaseGateDialog
          open={phaseId !== null}
          onOpenChange={(open) => !open && setPhaseId(null)}
          sessionId="s1"
          phase={phase}
          phases={value?.plan?.phases}
          models={[]}
        />
      </>
    );
  }

  it('opens the dialog on the gate the chip names, and on the editor from Add gate', () => {
    const value = parseSessionSubagents(
      wireSubagents([], {
        title: 'Plan',
        goal: null,
        revision: 1,
        phases: [
          wirePhase({
            id: 'research',
            title: 'Research',
            status: 'in_progress',
            gate: {
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
              holds: ['write'],
            },
          }),
          wirePhase({ id: 'write', title: 'Write', heldBy: 'research' }),
        ],
      })
    );
    render(<Harness value={value} />);
    fireEvent.click(screen.getByTestId('phase-gate-research'));
    const dialog = screen.getByTestId('phase-gate-dialog');
    expect(within(dialog).getByTestId('phase-gate-owner')).toHaveTextContent('Set by the agent');
    expect(within(dialog).getByTestId('phase-gate-status')).toHaveTextContent('Blocked: needs you');
    expect(within(dialog).getByTestId('phase-gate-history')).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole('button', { name: 'Close' }));
    expect(screen.queryByTestId('phase-gate-dialog')).toBeNull();

    fireEvent.click(screen.getByTestId('phase-add-gate-write'));
    expect(screen.getByRole('dialog', { name: 'Add a phase gate' })).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-editor')).toBeInTheDocument();
  });

  it('is how BaseChat wires it: the chip opens the dialog, and history lives inside it', () => {
    // openPhaseGate opens the dialog; it no longer opens the decision history itself.
    const open =
      BASECHAT.match(/const openPhaseGate = useCallback\(([\s\S]*?)\}, \[\]\);/)?.[1] ?? '';
    expect(open).toContain('setGatePhaseId(phaseId)');
    expect(open).not.toContain('openDecisionHistory');
    expect(BASECHAT).toMatch(/onOpenGate: openPhaseGate/);
    const dialog = BASECHAT.match(/<PhaseGateDialog[\s\S]*?\/>/)?.[0] ?? '';
    expect(dialog).toContain('open={gatePhaseId !== null}');
    expect(dialog).toContain('onOpenHistory={openDecisionHistory}');
  });

  it('tells the Monitors dialog when the chat is a subagent’s (item A)', () => {
    // A subagent's own monitors never reach subagents of its own, so the dialog
    // must know before the chat has an agent's monitor it could read that from.
    const monitors = BASECHAT.match(/<MonitorsDialog[\s\S]*?\n {10}\/>/)?.[0] ?? '';
    expect(monitors).toContain("isSubagentChat={subagentChatKind === 'subagent'}");
  });
});

describe('the section keeps the summary’s rules', () => {
  const CSS = readFileSync(join(__dirname, 'summaryDecisions.css'), 'utf8').replace(
    /\/\*[\s\S]*?\*\//g,
    ''
  );
  const TSX = readFileSync(join(__dirname, 'SummaryDecisions.tsx'), 'utf8');

  it('declares only the secondary (13) and supporting (12) sizes', () => {
    const sizes = [...CSS.matchAll(/font-size:\s*([^;]+);/g)].map((match) => match[1].trim());
    expect(sizes.length).toBeGreaterThan(0);
    for (const size of sizes) {
      expect(['var(--text-secondary)', 'var(--text-supporting)'], size).toContain(size);
    }
  });

  it('draws no line: every border it names is a reset', () => {
    const borders = [...CSS.matchAll(/border(?:-top|-bottom|-left|-right)?\s*:\s*([^;]+);/g)];
    for (const [, value] of borders) expect(['0', 'none'], value).toContain(value.trim());
  });

  it('uses no Button primitive, so the summary card keeps its three', () => {
    expect(TSX).not.toMatch(/<Button\b/);
    expect(TSX).not.toMatch(/from '\.\.\/ui\/button'/);
  });
});
