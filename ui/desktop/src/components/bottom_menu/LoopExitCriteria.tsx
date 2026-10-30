import { useEffect, useId, useState } from 'react';
import {
  getLoopExitCriteria,
  getBasetenDecisionModels,
  getProviderModels,
  providers,
  setLoopExitCriteria,
  type ExitCriteria,
  type ProviderDetails,
  type SessionClassification,
} from '../../api';
import { getDraftExitCriteria, setDraftExitCriteria } from '../../store/loopExitCriteria';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { Select } from '../ui/Select';
import { Textarea } from '../ui/textarea';

type DecisionKind = ExitCriteria['decision']['type'];
type EvaluationTiming = NonNullable<ExitCriteria['timing']>;
type Option = { value: string; label: string };

function optionValue(option: unknown): string {
  return (option as Option | null)?.value ?? '';
}

export function LoopExitCriteria({
  sessionId,
  draftScope,
  privacyTier,
}: {
  sessionId: string | null;
  draftScope: string;
  privacyTier?: SessionClassification;
}) {
  const id = useId();
  const [open, setOpen] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loaded, setLoaded] = useState(false);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [enabled, setEnabled] = useState(false);
  const [timing, setTiming] = useState<EvaluationTiming>('post_response');
  const [prompt, setPrompt] = useState(
    'Is the agent still making useful progress toward the user’s request?'
  );
  const [provider, setProvider] = useState('');
  const [model, setModel] = useState('');
  const [kind, setKind] = useState<DecisionKind>('boolean');
  const [interruptBoolean, setInterruptBoolean] = useState('false');
  const [noulThreshold, setNoulThreshold] = useState('0.5');
  const [rubric, setRubric] = useState(
    'No useful progress\nLimited progress\nModerate progress\nGood progress\nExcellent progress'
  );
  const [choices, setChoices] = useState('continue\nneeds_adjustment');
  const [interruptChoice, setInterruptChoice] = useState('needs_adjustment');
  const [min, setMin] = useState('0');
  const [max, setMax] = useState('10');
  const [threshold, setThreshold] = useState('5');
  const [available, setAvailable] = useState<ProviderDetails[]>([]);
  const [models, setModels] = useState<Option[]>([]);
  const [modelError, setModelError] = useState('');
  const [modelsLoading, setModelsLoading] = useState(false);
  const [modelListRetry, setModelListRetry] = useState(0);

  useEffect(() => {
    if (!open) return;
    let current = true;
    setLoading(true);
    setLoaded(false);
    setError('');
    const load = async () => {
      try {
        const [settings, catalog] = await Promise.all([
          sessionId
            ? getLoopExitCriteria({ path: { session_id: sessionId }, throwOnError: true }).then(
                (result) => result.data?.criteria ?? null
              )
            : Promise.resolve(getDraftExitCriteria(draftScope)),
          providers({ throwOnError: true }),
        ]);
        if (!current) return;
        setAvailable((catalog.data ?? []).filter((p) => p.is_configured));
        setEnabled(settings !== null);
        setTiming(settings?.timing ?? 'post_response');
        setProvider(settings?.provider ?? '');
        setModel(settings?.model ?? '');
        if (settings) {
          setPrompt(settings.prompt);
          setKind(settings.decision.type);
          switch (settings.decision.type) {
            case 'noul':
              setNoulThreshold(String(settings.decision.threshold));
              setInterruptBoolean(String(settings.decision.interrupt_when));
              break;
            case 'boolean':
              if (settings.provider === 'baseten') {
                setKind('noul');
                setNoulThreshold('0.5');
              }
              setInterruptBoolean(String(settings.decision.interrupt_when));
              break;
            case 'choice':
              setChoices(settings.decision.choices.join('\n'));
              setInterruptChoice(settings.decision.interrupt_when);
              break;
            case 'ranking':
              setMin(String(settings.decision.min));
              setMax(String(settings.decision.max));
              setThreshold(String(settings.decision.interrupt_below));
              if (settings.decision.levels?.length) setRubric(settings.decision.levels.join('\n'));
              break;
          }
        }
        setLoaded(true);
      } catch (cause) {
        if (current) setError(`Could not load loop exit criteria: ${String(cause)}`);
      } finally {
        if (current) setLoading(false);
      }
    };
    void load();
    return () => {
      current = false;
    };
  }, [open, sessionId, draftScope]);

  useEffect(() => {
    if (!open || !provider) return;
    let current = true;
    setModels([]);
    setModelError('');
    setModelsLoading(true);
    void (async () => {
      try {
        if (provider === 'baseten') {
          const result = await getBasetenDecisionModels();
          const status = result.response?.status;
          if (status === 404) {
            throw new Error(
              'The connected background service is older than this interface (HTTP 404). Closing Electron can leave that service running. Restart the background service, then retry the model list.'
            );
          }
          if (status === 401 || status === 403) {
            throw new Error(
              `The background service refused catalog access (HTTP ${status}). Reconnect the app, then retry the model list.`
            );
          }
          if (!result.response?.ok || !Array.isArray(result.data) || result.data.length === 0) {
            throw new Error(
              `Could not load the Baseten decision catalog${status ? ` (HTTP ${status})` : ''}. Check the backend connection and retry the model list.`
            );
          }
          if (current)
            setModels((result.data ?? []).map((m) => ({ value: m.slug, label: m.name })));
        } else {
          const known =
            available.find((p) => p.name === provider)?.metadata.known_models.map((m) => m.name) ??
            [];
          if (current) setModels(known.map((value) => ({ value, label: value })));
          const result = await getProviderModels({ path: { name: provider }, throwOnError: true });
          if (current)
            setModels(
              [...new Set([...known, ...(result.data ?? [])])].map((value) => ({
                value,
                label: value,
              }))
            );
        }
      } catch (cause) {
        if (current)
          setModelError(
            provider === 'baseten'
              ? cause instanceof Error
                ? cause.message
                : 'Could not load the Baseten decision catalog. Check the backend connection and retry the model list.'
              : 'Could not fetch the live model list. Select a known model or enter its exact ID.'
          );
      } finally {
        if (current) setModelsLoading(false);
      }
    })();
    return () => {
      current = false;
    };
  }, [open, provider, available, modelListRetry]);

  const choiceValues = choices
    .split('\n')
    .map((value) => value.trim())
    .filter(Boolean);
  const providerOptions = ['public', 'private'].map((tier) => ({
    label: tier === 'public' ? 'Public' : 'Private',
    options: available
      .filter((p) => p.resolved_tier === tier)
      .map((p) => ({
        value: p.name,
        label: p.metadata.display_name,
        isDisabled: privacyTier === 'private' && tier === 'public',
      })),
  }));
  const selectedProvider = available.find((p) => p.name === provider);
  const unresolved = Boolean(provider && !selectedProvider?.resolved_tier);

  async function save() {
    setError('');
    let criteria: ExitCriteria | null = null;
    if (enabled) {
      if (!prompt.trim() || !provider || !model.trim() || unresolved) {
        setError(
          'Enter a prompt and select a configured decision provider with a resolved privacy label and a model ID.'
        );
        return;
      }
      if (
        provider === 'baseten' &&
        !models.some((option) => option.value === model || option.label === model)
      ) {
        setError('Select elephant-1, mammoth-1, or joey-1 from the Baseten decision catalog.');
        return;
      }
      let decision: ExitCriteria['decision'];
      if (kind === 'noul') {
        const probability = Number(noulThreshold);
        if (
          !noulThreshold.trim() ||
          !Number.isFinite(probability) ||
          probability < 0 ||
          probability > 1
        ) {
          setError('Enter a P(yes) threshold between 0 and 1.');
          return;
        }
        decision = {
          type: 'noul',
          threshold: probability,
          interrupt_when: interruptBoolean === 'true',
        };
      } else if (kind === 'boolean')
        decision = { type: 'boolean', interrupt_when: interruptBoolean === 'true' };
      else if (kind === 'choice') {
        if (
          choiceValues.length < 2 ||
          choiceValues.length > 20 ||
          new Set(choiceValues).size !== choiceValues.length ||
          !choiceValues.includes(interruptChoice)
        ) {
          setError('Enter 2–20 unique choices and select one that interrupts the loop.');
          return;
        }
        decision = { type: 'choice', choices: choiceValues, interrupt_when: interruptChoice };
      } else {
        const levels =
          provider === 'baseten'
            ? rubric
                .split('\n')
                .map((level) => level.trim())
                .filter(Boolean)
            : [];
        if (
          provider === 'baseten' &&
          (levels.length < 2 || levels.length > 10 || new Set(levels).size !== levels.length)
        ) {
          setError('Enter 2–10 distinct rubric levels, ordered from lowest to highest.');
          return;
        }
        const low = provider === 'baseten' ? 0 : Number(min),
          high = provider === 'baseten' ? levels.length - 1 : Number(max),
          cutoff = Number(threshold);
        if (
          !min.trim() ||
          !max.trim() ||
          !threshold.trim() ||
          ![low, high, cutoff].every(Number.isFinite) ||
          low >= high ||
          cutoff < low ||
          cutoff > high
        ) {
          setError('Enter an increasing numeric range and a threshold within that range.');
          return;
        }
        decision = {
          type: 'ranking',
          min: low,
          max: high,
          interrupt_below: cutoff,
          ...(levels.length ? { levels } : {}),
        };
      }
      criteria = { prompt: prompt.trim(), provider, model: model.trim(), decision, timing };
    }
    setSaving(true);
    try {
      if (sessionId)
        await setLoopExitCriteria({
          path: { session_id: sessionId },
          body: { criteria },
          throwOnError: true,
        });
      else setDraftExitCriteria(draftScope, criteria);
      setOpen(false);
    } catch (cause) {
      setError(`Could not save loop exit criteria: ${String(cause)}`);
    } finally {
      setSaving(false);
    }
  }

  const field = (name: string) => `${id}-${name}`;
  const select = (
    name: string,
    value: string,
    options: Option[],
    change: (v: string) => void,
    optionsLoading = false
  ) => (
    <Select
      inputId={field(name)}
      isLoading={optionsLoading}
      isDisabled={!enabled || saving || optionsLoading}
      value={options.find((o) => o.value === value) ?? null}
      options={options}
      onChange={(option) => change(optionValue(option))}
      menuPortalTarget={document.body}
    />
  );

  return (
    <>
      <button
        type="button"
        className="h-7 rounded-md px-1 text-supporting text-text-muted hover:text-text-default"
        onClick={() => setOpen(true)}
        title="Choose a pre-send or post-response decision check"
      >
        Exit criteria
      </button>
      <ModalShell
        open={open}
        onOpenChange={setOpen}
        size="lg"
        purpose={saving ? 'required' : 'form'}
        title="Loop exit criteria"
        scrollBody
        subtitle="Choose when to evaluate and pause for adjustments when a decision fails."
        footer={
          <>
            <Button variant="secondary" onClick={() => setOpen(false)} disabled={saving}>
              Cancel
            </Button>
            <Button
              onClick={() => void save()}
              disabled={
                !loaded || loading || saving || (enabled && provider === 'baseten' && modelsLoading)
              }
            >
              {saving ? 'Saving…' : 'Save criteria'}
            </Button>
          </>
        }
      >
        <div className="flex flex-col gap-4">
          {loading ? (
            <p role="status">Loading criteria and configured models…</p>
          ) : (
            <>
              <label className="flex items-center gap-2 text-body">
                <input
                  type="checkbox"
                  checked={enabled}
                  onChange={(e) => setEnabled(e.target.checked)}
                />
                Evaluate each agentic iteration
              </label>
              <fieldset
                disabled={!enabled || saving}
                className="flex min-w-0 flex-col gap-3 disabled:opacity-50"
              >
                <label htmlFor={field('timing')} className="text-label">
                  Evaluation timing
                </label>
                {select(
                  'timing',
                  timing,
                  [
                    { value: 'pre_send', label: 'Pre-send' },
                    { value: 'post_response', label: 'Post-response' },
                  ],
                  (value) => setTiming(value as EvaluationTiming)
                )}
                <p className="text-supporting text-text-muted">
                  {timing === 'pre_send'
                    ? 'Before each main agent-model request. A failed check prevents that request from starting. The evaluator receives the input excerpt first; preparation and other helper calls are not covered.'
                    : 'After each completed iteration, including its response and tools. A failed check prevents the loop from continuing; streamed output and tool effects are not undone.'}
                </p>
                <label htmlFor={field('prompt')} className="text-label">
                  Evaluation prompt
                </label>
                <Textarea
                  id={field('prompt')}
                  value={prompt}
                  maxLength={8000}
                  onChange={(e) => setPrompt(e.target.value)}
                />
                <label htmlFor={field('provider')} className="text-label">
                  Decision provider · Public / Private
                </label>
                <Select
                  inputId={field('provider')}
                  value={
                    selectedProvider
                      ? { value: provider, label: selectedProvider.metadata.display_name }
                      : null
                  }
                  options={providerOptions}
                  onChange={(option) => {
                    const next = optionValue(option);
                    setProvider(next);
                    setModel('');
                    if (next === 'baseten') {
                      setKind('noul');
                      setThreshold('2');
                    }
                  }}
                  isDisabled={!enabled || saving}
                  menuPortalTarget={document.body}
                />
                {privacyTier === 'private' && (
                  <p className="text-supporting text-text-muted">
                    Public evaluators cannot receive this private chat.
                  </p>
                )}
                {unresolved && (
                  <p role="alert">
                    This provider’s privacy label could not be resolved. Select another provider.
                  </p>
                )}
                <label htmlFor={field('known-model')} className="text-label">
                  Available models
                </label>
                {select('known-model', model, models, setModel, modelsLoading)}
                <label htmlFor={field('model')} className="text-label">
                  Decision model ID
                </label>
                <Input
                  id={field('model')}
                  machine
                  readOnly={provider === 'baseten'}
                  value={model}
                  onChange={(e) => setModel(e.target.value)}
                  placeholder="e.g. elephant-1, mammoth-1, joey-1"
                />
                {modelError && (
                  <div className="flex flex-col items-start gap-2">
                    <p role="status" className="text-supporting text-text-muted">
                      {modelError}
                    </p>
                    <Button
                      type="button"
                      variant="secondary"
                      size="sm"
                      onClick={() => setModelListRetry((retry) => retry + 1)}
                      disabled={modelsLoading}
                    >
                      Retry model list
                    </Button>
                  </div>
                )}
                <label htmlFor={field('type')} className="text-label">
                  Decision type
                </label>
                {select(
                  'type',
                  kind,
                  [
                    { value: 'noul', label: 'Noul · probability of yes' },
                    ...(provider === 'baseten'
                      ? []
                      : [{ value: 'boolean', label: 'Boolean · pass/fail' }]),
                    { value: 'choice', label: 'Choice' },
                    {
                      value: 'ranking',
                      label:
                        provider === 'baseten'
                          ? 'Ranking · ordered rubric'
                          : 'Ranking · numeric score',
                    },
                  ],
                  (value) => setKind(value as DecisionKind)
                )}
                {kind === 'noul' && (
                  <>
                    <label htmlFor={field('noul-threshold')} className="text-label">
                      P(yes) threshold
                    </label>
                    <Input
                      id={field('noul-threshold')}
                      type="number"
                      min="0"
                      max="1"
                      step="0.01"
                      value={noulThreshold}
                      onChange={(e) => setNoulThreshold(e.target.value)}
                    />
                    <label htmlFor={field('noul-direction')} className="text-label">
                      Interrupt when P(yes) is
                    </label>
                    {select(
                      'noul-direction',
                      interruptBoolean,
                      [
                        { value: 'true', label: 'At or above threshold' },
                        { value: 'false', label: 'Below threshold' },
                      ],
                      setInterruptBoolean
                    )}
                  </>
                )}
                {kind === 'boolean' && (
                  <>
                    <label htmlFor={field('boolean')} className="text-label">
                      Interrupt when the result is
                    </label>
                    {select(
                      'boolean',
                      interruptBoolean,
                      [
                        { value: 'false', label: 'False' },
                        { value: 'true', label: 'True' },
                      ],
                      setInterruptBoolean
                    )}
                  </>
                )}
                {kind === 'choice' && (
                  <>
                    <label htmlFor={field('choices')} className="text-label">
                      Choices · one per line
                    </label>
                    <Textarea
                      id={field('choices')}
                      value={choices}
                      onChange={(e) => setChoices(e.target.value)}
                    />
                    <label htmlFor={field('interrupt-choice')} className="text-label">
                      Interrupt on this choice
                    </label>
                    {select(
                      'interrupt-choice',
                      interruptChoice,
                      choiceValues.map((value) => ({ value, label: value })),
                      setInterruptChoice
                    )}
                  </>
                )}
                {kind === 'ranking' &&
                  (provider === 'baseten' ? (
                    <>
                      <label htmlFor={field('rubric')} className="text-label">
                        Ordered rubric · one level per line
                      </label>
                      <Textarea
                        id={field('rubric')}
                        value={rubric}
                        onChange={(e) => setRubric(e.target.value)}
                      />
                      <p className="text-supporting text-text-muted">
                        Use 2–10 levels, lowest first. The first level scores 0; fractional scores
                        are possible.
                      </p>
                      <label htmlFor={field('score-threshold')} className="text-label">
                        Interrupt below
                      </label>
                      <Input
                        id={field('score-threshold')}
                        type="number"
                        step="any"
                        value={threshold}
                        onChange={(e) => setThreshold(e.target.value)}
                      />
                    </>
                  ) : (
                    <div className="grid grid-cols-3 gap-3">
                      <label className="text-label">
                        Minimum
                        <Input
                          type="number"
                          step="any"
                          value={min}
                          onChange={(e) => setMin(e.target.value)}
                        />
                      </label>
                      <label className="text-label">
                        Maximum
                        <Input
                          type="number"
                          step="any"
                          value={max}
                          onChange={(e) => setMax(e.target.value)}
                        />
                      </label>
                      <label className="text-label">
                        Interrupt below
                        <Input
                          type="number"
                          step="any"
                          value={threshold}
                          onChange={(e) => setThreshold(e.target.value)}
                        />
                      </label>
                    </div>
                  ))}
              </fieldset>
              {provider === 'baseten' && (
                <p className="text-supporting text-text-muted">
                  Baseten decision models use System One, not chat completions. This is a public
                  evaluator: do not send PHI or controlled data. Pre-send still sends the excerpt to
                  this evaluator; neither timing option can undo an earlier disclosure.
                </p>
              )}
              <p className="text-supporting text-text-muted">
                Adds one decision-model call at the selected boundary using a bounded excerpt,
                including available tool results. Evaluator errors also pause the loop. Saved
                changes apply at the next matching boundary; they do not restart a paused loop.
              </p>
              {!sessionId && (
                <p className="text-supporting text-text-muted">
                  These criteria will be attached before the first message of this new chat is sent.
                </p>
              )}
            </>
          )}
          {error && (
            <p role="alert" className="text-supporting text-text-danger">
              {error}
            </p>
          )}
        </div>
      </ModalShell>
    </>
  );
}
