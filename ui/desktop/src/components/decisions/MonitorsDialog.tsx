import { useCallback, useEffect, useId, useMemo, useRef, useState, type ReactNode } from 'react';
import { ModalShell } from '../ModalShell';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { Disclosure } from '../ui/disclosure';
import { EmptyState } from '../ui/empty-state';
import { Field, fieldLabelId } from '../ui/field';
import { InfoTip, useInfoTipId } from '../ui/info-tip';
import { Input } from '../ui/input';
import { RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { Slider } from '../ui/slider';
import { Switch } from '../ui/switch';
import { Textarea } from '../ui/textarea';
import { Check, Plus, SlidersHorizontal, X } from '../icons/app-icons';
import {
  DEFAULT_MONITOR_WINDOW,
  DecisionRequestError,
  MAX_MONITORS,
  PublicDecisionConfirmationError,
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
  type DecisionDraftChat,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionSettings,
  type DecisionTier,
  type PublicDecisionConfirmation,
  type MonitorAction,
  type MonitorCriterion,
  type MonitorDirection,
  type MonitorErrorPolicy,
  type MonitorTiming,
  type Question,
  type Threshold,
} from '../../utils/sessionDecisions';
import { isBrowserSurface } from '../../utils/surface';
import { DecisionModelInstall, type DecisionInstallReason } from './DecisionModelInstall';
import {
  DECISION_MODEL_OPTION_COPY,
  anyPublicModelRefused,
  decisionModelKey,
  decisionModelOptions,
  decisionModelReason,
  formatDecisionModelOption,
  isDecisionModelOptionDisabled,
  type DecisionModelOption,
} from './decisionModelOption';
import {
  TIER_EFFECT_COPY,
  decisionModelPlace,
  joinWithAnd,
  publicModelsSentence,
  ruleInWords,
  tierEffectNote,
} from './decisionWords';
import { loweringModels } from './draftDecisionSettings';
import { TierEffectLine } from './TierEffectLine';
import './decisionsPanel.css';
import './monitorsDialog.css';

const capitalize = (text: string) => (text ? text[0].toUpperCase() + text.slice(1) : text);

export const MONITORS_COPY = {
  title: 'Monitors',
  subtitle:
    'Choose the decision model this chat uses, and add monitors that ask it about the chat as it runs.',
  empty: 'No monitors yet',
  emptyDescription:
    'A monitor watches this chat and notifies you, tells the agent or stops the turn when its question is answered.',
  add: 'Add a monitor',
  limit: `A chat can have at most ${MAX_MONITORS} monitors.`,
  moreActions: (name: string) => `More actions for ${name}`,
  edit: 'Edit',
  remove: 'Remove',
  removed: (name: string) => `Removed ${name}.`,
  undo: 'Undo',
  undoLabel: (name: string) => `Undo removing ${name}`,
  saved: 'Saved',
  /** Draft mode's "Saved": nothing is stored until the chat's first message is sent. */
  savedDraft: 'Ready for the first message',
  saving: 'Saving…',
  name: 'Name',
  nameRequired: 'Give the monitor a name.',
  nameTooLong: `Keep the name to ${MONITOR_NAME_MAX} characters.`,
  question: 'Question',
  questionInfo:
    'Asked of the newest messages and tool results. Write it so yes means act, for example “Has the form been submitted?”',
  questionPlaceholder: 'For example: Does the newest message ask to delete files?',
  questionRequired: 'Write the question the model answers.',
  moreOptions: 'More options',
  answerType: 'Answer type',
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
  optionNameLabel: (n: number) => `Option ${n} name`,
  optionDescription: 'What it means (optional)',
  optionDescriptionLabel: (n: number) => `Option ${n} description`,
  addOption: 'Add an option',
  removeOption: (name: string) => `Remove option ${name || 'without a name'}`,
  optionsTooFew: 'A choice needs at least two options.',
  optionsDuplicate: 'Each option needs its own name.',
  firesOnMissing: 'Choose the option that fires the monitor.',
  firesWhen: 'Fires when',
  firesWhenInfo:
    'The monitor acts when the probability of the answer it watches crosses this line. Higher means it acts only when the model is surer.',
  watchedAnswer: 'Answer it watches',
  side: 'Side',
  threshold: 'Threshold',
  firesSentence: (phrase: string) => `Fires ${phrase}.`,
  chooseOptionPhrase: 'choose the option it fires on',
  chooseOptionSentence: 'Choose the option it fires on.',
  chooseOption: 'Choose an option',
  nameOptionsFirst: 'Name the options first',
  directions: { at_or_above: 'At or above', below: 'Below' } as Record<MonitorDirection, string>,
  model: 'Model',
  modelPublicRefused: DECISION_MODEL_OPTION_COPY.publicRefused,
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
  action: 'Then',
  actions: { stop: 'Stop the turn', notify: 'Notify me', steer: 'Tell the agent' } as Record<
    MonitorAction,
    string
  >,
  check: 'Check',
  timings: { post_iteration: 'After each step', turn_end: 'When the turn ends' } as Record<
    MonitorTiming,
    string
  >,
  ifNoAnswer: "If the model can't answer",
  ifNoAnswerInfo:
    'Pause the turn stops it whenever the decision model cannot answer, so a safety rule never goes unchecked.',
  onErrors: { notify: 'Notify me', pause: 'Pause the turn' } as Record<MonitorErrorPolicy, string>,
  stopOnlyOnce: 'Stop only once',
  stopOnlyOnceInfo: (outcome: string, p: string) =>
    `After it stops the turn, the monitor stays quiet until P(${outcome}) drops below ${p} again.`,
  stopOnlyOnceInfoBelow: (outcome: string, p: string) =>
    `After it stops the turn, the monitor stays quiet until P(${outcome}) rises to ${p} or more again.`,
  subagents: 'Also watch the subagents this chat starts',
  peekLabel: (name: string) => `Settings of ${name}`,
  peekUses: (model: string, place: string | null, timing: string) =>
    `Uses ${model}${place ? ` on ${place}` : ''}, ${timing}.`,
  peekTimings: { post_iteration: 'after each step', turn_end: 'when the turn ends' } as Record<
    MonitorTiming,
    string
  >,
  peekStopsOnce: 'stops only once',
  peekPauses: "pauses if the model can't answer",
  peekSubagents: 'also watches subagents',
  setByAgent: 'Set by the agent',
  setByAgentInfo:
    'The agent set this monitor. If you change it, it becomes yours and the agent can no longer change it.',
  inherited: 'Inherited',
  inheritedInfo: 'Copied from the chat that started this subagent.',
  defaultModel: DECISION_MODEL_OPTION_COPY.defaultModel,
  defaultModelInPeek: "the chat's default model",
  cancel: 'Cancel',
  save: 'Save',
  saveFailed: 'Could not save the monitors.',
  loading: 'Reading the monitors…',
  loadFailed: "Could not read this chat's monitors. Close and open again to retry.",
  /** Draft mode: a chat that has no message yet, so nothing is stored until Send. */
  draftNote: 'These monitors start with the chat when you send its first message.',
  /** The chat's default decision model (M1-1): one select above the monitors. */
  defaultField: 'Decision model for this chat',
  defaultFieldInfo: 'Monitors, gates and decisions that name no model use it.',
  automatic: 'Chosen automatically',
  automaticHint: 'a model on this computer first',
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
 * it. "Stop only once" has no field for the level, so a threshold moved past a
 * kept level would make a Save the dialog could not fix. The level is held at the
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
  const openedLevel = openedRearm(draft, opened);
  const level = typeof openedLevel === 'number' ? openedLevel : kept;
  return below
    ? { rearmBelow: draft.rearmBelow, rearmAbove: Math.max(level, threshold) }
    : { rearmBelow: Math.min(level, threshold), rearmAbove: draft.rearmAbove };
}

/** The re-arm level the monitor opened with, on the draft's side, or `null`. */
function openedRearm(draft: MonitorCriterion, opened: MonitorCriterion | null): number | null {
  const below = draft.direction === 'below';
  if (!opened || (opened.direction === 'below') !== below) return null;
  const level = below ? opened.rearmAbove : opened.rearmBelow;
  return typeof level === 'number' ? level : null;
}

/** The level "Stop only once" keeps: the draft's, on its side, or `null` when unchecked. */
function rearmLevel(monitor: MonitorCriterion): number | null {
  const level = monitor.direction === 'below' ? monitor.rearmAbove : monitor.rearmBelow;
  return typeof level === 'number' ? level : null;
}

/**
 * "Stop only once" checked: the re-arm level the monitor opened with, held on
 * the right side of the threshold, else the threshold itself. Unchecked: none.
 */
export function stopOnlyOnce(
  draft: MonitorCriterion,
  checked: boolean,
  opened: MonitorCriterion | null
): Pick<MonitorCriterion, 'rearmBelow' | 'rearmAbove'> {
  if (!checked) return { rearmBelow: null, rearmAbove: null };
  const level = openedRearm(draft, opened) ?? draft.threshold;
  return draft.direction === 'below'
    ? { rearmBelow: null, rearmAbove: Math.max(level, draft.threshold) }
    : { rearmBelow: Math.min(level, draft.threshold), rearmAbove: null };
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
 * Whether a monitor uses any setting "More options" hides, set differently from a
 * new monitor's: then the editor opens with it open, so the person sees what
 * defines this monitor without hunting for it.
 */
export function usesHiddenSettings(monitor: MonitorCriterion): boolean {
  const fresh = newMonitor();
  return (
    monitor.question.kind.type !== 'binary' ||
    (monitor.option !== null && monitor.option.toLowerCase() !== 'yes') ||
    monitor.direction === 'below' ||
    monitor.threshold !== fresh.threshold ||
    monitor.model !== null ||
    monitor.timing !== fresh.timing ||
    monitor.onError !== fresh.onError ||
    rearmLevel(monitor) !== null ||
    monitor.applyToSubagents === true
  );
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
  model: Pick<DecisionModelRef, 'provider' | 'model'> | null,
  models: readonly DecisionModelEntry[]
): DecisionModelEntry | undefined {
  if (!model) return undefined;
  return models.find(
    (entry) =>
      entry.provider === model.provider &&
      (entry.model === model.model || entry.displayName === model.model)
  );
}

/** A sentence a person reads: no backticks (R5). */
const plain = (text: string) => text.replace(/`/g, '').trim();

/**
 * What the daemon would refuse in one monitor (`settings.rs`'s `validate`), said
 * by the field it belongs to, and what every check of it would refuse: a model
 * the chat may not use, or, with `chatRefusal` (a Crew grant on the chat that no
 * longer stands), any model at all, the chat's decision model included. Empty
 * when the monitor can be saved.
 *
 * The model is judged only for a monitor that is on, as the daemon's save check
 * is: a monitor that is off never runs, so a person can always switch off (and
 * save) one whose model went away. A private chat and a public model is said in
 * the desktop's own line (R4), whatever the daemon's sentence holds.
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
  if (context.chatRefusal) errors.model = plain(context.chatRefusal);
  else if (entry) {
    const reason = decisionModelReason(entry, context.chatTier);
    if (reason) errors.model = reason;
  }
  return errors;
}

/** The opening of the Crew refusal the daemon gives a hosted model (`CREW_SCOPED_REFUSAL`). */
const CREW_REASON_OPENING = 'This chat works under a Crew grant';

function hasErrors(errors: MonitorErrors): boolean {
  return Object.values(errors).some(Boolean);
}

/** The outcome a rule watches, as `P(...)` names it: `yes`, `rrms`, `mild or a higher level`. */
function watchedOutcome(rule: Threshold, question: Question): string {
  switch (rule.type) {
    case 'yes':
      return 'yes';
    case 'no':
      return 'no';
    case 'option':
      return rule.name;
    case 'at_least_level':
      return `${outcomeLabel(question, String(rule.level))} or a higher level`;
  }
}

/**
 * A monitor row's second line: what it does, its rule in words and when it checks,
 * `Stop the turn · when P(yes) is at or above 0.85 · after each step` (never `≥`),
 * or, for a choice whose option is not chosen yet,
 * `Notify me · choose the option it fires on · after each step`. The peek says the
 * timing again, because a long line is cut with an ellipsis.
 */
export function monitorRowLine(monitor: MonitorCriterion): string {
  const rule = monitorThreshold(monitor);
  return [
    copy.actions[monitor.action],
    rule ? ruleInWords(rule, monitor.question) : copy.chooseOptionPhrase,
    copy.peekTimings[monitor.timing],
  ].join(' · ');
}

/**
 * A row's peek, at most two plain sentences: the model, where it runs and when it
 * checks (`Uses tev1:0.8b on this computer, after each step.`), then what else is
 * set (`Stops only once and pauses if the model can't answer.`).
 */
export function monitorPeek(
  monitor: MonitorCriterion,
  models: readonly DecisionModelEntry[],
  browser = isBrowserSurface()
): string {
  const entry = modelEntryFor(monitor.model, models);
  const name = decisionModelLabel(monitor.model) ?? copy.defaultModelInPeek;
  const place = monitor.model
    ? decisionModelPlace(monitor.model.provider, {
        local: entry?.local ?? null,
        tier: entry?.tier ?? null,
        browser,
      })
    : null;
  const uses = copy.peekUses(name, place, copy.peekTimings[monitor.timing]);
  const extras = [
    monitor.action === 'stop' && rearmLevel(monitor) !== null ? copy.peekStopsOnce : null,
    monitor.onError === 'pause' ? copy.peekPauses : null,
    monitor.applyToSubagents ? copy.peekSubagents : null,
  ].filter((part): part is string => !!part);
  return extras.length > 0 ? `${uses} ${capitalize(joinWithAnd(extras))}.` : uses;
}

/** The closed "More options" summary: the defaults it hides, in the row's own words. */
function hiddenSummary(monitor: MonitorCriterion): string {
  const kind =
    monitor.question.kind.type === 'choice'
      ? copy.choice
      : monitor.question.kind.type === 'score'
        ? copy.score
        : copy.yesNo;
  const rule = monitorThreshold(monitor);
  return [
    kind,
    rule ? ruleInWords(rule, monitor.question) : copy.chooseOptionPhrase,
    decisionModelLabel(monitor.model) ?? copy.defaultModel,
  ].join(' · ');
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

/** A refused write, in the daemon's words when it gave some. */
function refusalOf(error: unknown): string {
  const text = refusalText(error instanceof Error ? error.message : error);
  return text ? plain(text) : copy.saveFailed;
}

/** The ids of the editor's fields that a Save may need to reveal and focus. */
interface EditorIds {
  name: string;
  firstOption: string;
  watched: string;
  model: string;
}

function MonitorEditor({
  draft,
  onChange,
  errors,
  models,
  chatTier,
  chatRefusal,
  defaultModel,
  catalogProblem,
  onRetryCatalog,
  isSubagentChat,
  opened,
  moreOpen,
  onMoreOpenChange,
  ids,
  busy,
  refusal,
  effect,
  confirmation,
  onCancel,
  onSave,
}: {
  draft: MonitorCriterion;
  onChange: (next: MonitorCriterion) => void;
  errors: MonitorErrors;
  models: readonly DecisionModelEntry[];
  chatTier: DecisionTier | null;
  /** Why this chat may use no decision model at all (a Crew grant that no longer stands), or `null`. */
  chatRefusal: string | null;
  /** The chat's stored default model, named on the picker's Default row. */
  defaultModel: DecisionModelRef | null;
  /** Why the model list could not be read, or `null`. */
  catalogProblem: CatalogProblem | null;
  onRetryCatalog: () => void;
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
  moreOpen: boolean;
  onMoreOpenChange: (open: boolean) => void;
  ids: EditorIds;
  busy: boolean;
  /** The daemon's refusal of this monitor's save, or `null`. */
  refusal: string | null;
  /**
   * DR-33: the line this save would earn ("This chat will be public: joey-1 runs on
   * Baseten.") and its tip, shown above Save while the edit would make the chat
   * public; Save is then the person's answer and sends it. `null` otherwise.
   */
  effect: { line: string; info: string } | null;
  /** The daemon's question for a save it would not take unanswered, with its two buttons. */
  confirmation: ReactNode;
  onCancel: () => void;
  onSave: () => void;
}) {
  const base = useId();
  const stopInfoId = useInfoTipId();
  const kind = draft.question.kind;
  const below = draft.direction === 'below';
  const thresholdSteps = thresholdStepsFor(opened?.threshold ?? draft.threshold);

  const options: DecisionModelOption[] = decisionModelOptions({
    models,
    chatTier,
    chatRefusal,
    defaultModel,
  });
  const selectedKey = decisionModelKey(
    draft.model
      ? {
          provider: draft.model.provider,
          model: modelEntryFor(draft.model, models)?.model ?? draft.model.model,
        }
      : null
  );
  if (draft.model && !options.some((option) => option.value === selectedKey)) {
    // A stored model the list does not hold (removed, or not read yet): kept, by its alias.
    const name = decisionModelLabel(draft.model) ?? selectedKey;
    const reason = chatRefusal ? plain(chatRefusal) : null;
    options.push({
      value: selectedKey,
      name,
      label: [name, reason].filter(Boolean).join(', '),
      secondary: reason,
      isPrivate: false,
      model: draft.model,
      reason,
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
  const watchedLevelLabel =
    watchedLevel === null
      ? (draft.option ?? '')
      : outcomeLabel(draft.question, String(watchedLevel));
  const setChoiceOptions = (next: { name: string; description: string | null }[]) =>
    ({ ...draft.question, kind: { type: 'choice' as const, options: next } }) as Question;
  const setOptionName = (index: number, name: string) => {
    if (kind.type !== 'choice') return;
    const previous = kind.options[index]?.name;
    onChange({
      ...draft,
      question: setChoiceOptions(
        kind.options.map((option, i) => (i === index ? { ...option, name } : option))
      ),
      option: draft.option === previous ? name : draft.option,
    });
  };
  const setOptionDescription = (index: number, description: string) => {
    if (kind.type !== 'choice') return;
    onChange({
      ...draft,
      question: setChoiceOptions(
        kind.options.map((option, i) =>
          i === index ? { ...option, description: description || null } : option
        )
      ),
    });
  };

  const rule = monitorThreshold(draft);
  const sentence = rule
    ? copy.firesSentence(ruleInWords(rule, draft.question))
    : copy.chooseOptionSentence;
  const firesLabelId = `${base}-fires-label`;
  const sentenceId = `${base}-fires-sentence`;
  const namedChoices = choiceOptions
    .filter((option) => option.name.trim())
    .map((option) => ({ value: option.name, label: option.name }));
  const watchedChoice = namedChoices.find((option) => option.value === draft.option) ?? null;
  const keptLevel = rearmLevel(draft);
  const stopInfo = (below ? copy.stopOnlyOnceInfoBelow : copy.stopOnlyOnceInfo)(
    rule ? watchedOutcome(rule, draft.question) : 'yes',
    formatProbability(keptLevel ?? draft.threshold)
  );

  return (
    // A filled box, so the fields in it rest one step back on the page ground:
    // a fill never sits on a fill (design.md 9.11).
    <div className="br-monitors__editor br-fill-surface" data-testid="monitor-editor">
      <Field id={ids.name} label={copy.name} error={errors.name} required>
        <Input
          value={draft.name}
          maxLength={MONITOR_NAME_MAX + 20}
          data-testid="monitor-name"
          onChange={(event) => onChange({ ...draft, name: event.target.value })}
        />
      </Field>
      <Field
        id={`${base}-question`}
        label={copy.question}
        info={copy.questionInfo}
        error={errors.question}
        required
      >
        <Textarea
          rows={2}
          value={draft.question.instructions}
          placeholder={copy.questionPlaceholder}
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
        <span className="text-label text-text-default" id={`${base}-action`}>
          {copy.action}
        </span>
        <SegmentedControl
          aria-labelledby={`${base}-action`}
          value={draft.action}
          data-testid="monitor-action"
          options={(['notify', 'steer', 'stop'] as const).map((value) => ({
            value,
            label: copy.actions[value],
          }))}
          onValueChange={(value) => onChange({ ...draft, action: value as MonitorAction })}
        />
      </div>
      <Disclosure
        label={copy.moreOptions}
        summary={hiddenSummary(draft)}
        open={moreOpen}
        onOpenChange={onMoreOpenChange}
      >
        <div className="br-monitors__more" data-testid="monitor-more-options">
          <div className="br-form-field flex min-w-0 flex-col gap-1.5">
            <span className="text-label text-text-default" id={`${base}-kind`}>
              {copy.answerType}
            </span>
            <SegmentedControl
              aria-labelledby={`${base}-kind`}
              value={kind.type}
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
          {kind.type === 'score' ? (
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
              <p className="br-monitors__note">{copy.scoreReadOnly}</p>
            </div>
          ) : null}
          {kind.type === 'choice' ? (
            <div className="br-form-field flex min-w-0 flex-col gap-1.5">
              <span className="text-label text-text-default" id={`${base}-options`}>
                {copy.options}
              </span>
              <ul
                className="br-monitors__option-grid"
                aria-labelledby={`${base}-options`}
                data-testid="monitor-options"
              >
                {choiceOptions.map((option, index) => (
                  <li key={index} className="br-monitors__option-row">
                    <Input
                      id={index === 0 ? ids.firstOption : undefined}
                      aria-label={copy.optionNameLabel(index + 1)}
                      placeholder={copy.optionName(index + 1)}
                      value={option.name}
                      aria-invalid={errors.options ? true : undefined}
                      data-testid={`monitor-option-${index}`}
                      onChange={(event) => setOptionName(index, event.target.value)}
                    />
                    <Input
                      aria-label={copy.optionDescriptionLabel(index + 1)}
                      placeholder={copy.optionDescription}
                      value={option.description ?? ''}
                      data-testid={`monitor-option-description-${index}`}
                      onChange={(event) => setOptionDescription(index, event.target.value)}
                    />
                    <Button
                      type="button"
                      variant="ghost"
                      size="xs"
                      shape="round"
                      disabled={choiceOptions.length <= 2}
                      aria-label={copy.removeOption(option.name)}
                      onClick={() =>
                        onChange({
                          ...draft,
                          question: setChoiceOptions(choiceOptions.filter((_, i) => i !== index)),
                          option: draft.option === option.name ? null : draft.option,
                        })
                      }
                    >
                      <X aria-hidden="true" />
                    </Button>
                  </li>
                ))}
              </ul>
              {errors.options ? (
                <p className="br-monitors__error" role="alert">
                  {errors.options}
                </p>
              ) : null}
              <div>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={choiceOptions.length >= 10}
                  onClick={() =>
                    onChange({
                      ...draft,
                      question: setChoiceOptions([
                        ...choiceOptions,
                        { name: '', description: null },
                      ]),
                    })
                  }
                >
                  <Plus aria-hidden="true" />
                  {copy.addOption}
                </Button>
              </div>
            </div>
          ) : null}
          <div className="br-form-field flex min-w-0 flex-col gap-1.5">
            <div className="flex min-w-0 items-center">
              <span className="text-label text-text-default" id={firesLabelId}>
                {copy.firesWhen}
              </span>
              <InfoTip label={copy.firesWhen} help={copy.firesWhenInfo} />
            </div>
            <div
              role="group"
              aria-labelledby={firesLabelId}
              aria-describedby={sentenceId}
              className="br-monitors__fires"
              data-testid="monitor-fires-when"
            >
              <div className="br-monitors__fires-line">
                {kind.type === 'binary' ? (
                  <SegmentedControl
                    aria-label={copy.watchedAnswer}
                    value={draft.option?.toLowerCase() === 'no' ? 'no' : 'yes'}
                    data-testid="monitor-fires-on"
                    options={[
                      { value: 'yes', label: copy.yes },
                      { value: 'no', label: copy.no },
                    ]}
                    // Yes is the default the daemon reads from no option at all.
                    onValueChange={(value) =>
                      onChange({ ...draft, option: value === 'no' ? 'no' : null })
                    }
                  />
                ) : kind.type === 'choice' ? (
                  <div className="br-monitors__watched" data-testid="monitor-fires-on">
                    <Select
                      inputId={ids.watched}
                      aria-label={copy.watchedAnswer}
                      aria-invalid={errors.option ? true : undefined}
                      options={namedChoices}
                      value={watchedChoice}
                      placeholder={
                        namedChoices.length > 0 ? copy.chooseOption : copy.nameOptionsFirst
                      }
                      isDisabled={namedChoices.length === 0}
                      isSearchable={false}
                      menuPlacement="auto"
                      menuPortalTarget={menuPortalTarget}
                      onChange={(raw: unknown) => {
                        const picked = raw as { value: string } | null;
                        if (picked) onChange({ ...draft, option: picked.value });
                      }}
                    />
                  </div>
                ) : (
                  <p className="br-monitors__value" data-testid="monitor-score-fires-on">
                    {copy.scoreFiresOn(watchedLevelLabel)}
                  </p>
                )}
                <SegmentedControl
                  aria-label={copy.side}
                  value={below ? 'below' : 'at_or_above'}
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
              </div>
              <Slider
                aria-label={copy.threshold}
                aria-describedby={sentenceId}
                steps={thresholdSteps}
                value={stepValueFor(draft.threshold, thresholdSteps)}
                data-testid="monitor-threshold"
                className="min-w-0"
                // A kept re-arm level moves with a threshold that passes it, or the
                // daemon would refuse the Save and nothing here could change the level.
                onValueChange={(value) => {
                  const threshold = Number(value);
                  onChange({ ...draft, threshold, ...rearmForThreshold(draft, threshold, opened) });
                }}
              />
              <p className="br-monitors__note" id={sentenceId} data-testid="monitor-fires-sentence">
                {sentence}
              </p>
            </div>
            {errors.option ? (
              <p className="br-monitors__error" role="alert">
                {errors.option}
              </p>
            ) : null}
          </div>
          <div className="br-form-field flex min-w-0 flex-col gap-1.5" data-testid="monitor-model">
            <span className="flex min-w-0 items-center gap-1">
              <label htmlFor={ids.model} className="text-label text-text-default">
                {copy.model}
              </label>
              {/* A disabled row cannot hold a tip, so the way back from "Public model: it
                  can't read a private chat" is the field's. */}
              {anyPublicModelRefused(models, chatTier) ? (
                <InfoTip label={copy.model} help={TIER_EFFECT_COPY.publicRefusedInfo} />
              ) : null}
            </span>
            <Select
              inputId={ids.model}
              options={options}
              value={selected}
              isSearchable={false}
              menuPlacement="auto"
              menuPortalTarget={menuPortalTarget}
              isOptionDisabled={isDecisionModelOptionDisabled}
              formatOptionLabel={formatDecisionModelOption}
              onChange={(raw: unknown) =>
                onChange({ ...draft, model: (raw as DecisionModelOption | null)?.model ?? null })
              }
            />
            {catalogProblem ? (
              <p className="br-monitors__error" role="alert" data-testid="monitor-models-error">
                {catalogProblem.text}
                {catalogProblem.retry ? (
                  <>
                    {' '}
                    <Button
                      type="button"
                      variant="link"
                      size="xs"
                      className="br-monitors__inline-link"
                      data-testid="monitor-models-retry"
                      onClick={onRetryCatalog}
                    >
                      {copy.retry}
                    </Button>
                  </>
                ) : null}
              </p>
            ) : null}
            {errors.model ? (
              <p className="br-monitors__error" role="alert">
                {errors.model}
              </p>
            ) : null}
          </div>
          <div className="br-form-field flex min-w-0 flex-col gap-1.5">
            <span className="text-label text-text-default" id={`${base}-timing`}>
              {copy.check}
            </span>
            <SegmentedControl
              aria-labelledby={`${base}-timing`}
              value={draft.timing}
              data-testid="monitor-timing"
              options={(['post_iteration', 'turn_end'] as const).map((value) => ({
                value,
                label: copy.timings[value],
              }))}
              onValueChange={(value) => onChange({ ...draft, timing: value as MonitorTiming })}
            />
          </div>
          <Field id={`${base}-on-error`} label={copy.ifNoAnswer} info={copy.ifNoAnswerInfo}>
            <SegmentedControl
              aria-labelledby={fieldLabelId(`${base}-on-error`)}
              value={draft.onError}
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
          {draft.action === 'stop' ? (
            // The InfoTip is the label's next sibling, never inside it (its own rule).
            <div className="flex min-w-0 items-center" data-testid="monitor-stops-once">
              <label className="br-dh-check">
                <Checkbox
                  checked={keptLevel !== null}
                  aria-describedby={stopInfoId}
                  onChange={(event) =>
                    onChange({ ...draft, ...stopOnlyOnce(draft, event.target.checked, opened) })
                  }
                />
                <span>{copy.stopOnlyOnce}</span>
              </label>
              <InfoTip label={copy.stopOnlyOnce} help={stopInfo} id={stopInfoId} />
            </div>
          ) : null}
          {/* A subagent's own monitors never reach subagents of its own (`settings.rs`). The
              label wraps the box, so the square itself toggles it (`ui/Checkbox`). */}
          {isSubagentChat ? null : (
            <label className="br-dh-check" data-testid="monitor-subagents">
              <Checkbox
                checked={draft.applyToSubagents === true}
                onChange={(event) => onChange({ ...draft, applyToSubagents: event.target.checked })}
              />
              <span>{copy.subagents}</span>
            </label>
          )}
        </div>
      </Disclosure>
      {refusal ? (
        <p className="br-monitors__error" role="alert" data-testid="monitor-save-error">
          {refusal}
        </p>
      ) : null}
      {effect && !confirmation ? (
        <TierEffectLine
          line={effect.line}
          info={effect.info}
          infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
          testId="monitor-tier-effect"
        />
      ) : null}
      {confirmation}
      <div className="br-monitors__actions">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="monitor-cancel"
          onClick={onCancel}
        >
          {copy.cancel}
        </Button>
        <Button
          type="button"
          size="sm"
          disabled={busy}
          aria-busy={busy || undefined}
          data-testid="monitor-save"
          onClick={onSave}
        >
          {copy.save}
        </Button>
      </div>
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
   * stored now and holds every change until that answer arrives, and every write
   * reads it again: the caller's copy can predate a write made elsewhere
   * (`biorouter session monitors`, an agent's `workspace_monitor`, another window),
   * and a write replaces the whole record. When a read fails this copy is used.
   */
  settings: DecisionSettings | null;
  /** The stored value after each write, so the caller can refresh what it draws. */
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
  /** The chat's own model in words (`gpt-5.5`), for the "This chat will be public" tip. */
  chatModelName?: string | null;
  /**
   * Draft mode (DR-33, design §4): the chat has no session row yet (Home, or a blank
   * chat), so nothing is written to the daemon. `settings` is the draft; each change
   * is handed to `onChange`, and the composer sends it as `decisionSettings` with the
   * chat's first message. `chat` is the model the chat will start on, so each listed
   * model carries what it would do to that chat (`chatEffect`). Pass `sessionId` as
   * `''` with it.
   */
  draft?: MonitorsDraft;
}

/** {@link MonitorsDialogProps.draft}. */
export interface MonitorsDraft {
  /** The model the chat will start on, or `null` before it is known. */
  chat: DecisionDraftChat | null;
  /**
   * The person already accepted the draft as it stands making this chat public,
   * under this model: a change that adds no public model to it asks nothing again.
   */
  accepted?: boolean;
  /**
   * A change to the draft. `acceptedPublic` is true when the result makes the chat
   * public and the person saw that line and saved it (or answered the question).
   */
  onChange: (settings: DecisionSettings, acceptedPublic: boolean) => void;
}

/** The line a draft's change that makes the chat public is said with, built from the models' rows. */
function publicEffectFor(
  lowering: readonly DecisionModelEntry[],
  chatModelName: string | null
): { line: string; info: string } | null {
  if (lowering.length === 0) return null;
  return {
    line: TIER_EFFECT_COPY.willBePublic(publicModelsSentence(lowering)),
    info: TIER_EFFECT_COPY.publicInfo(chatModelName),
  };
}

type Editing = {
  /** The monitor's id, or `null` for a new one. */
  id: string | null;
  draft: MonitorCriterion;
  /** The monitor as it was stored when the edit began, `null` for a new one. */
  opened: MonitorCriterion | null;
};

type Status =
  | { kind: 'idle' }
  | { kind: 'saving' }
  | { kind: 'saved' }
  | { kind: 'removed'; name: string };

type FocusTarget =
  | { kind: 'row'; id: string }
  | { kind: 'add' }
  | { kind: 'undo' }
  | { kind: 'field'; id: string };

/** The change one write applies to what is stored now, by monitor id; `null` writes nothing. */
type Change = (fresh: MonitorCriterion[]) => MonitorCriterion[] | null;

/**
 * The chat's monitors: a decision model reading the newest part of the chat and
 * acting (notify, steer, stop) when its answer crosses a threshold.
 *
 * A manager that applies each change when it is made, like the knowledge base
 * manager: a row's Switch turns it on or off, `⋯` › Edit replaces the row with
 * its editor (Save writes it), `⋯` › Remove removes it with Undo beside the
 * "Removed" status. There is no footer; the × closes, also while a write is in
 * flight (which still finishes and still calls `onSaved`).
 *
 * Every write reads what is stored now (`GET`), applies only the person's one
 * change to that list by monitor id, and writes the whole record back
 * (`PUT /sessions/{id}/decision-settings`, with the person's proof), so a monitor
 * an agent or another window added meanwhile is kept, and Undo puts back only the
 * monitor it removed.
 *
 * The models come from `GET /decisions/models`, each with its tier and where it
 * runs; one the chat may not use is offered disabled with the reason. When the
 * list was read and none may read this chat, the install panel offers a local
 * one at the top.
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
  chatModelName = null,
  draft = undefined,
}: MonitorsDialogProps) {
  const [monitors, setMonitors] = useState<MonitorCriterion[]>(settings?.monitors ?? []);
  const [editing, setEditing] = useState<Editing | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const [showErrors, setShowErrors] = useState(false);
  const [editorRefusal, setEditorRefusal] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [status, setStatus] = useState<Status>({ kind: 'idle' });
  const [undo, setUndo] = useState<{ monitor: MonitorCriterion; index: number } | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  /** A row switch flipped and not yet answered: shown as asked until the write settles. */
  const [pendingToggle, setPendingToggle] = useState<{ id: string; enabled: boolean } | null>(null);
  /** The chat's default decision model as stored (or held in a draft), for its select. */
  const [storedDefault, setStoredDefault] = useState<DecisionModelRef | null>(
    settings?.defaultModel ?? null
  );
  /** A default picked and not yet answered: shown as picked until the write settles. */
  const [pendingDefault, setPendingDefault] = useState<{ model: DecisionModelRef | null } | null>(
    null
  );
  const [defaultError, setDefaultError] = useState<string | null>(null);
  const [generalError, setGeneralError] = useState<string | null>(null);
  /**
   * DR-33: a write the daemon would not take until the person says the chat may be
   * public, with the question it asked and the same write to send again with the
   * answer. `inEditor` puts it above the editor's Save rather than under the list.
   */
  const [asking, setAsking] = useState<{
    confirmation: PublicDecisionConfirmation;
    retry: () => void;
    inEditor: boolean;
    /** Asked about a change of the default decision model: shown under its select. */
    atDefault: boolean;
  } | null>(null);
  /** What the last write did to the chat's tier, said in one line (and a tip). */
  const [effectNote, setEffectNote] = useState<{ line: string; info: string | null } | null>(null);
  const [focusTarget, setFocusTarget] = useState<FocusTarget | null>(null);
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
  const [stored, setStored] = useState<'ready' | 'loading' | 'error'>('ready');
  /** What is stored, as last read or written: a write whose fresh read fails starts from it. */
  const lastStored = useRef<DecisionSettings | null>(settings);
  /** Keep `value` as what is stored, and draw its default decision model. */
  const remember = useCallback((value: DecisionSettings | null) => {
    lastStored.current = value;
    setStoredDefault(value?.defaultModel ?? null);
  }, []);
  /** Each open's number: a write that finishes after a close (or a reopen) changes nothing here. */
  const generation = useRef(0);
  const openRef = useRef(open);
  openRef.current = open;
  const lastProp = useRef(settings);
  const listRef = useRef<HTMLDivElement | null>(null);
  const addRef = useRef<HTMLButtonElement | null>(null);
  const undoRef = useRef<HTMLButtonElement | null>(null);
  const base = useId();
  const ids: EditorIds = {
    name: `${base}-name`,
    firstOption: `${base}-option-0`,
    watched: `${base}-watched`,
    model: `${base}-model`,
  };
  const locked = readOnly || stored !== 'ready';

  // Each open starts from what is stored NOW, not from the caller's copy.
  useEffect(() => {
    if (!open) return;
    generation.current += 1;
    setMonitors(settings?.monitors ?? []);
    remember(settings);
    lastProp.current = settings;
    setEditing(null);
    setMoreOpen(false);
    setShowErrors(false);
    setEditorRefusal(null);
    setBusy(false);
    setStatus({ kind: 'idle' });
    setUndo(null);
    setRowErrors({});
    setPendingToggle(null);
    setPendingDefault(null);
    setDefaultError(null);
    setGeneralError(null);
    setAsking(null);
    setEffectNote(null);
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
        remember(fresh);
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

  // Between writes, while nothing is being edited or written, a newer copy from the
  // caller's poll replaces the list: the dialog stays open across changes made elsewhere.
  useEffect(() => {
    if (settings === lastProp.current) return;
    lastProp.current = settings;
    if (!open || stored !== 'ready' || editing || busy || !settings) return;
    remember(settings);
    setMonitors(settings.monitors);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settings]);

  useEffect(() => {
    if (!open) setCatalogRead(false);
  }, [open]);

  useEffect(() => {
    if (!open || givenModels) return;
    const controller = new AbortController();
    // A new read says nothing until it answers: no sentence about an older answer.
    setCatalogIssue(null);
    // Named, so each model carries the daemon's own reason this chat may not use it;
    // in draft mode, the model the chat will start on, so each carries its `chatEffect`.
    fetchDecisionCatalog(
      controller.signal,
      draft ? (draft.chat ?? undefined) : sessionId || undefined
    )
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
    // The draft's chat by its two strings: a new object each render must not re-read.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    open,
    givenModels,
    sessionId,
    catalogAttempt,
    draft?.chat?.boundProvider,
    draft?.chat?.boundModel,
  ]);

  // Focus moves after the render that drew its target, and after a menu's own
  // focus return (Radix restores its trigger in a timeout of its own).
  useEffect(() => {
    if (!focusTarget) return;
    const timer = window.setTimeout(() => {
      setFocusTarget(null);
      let node: HTMLElement | null = null;
      if (focusTarget.kind === 'add') node = addRef.current;
      else if (focusTarget.kind === 'undo') node = undoRef.current;
      else if (focusTarget.kind === 'field') node = document.getElementById(focusTarget.id);
      else {
        const row = listRef.current?.querySelector<HTMLElement>(
          `[data-monitor-id="${CSS.escape(focusTarget.id)}"]`
        );
        node = row?.querySelector<HTMLElement>('[data-row-action]') ?? null;
      }
      node?.focus();
    }, 0);
    return () => window.clearTimeout(timer);
  }, [focusTarget]);

  // The list was read, and no model in it may read this chat. A Crew grant that no
  // longer stands (`chatRefusal`) refuses every model, an installed one too, so the
  // install panel would offer what cannot help.
  const nothingUsable =
    (givenModels !== undefined || catalogRead) &&
    !catalogIssue &&
    !chatRefusal &&
    !models.some((entry) => decisionModelReason(entry, chatTier) === null);
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
  const context = { chatTier, models, chatRefusal };

  const draftErrors = useMemo(
    () => (editing ? validateMonitor(editing.draft, { chatTier, models, chatRefusal }) : {}),
    [editing, chatTier, models, chatRefusal]
  );

  // DR-33: what Save would do to the chat's tier, from the rows the daemon marked
  // `makesPublic`, worked out on the list as it would stand after this save.
  const editorEffect = useMemo(() => {
    if (!editing) return null;
    const preview: DecisionSettings = {
      defaultModel: storedDefault,
      monitors:
        editing.id === null
          ? [...monitors, editing.draft]
          : monitors.some((monitor) => monitor.id === editing.id)
            ? monitors.map((monitor) => (monitor.id === editing.id ? editing.draft : monitor))
            : [...monitors, editing.draft],
    };
    return publicEffectFor(loweringModels(preview, models), chatModelName);
  }, [editing, monitors, models, chatModelName, storedDefault]);

  const clearRowError = (id: string) =>
    setRowErrors((current) => {
      if (!(id in current)) return current;
      const next = { ...current };
      delete next[id];
      return next;
    });

  /**
   * One write: read what is stored now, apply `change` to it, PUT the result,
   * draw what the PUT returns and tell the caller. The dialog stays open. A
   * write that finishes after the dialog closed still tells the caller.
   */
  const write = useCallback(
    async (
      change: Change,
      outcome: {
        done?: (stored: DecisionSettings, wrote: boolean) => void;
        refused?: (sentence: string) => void;
        /** The person's answer was asked for: the question is up, nothing was written. */
        asked?: () => void;
      } = {},
      options: {
        /** The person saw "This chat will be public: …" for this write (DR-33). */
        acceptPublic?: boolean;
        /** Put the question, if one is asked, above the editor's Save. */
        inEditor?: boolean;
        /**
         * The chat's default decision model this write sets (`null` clears it), or
         * `undefined` to keep what is stored. The question, if one is asked, goes
         * under the default's select.
         */
        defaultModel?: DecisionModelRef | null;
      } = {}
    ): Promise<void> => {
      const gen = generation.current;
      const current = () => openRef.current && generation.current === gen;
      const acceptPublic = options.acceptPublic === true;
      const defaultFor = (stored: DecisionSettings | null) =>
        options.defaultModel !== undefined ? options.defaultModel : (stored?.defaultModel ?? null);
      const ask = (confirmation: PublicDecisionConfirmation) => {
        setAsking({
          confirmation,
          inEditor: options.inEditor === true,
          atDefault: options.defaultModel !== undefined,
          retry: () => {
            setAsking(null);
            void write(change, outcome, { ...options, acceptPublic: true });
          },
        });
        outcome.asked?.();
      };
      setBusy(true);
      setStatus({ kind: 'saving' });
      setUndo(null);
      setGeneralError(null);
      setAsking(null);
      setEffectNote(null);
      if (draft) {
        // Draft mode: nothing is stored on the daemon yet. A change that would make the
        // chat public is held until the person answers, as the daemon would hold it.
        const base = lastStored.current;
        const freshMonitors = base?.monitors ?? [];
        const next = change(freshMonitors);
        const kept: DecisionSettings = {
          defaultModel: next === null ? (base?.defaultModel ?? null) : defaultFor(base),
          monitors: next ?? freshMonitors,
        };
        const lowering = next === null ? [] : loweringModels(kept, models);
        const before = draft.accepted ? loweringModels(base, models) : [];
        const alreadyAccepted =
          draft.accepted === true && lowering.every((entry) => before.includes(entry));
        setBusy(false);
        if (lowering.length > 0 && !acceptPublic && !alreadyAccepted) {
          setStatus({ kind: 'idle' });
          ask({
            message: TIER_EFFECT_COPY.willBePublic(publicModelsSentence(lowering)),
            info: TIER_EFFECT_COPY.publicInfo(chatModelName),
            models: lowering.map((entry) => ({
              provider: entry.provider,
              model: entry.model,
              adapter: null,
            })),
          });
          return;
        }
        remember(kept);
        setMonitors(kept.monitors);
        setStatus({ kind: 'saved' });
        if (next !== null) {
          draft.onChange(kept, lowering.length > 0);
          const effect = publicEffectFor(lowering, chatModelName);
          if (effect) setEffectNote(effect);
        }
        outcome.done?.(kept, next !== null);
        return;
      }
      let fresh = lastStored.current;
      if (sessionId) {
        try {
          const { data } = await fetchDecisionSettings(sessionId);
          if (data) fresh = data;
        } catch {
          // Unreadable: the last stored list this dialog holds is the best known.
        }
      }
      const freshMonitors = fresh?.monitors ?? [];
      const next = change(freshMonitors);
      if (next === null) {
        if (current()) {
          const kept: DecisionSettings = {
            defaultModel: fresh?.defaultModel ?? null,
            monitors: freshMonitors,
          };
          remember(kept);
          setMonitors(freshMonitors);
          setBusy(false);
          setStatus({ kind: 'saved' });
          outcome.done?.(kept, false);
        }
        return;
      }
      try {
        const saved = await saveDecisionSettings(
          sessionId,
          { defaultModel: defaultFor(fresh), monitors: next },
          { acceptPublicDecisionModels: acceptPublic }
        );
        const { tierEffect, tierEffectLine, ...stored } = saved;
        onSaved?.(stored);
        if (!current()) return;
        remember(stored);
        setMonitors(stored.monitors);
        setBusy(false);
        setStatus({ kind: 'saved' });
        setEffectNote(tierEffectNote(tierEffect, tierEffectLine, chatModelName));
        outcome.done?.(stored, true);
      } catch (error) {
        if (!current()) return;
        if (fresh) {
          remember(fresh);
          setMonitors(freshMonitors);
        }
        setBusy(false);
        setStatus({ kind: 'idle' });
        if (error instanceof PublicDecisionConfirmationError) ask(error.confirmation);
        else outcome.refused?.(refusalOf(error));
      }
    },
    [sessionId, onSaved, draft, models, chatModelName, remember]
  );

  const startEdit = (monitor: MonitorCriterion | null) => {
    setUndo(null);
    // R1-2: an earlier write's "Saved" (or "Removed") describes that write, not the
    // editor opening now: it would read as if the new monitor were saved.
    setStatus({ kind: 'idle' });
    setShowErrors(false);
    setEditorRefusal(null);
    setMoreOpen(monitor ? usesHiddenSettings(monitor) : false);
    setEditing(
      monitor
        ? { id: monitor.id, draft: monitor, opened: monitor }
        : { id: null, draft: newMonitor(), opened: null }
    );
    setFocusTarget({ kind: 'field', id: ids.name });
  };

  const cancelEdit = () => {
    const id = editing?.id ?? null;
    setEditing(null);
    setShowErrors(false);
    setEditorRefusal(null);
    setFocusTarget(id ? { kind: 'row', id } : { kind: 'add' });
  };

  const saveEdit = () => {
    if (!editing || busy || locked) return;
    const errors = draftErrors;
    const hiddenField = errors.options
      ? ids.firstOption
      : errors.option
        ? ids.watched
        : errors.model
          ? ids.model
          : null;
    if (hasErrors(errors)) {
      setShowErrors(true);
      if (hiddenField) {
        setMoreOpen(true);
        setFocusTarget({ kind: 'field', id: hiddenField });
      } else if (errors.name) {
        setFocusTarget({ kind: 'field', id: ids.name });
      }
      return;
    }
    // A model list that could not be read is said under Model: reveal it once.
    if (catalogIssue && !moreOpen) {
      setMoreOpen(true);
      setFocusTarget({ kind: 'field', id: ids.model });
      return;
    }
    const draft = editing.draft;
    const trimmed: MonitorCriterion = {
      ...draft,
      name: draft.name.trim(),
      question: {
        ...draft.question,
        instructions: draft.question.instructions.trim(),
        kind:
          draft.question.kind.type === 'choice'
            ? {
                type: 'choice' as const,
                options: draft.question.kind.options.map((option) => ({
                  name: option.name.trim(),
                  description: option.description?.trim() || null,
                })),
              }
            : draft.question.kind,
      },
    };
    // An agent's monitor the person changed is the person's now; opened and
    // saved with nothing changed, it stays the agent's.
    const original = editing.opened;
    const changed =
      original !== null &&
      JSON.stringify(monitorToWire(trimmed)) !== JSON.stringify(monitorToWire(original));
    const owned = changed ? ownedByThePerson(trimmed) : trimmed;
    let savedId = owned.id;
    // The line above Save said the chat will be public: Save is the person's answer.
    const acceptPublic = editorEffect !== null;
    void write(
      (fresh) => {
        if (editing.id !== null) {
          const at = fresh.findIndex((monitor) => monitor.id === editing.id);
          // Removed elsewhere meanwhile: the person just asked to save it, so it is added back.
          return at < 0
            ? [...fresh, owned]
            : fresh.map((monitor, i) => (i === at ? owned : monitor));
        }
        savedId = monitorSlug(owned.name, new Set(fresh.map((monitor) => monitor.id)));
        return [...fresh, { ...owned, id: savedId }];
      },
      {
        done: () => {
          setEditing(null);
          setShowErrors(false);
          setEditorRefusal(null);
          setFocusTarget({ kind: 'row', id: savedId });
        },
        refused: (sentence) => setEditorRefusal(sentence),
      },
      { acceptPublic, inEditor: true }
    );
  };

  const toggle = (monitor: MonitorCriterion, enabled: boolean) => {
    if (busy || locked || editing) return;
    clearRowError(monitor.id);
    // Only the monitor being switched on is judged; one switched off never runs.
    if (enabled) {
      const errors = validateMonitor({ ...monitor, enabled: true }, context);
      const first = errors.model ?? errors.option ?? errors.options ?? errors.question;
      if (first) {
        setRowErrors((current) => ({ ...current, [monitor.id]: first }));
        return;
      }
    }
    setPendingToggle({ id: monitor.id, enabled });
    void write(
      (fresh) => {
        const at = fresh.findIndex((entry) => entry.id === monitor.id);
        if (at < 0) return null;
        // Switched by the person, an agent's monitor is the person's.
        return fresh.map((entry, i) =>
          i === at ? ownedByThePerson({ ...entry, enabled }) : entry
        );
      },
      {
        done: () => setPendingToggle(null),
        refused: (sentence) => {
          setPendingToggle(null);
          setRowErrors((current) => ({ ...current, [monitor.id]: sentence }));
        },
        asked: () => setPendingToggle(null),
      }
    );
  };

  const remove = (monitor: MonitorCriterion) => {
    if (busy || locked || editing) return;
    clearRowError(monitor.id);
    const index = monitors.findIndex((entry) => entry.id === monitor.id);
    void write((fresh) => fresh.filter((entry) => entry.id !== monitor.id), {
      done: (_stored, wrote) => {
        if (!wrote) return;
        setStatus({ kind: 'removed', name: monitor.name });
        setUndo({ monitor, index: Math.max(0, index) });
        setFocusTarget({ kind: 'undo' });
      },
      refused: (sentence) => setRowErrors((current) => ({ ...current, [monitor.id]: sentence })),
    });
  };

  const undoRemove = () => {
    if (!undo || busy) return;
    const { monitor, index } = undo;
    void write(
      (fresh) => {
        // Put back elsewhere already: nothing new to write for it.
        if (fresh.some((entry) => entry.id === monitor.id)) return null;
        const at = Math.min(index, fresh.length);
        return [...fresh.slice(0, at), monitor, ...fresh.slice(at)];
      },
      {
        done: () => setFocusTarget({ kind: 'row', id: monitor.id }),
        refused: (sentence) => {
          // Still undoable: the removal it would put back stands.
          setUndo({ monitor, index });
          setGeneralError(sentence);
        },
      }
    );
  };

  /**
   * Set (or, with `null`, clear) the chat's default decision model, applied when
   * picked like a row's switch. A pick that makes the chat public is asked about
   * under the select, in the line Save would show (DR-33), and sent again with the
   * person's answer; in draft mode it is held for the first message.
   */
  const setDefault = (model: DecisionModelRef | null) => {
    if (busy || locked || editing) return;
    if (decisionModelKey(model) === decisionModelKey(storedDefault)) return;
    setDefaultError(null);
    setPendingDefault({ model });
    void write(
      (fresh) => fresh,
      {
        done: () => setPendingDefault(null),
        refused: (sentence) => {
          setPendingDefault(null);
          setDefaultError(sentence);
        },
        asked: () => setPendingDefault(null),
      },
      { defaultModel: model }
    );
  };

  const atLimit = monitors.length >= MAX_MONITORS;
  const rowsLocked = locked || busy || editing !== null;
  const showEmpty = stored === 'ready' && monitors.length === 0 && !editing;
  const statusText =
    status.kind === 'saving'
      ? copy.saving
      : status.kind === 'saved'
        ? draft
          ? copy.savedDraft
          : copy.saved
        : status.kind === 'removed'
          ? copy.removed(status.name)
          : atLimit && !readOnly
            ? copy.limit
            : '';

  // The daemon's question (or, in draft mode, the dialog's own from the rows it read):
  // one line, its tip, and the two answers. Make it public sends the same write again
  // with the person's answer.
  const askingNode = asking ? (
    <div className="br-monitors__confirm flex flex-col gap-2" data-testid="monitors-public-confirm">
      <TierEffectLine
        line={asking.confirmation.message}
        info={asking.confirmation.info}
        infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
      />
      <div className="br-monitors__actions">
        <Button
          type="button"
          variant="secondary"
          size="sm"
          data-testid="monitors-public-cancel"
          onClick={() => setAsking(null)}
        >
          {TIER_EFFECT_COPY.confirmCancel}
        </Button>
        <Button
          type="button"
          size="sm"
          disabled={busy}
          data-testid="monitors-public-accept"
          onClick={asking.retry}
        >
          {TIER_EFFECT_COPY.confirmSave}
        </Button>
      </div>
    </div>
  ) : null;

  const editor = editing ? (
    <MonitorEditor
      draft={editing.draft}
      errors={showErrors ? draftErrors : {}}
      models={models}
      chatTier={chatTier}
      chatRefusal={chatRefusal}
      defaultModel={storedDefault}
      catalogProblem={catalogIssue}
      onRetryCatalog={() => setCatalogAttempt((attempt) => attempt + 1)}
      isSubagentChat={isSubagentChat}
      opened={editing.opened}
      moreOpen={moreOpen}
      onMoreOpenChange={setMoreOpen}
      ids={ids}
      busy={busy}
      refusal={editorRefusal}
      effect={editorEffect}
      confirmation={asking?.inEditor ? askingNode : null}
      onCancel={cancelEdit}
      onSave={saveEdit}
      onChange={(draft) => setEditing({ ...editing, draft })}
    />
  ) : null;

  const footRow =
    !readOnly && stored === 'ready' ? (
      <div className="br-monitors__foot" data-testid="monitors-foot">
        {showEmpty || editing?.id === null ? null : (
          <Button
            ref={addRef}
            type="button"
            variant="secondary"
            size="sm"
            disabled={rowsLocked || atLimit}
            data-testid="monitor-add"
            onClick={() => startEdit(null)}
          >
            <Plus aria-hidden="true" />
            {copy.add}
          </Button>
        )}
        <span className="br-monitors__status-slot">
          <span
            className="br-monitors__status"
            role="status"
            data-state={status.kind}
            data-testid="monitors-status"
          >
            {status.kind === 'saved' ? <Check aria-hidden="true" /> : null}
            {statusText}
          </span>
          {undo ? (
            <Button
              ref={undoRef}
              type="button"
              variant="link"
              size="xs"
              className="br-monitors__inline-link"
              aria-label={copy.undoLabel(undo.monitor.name)}
              disabled={busy}
              data-testid="monitor-undo"
              onClick={undoRemove}
            >
              {copy.undo}
            </Button>
          ) : null}
        </span>
      </div>
    ) : null;

  return (
    <ModalShell
      open={open}
      onOpenChange={onOpenChange}
      size="lg"
      // Constant through a write, so the × never flickers on a row flip; a form while
      // an editor is open, so a backdrop click does not drop the draft.
      purpose={editing ? 'form' : 'info'}
      title={copy.title}
      subtitle={copy.subtitle}
      scrollBody
      bodyClassName="flex flex-col gap-4 py-4"
      testId="monitors-dialog"
    >
      <div className="br-monitors" ref={listRef}>
        {draft ? (
          <p className="br-monitors__note" data-testid="monitors-draft-note">
            {copy.draftNote}
          </p>
        ) : null}
        {nothingUsable && !readOnly ? (
          <DecisionModelInstall
            reason={installReason}
            // Read the list again: the model just installed is in it now.
            onInstalled={() => setCatalogAttempt((attempt) => attempt + 1)}
          />
        ) : null}
        {stored !== 'error' ? (
          <DefaultModelField
            id={`${base}-default-model`}
            models={models}
            chatTier={chatTier}
            value={pendingDefault ? pendingDefault.model : storedDefault}
            disabled={readOnly || rowsLocked}
            error={defaultError}
            confirmation={asking?.atDefault ? askingNode : null}
            onChange={setDefault}
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
        ) : showEmpty ? (
          <EmptyState
            compact
            icon={SlidersHorizontal}
            title={copy.empty}
            description={copy.emptyDescription}
            className="br-monitors__empty"
            actions={
              readOnly ? undefined : (
                <Button
                  ref={addRef}
                  type="button"
                  size="sm"
                  disabled={rowsLocked}
                  data-testid="monitor-add"
                  onClick={() => startEdit(null)}
                >
                  {copy.add}
                </Button>
              )
            }
          />
        ) : null}
        {stored !== 'error' && (monitors.length > 0 || editing) ? (
          <ul
            className="biorouter-list-shell br-rows-flat br-monitors__rows"
            aria-label={copy.title}
          >
            {monitors.map((monitor) =>
              editing && editing.id === monitor.id ? (
                <li key={monitor.id} className="br-monitors__editor-item">
                  {editor}
                </li>
              ) : (
                <MonitorRow
                  key={monitor.id}
                  monitor={monitor}
                  models={models}
                  readOnly={readOnly}
                  locked={rowsLocked}
                  checked={
                    pendingToggle?.id === monitor.id ? pendingToggle.enabled : monitor.enabled
                  }
                  error={
                    rowErrors[monitor.id] ??
                    (monitor.enabled ? (validateMonitor(monitor, context).model ?? null) : null)
                  }
                  onToggle={(enabled) => toggle(monitor, enabled)}
                  onEdit={() => startEdit(monitor)}
                  onRemove={() => remove(monitor)}
                />
              )
            )}
            {editing && editing.id === null ? (
              <li className="br-monitors__editor-item">{editor}</li>
            ) : null}
            {editing && editing.id !== null && !monitors.some((m) => m.id === editing.id) ? (
              // Its row went while it was open (changed elsewhere): the editor stays.
              <li className="br-monitors__editor-item">{editor}</li>
            ) : null}
          </ul>
        ) : null}
        {footRow}
        {effectNote && !editing ? (
          <TierEffectLine
            line={effectNote.line}
            info={effectNote.info}
            infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
            testId="monitors-tier-effect"
          />
        ) : null}
        {asking && !asking.inEditor && !asking.atDefault ? askingNode : null}
        {generalError ? (
          <p className="br-monitors__error" role="alert" data-testid="monitors-save-error">
            {generalError}
          </p>
        ) : null}
      </div>
    </ModalShell>
  );
}

/**
 * The chat's default decision model (live QA M1-1, design §4): one quiet select at
 * the top of the dialog, with the picker's own rows. Its first row clears the
 * default, so a decision that names no model is chosen automatically within the
 * chat's tier. A public row that would make the chat public says so on the row, and
 * the pick is asked about under the select before anything is kept.
 */
function DefaultModelField({
  id,
  models,
  chatTier,
  value,
  disabled,
  error,
  confirmation,
  onChange,
}: {
  id: string;
  models: readonly DecisionModelEntry[];
  chatTier: DecisionTier | null;
  value: DecisionModelRef | null;
  disabled: boolean;
  error: string | null;
  confirmation: ReactNode;
  onChange: (model: DecisionModelRef | null) => void;
}) {
  const options: DecisionModelOption[] = decisionModelOptions({ models, chatTier });
  // The default's own default: no model named, chosen when a decision runs.
  options[0] = {
    value: '',
    name: copy.automatic,
    label: `${copy.automatic}, ${copy.automaticHint}`,
    secondary: copy.automaticHint,
    isPrivate: false,
    model: null,
    reason: null,
  };
  const selectedKey = decisionModelKey(
    value
      ? { provider: value.provider, model: modelEntryFor(value, models)?.model ?? value.model }
      : null
  );
  if (value && !options.some((option) => option.value === selectedKey)) {
    // A stored model the list does not hold (removed, or not read yet): kept, by its alias.
    const name = decisionModelLabel(value) ?? selectedKey;
    options.push({
      value: selectedKey,
      name,
      label: name,
      secondary: null,
      isPrivate: false,
      model: value,
      reason: null,
    });
  }
  const selected = options.find((option) => option.value === selectedKey) ?? options[0];
  const info = anyPublicModelRefused(models, chatTier)
    ? `${copy.defaultFieldInfo} ${TIER_EFFECT_COPY.publicRefusedInfo}`
    : copy.defaultFieldInfo;
  return (
    <div
      className="br-form-field flex min-w-0 flex-col gap-1.5"
      data-testid="monitors-default-model"
    >
      <span className="flex min-w-0 items-center gap-1">
        <label htmlFor={id} className="text-label text-text-default">
          {copy.defaultField}
        </label>
        <InfoTip label={copy.defaultField} help={info} />
      </span>
      <Select
        inputId={id}
        options={options}
        value={selected}
        isSearchable={false}
        isDisabled={disabled}
        menuPlacement="auto"
        menuPortalTarget={menuPortalTarget}
        isOptionDisabled={isDecisionModelOptionDisabled}
        formatOptionLabel={formatDecisionModelOption}
        onChange={(raw: unknown) => onChange((raw as DecisionModelOption | null)?.model ?? null)}
      />
      {error ? (
        <p className="br-monitors__error" role="alert" data-testid="monitors-default-error">
          {error}
        </p>
      ) : null}
      {confirmation}
    </div>
  );
}

/**
 * One monitor: its name and owner badge, then two facts (what it does and its rule),
 * whose peek holds the rest; `⋯` (and a right-click) for Edit and Remove; the Switch.
 */
function MonitorRow({
  monitor,
  models,
  readOnly,
  locked,
  checked,
  error,
  onToggle,
  onEdit,
  onRemove,
}: {
  monitor: MonitorCriterion;
  models: readonly DecisionModelEntry[];
  readOnly: boolean;
  locked: boolean;
  checked: boolean;
  error: string | null;
  onToggle: (enabled: boolean) => void;
  onEdit: () => void;
  onRemove: () => void;
}) {
  const base = useId();
  const nameId = `${base}-name`;
  const lineId = `${base}-line`;
  const peekId = useInfoTipId();
  const errorId = `${base}-error`;
  const items: RowActionItem[] = [
    { label: copy.edit, onSelect: onEdit, disabled: locked, testId: 'monitor-edit' },
    { kind: 'separator' },
    {
      label: copy.remove,
      onSelect: onRemove,
      destructive: true,
      disabled: locked,
      testId: 'monitor-remove',
    },
  ];
  const row = (
    <li
      className="biorouter-list-row br-monitors__entry flex items-center gap-3 px-3"
      data-testid="monitor-item"
      data-monitor-id={monitor.id}
    >
      <div className="br-monitors__row-text">
        <div className="br-monitors__row-title">
          <span id={nameId} className="br-monitors__row-name text-label text-text-default">
            {monitor.name}
          </span>
          {monitor.setBy === 'agent' ? (
            <InfoTip label={copy.setByAgent} help={copy.setByAgentInfo} asChild>
              <Badge data-testid="monitor-set-by-agent">{copy.setByAgent}</Badge>
            </InfoTip>
          ) : monitor.inheritedFrom ? (
            <InfoTip label={copy.inherited} help={copy.inheritedInfo} asChild>
              <Badge data-testid="monitor-inherited">{copy.inherited}</Badge>
            </InfoTip>
          ) : null}
        </div>
        {/* Below the line (R1-3): above it, the peek covered the row's own name. */}
        <InfoTip
          label={copy.peekLabel(monitor.name)}
          help={monitorPeek(monitor, models)}
          id={peekId}
          side="bottom"
          asChild
        >
          <span
            id={lineId}
            className="br-monitors__row-line text-supporting text-text-muted"
            data-testid="monitor-line"
          >
            {monitorRowLine(monitor)}
          </span>
        </InfoTip>
        {error ? (
          <p
            id={errorId}
            className="br-monitors__error"
            role="alert"
            data-testid="monitor-row-error"
          >
            {error}
          </p>
        ) : null}
      </div>
      {readOnly ? null : (
        <RowActions size="xs" menuLabel={copy.moreActions(monitor.name)} menu={items} />
      )}
      <Switch
        checked={checked}
        disabled={readOnly || locked}
        aria-labelledby={nameId}
        aria-describedby={[lineId, peekId, error ? errorId : null].filter(Boolean).join(' ')}
        data-testid="monitor-switch"
        onCheckedChange={(next) => onToggle(next === true)}
      />
    </li>
  );
  return readOnly ? (
    row
  ) : (
    <RowContextMenu items={items} disabled={locked}>
      {row}
    </RowContextMenu>
  );
}

export default MonitorsDialog;
