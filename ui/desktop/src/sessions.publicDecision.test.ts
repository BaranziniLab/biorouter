import { beforeEach, describe, expect, it, vi } from 'vitest';
import { createSession } from './sessions';
import { isPublicChatDeclined } from './utils/sessionDecisions';

/**
 * DR-33 (design §4): a start carries the decision settings drafted before the first
 * message, and when the daemon answers that they (or a workflow's) would make the
 * chat public without the person's say-so, the start asks and starts again with it.
 */

const mocks = vi.hoisted(() => ({ startAgent: vi.fn() }));
vi.mock('./api', async (importOriginal) => ({
  ...(await importOriginal<typeof import('./api')>()),
  startAgent: mocks.startAgent,
}));
vi.mock('./utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'p' }),
}));

const question = {
  code: 'needsPublicDecisionConfirmation',
  message: 'This chat will be public: joey-1 runs on Baseten.',
  info: 'A chat is as private as its least private model.',
  models: ['baseten_decision/joey-1'],
};

const settings = {
  defaultModel: { provider: 'baseten_decision', model: 'joey-1', adapter: null },
  monitors: [],
};

beforeEach(() => {
  mocks.startAgent.mockReset();
});

describe('createSession and decision models (DR-33)', () => {
  it('sends a draft’s settings, its acceptance and the chat’s own model in the start', async () => {
    mocks.startAgent.mockResolvedValue({ data: { id: 'new' } });
    await createSession('/w', {
      decisionSettings: settings,
      acceptPublicDecisionModels: true,
      provider: 'versa_azure',
      model: 'gpt-5.5',
    });
    expect(mocks.startAgent.mock.calls[0][0].body).toEqual({
      working_dir: '/w',
      provider: 'versa_azure',
      model: 'gpt-5.5',
      decision_settings: {
        defaultModel: { provider: 'baseten_decision', model: 'joey-1' },
        monitors: [],
      },
      accept_public_decision_models: true,
    });
  });

  it('asks on the daemon’s question and starts again with the answer', async () => {
    mocks.startAgent.mockRejectedValueOnce(question).mockResolvedValueOnce({ data: { id: 'new' } });
    const confirm = vi.fn(async () => true);
    await expect(
      createSession('/w', { workflowId: 'wf', confirmPublicDecisionModels: confirm })
    ).resolves.toEqual({ id: 'new' });
    expect(confirm).toHaveBeenCalledWith({
      message: question.message,
      info: question.info,
      models: [{ provider: 'baseten_decision', model: 'joey-1', adapter: null }],
    });
    expect(mocks.startAgent.mock.calls[0][0].body).not.toHaveProperty(
      'accept_public_decision_models'
    );
    expect(mocks.startAgent.mock.calls[1][0].body).toMatchObject({
      workflow_id: 'wf',
      accept_public_decision_models: true,
    });
  });

  it('a person who says no starts nothing, and the caller is told it was declined', async () => {
    mocks.startAgent.mockRejectedValueOnce(question);
    const error = await createSession('/w', {
      decisionSettings: settings,
      confirmPublicDecisionModels: async () => false,
    }).catch((thrown: unknown) => thrown);
    expect(isPublicChatDeclined(error)).toBe(true);
    expect(mocks.startAgent).toHaveBeenCalledTimes(1);
  });

  it('with nobody to ask, the question is a decline, never an acceptance', async () => {
    mocks.startAgent.mockRejectedValueOnce(question);
    const error = await createSession('/w', { decisionSettings: settings }).catch(
      (thrown: unknown) => thrown
    );
    expect(isPublicChatDeclined(error)).toBe(true);
    expect(mocks.startAgent).toHaveBeenCalledTimes(1);
  });

  it('any other refusal is thrown as it came', async () => {
    const refusal = { message: 'The workflow was not started: joey-1 cannot read this chat.' };
    mocks.startAgent.mockRejectedValueOnce(refusal);
    const confirm = vi.fn(async () => true);
    await expect(createSession('/w', { confirmPublicDecisionModels: confirm })).rejects.toBe(
      refusal
    );
    expect(confirm).not.toHaveBeenCalled();
  });
});
