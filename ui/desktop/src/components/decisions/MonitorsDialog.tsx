import { useEffect, useId, useMemo, useState } from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { Field } from '../ui/field';
import { Input } from '../ui/input';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { Slider } from '../ui/slider';
import { Switch } from '../ui/switch';
import { Textarea } from '../ui/textarea';
import { Pencil, Plus, Trash2, X } from '../icons/app-icons';
import {
  DEFAULT_MONITOR_WINDOW,
  MAX_MONITORS,
  MONITOR_NAME_MAX,
  decisionModelLabel,
  fetchDecisionCatalog,
  fetchDecisionSettings,
  formatProbability,
  refusalText,
  saveDecisionSettings,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionSettings,
  type DecisionTier,
  type MonitorAction,
  type MonitorCriterion,
  type MonitorTiming,
} from '../../utils/sessionDecisions';
import './decisionsPanel.css';

export const MONITORS_COPY = {
  title: 'Monitors',
  subtitle:
    'A decision model reads the newest part of this chat and acts when its answer reaches the threshold.',
  empty: 'No monitors yet.',
  add: 'Add a monitor',
  limit: `A chat can have at most ${MAX_MONITORS} monitors.`,
  edit: (name: string) => `Edit ${name}`,
  remove: (name: string) => `Remove ${name}`,
  enabled: (name: string) => `${name} on`,
  name: 'Name',
  nameRequired: 'Give the monitor a name.',
  nameTooLong: `Keep the name to ${MONITOR_NAME_MAX} characters.`,
  question: 'Question',
  questionHelp:
    'Asked of the newest messages and tool results, for example "Has the form been submitted?"',
  questionRequired: 'Write the question the model answers.',
  answerType: 'Answers',
  yesNo: 'Yes or no',
  choice: 'One of several options',
  options: 'Options',
  optionName: (n: number) => `Option ${n}`,
  addOption: 'Add an option',
  removeOption: (name: string) => `Remove option ${name || 'without a name'}`,
  optionsTooFew: 'A choice needs at least two options.',
  optionsDuplicate: 'Each option needs its own name.',
  firesOn: 'Fires on',
  firesOnHelp: 'The option whose probability is compared with the threshold.',
  firesOnMissing: 'Choose the option that fires the monitor.',
  model: 'Model',
  chatDefault: "The chat's decision model",
  modelPublicRefused: 'This chat is private, so a public model may not read it.',
  modelHelp: 'Only a private model may read a private chat.',
  threshold: 'Fires at',
  thresholdHelp: 'P(yes), or P(option), at or above this.',
  action: 'Then',
  actions: { stop: 'Stop the turn', notify: 'Notify me', steer: 'Tell the agent' } as Record<
    MonitorAction,
    string
  >,
  timing: 'Checks',
  timings: { post_iteration: 'After each step', turn_end: 'When the turn ends' } as Record<
    MonitorTiming,
    string
  >,
  on: 'On',
  done: 'Done',
  cancel: 'Cancel',
  save: 'Save',
  saving: 'Saving',
  saveFailed: 'Could not save the monitors.',
  loading: 'Reading the monitors…',
  loadFailed: "Could not read this chat's monitors. Close and open again to retry.",
  private: 'Private',
  public: 'Public',
  local: 'this computer',
};

const copy = MONITORS_COPY;

/** The threshold's stops: 0.50 to 0.95 by 0.05, then 0.99. */
export const THRESHOLD_STEPS = [
  ...Array.from({ length: 10 }, (_, i) => (0.5 + i * 0.05).toFixed(2)),
  '0.99',
].map((value) => ({ value, label: value }));

function nearestStep(p: number): string {
  let best = THRESHOLD_STEPS[0].value;
  for (const step of THRESHOLD_STEPS) {
    if (Math.abs(Number(step.value) - p) < Math.abs(Number(best) - p)) best = step.value;
  }
  return best;
}

