import { beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * DR-33 on the wire: what a decision-settings write sends and reads back about the
 * chat's tier (the person's acceptance, the effect, the 409 question), the draft
 * listing's query, a session read's `privacyBasis`, and a gate write's same shape.
 */

const mocks = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn(), headers: vi.fn() }));
vi.mock('../api/client.gen', () => ({ client: { get: mocks.get, put: mocks.put } }));
vi.mock('./userAction', () => ({ userActionHeaders: mocks.headers }));

import {
  PublicDecisionConfirmationError,
  fetchDecisionCatalog,
  isPublicChatDeclined,
  parseDecisionModelEntry,
  parsePrivacyBasis,
  parsePublicDecisionConfirmation,
  PublicChatDeclinedError,
  saveDecisionSettings,
} from './sessionDecisions';
import { setPhaseGate, newGateDraft } from './phaseGates';

const ok = (data: unknown) => ({ data, error: undefined, response: { status: 200 } });
const confirmationBody = {
  code: 'needsPublicDecisionConfirmation',
  message: 'This chat will be public: joey-1 runs on Baseten.',
  info: 'A chat is as private as its least private model.',
  models: ['baseten_decision/joey-1'],
};

beforeEach(() => {
  mocks.get.mockReset();
  mocks.put.mockReset();
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'proof' });
});

describe('a decision-settings save and the chat tier', () => {
  it('sends the person’s acceptance only when asked to, beside the settings', async () => {
    mocks.put.mockResolvedValue(ok({ defaultModel: null, monitors: [], tierEffect: 'none' }));
    await saveDecisionSettings('s1', { defaultModel: null, monitors: [] });
    expect(mocks.put.mock.calls[0][0].body).toEqual({ defaultModel: null, monitors: [] });

    await saveDecisionSettings(
      's1',
      { defaultModel: null, monitors: [] },
      { acceptPublicDecisionModels: true }
    );
    expect(mocks.put.mock.calls[1][0].body).toEqual({
      defaultModel: null,
      monitors: [],
      acceptPublicDecisionModels: true,
    });
  });

  it('reads back what the save did to the tier, and the line for it', async () => {
    mocks.put.mockResolvedValue(
      ok({
        defaultModel: null,
        monitors: [],
        tierEffect: 'becomesPublic',
        tierEffectLine: 'This chat will be public: joey-1 runs on Baseten.',
      })
    );
    await expect(
      saveDecisionSettings('s1', { defaultModel: null, monitors: [] })
    ).resolves.toMatchObject({
      tierEffect: 'becomesPublic',
      tierEffectLine: 'This chat will be public: joey-1 runs on Baseten.',
    });
  });

  it('an older daemon that says nothing about the tier reads as no effect known', async () => {
    mocks.put.mockResolvedValue(ok({ defaultModel: null, monitors: [] }));
    await expect(
      saveDecisionSettings('s1', { defaultModel: null, monitors: [] })
    ).resolves.toMatchObject({ tierEffect: null, tierEffectLine: null });
  });

  it('a 409 asking for the person’s answer is a question, not a refusal', async () => {
    mocks.put.mockResolvedValue({
      data: undefined,
      error: confirmationBody,
      response: { status: 409 },
    });
    const error = await saveDecisionSettings('s1', { defaultModel: null, monitors: [] }).catch(
      (thrown: unknown) => thrown
    );
    expect(error).toBeInstanceOf(PublicDecisionConfirmationError);
    expect((error as PublicDecisionConfirmationError).confirmation).toEqual({
      message: 'This chat will be public: joey-1 runs on Baseten.',
      info: 'A chat is as private as its least private model.',
      models: [{ provider: 'baseten_decision', model: 'joey-1', adapter: null }],
    });
  });

  it('a plain-text 409 stays the daemon’s refusal', async () => {
    const sentence = "This chat has started, so it can't be made public by a decision model now.";
    mocks.put.mockResolvedValue({ data: undefined, error: sentence, response: { status: 409 } });
    const error = await saveDecisionSettings('s1', { defaultModel: null, monitors: [] }).catch(
      (thrown: unknown) => thrown
    );
    expect(error).not.toBeInstanceOf(PublicDecisionConfirmationError);
    expect((error as Error).message).toBe(sentence);
  });
});

