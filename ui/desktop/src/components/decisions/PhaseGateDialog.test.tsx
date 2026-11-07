import { fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { parseDecisionModelEntries } from '../../utils/sessionDecisions';
import { parseSessionSubagents, type PhaseSnapshot } from '../../utils/sessionSubagents';
import { wirePhase, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { PHASE_GATE_COPY, PhaseGateDialog, type PhaseGateDialogProps } from './PhaseGateDialog';

/*
 * The generated client functions are watched where they are called from, so a
 * test sees the exact request each control sends, its proof header included.
 */
const api = vi.hoisted(() => ({
  decide: vi.fn(),
  put: vi.fn(),
  remove: vi.fn(),
  headers: vi.fn(),
}));
vi.mock('../../api/sdk.gen', async (original) => ({
  ...(await original<typeof import('../../api/sdk.gen')>()),
  decidePhaseGate: api.decide,
  putPhaseGate: api.put,
  deletePhaseGate: api.remove,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: api.headers }));

const PROOF = { 'X-User-Action': 'proof-of-the-person' };

const MODELS = parseDecisionModelEntries([
  {
    provider: 'ollama_decision',
    model: 'tev1:0.8b',
    displayName: 'tev1:0.8b',
    kinds: ['binary'],
    tier: 'private',
    local: true,
  },
  {
    provider: 'baseten_decision',
    model: 'decision-models/baseten-joey-2b-latest',
    displayName: 'joey-1',
    kinds: ['binary'],
    tier: 'public',
    unavailableReason: 'This chat is private, so a public decision model may not read it.',
  },
])!;

const GATE = {
  threshold: 0.9,
  questionCount: 1,
  attempts: 0,
  maxAttempts: 1,
  lastPassed: null,
  lastMinP: null,
  blocked: false,
  owner: 'agent',
  questions: { y2031: 'Does the hand-off cite a paper published in 2031?' },
  model: null,
  allowUncalibrated: false,
  lastError: null,
  personDecision: null,
  retriesGranted: 0,
  needsPerson: false,
  holds: ['write', 'review'],
};

/** A three-phase plan whose first phase carries `gate`, as the daemon sends it. */
function plan(gate: Record<string, unknown> | null, research: Record<string, unknown> = {}) {
  return parseSessionSubagents(
    wireSubagents([], {
      title: 'Plan',
      goal: null,
      revision: 2,
      phases: [
        wirePhase({ id: 'research', title: 'Research', status: 'in_progress', gate, ...research }),
        wirePhase({ id: 'write', title: 'Write', heldBy: gate ? 'research' : null }),
        wirePhase({
          id: 'review',
          title: 'Review',
          kind: 'review',
          heldBy: gate ? 'research' : null,
        }),
      ],
    })
  )!.plan!;
}

function phaseOf(
  gate: Record<string, unknown> | null,
  research: Record<string, unknown> = {}
): { phase: PhaseSnapshot; phases: PhaseSnapshot[] } {
  const phases = plan(gate, research).phases;
  return { phase: phases[0], phases };
}

/** A 200 the gate routes answer: the daemon's sentence and the plan after the write. */
function answered(message: string, applied: boolean, gate: Record<string, unknown> | null = GATE) {
  return {
    data: {
      message,
      applied,
      plan: {
        title: 'Plan',
        goal: null,
        revision: 3,
        phases: [wirePhase({ id: 'research', title: 'Research', status: 'in_progress', gate })],
      },
    },
    response: { status: 200 },
  };
}

/** A refusal: plain text with its status, as the routes send one. */
function refused(status: number, sentence: string) {
  return { error: sentence, response: { status } };
}

function draw(overrides: Partial<PhaseGateDialogProps> = {}) {
  const onOpenChange = vi.fn();
  const onOpenHistory = vi.fn();
  const onChanged = vi.fn();
  const { phase, phases } = phaseOf(GATE);
  const utils = render(
    <PhaseGateDialog
      open
      onOpenChange={onOpenChange}
      sessionId="s1"
      phase={phase}
      phases={phases}
      onOpenHistory={onOpenHistory}
      onChanged={onChanged}
      models={MODELS}
      {...overrides}
    />
  );
  return { ...utils, onOpenChange, onOpenHistory, onChanged };
}

function thresholdThumb() {
  return within(screen.getByTestId('phase-gate-threshold')).getByRole('slider');
}

beforeEach(() => {
  api.decide.mockReset();
  api.put.mockReset();
  api.remove.mockReset();
  api.headers.mockReset();
  api.headers.mockResolvedValue(PROOF);
});

describe('what the dialog says about a gate', () => {
  it('names who set the gate: the agent, the person or the workflow', () => {
    const { rerender } = draw();
    expect(screen.getByTestId('phase-gate-owner')).toHaveTextContent('Set by the agent');
    for (const [owner, words] of [
      ['person', 'Set by you'],
      ['workflow', 'Set by the workflow'],
    ] as const) {
      const { phase, phases } = phaseOf({ ...GATE, owner });
      rerender(
        <PhaseGateDialog
          open
          onOpenChange={() => {}}
          sessionId="s1"
          phase={phase}
          phases={phases}
          models={MODELS}
        />
      );
      expect(screen.getByTestId('phase-gate-owner')).toHaveTextContent(words);
    }
  });

  it('shows the questions, the attempts, the phases it holds and the model', () => {
    const { phase, phases } = phaseOf({
      ...GATE,
      attempts: 1,
      lastPassed: false,
      lastMinP: 0.12,
      model: 'ollama_decision/tev1:0.8b',
    });
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-questions')).toHaveTextContent(
      'Does the hand-off cite a paper published in 2031?'
    );
    expect(screen.getByTestId('phase-gate-attempts-used')).toHaveTextContent(
      'Every question must reach P(yes) 0.90. 1 of 1 attempt used'
    );
    expect(screen.getByTestId('phase-gate-last-check')).toHaveTextContent(
      'Lowest P(yes) at the last check: 0.12, needs 0.90.'
    );
    // By the phases' titles, not their ids.
    expect(screen.getByTestId('phase-gate-holds')).toHaveTextContent('Holds Write and Review');
    expect(screen.getByTestId('phase-gate-model-line')).toHaveTextContent(
      'Decision model: tev1:0.8b'
    );
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Failed');
  });

  it('says why the last check decided nothing', () => {
    const { phase, phases } = phaseOf({ ...GATE, lastError: 'Ollama is not running.' });
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-last-error')).toHaveTextContent(
      'The last check decided nothing: Ollama is not running.'
    );
  });

  it('says a gate that failed every attempt waits for the person, and offers Try again', () => {
    const { phase, phases } = phaseOf(
      { ...GATE, attempts: 1, lastPassed: false, lastMinP: 0.02, blocked: true, needsPerson: true },
      { status: 'in_progress' }
    );
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Blocked: needs you');
    expect(screen.getByTestId('phase-gate-needs-person')).toHaveTextContent(
      PHASE_GATE_COPY.needsPerson
    );
    expect(screen.getByTestId('phase-gate-retry')).toHaveTextContent('Try again');
    expect(screen.getByTestId('phase-gate-pass')).toBeEnabled();
    expect(screen.getByTestId('phase-gate-skip')).toBeEnabled();
  });

  it('offers Try again only once an attempt was used or the gate blocked', () => {
    draw();
    expect(screen.queryByTestId('phase-gate-retry')).toBeNull();
    expect(screen.queryByTestId('phase-gate-needs-person')).toBeNull();
  });

  it('shows a closed phase as passed or skipped by the person, with history and nothing else', () => {
    const { phase, phases } = phaseOf(
      { ...GATE, personDecision: 'pass', holds: [] },
      { status: 'completed' }
    );
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Passed by you');
    expect(screen.queryByTestId('phase-gate-pass')).toBeNull();
    expect(screen.queryByTestId('phase-gate-edit')).toBeNull();
    expect(screen.getByTestId('phase-gate-history')).toBeInTheDocument();
  });
});

