import { useEffect, useId, useMemo, useState } from 'react';
import { ModalShell } from '../ModalShell';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { Field, fieldLabelId } from '../ui/field';
import { Input } from '../ui/input';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { Slider } from '../ui/slider';
import { Switch } from '../ui/switch';
import { Textarea } from '../ui/textarea';
import { Pencil, Plus, Trash2, X } from '../icons/app-icons';
import {
  DEFAULT_MONITOR_WINDOW,
  DecisionRequestError,
  MAX_MONITORS,
  MONITOR_NAME_MAX,
  decisionModelLabel,
  fetchDecisionCatalog,
  fetchDecisionSettings,
  formatProbability,
  monitorThreshold,
  monitorToWire,
  outcomeLabel,
  refusalText,
  saveDecisionSettings,
  scoreLevelIndex,
  thresholdSummary,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionSettings,
  type DecisionTier,
  type MonitorAction,
  type MonitorCriterion,
  type MonitorDirection,
  type MonitorErrorPolicy,
  type MonitorTiming,
} from '../../utils/sessionDecisions';
import {
  DECISION_INSTALL_COPY,
  DecisionModelInstall,
  type DecisionInstallReason,
} from './DecisionModelInstall';
import './decisionsPanel.css';

export const MONITORS_COPY = {
  title: 'Monitors',
  subtitle:
    'A decision model reads the newest part of this chat and acts when its answer crosses the threshold.',
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
  score: 'Score',
  yes: 'Yes',
  no: 'No',
  levels: 'Levels, lowest first',
  scoreFiresOn: (level: string) => `${level} or a higher level`,
  scoreReadOnly: "This dialog cannot change a score question's levels or the level it watches.",
  options: 'Options',
  optionName: (n: number) => `Option ${n}`,
  addOption: 'Add an option',
  removeOption: (name: string) => `Remove option ${name || 'without a name'}`,
  optionsTooFew: 'A choice needs at least two options.',
  optionsDuplicate: 'Each option needs its own name.',
  firesOn: 'Fires on',
  firesOnHelp: 'The option whose probability is compared with the threshold.',
  firesOnAnswerHelp: 'The answer whose probability is compared with the threshold.',
  firesOnMissing: 'Choose the option that fires the monitor.',
  model: 'Model',
  chatDefault: "The chat's decision model",
  modelPublicRefused: 'This chat is private, so a public model may not read it.',
  modelHelp: 'Only a private model may read a private chat.',
  catalogMissing:
    'This Biorouter service is older than the app, so it cannot list decision models. Restart Biorouter to finish updating.',
  catalogRefused: (reason: string | null) =>
    reason
      ? `Biorouter refused to list the decision models. ${reason}`
      : 'Biorouter refused to list the decision models.',
  catalogFailed: (detail: string | null) =>
    detail
      ? `Could not load the decision models. ${detail}`
      : 'Could not load the decision models.',
  catalogStatus: (status: number) => `Biorouter answered with status ${status}.`,
  retry: 'Retry',
  firesWhen: 'Fires when',
  directions: { at_or_above: 'At or above', below: 'Below' } as Record<MonitorDirection, string>,
  threshold: 'Threshold',
  thresholdHelp: 'Fires when the probability of the answer it watches is at or above this.',
  thresholdHelpBelow: 'Fires when the probability of the answer it watches falls below this.',
  action: 'Then',
  actions: { stop: 'Stop the turn', notify: 'Notify me', steer: 'Tell the agent' } as Record<
    MonitorAction,
    string
  >,
  /**
   * A stop monitor with a re-arm level (set over HTTP or the CLI). This dialog
   * keeps it, held at the threshold while the threshold is past it.
   */
  stopsOnce: (p: string) => `Stops once, then stays quiet until P falls below ${p}`,
  stopsOnceBelow: (p: string) => `Stops once, then stays quiet until P rises to ${p}`,
  onError: 'If a check fails',
  onErrors: { notify: 'Notify me', pause: 'Pause the turn' } as Record<MonitorErrorPolicy, string>,
  onErrorHelp:
    'Pause stops the turn whenever the decision model cannot answer, so a safety rule never goes unchecked.',
  pausesOnError: 'pauses if a check fails',
  subagents: 'Also watch the subagents this chat starts',
  setByAgent: 'Set by the agent',
  inherited: 'From the chat that started this subagent',
  noUsableModel:
    'No decision model this chat may use is installed, so this monitor cannot run until one is.',
  /** The install panel's first sentence, by why no model is usable (`DecisionModelInstall`). */
  install: { ...DECISION_INSTALL_COPY.why, about: DECISION_INSTALL_COPY.about },
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

/**
 * The threshold's stops: 0.01, then 0.05 to 0.95 by 0.05, then 0.99 (21 stops).
 * Low ones are for a monitor that fires below its threshold ("P(yes) < 0.10").
 */
export const THRESHOLD_STEPS = [
  0.01,
  ...Array.from({ length: 19 }, (_, i) => (i + 1) * 0.05),
  0.99,
].map((p) => ({ value: p.toFixed(2), label: p.toFixed(2) }));

/**
 * The stops for a monitor whose threshold is `p`: {@link THRESHOLD_STEPS}, plus
 * `p` as a stop of its own, in order, when it is none of them. A threshold set
 * over the CLI or HTTP (0.33) then sits where it is and is saved unchanged;
 * snapping it to the nearest stop would change a rule nobody touched.
 */
export function thresholdStepsFor(p: number): { value: string; label: string }[] {
  if (!Number.isFinite(p) || THRESHOLD_STEPS.some((step) => Number(step.value) === p)) {
    return THRESHOLD_STEPS;
  }
  const own = { value: String(p), label: formatProbability(p) };
  const at = THRESHOLD_STEPS.findIndex((step) => Number(step.value) > p);
  return at < 0
    ? [...THRESHOLD_STEPS, own]
    : [...THRESHOLD_STEPS.slice(0, at), own, ...THRESHOLD_STEPS.slice(at)];
}

/** The stop whose value is `p` exactly, among `steps` (which hold it: `thresholdStepsFor`). */
function stepValueFor(p: number, steps: readonly { value: string }[]): string {
  return steps.find((step) => Number(step.value) === p)?.value ?? steps[0].value;
}

/**
 * The re-arm levels a monitor keeps when its threshold moves to `threshold`.
 *
 * The daemon refuses a level on the far side of the threshold (`settings.rs`'s
 * `validate`): a monitor that fires at or above re-arms below a level no higher
 * than its threshold, and one that fires below re-arms at a level no lower than
 * it. This dialog has no field for the level, so a threshold moved past a kept
 * level would make a Save the dialog could not fix. The level is held at the
 * threshold instead, so a stop monitor still stops once and then waits for P to
 * cross back over the threshold.
 *
 * It is held from the level the monitor had when the edit began (`opened`), so a
 * drag past it and back gives it back. A monitor with no level gets none, and
 * neither does one whose level the direction switch cleared.
 */
export function rearmForThreshold(
  draft: MonitorCriterion,
  threshold: number,
  opened: MonitorCriterion | null
): Pick<MonitorCriterion, 'rearmBelow' | 'rearmAbove'> {
  const below = draft.direction === 'below';
  const kept = below ? draft.rearmAbove : draft.rearmBelow;
  if (typeof kept !== 'number') {
    return { rearmBelow: draft.rearmBelow, rearmAbove: draft.rearmAbove };
  }
  const openedLevel =
    opened && (opened.direction === 'below') === below
      ? below
        ? opened.rearmAbove
        : opened.rearmBelow
      : null;
  const level = typeof openedLevel === 'number' ? openedLevel : kept;
  return below
    ? { rearmBelow: draft.rearmBelow, rearmAbove: Math.max(level, threshold) }
    : { rearmBelow: Math.min(level, threshold), rearmAbove: draft.rearmAbove };
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
    // As the daemon and the CLI default: a flaky local model must not stop the
    // turns of a person who did not ask for that.
    onError: 'notify',
    maxEvalsPerTurn: 50,
    enabled: true,
    direction: 'at_or_above',
    rearmAbove: null,
    setBy: 'person',
    setBySession: null,
    inheritedFrom: null,
    applyToSubagents: false,
  };
}