/** `Form reached` -> `form-reached`, unique among `taken`. */
export function monitorSlug(name: string, taken: ReadonlySet<string>): string {
  const base =
    name
      .toLowerCase()
      .normalize('NFKD')
      .replace(/[^a-z0-9]+/g, '-')
      .replace(/^-+|-+$/g, '')
      .slice(0, 40) || 'monitor';
  if (!taken.has(base)) return base;
  for (let n = 2; ; n += 1) {
    const candidate = `${base}-${n}`;
    if (!taken.has(candidate)) return candidate;
  }
}

export function newMonitor(): MonitorCriterion {
  return {
    id: '',
    name: '',
    question: { instructions: '', kind: { type: 'binary' }, threshold: null },
    model: null,
    timing: 'post_iteration',
    window: { ...DEFAULT_MONITOR_WINDOW },
    threshold: 0.85,
    option: null,
    consecutive: 1,
    rearmBelow: null,
    action: 'notify',
    onError: 'notify',
    maxEvalsPerTurn: 50,
    enabled: true,
  };
}

export interface MonitorErrors {
  name?: string;
  question?: string;
  options?: string;
  option?: string;
  model?: string;
}

function modelEntryFor(
  model: DecisionModelRef | null,
  models: readonly DecisionModelEntry[]
): DecisionModelEntry | undefined {
  if (!model) return undefined;
  return models.find(
    (entry) =>
      entry.provider === model.provider &&
      (entry.model === model.model || entry.displayName === model.model)
  );
}

/**
 * What the daemon would refuse in one monitor (`settings.rs`'s `validate`), said
 * by the field it belongs to, and what every check of it would refuse: a model
 * the chat may not use, or, with `chatRefusal` (a Crew grant on the chat), any
 * model at all, the chat's decision model included. Empty when the monitor can
 * be saved.
 */
export function validateMonitor(
  monitor: MonitorCriterion,
  context: {
    chatTier: DecisionTier | null;
    models: readonly DecisionModelEntry[];
    chatRefusal?: string | null;
  }
): MonitorErrors {
  const errors: MonitorErrors = {};
  const name = monitor.name.trim();
  if (!name) errors.name = copy.nameRequired;
  else if (name.length > MONITOR_NAME_MAX) errors.name = copy.nameTooLong;
  if (!monitor.question.instructions.trim()) errors.question = copy.questionRequired;
  if (monitor.question.kind.type === 'choice') {
    const names = monitor.question.kind.options.map((option) => option.name.trim());
    if (names.filter(Boolean).length < 2) errors.options = copy.optionsTooFew;
    else if (
      names.some((n) => !n) ||
      new Set(names.map((n) => n.toLowerCase())).size !== names.length
    )
      errors.options = copy.optionsDuplicate;
    if (!monitor.option || !names.includes(monitor.option)) errors.option = copy.firesOnMissing;
  }
  const entry = modelEntryFor(monitor.model, context.models);
  if (context.chatRefusal) errors.model = context.chatRefusal;
  else if (entry?.unavailableReason) errors.model = entry.unavailableReason;
  else if (context.chatTier === 'private' && entry?.tier === 'public') {
    errors.model = copy.modelPublicRefused;
  }
  return errors;
}

function hasErrors(errors: MonitorErrors): boolean {
  return Object.values(errors).some(Boolean);
}

/** One line under a monitor's name: its threshold, action, timing and model. */
export function monitorSummary(monitor: MonitorCriterion): string {
  const target =
    monitor.question.kind.type === 'choice' && monitor.option ? `P(${monitor.option})` : 'P(yes)';
  return [
    `${target} ≥ ${formatProbability(monitor.threshold)}`,
    copy.actions[monitor.action],
    copy.timings[monitor.timing],
    decisionModelLabel(monitor.model) ?? copy.chatDefault,
  ].join(' · ');
}

interface ModelOption {
  value: string;
  label: string;
  model: DecisionModelRef | null;
  reason: string | null;
}

const menuPortalTarget = typeof document === 'undefined' ? undefined : document.body;

function modelKey(model: DecisionModelRef | null): string {
  return model ? `${model.provider}/${model.model}` : '';
}

