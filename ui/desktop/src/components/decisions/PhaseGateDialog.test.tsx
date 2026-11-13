import { act, fireEvent, render, screen, waitFor, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { parseDecisionModelEntries } from '../../utils/sessionDecisions';
import { parseSessionSubagents, type PhaseSnapshot } from '../../utils/sessionSubagents';
import { wirePhase, wireSubagents } from '../../utils/sessionSubagents.fixture';
import {
  PHASE_GATE_COPY,
  PhaseGateDialog,
  phaseGateStatus,
  refusalField,
  type PhaseGateDialogProps,
} from './PhaseGateDialog';
import { DECISION_MODEL_OPTION_COPY } from './decisionModelOption';

/*
 * The generated client functions are watched where they are called from, so a
 * test sees the exact request each control sends, its proof header included.
 */
const api = vi.hoisted(() => ({
  decide: vi.fn(),
  put: vi.fn(),
  remove: vi.fn(),
  settings: vi.fn(),
  headers: vi.fn(),
}));
vi.mock('../../api/sdk.gen', async (original) => ({
  ...(await original<typeof import('../../api/sdk.gen')>()),
  decidePhaseGate: api.decide,
  putPhaseGate: api.put,
  deletePhaseGate: api.remove,
  getDecisionSettings: api.settings,
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
  api.settings.mockReset();
  // A daemon with no stored settings for the chat: the default row stays unnamed.
  api.settings.mockResolvedValue({ data: undefined, response: { status: 404 } });
});

/** Opens the footer's `⋯` and picks an item by its test id. */
async function pickFromMenu(testId: string) {
  await userEvent.click(screen.getByTestId('phase-gate-more'));
  await userEvent.click(await screen.findByTestId(testId));
}

/** The menu's items, by their visible words, in order. */
async function menuItems() {
  await userEvent.click(screen.getByTestId('phase-gate-more'));
  const items = (await screen.findAllByRole('menuitem')).map((item) => item.textContent);
  await userEvent.keyboard('{Escape}');
  return items;
}

function openDetails() {
  fireEvent.click(
    within(screen.getByTestId('phase-gate-facts')).getByRole('button', {
      name: PHASE_GATE_COPY.details,
    })
  );
  return screen.getByTestId('phase-gate-details');
}

function openMoreOptions() {
  if (screen.queryByTestId('phase-gate-more-options')) return;
  fireEvent.click(screen.getByRole('button', { name: PHASE_GATE_COPY.editor.moreOptions }));
}

describe('what the dialog says about a gate', () => {
  it('names who set the gate under Details: the agent, the person or the workflow', () => {
    const { rerender } = draw();
    expect(screen.queryByTestId('phase-gate-owner')).toBeNull();
    openDetails();
    expect(screen.getByTestId('phase-gate-owner')).toHaveTextContent('The agent');
    for (const [owner, words] of [
      ['person', 'You'],
      ['workflow', 'The workflow'],
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

  it('shows the status, the last check, the questions and what it holds up front; the rest under Details', () => {
    const { phase, phases } = phaseOf({
      ...GATE,
      attempts: 1,
      maxAttempts: 2,
      lastPassed: false,
      lastMinP: 0.12,
      model: 'ollama_decision/tev1:0.8b',
    });
    draw({ phase, phases });
    // R1: a gate that did not pass reads Not passed, never Failed.
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent(
      'Not passed · 1 of 2 attempts used'
    );
    expect(screen.getByTestId('phase-gate-dialog')).not.toHaveTextContent('Failed');
    // R2: the probability is named, beside the threshold.
    expect(screen.getByTestId('phase-gate-last-check')).toHaveTextContent(
      /^Lowest P\(yes\) 0\.12 · threshold 0\.90$/
    );
    expect(screen.getByTestId('phase-gate-questions')).toHaveTextContent(
      'Does the hand-off cite a paper published in 2031?'
    );
    // By the phases' titles, not their ids.
    expect(screen.getByTestId('phase-gate-holds')).toHaveTextContent(
      'Write and Review wait until this gate passes.'
    );
    // What Pass and Skip do, in an InfoTip after the status.
    expect(
      within(screen.getByTestId('phase-gate-facts')).getByRole('button', {
        name: `About ${PHASE_GATE_COPY.statusHelpLabel}`,
      })
    ).toBeInTheDocument();
    expect(screen.queryByTestId('phase-gate-model-line')).toBeNull();
    const details = openDetails();
    expect(details).toHaveTextContent('ThresholdEvery question must reach P(yes) 0.90');
    expect(screen.getByTestId('phase-gate-attempts-used')).toHaveTextContent(/^1 of 2 used$/);
    // R3: the alias and where it runs, never the provider key.
    expect(screen.getByTestId('phase-gate-model-line')).toHaveTextContent(
      /^tev1:0\.8b · this computer$/
    );
    expect(details).not.toHaveTextContent('ollama_decision');
    // Up front already: the last check is not repeated under Details.
    expect(details).not.toHaveTextContent('Last check');
  });

  it('keeps a passed gate’s last check under Details, with the retries given', () => {
    const { phase, phases } = phaseOf({
      ...GATE,
      attempts: 1,
      lastPassed: true,
      lastMinP: 0.95,
      retriesGranted: 1,
      allowUncalibrated: true,
    });
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent(/^Passed$/);
    expect(screen.queryByTestId('phase-gate-last-check')).toBeNull();
    const details = openDetails();
    expect(details).toHaveTextContent('Last checkLowest P(yes) 0.95 · threshold 0.90');
    expect(screen.getByTestId('phase-gate-attempts-used')).toHaveTextContent(
      '1 of 1 used · given again once'
    );
    expect(details).toHaveTextContent('Accepts uncalibrated probabilities');
    expect(details).toHaveTextContent("ModelThis chat's decision model");
  });

  it('names a held phase it cannot find as "a later phase", never by its id', () => {
    const { phase, phases } = phaseOf({ ...GATE, holds: ['write', 'phase-7f3a'] });
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-holds')).toHaveTextContent(
      'Write and a later phase wait until this gate passes.'
    );
    expect(screen.getByTestId('phase-gate-dialog')).not.toHaveTextContent('phase-7f3a');
  });

  it('says why the last check decided nothing', () => {
    const { phase, phases } = phaseOf({ ...GATE, lastError: 'Ollama is not running.' });
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-last-error')).toHaveTextContent(
      'The last check decided nothing: Ollama is not running.'
    );
  });

  it('says a gate that failed every attempt waits for the person, and makes Pass primary', async () => {
    const { phase, phases } = phaseOf(
      { ...GATE, attempts: 1, lastPassed: false, lastMinP: 0.02, blocked: true, needsPerson: true },
      { status: 'in_progress' }
    );
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Blocked: needs you');
    // R2-7: one short line; the reason is the tip beside it.
    const note = screen.getByTestId('phase-gate-needs-person');
    expect(note.querySelector('.br-gate-waiting')?.firstChild?.textContent).toBe(
      PHASE_GATE_COPY.waitingForYou
    );
    expect(within(note).getByRole('button', { name: /Why it waits/ })).toHaveAccessibleDescription(
      PHASE_GATE_COPY.needsPerson
    );
    expect(screen.getByTestId('phase-gate-last-check')).toHaveTextContent(
      'Lowest P(yes) 0.02 · threshold 0.90'
    );
    expect(screen.getByTestId('phase-gate-pass')).toHaveAttribute('data-variant', 'default');
    // Skip is quiet (R2-7): never a filled button beside the primary.
    expect(screen.getByTestId('phase-gate-skip')).toHaveAttribute('data-variant', 'ghost');
    expect(screen.getByTestId('phase-gate-pass')).toBeEnabled();
    expect(await menuItems()).toEqual([
      PHASE_GATE_COPY.retry,
      PHASE_GATE_COPY.edit,
      PHASE_GATE_COPY.history,
      PHASE_GATE_COPY.remove,
    ]);
  });

  it('has no primary in the footer when the person is not needed', () => {
    for (const gate of [
      GATE,
      { ...GATE, attempts: 1, maxAttempts: 2, lastPassed: false, lastMinP: 0.1 },
      { ...GATE, attempts: 1, lastPassed: true },
      { ...GATE, personDecision: 'pass' },
    ]) {
      const { phase, phases } = phaseOf(gate);
      const { unmount } = draw({ phase, phases });
      expect(screen.getByTestId('phase-gate-pass')).toHaveAttribute('data-variant', 'secondary');
      expect(screen.getByTestId('phase-gate-skip')).toHaveAttribute('data-variant', 'ghost');
      unmount();
    }
  });

  it('puts Skip and Pass in the footer with tooltips saying what each does', async () => {
    draw();
    // A focus the Tab key moved shows a label tooltip (Tooltip's Tab gate).
    const tabTo = (element: HTMLElement) => {
      fireEvent.keyDown(document.activeElement ?? document.body, { key: 'Tab' });
      act(() => element.focus());
    };
    tabTo(screen.getByTestId('phase-gate-skip'));
    expect((await screen.findAllByText(PHASE_GATE_COPY.skipTip)).length).toBeGreaterThan(0);
    tabTo(screen.getByTestId('phase-gate-pass'));
    expect((await screen.findAllByText(PHASE_GATE_COPY.passTip)).length).toBeGreaterThan(0);
    // The body holds no buttons of its own any more.
    expect(screen.queryByTestId('phase-gate-actions')).toBeNull();
  });

  it('offers Try again only once an attempt was used or the gate blocked', async () => {
    draw();
    expect(await menuItems()).toEqual([
      PHASE_GATE_COPY.edit,
      PHASE_GATE_COPY.history,
      PHASE_GATE_COPY.remove,
    ]);
    expect(screen.queryByTestId('phase-gate-needs-person')).toBeNull();
  });

  it('renders the menu from the shared row-action items, Remove last and destructive', async () => {
    draw();
    await userEvent.click(screen.getByTestId('phase-gate-more'));
    const remove = await screen.findByTestId('phase-gate-remove');
    expect(remove).toHaveAttribute('data-variant', 'destructive');
    const menu = remove.closest('[role="menu"]')!;
    expect(menu.querySelector('[role="separator"]')).not.toBeNull();
    const items = within(menu as HTMLElement).getAllByRole('menuitem');
    expect(items[items.length - 1]).toBe(remove);
  });

  it('says Passed for a phase its gate passed, as the side panel’s chip does (R2-6)', () => {
    const { phase, phases } = phaseOf(
      { ...GATE, attempts: 1, lastPassed: true, lastMinP: 0.95, holds: [] },
      { status: 'completed' }
    );
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent(/^Passed$/);
    expect(phaseGateStatus(phase, phase.gate!)).toBe(PHASE_GATE_COPY.status.passed);
  });

  it('shows a closed phase as passed or skipped by the person, with Decision history alone', () => {
    const { phase, phases } = phaseOf(
      { ...GATE, personDecision: 'pass', holds: [] },
      { status: 'completed' }
    );
    draw({ phase, phases });
    expect(screen.getByTestId('phase-gate-status')).toHaveTextContent('Passed by you');
    expect(screen.queryByTestId('phase-gate-pass')).toBeNull();
    expect(screen.queryByTestId('phase-gate-more')).toBeNull();
    expect(screen.getByTestId('phase-gate-history')).toHaveTextContent('Decision history');
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
    expect(screen.getByTestId('phase-gate-applied')).toHaveAttribute('role', 'status');
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

  it('gives the gate its attempts again with Try again, from the menu', async () => {
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
    await pickFromMenu('phase-gate-retry');
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
    const refusal = await screen.findByTestId('phase-gate-refusal');
    expect(refusal).toHaveTextContent(
      'Only the person at the keyboard may pass, skip or retry a phase gate.'
    );
    expect(refusal).toHaveAttribute('role', 'alert');
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

  it('asks before removing the gate: Cancel removes nothing', async () => {
    draw();
    await pickFromMenu('phase-gate-remove');
    const confirm = await screen.findByRole('dialog', {
      name: PHASE_GATE_COPY.confirmRemove.title,
    });
    expect(confirm).toHaveTextContent(
      'Its questions and attempts are discarded, and the agent can complete Research without a check.'
    );
    fireEvent.click(within(confirm).getByRole('button', { name: 'Cancel' }));
    await waitFor(() =>
      expect(screen.queryByRole('dialog', { name: PHASE_GATE_COPY.confirmRemove.title })).toBeNull()
    );
    expect(api.remove).not.toHaveBeenCalled();
    expect(screen.getByTestId('phase-gate-dialog')).toBeInTheDocument();
  });

  it('removes the gate with the proof once confirmed, and the phase reads as ungated', async () => {
    api.remove.mockResolvedValue(
      answered(
        'Phase "research" has no gate now: the agent can complete it without one.',
        true,
        null
      )
    );
    draw();
    await pickFromMenu('phase-gate-remove');
    const confirm = await screen.findByRole('dialog', {
      name: PHASE_GATE_COPY.confirmRemove.title,
    });
    fireEvent.click(
      within(confirm).getByRole('button', { name: PHASE_GATE_COPY.confirmRemove.confirm })
    );
    expect(await screen.findByTestId('phase-gate-applied')).toHaveTextContent('has no gate now');
    expect(api.remove.mock.calls[0][0]).toMatchObject({
      path: { session_id: 's1', phase_id: 'research' },
      headers: PROOF,
    });
    expect(screen.getByTestId('phase-gate-none')).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-add')).toHaveAttribute('data-variant', 'default');
  });

  it('opens the gate’s decision history from the menu, closing itself first', async () => {
    const { onOpenHistory, onOpenChange } = draw();
    await pickFromMenu('phase-gate-history');
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(onOpenHistory).toHaveBeenCalledWith({ origin: 'gate', originId: 'research' });
  });
});

describe('the editor', () => {
  it('edits in place: a title of its own, the threshold in words, Cancel back to the gate', async () => {
    draw();
    await pickFromMenu('phase-gate-edit');
    expect(screen.getByRole('dialog', { name: PHASE_GATE_COPY.editTitle })).toBeInTheDocument();
    expect(screen.getByText(PHASE_GATE_COPY.editor.replaces)).toBeInTheDocument();
    // R2: the helper names its outcome.
    expect(screen.getByTestId('phase-gate-threshold-help')).toHaveTextContent(
      "Passes when every question's P(yes) is at or above 0.90."
    );
    for (const label of [PHASE_GATE_COPY.editor.questions, PHASE_GATE_COPY.editor.threshold]) {
      expect(screen.getByRole('button', { name: `About ${label}` })).toBeInTheDocument();
    }
    // The inner filled card is gone: the fields sit on the dialog ground.
    expect(screen.getByTestId('phase-gate-editor')).not.toHaveClass('br-fill-surface');
    fireEvent.click(screen.getByRole('button', { name: PHASE_GATE_COPY.editor.cancel }));
    expect(screen.queryByTestId('phase-gate-editor')).toBeNull();
    expect(screen.getByTestId('phase-gate-facts')).toBeInTheDocument();
  });

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
    await pickFromMenu('phase-gate-edit');
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
    // GATE has one attempt (not the default three): More options starts open.
    expect(screen.getByTestId('phase-gate-more-options')).toBeInTheDocument();
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
    openDetails();
    expect(screen.getByTestId('phase-gate-owner')).toHaveTextContent('You');
  });

  it('shows a 422 sentence inline and keeps the draft', async () => {
    api.put.mockResolvedValue(
      refused(
        422,
        'Phase "research"\'s gate cannot be answered in this chat: the questions could not be read. Nothing was changed.'
      )
    );
    draw();
    await pickFromMenu('phase-gate-edit');
    fireEvent.change(screen.getByTestId('phase-gate-question-0'), {
      target: { value: 'Is every claim cited?' },
    });
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(await screen.findByTestId('phase-gate-refusal')).toHaveTextContent(
      /^Phase "research"'s gate cannot be answered in this chat/
    );
    expect(screen.getByTestId('phase-gate-editor')).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-question-0')).toHaveValue('Is every claim cited?');
  });

  it('opens More options and focuses the model when a refusal names it', async () => {
    api.put.mockResolvedValue(
      refused(
        422,
        'Phase "research"\'s gate cannot be answered in this chat: the decision model is not available. Nothing was changed.'
      )
    );
    const { phase, phases } = phaseOf({ ...GATE, maxAttempts: 3 });
    draw({ phase, phases });
    await pickFromMenu('phase-gate-edit');
    // Three attempts and the chat's model: More options starts closed, with its summary.
    expect(screen.queryByTestId('phase-gate-more-options')).toBeNull();
    expect(screen.getByText("3 attempts · This chat's decision model")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    const refusal = await screen.findByTestId('phase-gate-refusal');
    expect(refusal).toHaveTextContent(/^Model: Phase "research"'s gate cannot be answered/);
    expect(await screen.findByTestId('phase-gate-more-options')).toBeInTheDocument();
    const model = within(screen.getByTestId('phase-gate-model')).getByRole('combobox');
    await waitFor(() => expect(model).toHaveFocus());
  });

  it('reads which field a refusal names', () => {
    expect(refusalField('the decision model is not available')).toBe('model');
    expect(refusalField('maxAttempts must be from 1 to 5')).toBe('attempts');
    expect(refusalField('uncalibrated probabilities are not accepted here')).toBe('uncalibrated');
    expect(refusalField('a question is empty')).toBeNull();
  });

  it('refuses an empty question before anything is sent', async () => {
    draw();
    await pickFromMenu('phase-gate-edit');
    fireEvent.change(screen.getByTestId('phase-gate-question-0'), { target: { value: ' ' } });
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(screen.getByText('Write the question, or remove it.')).toBeInTheDocument();
    expect(api.put).not.toHaveBeenCalled();
  });

  it('adds up to eight questions, each with a name of its own', async () => {
    api.put.mockResolvedValue(answered('Phase "research" now has your gate.', true));
    draw();
    await pickFromMenu('phase-gate-edit');
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

  it('keeps the adapter the gate’s model is asked through when an edit changes only a question', async () => {
    api.put.mockResolvedValue(answered('Phase "research" now has your gate.', true));
    const { phase, phases } = phaseOf({
      ...GATE,
      owner: 'workflow',
      model: 'ollama_decision/tev1:0.8b',
      modelAdapter: 'logprob',
    });
    draw({ phase, phases });
    await pickFromMenu('phase-gate-edit');
    fireEvent.change(screen.getByTestId('phase-gate-question-0'), {
      target: { value: 'Is every claim cited?' },
    });
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));
    // The write replaces the whole gate: without the adapter it would move to tev1's default.
    expect(api.put.mock.calls[0][0].body.model).toEqual({
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      adapter: 'logprob',
    });
  });

  it('picking the gate’s own model again, even after another, keeps its adapter', async () => {
    api.put.mockResolvedValue(answered('Phase "research" now has your gate.', true));
    const { phase, phases } = phaseOf({
      ...GATE,
      model: 'ollama_decision/tev1:0.8b',
      modelAdapter: 'logprob',
    });
    draw({ phase, phases });
    await pickFromMenu('phase-gate-edit');
    openMoreOptions();
    const input = () => within(screen.getByTestId('phase-gate-model')).getByRole('combobox');
    const pick = async (label: string) => {
      fireEvent.keyDown(input(), { key: 'ArrowDown' });
      const row = (await screen.findAllByText(label))
        .map((node) => node.closest('[role="option"]'))
        .find(Boolean);
      fireEvent.click(row!);
    };
    await pick(DECISION_MODEL_OPTION_COPY.defaultModel);
    await pick('tev1:0.8b');
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));
    expect(api.put.mock.calls[0][0].body.model).toEqual({
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      adapter: 'logprob',
    });
  });

  it('names what the default model resolves to, read from the chat’s decision settings', async () => {
    api.settings.mockResolvedValue({
      data: { defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b' }, monitors: [] },
      response: { status: 200 },
    });
    draw();
    await pickFromMenu('phase-gate-edit');
    openMoreOptions();
    await waitFor(() =>
      expect(screen.getByTestId('phase-gate-model')).toHaveTextContent(
        DECISION_MODEL_OPTION_COPY.defaultModelNamed('tev1:0.8b')
      )
    );
    expect(api.settings.mock.calls[0][0].path).toEqual({ session_id: 's1' });
  });

  it('keeps the default model unnamed when the chat stores none or the read fails', async () => {
    draw({ defaultModel: null });
    await pickFromMenu('phase-gate-edit');
    openMoreOptions();
    const field = screen.getByTestId('phase-gate-model');
    expect(field).toHaveTextContent(DECISION_MODEL_OPTION_COPY.defaultModel);
    expect(field).not.toHaveTextContent('Default model (');
    // Given by the caller, so nothing is read.
    expect(api.settings).not.toHaveBeenCalled();
  });

  it('a default model the caller names is shown without a read', async () => {
    draw({ defaultModel: { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: null } });
    await pickFromMenu('phase-gate-edit');
    openMoreOptions();
    expect(screen.getByTestId('phase-gate-model')).toHaveTextContent('Default model (tev1:0.8b)');
    expect(api.settings).not.toHaveBeenCalled();
  });

  it('offers each model by alias and place, a model this chat may not use disabled with its reason shown', async () => {
    draw({ chatTier: 'private' });
    await pickFromMenu('phase-gate-edit');
    openMoreOptions();
    // In a private chat the label's InfoTip says only a private model may be used.
    expect(
      within(screen.getByTestId('phase-gate-model')).getByRole('button', { name: 'About Model' })
    ).toBeInTheDocument();
    const input = within(screen.getByTestId('phase-gate-model')).getByRole('combobox');
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    const option = await screen.findByText('joey-1');
    const row = option.closest('[role="option"]') ?? option.closest('[aria-disabled]');
    expect(row).toHaveAttribute('aria-disabled', 'true');
    // R4: the person's line, not the daemon's agent-facing sentence.
    expect(row).toHaveTextContent(DECISION_MODEL_OPTION_COPY.publicRefused);
    const local = (await screen.findByText('tev1:0.8b')).closest('[role="option"]')!;
    expect(local).toHaveTextContent('this computer');
    expect(local).not.toHaveTextContent('Private ·');
    expect(document.body).not.toHaveTextContent('ollama_decision');
  });

  it('opens on the editor for a phase with no gate, and Cancel leaves with nothing set', () => {
    const { phase, phases } = phaseOf(null);
    const { onOpenChange } = draw({ phase, phases });
    expect(screen.getByRole('dialog', { name: PHASE_GATE_COPY.addTitle })).toBeInTheDocument();
    expect(screen.getByTestId('phase-gate-editor')).toBeInTheDocument();
    // A new gate starts at the daemon's defaults: 0.80, three attempts.
    expect(thresholdThumb()).toHaveAttribute('aria-valuetext', '0.80');
    openMoreOptions();
    expect(
      within(screen.getByTestId('phase-gate-attempts')).getByRole('radio', { name: '3' })
    ).toBeChecked();
    fireEvent.click(screen.getByRole('button', { name: PHASE_GATE_COPY.editor.cancel }));
    expect(onOpenChange).toHaveBeenCalledWith(false);
    expect(api.put).not.toHaveBeenCalled();
  });
});
