import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { fireEvent, render, screen, within } from '@testing-library/react';
import { useState } from 'react';
import { describe, expect, it, vi } from 'vitest';
import {
  parseDecisionSettings,
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

const toggle = () => screen.getByTestId('summary-decisions-toggle');
const openSection = () => {
  if (toggle().getAttribute('aria-expanded') !== 'true') fireEvent.click(toggle());
};

/** An aggregate for a call that asked several monitors at once (`a+b`). */
const combined = (originId: string, count: number, errors = 0) => ({
  origin: 'monitor',
  originId,
  label: originId,
  count,
  passed: count,
  failed: 0,
  errors,
  lastAt: '2026-11-04T12:00:00Z',
  lastAnswer: 'yes',
  lastPassProbability: 0.99,
});

const one = (originId: string, count: number, p: number | null, errors = 0) => ({
  origin: 'monitor',
  originId,
  label: originId,
  count,
  passed: 0,
  failed: count,
  errors,
  lastAt: '2026-11-04T11:00:00Z',
  lastAnswer: p === null ? null : 'no',
  lastPassProbability: p,
});

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
    expect(toggle()).toHaveTextContent('None fired');
    openSection();
    const row = screen.getByTestId('decision-source-monitor:form');
    expect(row).toHaveAttribute('data-state', 'pending');
    expect(row).toHaveTextContent('Not checked yet');
  });

  it('aggregates 5,000 calls into one row per source, never one per call, and no gate rows', () => {
    draw();
    openSection();
    const section = screen.getByTestId('summary-decisions');
    const rows = within(section).getAllByRole('listitem');
    // The monitor and every decide batch together; the gate is its phase's chip (F-1).
    expect(rows).toHaveLength(2);
    expect(screen.queryByTestId('decision-source-gate:verify')).toBeNull();
    const decide = screen.getByTestId('decision-source-tool');
    expect(decide).toHaveTextContent('Agent checks');
    expect(decide).toHaveTextContent('4,800');
    expect(decide.getAttribute('aria-label')).toBe('Agent checks, 4,800 checks, 2 errors');
    const form = screen.getByTestId('decision-source-monitor:form');
    expect(form).toHaveAttribute('data-state', 'quiet');
    expect(form).toHaveTextContent('Quiet');
    expect(form.getAttribute('aria-label')).toBe('Quiet, Form reached, checked 180 times');
  });

  /** R1 and R2: no passed or failed for a monitor, and no probability beside an answer. */
  it('never says Passed, Failed or a probability, and draws no bar', () => {
    const { container } = draw({ fired: new Set(['form']) });
    openSection();
    const text = container.textContent ?? '';
    expect(text).not.toMatch(/\bPassed\b|\bFailed\b|\bP \d|P\(/);
    expect(container.querySelector('.br-pbar, [role="meter"]')).toBeNull();
    for (const row of container.querySelectorAll('.br-decisions__row')) {
      expect(row.getAttribute('aria-label') ?? '').not.toMatch(/passed|failed/i);
    }
  });

  it('marks a quiet monitor with an open eye, never a cross', () => {
    draw();
    openSection();
    const glyph = screen
      .getByTestId('decision-source-monitor:form')
      .querySelector('.br-decisions__glyph svg');
    expect(glyph?.getAttribute('class') ?? '').toMatch(/eye/);
    expect(glyph?.getAttribute('class') ?? '').not.toMatch(/\bx\b|lucide-x/);
  });

  it('opens the decision history filtered to the row it came from', () => {
    const { onOpenHistory } = draw();
    openSection();
    fireEvent.click(screen.getByTestId('decision-source-monitor:form'));
    expect(onOpenHistory).toHaveBeenLastCalledWith({ origin: 'monitor', originId: 'form' });
    fireEvent.click(screen.getByTestId('decision-source-tool'));
    expect(onOpenHistory).toHaveBeenLastCalledWith({ origin: 'tool', originId: null });
  });

  it('opens the Monitors dialog from a quiet icon button named Monitors, open or closed', () => {
    const { onOpenMonitors } = draw({
      data: {
        summary: heavySummary(),
        settings: settings([monitor(), monitor({ id: 'idle', name: 'Idle', enabled: false })]),
      },
    });
    const button = screen.getByRole('button', { name: 'Monitors' });
    expect(button).toBe(screen.getByTestId('summary-decisions-monitors'));
    expect(button).toHaveAttribute('aria-haspopup', 'dialog');
    expect(button).toHaveTextContent('');
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
    fireEvent.click(button);
    expect(onOpenMonitors).toHaveBeenCalledTimes(1);
    openSection();
    expect(screen.getByTestId('decision-source-monitor:idle')).toHaveAttribute('data-state', 'off');
    expect(screen.getByTestId('decision-source-monitor:idle')).toHaveTextContent('Off');
  });

  it('draws no Monitors button in a decision chat', () => {
    draw({ decisionChat: true });
    expect(screen.queryByRole('button', { name: 'Monitors' })).toBeNull();
  });

  it('marks a monitor the newest turn marker says fired, and offers Retry after a failed read', () => {
    const { refresh } = draw({ fired: new Set(['form']), error: true });
    // A fire opens the section on its own.
    expect(toggle()).toHaveAttribute('aria-expanded', 'true');
    expect(toggle()).toHaveTextContent('1 fired');
    const row = screen.getByTestId('decision-source-monitor:form');
    expect(row).toHaveAttribute('data-state', 'fired');
    expect(row).toHaveTextContent('Fired');
    fireEvent.click(screen.getByRole('button', { name: 'Retry' }));
    expect(refresh).toHaveBeenCalled();
  });
});