/**
 * A person's change to a monitor an agent set makes it the person's: the agent
 * then can neither change nor remove it (`apply_agent_edit` in `settings.rs`
 * never touches a person's monitor). Any other monitor is returned as it is.
 */
export function ownedByThePerson(monitor: MonitorCriterion): MonitorCriterion {
  return monitor.setBy === 'agent' ? { ...monitor, setBy: 'person', setBySession: null } : monitor;
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
 * the chat may not use, or, with `chatRefusal` (a Crew grant on the chat that no
 * longer stands), any model at all, the chat's decision model included. Empty
 * when the monitor can be saved.
 *
 * The model is judged only for a monitor that is on, as the daemon's save check
 * is: a monitor that is off never runs, so a person can always switch off (and
 * save) one whose model went away.
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
  if (!monitor.enabled) return errors;
  const entry = modelEntryFor(monitor.model, context.models);
  if (context.chatRefusal) errors.model = context.chatRefusal;
  else if (entry?.unavailableReason) errors.model = entry.unavailableReason;
  else if (context.chatTier === 'private' && entry?.tier === 'public') {
    errors.model = copy.modelPublicRefused;
  }
  return errors;
}

/** Why the chat may not use a listed model, or `null` when it may (the picker's rule). */
function modelReason(entry: DecisionModelEntry, chatTier: DecisionTier | null): string | null {
  // The daemon's own reason first (tier, an institution, a Crew grant); the tier
  // rule here only when it gave none (an older daemon).
  return (
    entry.unavailableReason ??
    (chatTier === 'private' && entry.tier === 'public' ? copy.modelPublicRefused : null)
  );
}

