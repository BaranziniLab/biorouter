import { describe, expect, it } from 'vitest';
import type { DecisionModelEntry, MonitorCriterion } from '../../utils/sessionDecisions';
import {
  TIER_EFFECT_COPY,
  declassifyDecisionLines,
  privacyBasisTip,
  publicModelsList,
  publicModelsSentence,
  tierEffectNote,
} from './decisionWords';
import {
  decisionModelOptions,
  anyPublicModelRefused,
  DECISION_MODEL_OPTION_COPY,
} from './decisionModelOption';
import {
  chatModelKey,
  draftStartFields,
  isEmptyDraft,
  loweringModels,
} from './draftDecisionSettings';
import { newMonitor } from './MonitorsDialog';
import { publicDecisionQuestion } from './usePublicDecisionConfirm';
import { privacyBasisKeepsPublic } from '../settings/models/subcomponents/SwitchModelModal';
import { draftDecisionStartOptions } from '../BaseChat';

/** DR-33: the words, the picker row and the draft's arithmetic for a chat's tier. */

const entry = (fields: Partial<DecisionModelEntry>): DecisionModelEntry => ({
  provider: 'baseten_decision',
  model: 'joey-1',
  displayName: 'joey-1',
  contextTokens: 32_000,
  kinds: ['binary'],
  tier: 'public',
  adapter: null,
  local: false,
  unavailableReason: null,
  chatEffect: null,
  ...fields,
});
const joey = entry({ chatEffect: 'makesPublic' });
const kim = entry({ model: 'kim-2', displayName: 'kim-2', chatEffect: 'makesPublic' });
const tev = entry({
  provider: 'ollama_decision',
  model: 'tev1:0.8b',
  displayName: 'tev1:0.8b',
  tier: 'private',
  local: true,
});

const monitor = (fields: Partial<MonitorCriterion>): MonitorCriterion => ({
  ...newMonitor(),
  id: 'm',
  name: 'Done',
  ...fields,
});
const ref = (e: DecisionModelEntry) => ({ provider: e.provider, model: e.model, adapter: null });

describe('the line a public decision model earns', () => {
  it('names each place once, with its models before it (R3)', () => {
    expect(publicModelsSentence([joey])).toBe('joey-1 runs on Baseten');
    expect(publicModelsSentence([joey, kim, joey])).toBe('joey-1 and kim-2 run on Baseten');
    expect(TIER_EFFECT_COPY.willBePublic(publicModelsSentence([joey]))).toBe(
      'This chat will be public: joey-1 runs on Baseten.'
    );
    expect(publicModelsList([joey, kim])).toBe('joey-1 · Baseten and kim-2 · Baseten');
  });

  it('keeps its tip to two sentences and names the chat model when it is known', () => {
    const tip = TIER_EFFECT_COPY.publicInfo('gpt-5.5');
    expect(tip).toContain('so gpt-5.5 keeps answering');
    expect(tip.split('. ').length).toBe(2);
    expect(TIER_EFFECT_COPY.publicInfo(null)).toContain('its language model keeps answering');
  });

  it('a save’s answer earns a line only when it moved, or kept, the tier', () => {
    expect(tierEffectNote('none', null)).toBeNull();
    expect(tierEffectNote('becomesPublic', 'This chat will be public: x.')).toMatchObject({
      line: 'This chat will be public: x.',
    });
    expect(tierEffectNote('becomesPrivateAtNextMessage', null)).toEqual({
      line: TIER_EFFECT_COPY.becomesPrivate,
      info: TIER_EFFECT_COPY.becomesPrivateInfo,
    });
  });

  it('a workflow start asks with the models it uses, a Send with the daemon’s line', () => {
    const confirmation = {
      message: 'This chat will be public: joey-1 runs on Baseten.',
      info: null,
      models: [ref(joey)],
    };
    expect(publicDecisionQuestion(confirmation, 'workflow')).toEqual({
      title: 'Start this workflow?',
      message: 'This workflow uses joey-1 · Baseten, so this chat will be public.',
      confirmLabel: 'Start it',
    });
    expect(publicDecisionQuestion(confirmation, 'send')).toEqual({
      title: 'Make this chat public?',
      message: confirmation.message,
      confirmLabel: 'Send',
    });
  });
});

describe('a chat a decision model made public, said where the chat says its tier', () => {
  it('the pill’s tip names the monitor and where its model runs', () => {
    expect(
      privacyBasisTip({ source: 'monitor', label: 'Done', model: 'joey-1', place: 'Baseten' })
    ).toBe(
      'Public: monitor "Done" uses joey-1 · Baseten. A chat is as private as its least private model, so it can\'t use private extensions, knowledge bases or chats.'
    );
    expect(privacyBasisTip({ source: 'parent', label: 'p', model: '', place: '' })).toBe(
      "Public: it is part of a public chat's work."
    );
  });

  it('the model switch says the chat stays public, by its decision models or its parent', () => {
    expect(
      privacyBasisKeepsPublic({
        source: 'default',
        label: '',
        model: 'joey-1',
        place: 'Baseten',
        summary: '',
      })
    ).toBe('This chat stays public: its decision models include joey-1 · Baseten.');
    expect(
      privacyBasisKeepsPublic({ source: 'parent', label: 'p', model: '', place: '', summary: '' })
    ).toBe("This chat stays public: it is part of a public chat's work.");
    expect(privacyBasisKeepsPublic(null)).toBeNull();
  });

  it('the declassify dialog lists the enabled public monitors and the public default', () => {
    expect(
      declassifyDecisionLines(
        {
          defaultModel: ref(joey),
          monitors: [
            { name: 'Done', enabled: true, model: ref(joey) },
            { name: 'Local', enabled: true, model: ref(tev) },
            { name: 'Off', enabled: false, model: ref(joey) },
          ],
        },
        [joey, tev]
      )
    ).toEqual([
      'Monitor "Done" uses joey-1 · Baseten and will be turned off.',
      'The default decision model joey-1 · Baseten will be cleared.',
    ]);
    expect(declassifyDecisionLines(null, [joey])).toEqual([]);
  });
});