describe('the closed line and when it opens (F-1)', () => {
  const summary = (aggregates: Record<string, unknown>[], total = 10) =>
    parseSessionDecisionSummary({ total, aggregates, recentTurns: [] })!;

  // R2-6: a gate's checks are its phase chip's, so a chat whose only decisions
  // were a gate's read "Decisions  0 checks" beside "Gate checked once, passed".
  it('draws nothing when the only checks were a phase gate’s and there are no monitors', () => {
    const gateOnly = summary(
      [
        {
          origin: 'gate',
          originId: 'collect',
          label: 'Collect',
          count: 1,
          passed: 1,
          failed: 0,
          errors: 0,
          lastAt: '2026-11-10T10:00:00Z',
          lastAnswer: 'yes',
          lastPassProbability: 0.97,
        },
      ],
      1
    );
    draw({ data: { summary: gateOnly, settings: settings([]) } });
    expect(screen.queryByTestId('summary-decisions')).toBeNull();
    expect(screen.queryByText(/0 checks/)).toBeNull();
  });

  it('is closed by default when nothing fired and nothing errored', () => {
    draw({ data: { summary: summary([one('form', 4, 0.2)]), settings: settings([monitor()]) } });
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
    expect(toggle()).toHaveTextContent('None fired');
    expect(screen.queryByRole('list', { name: 'Decision sources' })).toBeNull();
  });

  it('opens by default on an error, and says so', () => {
    draw({
      data: { summary: summary([one('form', 4, null, 4)]), settings: settings([monitor()]) },
    });
    expect(toggle()).toHaveAttribute('aria-expanded', 'true');
    expect(toggle()).toHaveTextContent('1 error');
    expect(screen.getByTestId('decision-source-monitor:form')).toHaveTextContent('Error');
  });

  it('names both a fire and an error', () => {
    draw({
      fired: new Set(['form']),
      data: {
        summary: summary([one('form', 4, 0.9), one('idle', 2, null, 2)]),
        settings: settings([monitor(), monitor({ id: 'idle', name: 'Idle' })]),
      },
    });
    expect(toggle()).toHaveTextContent('1 fired · 1 error');
  });

  it('counts the checks when there are no monitors', () => {
    draw({
      data: {
        summary: summary([
          {
            origin: 'tool',
            originId: null,
            label: 'decide',
            count: 10,
            passed: 5,
            failed: 5,
            errors: 0,
            lastAt: '2026-11-04T10:00:00Z',
            lastAnswer: 'yes',
            lastPassProbability: 0.9,
          },
        ]),
        settings: settings(),
      },
    });
    expect(toggle()).toHaveTextContent('10 checks');
  });

  it('keeps a close the person made after a later fire', () => {
    const data = { summary: summary([one('form', 4, 0.2)]), settings: settings([monitor()]) };
    const { rerender } = draw({ data });
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-expanded', 'true');
    fireEvent.click(toggle());
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
    rerender(
      <SummaryDecisions
        sessionId="s1"
        data={data}
        loading={false}
        error={false}
        refresh={() => {}}
        onOpenHistory={() => {}}
        onOpenMonitors={() => {}}
        fired={new Set(['form'])}
      />
    );
    expect(toggle()).toHaveTextContent('1 fired');
    expect(toggle()).toHaveAttribute('aria-expanded', 'false');
  });
});