function MonitorEditor({
  draft,
  onChange,
  errors,
  models,
  chatTier,
  chatRefusal,
  readOnly,
}: {
  draft: MonitorCriterion;
  onChange: (next: MonitorCriterion) => void;
  errors: MonitorErrors;
  models: readonly DecisionModelEntry[];
  chatTier: DecisionTier | null;
  /** Why this chat may use no decision model at all (a Crew grant), or `null`. */
  chatRefusal: string | null;
  readOnly: boolean;
}) {
  const base = useId();
  const kind = draft.question.kind;
  const options: ModelOption[] = [
    // No row stands for the chat's decision model, so the chat-wide reason is its own.
    { value: '', label: copy.chatDefault, model: null, reason: chatRefusal },
    ...models.map((entry) => {
      const refused = chatTier === 'private' && entry.tier === 'public';
      return {
        value: modelKey({ provider: entry.provider, model: entry.model, adapter: null }),
        label: [
          entry.displayName,
          entry.tier === 'private' ? copy.private : entry.tier === 'public' ? copy.public : null,
          entry.local ? copy.local : null,
        ]
          .filter(Boolean)
          .join(' · '),
        model: { provider: entry.provider, model: entry.model, adapter: null },
        // The daemon's own reason first (tier, or an institution the model does not
        // cover); the tier rule here only when it gave none.
        reason: entry.unavailableReason ?? (refused ? copy.modelPublicRefused : null),
      };
    }),
  ];
  const selectedKey = modelKey(
    draft.model
      ? {
          provider: draft.model.provider,
          model: modelEntryFor(draft.model, models)?.model ?? draft.model.model,
          adapter: null,
        }
      : null
  );
  if (draft.model && !options.some((option) => option.value === selectedKey)) {
    options.push({
      value: selectedKey,
      label: decisionModelLabel(draft.model) ?? selectedKey,
      model: draft.model,
      reason: chatRefusal,
    });
  }
  const selected = options.find((option) => option.value === selectedKey) ?? options[0];

  const setKind = (type: 'binary' | 'choice') => {
    if (type === kind.type) return;
    onChange({
      ...draft,
      question: {
        ...draft.question,
        kind:
          type === 'binary'
            ? { type: 'binary' }
            : {
                type: 'choice',
                options: [
                  { name: '', description: null },
                  { name: '', description: null },
                ],
              },
      },
      option: null,
    });
  };
  const choiceOptions = kind.type === 'choice' ? kind.options : [];
  const setOption = (index: number, name: string) => {
    if (kind.type !== 'choice') return;
    const previous = kind.options[index]?.name;
    const next = kind.options.map((option, i) => (i === index ? { ...option, name } : option));
    onChange({
      ...draft,
      question: { ...draft.question, kind: { type: 'choice', options: next } },
      option: draft.option === previous ? name : draft.option,
    });
  };

  return (
    // A filled box, so the fields in it rest one step back on the page ground:
    // a fill never sits on a fill (design.md 9.11).
    <div className="br-monitors__editor br-fill-surface" data-testid="monitor-editor">
      <Field id={`${base}-name`} label={copy.name} error={errors.name} required>
        <Input
          value={draft.name}
          maxLength={MONITOR_NAME_MAX + 20}
          disabled={readOnly}
          data-testid="monitor-name"
          onChange={(event) => onChange({ ...draft, name: event.target.value })}
        />
      </Field>
      <Field
        id={`${base}-question`}
        label={copy.question}
        helper={copy.questionHelp}
        error={errors.question}
        required
      >
        <Textarea
          rows={2}
          value={draft.question.instructions}
          disabled={readOnly}
          data-testid="monitor-question"
          onChange={(event) =>
            onChange({
              ...draft,
              question: { ...draft.question, instructions: event.target.value },
            })
          }
        />
      </Field>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-kind`}>
          {copy.answerType}
        </span>
        <SegmentedControl
          aria-labelledby={`${base}-kind`}
          value={kind.type === 'choice' ? 'choice' : 'binary'}
          disabled={readOnly}
          data-testid="monitor-kind"
          options={[
            { value: 'binary', label: copy.yesNo },
            { value: 'choice', label: copy.choice },
          ]}
          onValueChange={(value) => setKind(value as 'binary' | 'choice')}
        />
      </div>
      {kind.type === 'choice' ? (
        <>
          <Field id={`${base}-options`} label={copy.options} error={errors.options}>
            <ul className="br-monitors__options" aria-label={copy.options}>
              {choiceOptions.map((option, index) => (
                <li key={index} className="br-monitors__option">
                  <Input
                    aria-label={copy.optionName(index + 1)}
                    value={option.name}
                    disabled={readOnly}
                    data-testid={`monitor-option-${index}`}
                    onChange={(event) => setOption(index, event.target.value)}
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="xs"
                    shape="round"
                    disabled={readOnly || choiceOptions.length <= 2}
                    aria-label={copy.removeOption(option.name)}
                    onClick={() =>
                      onChange({
                        ...draft,
                        question: {
                          ...draft.question,
                          kind: {
                            type: 'choice',
                            options: choiceOptions.filter((_, i) => i !== index),
                          },
                        },
                        option: draft.option === option.name ? null : draft.option,
                      })
                    }
                  >
                    <X aria-hidden="true" />
                  </Button>
                </li>
              ))}
            </ul>
          </Field>
          <div>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              disabled={readOnly || choiceOptions.length >= 10}
              onClick={() =>
                onChange({
                  ...draft,
                  question: {
                    ...draft.question,
                    kind: {
                      type: 'choice',
                      options: [...choiceOptions, { name: '', description: null }],
                    },
                  },
                })
              }
            >
              <Plus aria-hidden="true" />
              {copy.addOption}
            </Button>
          </div>
          <Field
            id={`${base}-fires-on`}
            label={copy.firesOn}
            helper={copy.firesOnHelp}
            error={errors.option}
          >
            <SegmentedControl
              value={draft.option ?? ''}
              disabled={readOnly}
              data-testid="monitor-fires-on"
              options={choiceOptions
                .filter((option) => option.name.trim())
                .map((option) => ({ value: option.name, label: option.name }))}
              onValueChange={(value) => onChange({ ...draft, option: value })}
            />
          </Field>
        </>
      ) : null}
      <div className="br-form-field flex min-w-0 flex-col gap-1.5" data-testid="monitor-model">
        <label htmlFor={`${base}-model`} className="text-label text-text-default">
          {copy.model}
        </label>
        <Select
          inputId={`${base}-model`}
          options={options}
          value={selected}
          isSearchable={false}
          isDisabled={readOnly}
          menuPlacement="auto"
          menuPortalTarget={menuPortalTarget}
          isOptionDisabled={(raw: unknown) => !!(raw as ModelOption).reason}
          formatOptionLabel={(raw: unknown) => {
            const option = raw as ModelOption;
            return (
              <span className="flex min-w-0 flex-col">
                <span className="truncate">{option.label}</span>
                {option.reason ? (
                  <span className="text-supporting text-text-muted">{option.reason}</span>
                ) : null}
              </span>
            );
          }}
          onChange={(raw: unknown) =>
            onChange({ ...draft, model: (raw as ModelOption | null)?.model ?? null })
          }
        />
        {errors.model ? (
          <p className="br-monitors__error" role="alert">
            {errors.model}
          </p>
        ) : chatTier === 'private' ? (
          <p className="br-monitors__note">{copy.modelHelp}</p>
        ) : null}
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-threshold`}>
          {copy.threshold}
        </span>
        <div className="br-monitors__threshold">
          <Slider
            aria-labelledby={`${base}-threshold`}
            description={copy.thresholdHelp}
            steps={THRESHOLD_STEPS}
            value={nearestStep(draft.threshold)}
            disabled={readOnly}
            data-testid="monitor-threshold"
            className="min-w-0 flex-1"
            onValueChange={(value) => onChange({ ...draft, threshold: Number(value) })}
          />
          <span className="br-monitors__threshold-value" aria-hidden="true">
            {formatProbability(draft.threshold)}
          </span>
        </div>
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-action`}>
          {copy.action}
        </span>
        <SegmentedControl
          aria-labelledby={`${base}-action`}
          value={draft.action}
          disabled={readOnly}
          data-testid="monitor-action"
          options={(['notify', 'steer', 'stop'] as const).map((value) => ({
            value,
            label: copy.actions[value],
          }))}
          onValueChange={(value) => onChange({ ...draft, action: value as MonitorAction })}
        />
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-timing`}>
          {copy.timing}
        </span>
        <SegmentedControl
          aria-labelledby={`${base}-timing`}
          value={draft.timing}
          disabled={readOnly}
          data-testid="monitor-timing"
          options={(['post_iteration', 'turn_end'] as const).map((value) => ({
            value,
            label: copy.timings[value],
          }))}
          onValueChange={(value) => onChange({ ...draft, timing: value as MonitorTiming })}
        />
      </div>
      <label className="br-dh-check">
        <Switch
          checked={draft.enabled}
          disabled={readOnly}
          data-testid="monitor-enabled"
          onCheckedChange={(checked) => onChange({ ...draft, enabled: checked === true })}
        />
        <span>{copy.on}</span>
      </label>
    </div>
  );
}