/** The opening of the Crew refusal the daemon gives a hosted model (`CREW_SCOPED_REFUSAL`). */
const CREW_REASON_OPENING = 'This chat works under a Crew grant';

function hasErrors(errors: MonitorErrors): boolean {
  return Object.values(errors).some(Boolean);
}

/**
 * One line under a monitor's name: its threshold, action, timing and model, and
 * that it pauses the turn when a check fails. The threshold reads as the decision
 * card and history read the same monitor's calls (`thresholdSummary`):
 * `P(no) ≥ 0.85`, `P(rrms) ≥ 0.70`, `P(level ≥ mild) ≥ 0.85`, and `P(yes) < 0.30`
 * for a monitor that fires below its threshold.
 */
export function monitorSummary(monitor: MonitorCriterion): string {
  const threshold = monitorThreshold(monitor);
  const sign = monitor.direction === 'below' ? '<' : '≥';
  return [
    threshold
      ? thresholdSummary(threshold, monitor.question)
      : `P(${monitor.option ?? 'yes'}) ${sign} ${formatProbability(monitor.threshold)}`,
    copy.actions[monitor.action],
    copy.timings[monitor.timing],
    decisionModelLabel(monitor.model) ?? copy.chatDefault,
    ...(monitor.onError === 'pause' ? [copy.pausesOnError] : []),
  ].join(' · ');
}

interface ModelOption {
  value: string;
  label: string;
  model: DecisionModelRef | null;
  reason: string | null;
}

const menuPortalTarget = typeof document === 'undefined' ? undefined : document.body;

/** Why the model list could not be read, said by the model picker, and whether Retry can help. */
interface CatalogProblem {
  text: string;
  retry: boolean;
}