describe('the person’s decisions', () => {
  it('passes the phase with the person’s proof, and says it was applied', async () => {
    api.decide.mockResolvedValue(
      answered('Phase "research" is completed: you passed its gate.', true, {
        ...GATE,
        personDecision: 'pass',
        holds: [],
      })
    );
    const { onChanged } = draw();
    fireEvent.click(screen.getByTestId('phase-gate-pass'));
    expect(await screen.findByTestId('phase-gate-applied')).toHaveTextContent(
      'Phase "research" is completed: you passed its gate.'
    );
    expect(api.decide).toHaveBeenCalledTimes(1);
    expect(api.decide.mock.calls[0][0]).toMatchObject({
      path: { session_id: 's1', phase_id: 'research' },
      body: { decision: 'pass' },
      headers: PROOF,
    });
    expect(api.headers).toHaveBeenCalled();
    expect(onChanged).toHaveBeenCalledTimes(1);
    expect(onChanged.mock.calls[0][0].revision).toBe(3);
    expect(screen.queryByTestId('phase-gate-standing')).toBeNull();
  });

  it('says a skip that waits for working subagents stands, not that it was applied', async () => {
    api.decide.mockResolvedValue(
      answered(
        'Your skip stands: phase "research" is skipped without its gate once its 1 working subagent finishes and the agent completes it.',
        false,
        { ...GATE, personDecision: 'skip' }
      )
    );
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-skip'));
    expect(await screen.findByTestId('phase-gate-standing')).toHaveTextContent(
      /^Your skip stands: phase "research"/
    );
    expect(api.decide.mock.calls[0][0]).toMatchObject({
      body: { decision: 'skip' },
      headers: PROOF,
    });
    expect(screen.queryByTestId('phase-gate-applied')).toBeNull();
    // The plan it answered with is what the dialog shows now.
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Your skip stands');
    expect(screen.getByTestId('phase-gate-standing-note')).toHaveTextContent(
      PHASE_GATE_COPY.standingSkip
    );
  });

  it('gives the gate its attempts again with Try again', async () => {
    api.decide.mockResolvedValue(
      answered(
        'Phase "research"\'s gate has its 1 attempt again. The agent can complete the phase once more to ask it.',
        true
      )
    );
    const { phase, phases } = phaseOf({
      ...GATE,
      attempts: 1,
      lastPassed: false,
      blocked: true,
      needsPerson: true,
    });
    draw({ phase, phases });
    fireEvent.click(screen.getByTestId('phase-gate-retry'));
    expect(await screen.findByTestId('phase-gate-applied')).toHaveTextContent(
      'has its 1 attempt again'
    );
    expect(api.decide.mock.calls[0][0]).toMatchObject({
      path: { session_id: 's1', phase_id: 'research' },
      body: { decision: 'retry' },
      headers: PROOF,
    });
  });

  it('shows a 403 or 409 sentence inline, word for word, and changes nothing', async () => {
    api.decide.mockResolvedValue(
      refused(403, 'Only the person at the keyboard may pass, skip or retry a phase gate.')
    );
    const { onChanged } = draw();
    fireEvent.click(screen.getByTestId('phase-gate-pass'));
    expect(await screen.findByTestId('phase-gate-refusal')).toHaveTextContent(
      'Only the person at the keyboard may pass, skip or retry a phase gate.'
    );
    expect(onChanged).not.toHaveBeenCalled();
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Not asked yet');

    api.decide.mockResolvedValue(
      refused(409, 'Phase "research" is already completed; a gate holds only an open phase.')
    );
    fireEvent.click(screen.getByTestId('phase-gate-skip'));
    await waitFor(() =>
      expect(screen.getByTestId('phase-gate-refusal')).toHaveTextContent(
        'a gate holds only an open phase'
      )
    );
  });

  it('says so when Biorouter could not be reached', async () => {
    api.decide.mockRejectedValue(new TypeError('Failed to fetch'));
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-pass'));
    expect(await screen.findByTestId('phase-gate-refusal')).toHaveTextContent(
      'Could not reach Biorouter. Nothing was changed.'
    );
  });

  it('removes the gate with the proof, and the phase reads as ungated', async () => {
    api.remove.mockResolvedValue(
      answered(
        'Phase "research" has no gate now: the agent can complete it without one.',
        true,
        null
      )
    );
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-remove'));
    expect(await screen.findByTestId('phase-gate-applied')).toHaveTextContent('has no gate now');
    expect(api.remove.mock.calls[0][0]).toMatchObject({
      path: { session_id: 's1', phase_id: 'research' },
      headers: PROOF,
    });
    expect(screen.getByTestId('phase-gate-none')).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-add')).toBeInTheDocument();
  });

  it('opens the gate’s decision history, closing itself first', () => {
    const { onOpenHistory, onOpenChange } = draw();
    fireEvent.click(screen.getByTestId('phase-gate-history'));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onOpenHistory).toHaveBeenCalledWith({ origin: 'gate', originId: 'research' });
  });
});