/** A-1 (major): one row per monitor that exists now, a combined call counted on each. */
describe('monitor rows (A-1)', () => {
  it('counts a call that asked A and B on both rows, and never draws an "A + B" row', () => {
    const monitors = settings([
      monitor({ id: 'a', name: 'Alpha' }),
      monitor({ id: 'b', name: 'Beta' }),
      monitor({ id: 'c', name: 'Gamma' }),
    ]);
    const { container } = draw({
      data: {
        summary: parseSessionDecisionSummary({
          total: 9,
          aggregates: [one('a', 2, 0.1), combined('a+b', 5), one('c', 2, 0.1)],
          recentTurns: [],
        })!,
        settings: monitors,
      },
    });
    openSection();
    const rows = [...container.querySelectorAll('.br-decisions__row')];
    expect(rows.map((row) => row.getAttribute('data-testid'))).toEqual([
      'decision-source-monitor:a',
      'decision-source-monitor:b',
      'decision-source-monitor:c',
    ]);
    expect(container.textContent).not.toMatch(/\+/);
    expect(screen.getByTestId('decision-source-monitor:a').getAttribute('aria-label')).toBe(
      'Quiet, Alpha, checked 7 times'
    );
    expect(screen.getByTestId('decision-source-monitor:b').getAttribute('aria-label')).toBe(
      'Quiet, Beta, checked 5 times'
    );
  });

  it('draws no row for a monitor that was removed; its checks stay in the history', () => {
    draw({
      data: {
        summary: parseSessionDecisionSummary({
          total: 6,
          aggregates: [one('form', 2, 0.1), one('gone', 4, 0.1)],
          recentTurns: [],
        })!,
        settings: settings([monitor()]),
      },
    });
    openSection();
    expect(screen.queryByTestId('decision-source-monitor:gone')).toBeNull();
    expect(screen.getByTestId('decision-source-monitor:form')).toBeInTheDocument();
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
    expect(chip).toHaveTextContent('Not passed');
    expect(chip.getAttribute('aria-label')).toBe(
      'Gate not passed · P(yes) 0.62 · threshold 0.80 · attempt 2 of 3, Check the numbers'
    );
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
          // The chat's default model, instead of the editor's own
          // `GET /sessions/{id}/decision-settings` read (056503ba9).
          defaultModel={null}
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
    // The dialog's own layout (facts under Details, history in its menu) is the
    // decision package's to test; here, that the chip opened THIS gate's dialog.
    const dialog = screen.getByTestId('phase-gate-dialog');
    expect(within(dialog).getByTestId('phase-gate-status')).toHaveTextContent('Blocked: needs you');
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

  it('draws no line: every border it names is a reset, but the gate chip’s transparent one', () => {
    const borders = [...CSS.matchAll(/border(?:-top|-bottom|-left|-right)?\s*:\s*([^;]+);/g)];
    for (const [, value] of borders)
      expect(['0', 'none', '1px solid transparent'], value).toContain(value.trim());
    // The one transparent border is the chip's, which forced colours draw (spec 3.3).
    const chip = CSS.match(/\n\.br-agents__gate \{([^}]*)\}/)?.[1] ?? '';
    expect(chip).toMatch(/border: 1px solid transparent;/);
    expect([...CSS.matchAll(/1px solid transparent/g)]).toHaveLength(1);
  });

  it('uses the Button primitive only for the icon-only Monitors button, so no third type size', () => {
    const buttons = TSX.match(/<Button\b[\s\S]*?>/g) ?? [];
    expect(buttons).toHaveLength(1);
    expect(buttons[0]).toMatch(/aria-label=\{copy\.monitors\}/);
    expect(buttons[0]).toMatch(/size="xs"/);
    expect(buttons[0]).toMatch(/shape="round"/);
  });
});