export interface MonitorsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessionId: string;
  /** The chat's tier: a private chat refuses a public model. `null` when not known. */
  chatTier: DecisionTier | null;
  /**
   * The chat's settings as the caller last read them (`useSessionDecisions`), or
   * `null` before they are known. Shown at once, but every open reads what is
   * stored now and holds every change until that answer arrives: the caller's copy
   * can predate a write made elsewhere (`biorouter session monitors`, another
   * window), and Save replaces the whole record, so a Save must never replace
   * monitors it has not seen. When the read fails this copy is used.
   */
  settings: DecisionSettings | null;
  /** The stored value after a save, so the caller can refresh what it draws. */
  onSaved?: (settings: DecisionSettings) => void;
  /** For tests: the catalog, instead of `GET /decisions/models`. */
  models?: DecisionModelEntry[];
  /** For tests, with `models`: the catalog's `sessionUnavailableReason`. */
  chatRefusal?: string | null;
  /** Show the monitors without letting them change (a read-only surface). */
  readOnly?: boolean;
}

/**
 * The chat's monitors: a decision model reading the newest part of the chat and
 * acting (notify, steer, stop) when its answer reaches a threshold. List, add,
 * edit and remove, then Save writes them all with
 * `PUT /sessions/{id}/decision-settings` and the person's proof.
 *
 * The models come from `GET /decisions/models`, each with its tier; a public
 * model is offered to a private chat disabled, with the reason, because Gate H
 * refuses it before anything is sent. What the daemon would refuse is said by
 * the field before Save is pressed, and a refusal the daemon sends anyway is
 * shown above the footer.
 *
 * The write needs the person's proof on a daemon that holds a user-action key,
 * and none on one that holds no key (a `biorouter serve` browser), so the
 * dialog works the same on both surfaces; a refusal is shown, never swallowed.
 */
