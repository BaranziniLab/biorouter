import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import { parseDecisionModelEntries } from '../../utils/sessionDecisions';
import { parseSessionSubagents } from '../../utils/sessionSubagents';
import { wirePhase, wireSubagents } from '../../utils/sessionSubagents.fixture';
import { PhaseGateDialog } from './PhaseGateDialog';

/**
 * DR-33 (design §3.6, §5): a person's gate on a chat that can still be made public
 * by its decision models says "This chat will be public: …" above Save and sends the
 * person's answer with it; a gate write the daemon asks about turns Save into
 * "Make it public", which sends the same gate again with the answer.
 */

const api = vi.hoisted(() => ({ put: vi.fn(), settings: vi.fn(), headers: vi.fn() }));
vi.mock('../../api/sdk.gen', async (original) => ({
  ...(await original<typeof import('../../api/sdk.gen')>()),
  putPhaseGate: api.put,
  getDecisionSettings: api.settings,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: api.headers }));

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
    model: 'joey-1',
    displayName: 'joey-1',
    kinds: ['binary'],
    tier: 'public',
    chatEffect: 'makesPublic',
  },
])!;

const gateOn = (model: string) => ({
  threshold: 0.9,
  questionCount: 1,
  attempts: 0,
  maxAttempts: 3,
  lastPassed: null,
  lastMinP: null,
  blocked: false,
  owner: 'person',
  questions: { done: 'Is the research done?' },
  model,
  allowUncalibrated: false,
  lastError: null,
  personDecision: null,
  retriesGranted: 0,
  needsPerson: false,
  holds: [],
});

function phaseWith(gate: Record<string, unknown>) {
  const phases = parseSessionSubagents(
    wireSubagents([], {
      title: 'Plan',
      goal: null,
      revision: 1,
      phases: [wirePhase({ id: 'research', title: 'Research', status: 'in_progress', gate })],
    })
  )!.plan!.phases;
  return { phase: phases[0], phases };
}

const ok = {
  data: { message: 'The gate on Research is yours now.', applied: true, plan: null },
  response: { status: 200 },
};

beforeEach(() => {
  api.put.mockReset();
  api.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'proof' });
  api.settings.mockReset().mockResolvedValue({ data: undefined, response: { status: 404 } });
});

async function edit(gate: Record<string, unknown>) {
  render(
    <PhaseGateDialog
      open
      onOpenChange={vi.fn()}
      sessionId="s1"
      models={MODELS}
      {...phaseWith(gate)}
    />
  );
  await userEvent.click(screen.getByTestId('phase-gate-more'));
  await userEvent.click(await screen.findByTestId('phase-gate-edit'));
}

describe('a gate that would make the chat public', () => {
  it('says so above Save, and Save sends the person’s answer', async () => {
    api.put.mockResolvedValue(ok);
    await edit(gateOn('baseten_decision/joey-1'));
    expect(screen.getByTestId('phase-gate-tier-effect')).toHaveTextContent(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));
    expect(api.put.mock.calls[0][0].body).toMatchObject({ acceptPublicDecisionModels: true });
  });

  it('a gate on a model on this computer sends no answer and shows no line', async () => {
    api.put.mockResolvedValue(ok);
    await edit(gateOn('ollama_decision/tev1:0.8b'));
    expect(screen.queryByTestId('phase-gate-tier-effect')).toBeNull();
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(1));
    expect(api.put.mock.calls[0][0].body).not.toHaveProperty('acceptPublicDecisionModels');
  });

  it('when the daemon asks, Save becomes the answer and sends the gate again', async () => {
    api.put
      .mockResolvedValueOnce({
        error: {
          code: 'needsPublicDecisionConfirmation',
          message: 'This chat will be public: joey-1 runs on Baseten.',
          info: 'A chat is as private as its least private model.',
          models: ['baseten_decision/joey-1'],
        },
        response: { status: 409 },
      })
      .mockResolvedValueOnce(ok);
    await edit(gateOn('ollama_decision/tev1:0.8b'));
    fireEvent.click(screen.getByTestId('phase-gate-save'));
    expect(await screen.findByTestId('phase-gate-tier-effect')).toHaveTextContent(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    expect(screen.queryByTestId('phase-gate-refusal')).toBeNull();
    const save = screen.getByTestId('phase-gate-save');
    expect(save).toHaveTextContent('Make it public');
    fireEvent.click(save);
    await waitFor(() => expect(api.put).toHaveBeenCalledTimes(2));
    expect(api.put.mock.calls[1][0].body).toMatchObject({ acceptPublicDecisionModels: true });
  });
});
