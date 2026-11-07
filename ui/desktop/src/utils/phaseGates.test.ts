import { describe, expect, it } from 'vitest';
import { gateDraftFrom, gateModelPick, parseGateModel, phaseGateDraftToWire } from './phaseGates';
import { parsePhaseGateSnapshot } from './sessionDecisions';

/** A gate as the daemon's snapshot sends it, naming `model` and its adapter. */
function gate(fields: Record<string, unknown>) {
  return parsePhaseGateSnapshot({
    threshold: 0.8,
    attempts: 0,
    maxAttempts: 3,
    owner: 'workflow',
    questions: { cited: 'Is every claim cited?' },
    ...fields,
  })!;
}

describe('a gate written back from the editor', () => {
  it('keeps the adapter its model is asked through when only a question changes', () => {
    // A gate declared with `model: {provider, model, adapter: logprob}` (a
    // workflow, the agent tool or the route): Save replaces the whole gate, so
    // the adapter must ride the PUT or the gate moves to the model's default.
    const draft = gateDraftFrom(
      gate({ model: 'ollama_decision/tev1:0.8b', modelAdapter: 'logprob' })
    );
    draft.questions[0] = { ...draft.questions[0], question: 'Is every claim cited twice?' };
    expect(phaseGateDraftToWire(draft)).toEqual({
      questions: { cited: 'Is every claim cited twice?' },
      threshold: 0.8,
      maxAttempts: 3,
      model: { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: 'logprob' },
      allowUncalibrated: false,
    });
  });

  it('sends no adapter for a gate whose model names none, and no model for one that names none', () => {
    expect(
      phaseGateDraftToWire(gateDraftFrom(gate({ model: 'ollama_decision/tev1:0.8b' }))).model
    ).toEqual({ provider: 'ollama_decision', model: 'tev1:0.8b' });
    // An adapter with no model names nothing (an older or odd snapshot).
    expect(phaseGateDraftToWire(gateDraftFrom(gate({ modelAdapter: 'logprob' }))).model).toBeNull();
  });

  it('reads a model label with its adapter, the model part keeping its own `/`', () => {
    expect(parseGateModel('baseten_decision/decision-models/joey', 'system_one')).toEqual({
      provider: 'baseten_decision',
      model: 'decision-models/joey',
      adapter: 'system_one',
    });
    expect(parseGateModel('ollama_decision/tev1:0.8b')).toEqual({
      provider: 'ollama_decision',
      model: 'tev1:0.8b',
      adapter: null,
    });
    expect(parseGateModel('no-slash', 'logprob')).toBeNull();
  });
});

describe('a pick in the editor’s model list', () => {
  const opened = { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: 'logprob' as const };

  it('stands for the gate’s own model exactly when it is that model’s row', () => {
    expect(
      gateModelPick({ provider: 'ollama_decision', model: 'tev1:0.8b', adapter: null }, opened)
    ).toBe(opened);
  });

  it('is any other model on its own adapter, or the chat’s model', () => {
    const other = { provider: 'ollama_decision', model: 'clef-flash', adapter: null };
    expect(gateModelPick(other, opened)).toBe(other);
    expect(gateModelPick(null, opened)).toBeNull();
    const plain = { provider: 'ollama_decision', model: 'tev1:0.8b', adapter: null };
    expect(gateModelPick(plain, null)).toBe(plain);
  });
});