export function MonitorsDialog({
  open,
  onOpenChange,
  sessionId,
  chatTier,
  settings,
  onSaved,
  models: givenModels,
  chatRefusal: givenChatRefusal,
  readOnly = false,
}: MonitorsDialogProps) {
  const [monitors, setMonitors] = useState<MonitorCriterion[]>(settings?.monitors ?? []);
  const [editing, setEditing] = useState<{ index: number | null; draft: MonitorCriterion } | null>(
    null
  );
  const [showErrors, setShowErrors] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState<string | null>(null);
  const [fetchedModels, setFetchedModels] = useState<DecisionModelEntry[]>([]);
  const [fetchedChatRefusal, setFetchedChatRefusal] = useState<string | null>(null);
  const models = givenModels ?? fetchedModels;
  // A Crew grant rules out every model, the chat's decision model included: no
  // row stands for that one, so the catalog says it once for the chat.
  const chatRefusal = givenModels ? (givenChatRefusal ?? null) : fetchedChatRefusal;
  // The dialog's own read, for an open that came before the caller's.
  const [ownSettings, setOwnSettings] = useState<DecisionSettings | null>(null);
  const [stored, setStored] = useState<'ready' | 'loading' | 'error'>('ready');
  const base = ownSettings ?? settings;
  const locked = readOnly || stored !== 'ready';

  // Each open starts from what is stored NOW, not from the caller's copy.
  useEffect(() => {
    if (!open) return;
    setMonitors(settings?.monitors ?? []);
    setEditing(null);
    setShowErrors(false);
    setSaveError(null);
    setOwnSettings(null);
    if (!sessionId) {
      setStored('ready');
      return;
    }
    setStored('loading');
    const known = settings;
    const controller = new AbortController();
    fetchDecisionSettings(sessionId, controller.signal)
      .then(({ data }) => {
        if (controller.signal.aborted) return;
        const fresh = data ?? known;
        setOwnSettings(fresh);
        setMonitors(fresh?.monitors ?? []);
        setStored('ready');
      })
      .catch(() => {
        // Nothing newer could be read: the caller's copy, when there is one, is the best known.
        if (!controller.signal.aborted) setStored(known ? 'ready' : 'error');
      });
    return () => controller.abort();
    // Only on open: a poll that re-reads the same settings must not discard an edit.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  useEffect(() => {
    if (!open || givenModels) return;
    const controller = new AbortController();
    // Named, so each model carries the daemon's own reason this chat may not use it.
    fetchDecisionCatalog(controller.signal, sessionId || undefined)
      .then((catalog) => {
        if (controller.signal.aborted) return;
        setFetchedModels(catalog.models);
        setFetchedChatRefusal(catalog.sessionUnavailableReason);
      })
      .catch(() => {});
    return () => controller.abort();
  }, [open, givenModels, sessionId]);

  const draftErrors = useMemo(
    () => (editing ? validateMonitor(editing.draft, { chatTier, models, chatRefusal }) : {}),
    [editing, chatTier, models, chatRefusal]
  );

  const commitDraft = (): MonitorCriterion[] | null => {
    if (!editing) return monitors;
    if (hasErrors(draftErrors)) {
      setShowErrors(true);
      return null;
    }
    const taken = new Set(
      monitors.filter((_, i) => i !== editing.index).map((monitor) => monitor.id)
    );
    const draft = {
      ...editing.draft,
      name: editing.draft.name.trim(),
      id: editing.draft.id || monitorSlug(editing.draft.name, taken),
      question: {
        ...editing.draft.question,
        instructions: editing.draft.question.instructions.trim(),
        kind:
          editing.draft.question.kind.type === 'choice'
            ? {
                type: 'choice' as const,
                options: editing.draft.question.kind.options.map((option) => ({
                  ...option,
                  name: option.name.trim(),
                })),
              }
            : editing.draft.question.kind,
      },
    };
    const next =
      editing.index === null
        ? [...monitors, draft]
        : monitors.map((monitor, i) => (i === editing.index ? draft : monitor));
    setMonitors(next);
    setEditing(null);
    setShowErrors(false);
    return next;
  };

  const save = async () => {
    if (stored !== 'ready') return;
    const next = commitDraft();
    if (!next) return;
    // Every monitor is checked again: a stored one may name a model this chat can no longer use.
    const invalid = next.findIndex((monitor) =>
      hasErrors(validateMonitor(monitor, { chatTier, models, chatRefusal }))
    );
    if (invalid >= 0) {
      setEditing({ index: invalid, draft: next[invalid] });
      setShowErrors(true);
      return;
    }
    setSaving(true);
    setSaveError(null);
    try {
      const stored = await saveDecisionSettings(sessionId, {
        defaultModel: base?.defaultModel ?? null,
        monitors: next,
      });
      onSaved?.(stored);
      onOpenChange(false);
    } catch (error) {
      setSaveError(refusalText(error instanceof Error ? error.message : error) ?? copy.saveFailed);
    } finally {
      setSaving(false);
    }
  };

  const atLimit = monitors.length >= MAX_MONITORS;
  const footer = (
    <>
      <Button type="button" variant="ghost" onClick={() => onOpenChange(false)}>
        {copy.cancel}
      </Button>
      <Button
        type="button"
        disabled={locked || saving}
        aria-busy={saving || undefined}
        data-testid="monitors-save"
        onClick={() => void save()}
      >
        {saving ? copy.saving : copy.save}
      </Button>
    </>
  );

  return (
    <ModalShell
      open={open}
      onOpenChange={onOpenChange}
      size="lg"
      purpose={saving ? 'required' : 'form'}
      title={copy.title}
      subtitle={copy.subtitle}
      footer={footer}
      scrollBody
      testId="monitors-dialog"
    >
      <div className="br-monitors">
        {stored === 'loading' ? (
          <p className="br-monitors__note" role="status" data-testid="monitors-loading">
            {copy.loading}
          </p>
        ) : stored === 'error' ? (
          <p className="br-monitors__error" role="alert" data-testid="monitors-load-error">
            {copy.loadFailed}
          </p>
        ) : monitors.length === 0 && !editing ? (
          <p className="br-monitors__note">{copy.empty}</p>
        ) : null}
        {monitors.length > 0 ? (
          <ul className="br-monitors__list" aria-label={copy.title}>
            {monitors.map((monitor, index) =>
              editing?.index === index ? null : (
                <li
                  key={monitor.id || index}
                  className="br-monitors__item br-fill-surface"
                  data-testid="monitor-item"
                >
                  <span className="br-monitors__item-text">
                    <span className="br-monitors__item-name">{monitor.name}</span>
                    <span className="br-monitors__item-meta">{monitorSummary(monitor)}</span>
                  </span>
                  <Switch
                    checked={monitor.enabled}
                    disabled={locked}
                    aria-label={copy.enabled(monitor.name)}
                    onCheckedChange={(checked) =>
                      setMonitors((current) =>
                        current.map((entry, i) =>
                          i === index ? { ...entry, enabled: checked === true } : entry
                        )
                      )
                    }
                  />
                  <Button
                    type="button"
                    variant="ghost"
                    size="xs"
                    shape="round"
                    disabled={locked || editing !== null}
                    aria-label={copy.edit(monitor.name)}
                    onClick={() => setEditing({ index, draft: monitor })}
                  >
                    <Pencil aria-hidden="true" />
                  </Button>
                  <Button
                    type="button"
                    variant="ghost"
                    size="xs"
                    shape="round"
                    disabled={locked || editing !== null}
                    aria-label={copy.remove(monitor.name)}
                    onClick={() => setMonitors((current) => current.filter((_, i) => i !== index))}
                  >
                    <Trash2 aria-hidden="true" />
                  </Button>
                </li>
              )
            )}
          </ul>
        ) : null}
        {editing ? (
          <>
            <MonitorEditor
              draft={editing.draft}
              errors={showErrors ? draftErrors : {}}
              models={models}
              chatTier={chatTier}
              chatRefusal={chatRefusal}
              readOnly={readOnly}
              onChange={(draft) => setEditing({ ...editing, draft })}
            />
            <div className="br-monitors__actions">
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() => {
                  setEditing(null);
                  setShowErrors(false);
                }}
              >
                {copy.cancel}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                disabled={readOnly}
                data-testid="monitor-done"
                onClick={() => void commitDraft()}
              >
                {copy.done}
              </Button>
            </div>
          </>
        ) : (
          <div>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={locked || atLimit}
              data-testid="monitor-add"
              onClick={() => {
                setEditing({ index: null, draft: newMonitor() });
                setShowErrors(false);
              }}
            >
              <Plus aria-hidden="true" />
              {copy.add}
            </Button>
            {atLimit ? <p className="br-monitors__note">{copy.limit}</p> : null}
          </div>
        )}
        {saveError ? (
          <p className="br-monitors__error" role="alert" data-testid="monitors-save-error">
            {saveError}
          </p>
        ) : null}
      </div>
    </ModalShell>
  );
}

export default MonitorsDialog;