/** A refused or failed `GET /decisions/models`, in the dialog's words. */
function catalogProblemFor(error: unknown): CatalogProblem {
  if (error instanceof DecisionRequestError) {
    if (error.status === 401 || error.status === 403) {
      return { text: copy.catalogRefused(error.reason), retry: true };
    }
    const detail = error.reason ?? (error.status ? copy.catalogStatus(error.status) : null);
    return { text: copy.catalogFailed(detail), retry: true };
  }
  const detail = error instanceof Error ? error.message.trim() || null : refusalText(error);
  return { text: copy.catalogFailed(detail), retry: true };
}

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
  catalogProblem,
  onRetryCatalog,
  readOnly,
  nothingUsable,
  isSubagentChat,
  opened,
}: {
  draft: MonitorCriterion;
  onChange: (next: MonitorCriterion) => void;
  errors: MonitorErrors;
  models: readonly DecisionModelEntry[];
  chatTier: DecisionTier | null;
  /** Why this chat may use no decision model at all (a Crew grant that no longer stands), or `null`. */
  chatRefusal: string | null;
  /** Why the model list could not be read, or `null`. */
  catalogProblem: CatalogProblem | null;
  onRetryCatalog: () => void;
  readOnly: boolean;
  /** The list was read and no model in it may read this chat: said under Model, never a refusal. */
  nothingUsable: boolean;
  /** A subagent's chat, whose monitors cannot be set to watch subagents of its own. */
  isSubagentChat: boolean;
  /**
   * The monitor as it was when this edit began (`null` for a new one). The
   * slider's stops are built from its threshold, not from the draft's: a stored
   * 0.33 keeps its stop for the whole edit, so a drag off it neither shrinks the
   * scale under the pointer nor loses 0.33 as a value to come back to. Its re-arm
   * level is the one a threshold drag holds back and gives back
   * (`rearmForThreshold`), for the same reason.
   */
  opened: MonitorCriterion | null;
}) {
  const base = useId();
  const kind = draft.question.kind;
  const below = draft.direction === 'below';
  const thresholdSteps = thresholdStepsFor(opened?.threshold ?? draft.threshold);
  const options: ModelOption[] = [
    // No row stands for the chat's decision model, so the chat-wide reason is its own.
    { value: '', label: copy.chatDefault, model: null, reason: chatRefusal },
    ...models.map((entry) => ({
      value: modelKey({ provider: entry.provider, model: entry.model, adapter: null }),
      label: [
        entry.displayName,
        entry.tier === 'private' ? copy.private : entry.tier === 'public' ? copy.public : null,
        entry.local ? copy.local : null,
      ]
        .filter(Boolean)
        .join(' · '),
      model: { provider: entry.provider, model: entry.model, adapter: null },
      reason: modelReason(entry, chatTier),
    })),
  ];
  // A stop monitor with a re-arm level stops once, not every turn its answer holds.
  const rearm = below ? draft.rearmAbove : draft.rearmBelow;
  const stopsOnce =
    draft.action === 'stop' && typeof rearm === 'number'
      ? (below ? copy.stopsOnceBelow : copy.stopsOnce)(formatProbability(rearm))
      : null;
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
  // A score monitor (set over HTTP; neither this dialog nor the CLI writes one)
  // keeps its levels and watched level: shown here, and saved as they are.
  const scoreLevels = kind.type === 'score' ? kind.levels : [];
  const watchedLevel = scoreLevelIndex(scoreLevels, draft.option);
  // Named as the summary line names it (`thresholdSummary`): the label, or "Level n".
  const watchedLevelLabel =
    watchedLevel === null
      ? (draft.option ?? '')
      : outcomeLabel(draft.question, String(watchedLevel));
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
          value={kind.type}
          disabled={readOnly}
          data-testid="monitor-kind"
          options={[
            { value: 'binary', label: copy.yesNo },
            { value: 'choice', label: copy.choice },
            // Only for a monitor that already asks for a score: this dialog makes none.
            ...(kind.type === 'score' ? [{ value: 'score', label: copy.score }] : []),
          ]}
          onValueChange={(value) => {
            if (value === 'binary' || value === 'choice') setKind(value);
          }}
        />
      </div>
      {kind.type === 'binary' ? (
        <Field id={`${base}-fires-on`} label={copy.firesOn} helper={copy.firesOnAnswerHelp}>
          <SegmentedControl
            aria-labelledby={fieldLabelId(`${base}-fires-on`)}
            value={draft.option?.toLowerCase() === 'no' ? 'no' : 'yes'}
            disabled={readOnly}
            data-testid="monitor-fires-on"
            options={[
              { value: 'yes', label: copy.yes },
              { value: 'no', label: copy.no },
            ]}
            // Yes is the default the daemon reads from no option at all.
            onValueChange={(value) => onChange({ ...draft, option: value === 'no' ? 'no' : null })}
          />
        </Field>
      ) : null}
      {kind.type === 'score' ? (
        <>
          <div className="br-form-field flex min-w-0 flex-col gap-1.5">
            <span className="text-label text-text-default" id={`${base}-levels`}>
              {copy.levels}
            </span>
            <ol
              className="br-monitors__levels"
              aria-labelledby={`${base}-levels`}
              data-testid="monitor-levels"
            >
              {scoreLevels.map((level, index) => (
                <li key={index}>{level}</li>
              ))}
            </ol>
          </div>
          <div className="br-form-field flex min-w-0 flex-col gap-1.5">
            <span className="text-label text-text-default">{copy.firesOn}</span>
            <p className="br-monitors__value" data-testid="monitor-score-fires-on">
              {copy.scoreFiresOn(watchedLevelLabel)}
            </p>
            <p className="br-monitors__note">{copy.scoreReadOnly}</p>
          </div>
        </>
      ) : null}
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
              aria-labelledby={fieldLabelId(`${base}-fires-on`)}
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
      <Field id={`${base}-fires-when`} label={copy.firesWhen}>
        <SegmentedControl
          aria-labelledby={fieldLabelId(`${base}-fires-when`)}
          value={below ? 'below' : 'at_or_above'}
          disabled={readOnly}
          data-testid="monitor-direction"
          options={(['at_or_above', 'below'] as const).map((value) => ({
            value,
            label: copy.directions[value],
          }))}
          // A re-arm level is on the other side of the threshold in each direction,
          // so neither one survives the switch.
          onValueChange={(value) =>
            onChange({
              ...draft,
              direction: value === 'below' ? 'below' : 'at_or_above',
              rearmBelow: null,
              rearmAbove: null,
            })
          }
        />
      </Field>
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
        {catalogProblem ? (
          <p className="br-monitors__error" role="alert" data-testid="monitor-models-error">
            {catalogProblem.text}
            {catalogProblem.retry ? (
              <>
                {' '}
                <button
                  type="button"
                  className="br-dh-link"
                  data-testid="monitor-models-retry"
                  onClick={onRetryCatalog}
                >
                  {copy.retry}
                </button>
              </>
            ) : null}
          </p>
        ) : null}
        {errors.model ? (
          <p className="br-monitors__error" role="alert">
            {errors.model}
          </p>
        ) : nothingUsable ? (
          // A note, not a refusal: Save stays open, or a stopped Ollama would keep
          // a person from turning monitors off or removing them.
          <p className="br-monitors__note" data-testid="monitor-no-usable-model">
            {copy.noUsableModel}
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
            aria-describedby={`${base}-threshold-help`}
            steps={thresholdSteps}
            value={stepValueFor(draft.threshold, thresholdSteps)}
            disabled={readOnly}
            data-testid="monitor-threshold"
            className="min-w-0 flex-1"
            // A kept re-arm level moves with a threshold that passes it, or the
            // daemon would refuse the Save and nothing here could change the level.
            onValueChange={(value) => {
              const threshold = Number(value);
              onChange({ ...draft, threshold, ...rearmForThreshold(draft, threshold, opened) });
            }}
          />
          <span className="br-monitors__threshold-value" aria-hidden="true">
            {formatProbability(draft.threshold)}
          </span>
        </div>
        <p className="br-monitors__note" id={`${base}-threshold-help`}>
          {below ? copy.thresholdHelpBelow : copy.thresholdHelp}
        </p>
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
        {stopsOnce ? (
          <p className="br-monitors__note" data-testid="monitor-stops-once">
            {stopsOnce}
          </p>
        ) : null}
      </div>
      <Field id={`${base}-on-error`} label={copy.onError} helper={copy.onErrorHelp}>
        <SegmentedControl
          aria-labelledby={fieldLabelId(`${base}-on-error`)}
          value={draft.onError}
          disabled={readOnly}
          data-testid="monitor-on-error"
          options={(['notify', 'pause'] as const).map((value) => ({
            value,
            label: copy.onErrors[value],
          }))}
          onValueChange={(value) =>
            onChange({ ...draft, onError: value === 'pause' ? 'pause' : 'notify' })
          }
        />
      </Field>
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
      {/* A subagent's own monitors never reach subagents of its own (`settings.rs`). The
          label wraps the box, so the square itself toggles it (`ui/Checkbox`). */}
      {isSubagentChat ? null : (
        <label className="br-dh-check" data-testid="monitor-subagents">
          <Checkbox
            checked={draft.applyToSubagents === true}
            disabled={readOnly}
            onChange={(event) => onChange({ ...draft, applyToSubagents: event.target.checked })}
          />
          <span>{copy.subagents}</span>
        </label>
      )}
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
  /**
   * The chat is a subagent's: its monitors cannot be set to also watch subagents
   * of its own. Also read from the monitors themselves (an agent's, or one copied
   * from the chat that started it, live only in a subagent's chat).
   */
  isSubagentChat?: boolean;
  /**
   * The chat works under a Crew grant, for the install panel's first sentence.
   * Also read from the catalog: the daemon names the grant in each hosted model's reason.
   */
  isCrewChat?: boolean;
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
 * The daemon checks that what a save turns on can run for this chat now and
 * answers 422 with its sentence when it cannot; that sentence is shown too.
 *
 * When the list was read and no model in it may read this chat, the install
 * panel (`DecisionModelInstall`) offers to install a local one at the top, and
 * the model list is read again once it is installed. Save is never held back for
 * that: a monitor may be removed or switched off whatever its model.
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
  isSubagentChat: givenSubagentChat = false,
  isCrewChat: givenCrewChat = false,
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
  // Why the model list could not be read; and Retry's count, which reads it again
  // without touching the monitor being edited.
  const [catalogIssue, setCatalogIssue] = useState<CatalogProblem | null>(null);
  const [catalogAttempt, setCatalogAttempt] = useState(0);
  // Whether a list has been read at all this open: until then, "none usable" is unknown.
  const [catalogRead, setCatalogRead] = useState(false);
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
    if (!open) setCatalogRead(false);
  }, [open]);

  useEffect(() => {
    if (!open || givenModels) return;
    const controller = new AbortController();
    // A new read says nothing until it answers: no sentence about an older answer.
    setCatalogIssue(null);
    // Named, so each model carries the daemon's own reason this chat may not use it.
    fetchDecisionCatalog(controller.signal, sessionId || undefined)
      .then((catalog) => {
        if (controller.signal.aborted) return;
        setFetchedModels(catalog.models);
        setFetchedChatRefusal(catalog.sessionUnavailableReason);
        setCatalogRead(true);
        // An older daemon lists nothing because it cannot; Retry would get the same answer.
        if (catalog.missing) setCatalogIssue({ text: copy.catalogMissing, retry: false });
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) setCatalogIssue(catalogProblemFor(error));
      });
    return () => controller.abort();
  }, [open, givenModels, sessionId, catalogAttempt]);

  // The list was read, and no model in it may read this chat. A Crew grant that no
  // longer stands (`chatRefusal`) refuses every model, an installed one too, so the
  // install panel would offer what cannot help.
  const nothingUsable =
    (givenModels !== undefined || catalogRead) &&
    !catalogIssue &&
    !chatRefusal &&
    !models.some((entry) => modelReason(entry, chatTier) === null);
  const isSubagentChat =
    givenSubagentChat ||
    monitors.some((monitor) => monitor.setBy === 'agent' || !!monitor.inheritedFrom);
  const isCrewChat =
    givenCrewChat ||
    models.some((entry) => entry.unavailableReason?.startsWith(CREW_REASON_OPENING) === true);
  const installReason: DecisionInstallReason = isCrewChat
    ? 'crew'
    : chatTier === 'private'
      ? 'private'
      : 'none';

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
    const trimmed: MonitorCriterion = {
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
    // An agent's monitor the person changed is the person's now; opened and
    // closed with nothing changed, it stays the agent's.
    const original = editing.index === null ? null : monitors[editing.index];
    const changed =
      original !== null &&
      JSON.stringify(monitorToWire(trimmed)) !== JSON.stringify(monitorToWire(original));
    const draft = changed ? ownedByThePerson(trimmed) : trimmed;
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
    // Every monitor that is on is checked again: a stored one may name a model this
    // chat can no longer use. One that is off is not, so it can always be saved off.
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
        {nothingUsable && !readOnly ? (
          <DecisionModelInstall
            reason={installReason}
            // Read the list again: the model just installed is in it now.
            onInstalled={() => setCatalogAttempt((attempt) => attempt + 1)}
          />
        ) : null}
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
                    {monitor.setBy === 'agent' || monitor.inheritedFrom ? (
                      <span className="flex flex-wrap gap-1 pt-1">
                        {monitor.setBy === 'agent' ? (
                          <Badge data-testid="monitor-set-by-agent">{copy.setByAgent}</Badge>
                        ) : null}
                        {monitor.inheritedFrom ? (
                          <Badge data-testid="monitor-inherited">{copy.inherited}</Badge>
                        ) : null}
                      </span>
                    ) : null}
                  </span>
                  <Switch
                    checked={monitor.enabled}
                    disabled={locked}
                    aria-label={copy.enabled(monitor.name)}
                    onCheckedChange={(checked) =>
                      setMonitors((current) =>
                        current.map((entry, i) =>
                          // Switched by the person, an agent's monitor is the person's.
                          i === index
                            ? ownedByThePerson({ ...entry, enabled: checked === true })
                            : entry
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
              catalogProblem={catalogIssue}
              onRetryCatalog={() => setCatalogAttempt((attempt) => attempt + 1)}
              readOnly={readOnly}
              nothingUsable={nothingUsable}
              isSubagentChat={isSubagentChat}
              opened={editing.index === null ? null : (monitors[editing.index] ?? null)}
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