describe('the parsers', () => {
  it('reads a row’s chatEffect, and nothing else as one', () => {
    const row = { provider: 'baseten_decision', model: 'joey-1', tier: 'public' };
    expect(parseDecisionModelEntry({ ...row, chatEffect: 'makesPublic' })?.chatEffect).toBe(
      'makesPublic'
    );
    expect(parseDecisionModelEntry({ ...row, chatEffect: 'somethingNew' })?.chatEffect).toBeNull();
    expect(parseDecisionModelEntry(row)?.chatEffect).toBeNull();
  });

  it('a confirmation needs its code', () => {
    expect(parsePublicDecisionConfirmation({ ...confirmationBody, code: 'other' })).toBeNull();
    expect(parsePublicDecisionConfirmation('This chat will be public')).toBeNull();
    expect(parsePublicDecisionConfirmation(confirmationBody)?.message).toBe(
      confirmationBody.message
    );
  });

  it('reads a session’s privacyBasis, and none when it is absent or unknown', () => {
    expect(
      parsePrivacyBasis({
        id: 's1',
        privacyBasis: {
          kind: 'decisionModel',
          source: 'monitor',
          label: 'Done',
          model: 'joey-1',
          host: { kind: 'baseten' },
          place: 'Baseten',
          summary: 'monitor "Done" uses joey-1, a Baseten model',
        },
      })
    ).toEqual({
      source: 'monitor',
      label: 'Done',
      model: 'joey-1',
      place: 'Baseten',
      summary: 'monitor "Done" uses joey-1, a Baseten model',
    });
    expect(parsePrivacyBasis({ id: 's1' })).toBeNull();
    expect(parsePrivacyBasis({ privacyBasis: { source: 'somewhere' } })).toBeNull();
  });

  it('a declined start reads as declined, by instance or by name', () => {
    expect(isPublicChatDeclined(new PublicChatDeclinedError())).toBe(true);
    expect(isPublicChatDeclined({ name: 'PublicChatDeclinedError' })).toBe(true);
    expect(isPublicChatDeclined(new Error('offline'))).toBe(false);
  });
});

describe('the draft listing', () => {
  it('names the model a chat not started yet will run on, and no session', async () => {
    mocks.get.mockResolvedValue(ok({ models: [] }));
    await fetchDecisionCatalog(undefined, { boundProvider: 'versa_azure', boundModel: 'gpt-5.5' });
    expect(mocks.get.mock.calls[0][0].query).toEqual({
      boundProvider: 'versa_azure',
      boundModel: 'gpt-5.5',
    });
    await fetchDecisionCatalog(undefined, 's1');
    expect(mocks.get.mock.calls[1][0].query).toEqual({ sessionId: 's1' });
  });
});

describe('a gate write and the chat tier', () => {
  const draft = () => ({
    ...newGateDraft(),
    questions: [{ name: 'q1', question: 'Is it done?' }],
    model: { provider: 'baseten_decision', model: 'joey-1', adapter: null },
  });

  it('sends the acceptance only when asked, and reads the effect back', async () => {
    mocks.put.mockResolvedValue(
      ok({
        applied: true,
        message: 'The gate is yours now.',
        tierEffect: 'becomesPublic',
        tierEffectLine: 'This chat will be public: joey-1 runs on Baseten.',
      })
    );
    const result = await setPhaseGate('s1', 'p1', draft(), { acceptPublicDecisionModels: true });
    expect(mocks.put.mock.calls[0][0].body).toMatchObject({ acceptPublicDecisionModels: true });
    expect(result).toMatchObject({ ok: true, tierEffect: 'becomesPublic' });

    await setPhaseGate('s1', 'p1', draft());
    expect(mocks.put.mock.calls[1][0].body).not.toHaveProperty('acceptPublicDecisionModels');
  });

  it('a 409 question comes back as the confirmation, not as a refusal', async () => {
    mocks.put.mockResolvedValue({
      data: undefined,
      error: confirmationBody,
      response: { status: 409 },
    });
    const result = await setPhaseGate('s1', 'p1', draft());
    expect(result).toMatchObject({
      ok: false,
      status: 409,
      message: confirmationBody.message,
      confirmation: { message: confirmationBody.message },
    });
  });
});