describe('the picker row', () => {
  it('a model whose choice makes the chat public is offered, and says so', () => {
    const rows = decisionModelOptions({ models: [joey, tev], chatTier: 'public' });
    const row = rows.find((option) => option.model?.model === 'joey-1')!;
    expect(row.reason).toBeNull();
    expect(row.makesPublic).toBe(true);
    expect(row.secondary).toBe('Baseten · Makes this chat public');
    expect(rows.find((option) => option.model?.model === 'tev1:0.8b')?.makesPublic).toBeUndefined();
  });

  it('a public row on a chat that becomes private keeps the short line; the field the tip', () => {
    const refused = entry({
      unavailableReason:
        "Public model: it can't read a private chat. Start a new chat and choose this model before its first message, or make this chat public from History.",
    });
    const row = decisionModelOptions({ models: [refused], chatTier: 'public' })[1];
    expect(row.reason).toBe(DECISION_MODEL_OPTION_COPY.publicRefused);
    expect(anyPublicModelRefused([refused], 'public')).toBe(true);
    expect(anyPublicModelRefused([joey], 'public')).toBe(false);
  });

  it('a private chat still refuses a public model, whatever the row says', () => {
    const row = decisionModelOptions({ models: [joey], chatTier: 'private' })[1];
    expect(row.reason).toBe(DECISION_MODEL_OPTION_COPY.publicRefused);
    expect(row.makesPublic).toBeUndefined();
  });
});

describe('a draft before the first message', () => {
  it('finds the models the daemon marked, through the default for a monitor with none', () => {
    expect(
      loweringModels({ defaultModel: null, monitors: [monitor({ model: ref(joey) })] }, [joey, tev])
    ).toEqual([joey]);
    expect(
      loweringModels({ defaultModel: ref(joey), monitors: [monitor({ model: null })] }, [joey])
    ).toEqual([joey]);
    expect(
      loweringModels(
        { defaultModel: null, monitors: [monitor({ model: ref(joey), enabled: false })] },
        [joey]
      )
    ).toEqual([]);
    expect(
      loweringModels({ defaultModel: null, monitors: [monitor({ model: ref(tev) })] }, [tev])
    ).toEqual([]);
  });

  it('sends the acceptance only while it names the model the chat starts on', () => {
    const settings = { defaultModel: null, monitors: [monitor({ model: ref(joey) })] };
    const draft = { settings, acceptedFor: chatModelKey('versa_azure', 'gpt-5.5') };
    expect(draftStartFields(draft, 'versa_azure/gpt-5.5')).toEqual({
      decisionSettings: settings,
      acceptPublicDecisionModels: true,
    });
    // The chat model changed after the draft: the daemon asks again at Send.
    expect(draftStartFields(draft, 'versa_azure/gpt-5.6')).toEqual({
      decisionSettings: settings,
      acceptPublicDecisionModels: false,
    });
    expect(draftStartFields(null, 'versa_azure/gpt-5.5')).toBeNull();
    expect(
      draftStartFields(
        { settings: { defaultModel: null, monitors: [] }, acceptedFor: null },
        'versa_azure/gpt-5.5'
      )
    ).toBeNull();
    expect(isEmptyDraft({ defaultModel: null, monitors: [] })).toBe(true);
  });

  it('a blank chat starts on its held model with the draft, and on the app-wide one without', () => {
    const settings = { defaultModel: null, monitors: [monitor({ model: ref(joey) })] };
    const held = { name: 'gpt-5.5', provider: 'versa_azure' };
    expect(
      draftDecisionStartOptions({ settings, acceptedFor: 'versa_azure/gpt-5.5' }, held, {
        provider: 'claude_code',
        model: 'opus',
      })
    ).toEqual({
      decisionSettings: settings,
      acceptPublicDecisionModels: true,
      provider: 'versa_azure',
      model: 'gpt-5.5',
    });
    expect(
      draftDecisionStartOptions({ settings, acceptedFor: 'versa_azure/gpt-5.5' }, null, {
        provider: 'versa_azure',
        model: 'gpt-5.5',
      })
    ).toEqual({ decisionSettings: settings, acceptPublicDecisionModels: true });
    // No draft: the start is the request it always was, held model or not.
    expect(draftDecisionStartOptions(null, held, { provider: null, model: null })).toEqual({});
  });
});