describe('the editor', () => {
  it('accepts a threshold under 0.50 from the person, and saves it with the proof', async () => {
    api.put.mockResolvedValue(
      answered(
        'Phase "research" now has your gate: 1 question, each needing P(yes) of 0.30.',
        true,
        {
          ...GATE,
          owner: 'person',
          threshold: 0.3,
        }
      )
    );
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-edit'));
    expect(screen.getByTestId('phase-gate-question-0')).toHaveValue(
      'Does the hand-off cite a paper published in 2031?'
    );
    // 0.05 to 0.99 by 0.01: Home is 0.05, and 25 steps up is 0.30.
    expect(thresholdThumb()).toHaveAttribute('aria-valuemax', '94');
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.90');
    fireEvent.keyDown(thresholdThumb(), { key: 'Home' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.05');
    for (let i = 0; i < 25; i += 1) fireEvent.keyDown(thresholdThumb(), { key: 'ArrowRight' });
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.30');
    expect(screen.getByText(PHASE_GATE_COPY.editor.thresholdLow)).toBeInTheDocument();
    fireEvent.click(
      within(screen.getByTestId('phase-gate-attempts')).getByRole('radio', { name: '2' })
    );
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(await screen.findByTestId('phase-gate-applied')).toHaveTextContent('now has your gate');
    expect(api.put.mock.calls[0][0]).toMatchObject({
      path: { session_id: 's1', phase_id: 'research' },
      body: {
        questions: { y2031: 'Does the hand-off cite a paper published in 2031?' },
        threshold: 0.3,
        maxAttempts: 2,
        model: null,
        allowUncalibrated: false,
      },
      headers: PROOF,
    });
    // Back on the gate, now the person's.
    expect(screen.queryByTestId('phase-gate-editor')).toBeNull();
    expect(screen.getByTestId('phase-gate-owner')).toHaveTextContent('Set by you');
  });

  it('shows a 422 sentence inline and keeps the draft', async () => {
    api.put.mockResolvedValue(
      refused(
        422,
        'Phase "research"\'s gate cannot be answered in this chat: the decision model is not available. Nothing was changed.'
      )
    );
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-edit'));
    fireEvent.change(screen.getByTestId('phase-gate-question-0'), {
      target: { value: 'Is every claim cited?' },
    });
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(await screen.findByTestId('phase-gate-refusal')).toHaveTextContent(
      'cannot be answered in this chat'
    );
    expect(screen.getByTestId('phase-gate-editor')).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-question-0')).toHaveValue('Is every claim cited?');
  });

  it('refuses an empty question before anything is sent', () => {
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-edit'));
    fireEvent.change(screen.getByTestId('phase-gate-question-0'), { target: { value: ' ' } });
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(screen.getByText('Write the question, or remove it.')).toBeInTheDocument();
    expect(api.put).not.toHaveBeenCalled();
  });

  it('adds up to eight questions, each with a name of its own', async () => {
    api.put.mockResolvedValue(answered('Phase "research" now has your gate.', true));
    draw();
    fireEvent.click(screen.getByTestId('phase-gate-edit'));
    for (let i = 0; i < 7; i += 1) fireEvent.click(screen.getByTestId('phase-gate-add-question'));
    expect(screen.getByTestId('phase-gate-add-question')).toBeDisabled();
    for (let i = 1; i < 8; i += 1) {
      fireEvent.change(screen.getByTestId(`phase-gate-question-${i}`), {
        target: { value: `Question ${i}?` },
      });
    }
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));
    expect(Object.keys(api.put.mock.calls[0][0].body.questions)).toEqual([
      'y2031',
      'q1',
      'q2',
      'q3',
      'q4',
      'q5',
      'q6',
      'q7',
    ]);
  });

  it('offers each decision model, a model this chat may not use disabled with the daemon’s reason', async () => {
    draw({ chatTier: 'private' });
    fireEvent.click(screen.getByTestId('phase-gate-edit'));
    const input = within(screen.getByTestId('phase-gate-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    const option = await screen.findByText('joey-1 · Public');
    const row = option.closest('[role="option"]') ?? option.closest('[aria-disabled]');
    expect(row).toHaveAttribute('aria-disabled', 'true');
    expect(
      screen.getByText('This chat is private, so a public decision model may not read it.')
    ).toBeInTheDocument();
    expect(screen.getByText('tev1:0.8b · Private · this computer')).toBeInTheDocument();
  });

  it('opens on the editor for a phase with no gate, and Cancel leaves with nothing set', () => {
    const { phase, phases } = phaseOf(null);
    const { onOpenChange } = draw({ phase, phases });
    expect(screen.getByRole('dialog', { name: PHASE_GATE_COPY.addTitle })).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-editor')).toBeInTheDocument();
    // A new gate starts at the daemon's defaults: 0.80, three attempts.
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.80');
    expect(
      within(screen.getByTestId('phase-gate-attempts')).getByRole('radio', { name: '3' })
    ).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: PHASE_GATE_COPY.editor.cancel }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(api.put).not.toHaveBeenCalled();
  });
});
