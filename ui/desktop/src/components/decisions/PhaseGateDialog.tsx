import { useEffect, useId, useRef, useState, type ComponentProps, type ReactNode } from 'react';
import { ModalShell } from '../ModalShell';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { Disclosure } from '../ui/disclosure';
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { InfoTip } from '../ui/info-tip';
import { Note } from '../ui/note';
import { RowActionMenuItems, type RowActionItem } from '../ui/row-actions';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { Slider } from '../ui/slider';
import { useSurfaceTriggerHint } from '../ui/surfaceTriggerHint';
import { Textarea } from '../ui/textarea';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { MoreHorizontal, Plus, X } from '../icons/app-icons';
import {
  fetchDecisionCatalog,
  fetchDecisionSettings,
  formatProbability,
  gateJudgeWords,
  phaseGateState,
  type DecisionHistoryFilter,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionTier,
  type GateOwner,
  type PhaseGateSnapshot,
  type PublicDecisionConfirmation,
} from '../../utils/sessionDecisions';
import {
  GATE_ATTEMPTS_MAX,
  GATE_DEFAULT_ATTEMPTS,
  GATE_DEFAULT_THRESHOLD,
  GATE_QUESTIONS_MAX,
  decidePhaseGate,
  gateDraftFrom,
  gateDraftHasErrors,
  gateModelPick,
  gateQuestionName,
  gateThresholdSteps,
  gateThresholdValue,
  newGateDraft,
  parseGateModel,
  removePhaseGate,
  setPhaseGate,
  validateGateDraft,
  type PhaseGateChoice,
  type PhaseGateDraft,
  type PhaseGateResult,
} from '../../utils/phaseGates';
import {
  isClosedPhase,
  type OrchestrationSnapshot,
  type PhaseSnapshot,
} from '../../utils/sessionSubagents';
import {
  anyPublicModelRefused,
  decisionModelKey,
  decisionModelOptions,
  formatDecisionModelOption,
  isDecisionModelOptionDisabled,
  type DecisionModelOption,
} from './decisionModelOption';
import {
  GATE_STATE_ICONS,
  TIER_EFFECT_COPY,
  decisionModelDisplay,
  publicModelsSentence,
  tierEffectNote,
} from './decisionWords';
import { TierEffectLine } from './TierEffectLine';
import './decisionsPanel.css';

export const PHASE_GATE_COPY = {
  title: 'Phase gate',
  addTitle: 'Add a phase gate',
  editTitle: 'Edit gate',
  owner: {
    agent: 'The agent',
    person: 'You',
    workflow: 'The workflow',
  } as Record<GateOwner, string>,
  status: {
    needsYou: 'Blocked: needs you',
    blocked: 'Blocked',
    passed: 'Passed',
    failed: 'Not passed',
    pending: 'Not asked yet',
    completed: 'Completed',
    skipped: 'Skipped',
    passedByYou: 'Passed by you',
    skippedByYou: 'Skipped by you',
    passStands: 'Your pass stands',
    skipStands: 'Your skip stands',
  },
  statusHelp:
    'Pass this phase counts it as passed and lets the agent complete it. Skip this phase lets the agent move on without it, and the phase is marked skipped.',
  statusHelpLabel: 'Pass and Skip',
  attempts: (used: number, max: number) =>
    `${used} of ${max} ${max === 1 ? 'attempt' : 'attempts'} used`,
  attemptsFact: (used: number, max: number) => `${used} of ${max} used`,
  retried: (n: number) => `given again ${n === 1 ? 'once' : n === 2 ? 'twice' : `${n} times`}`,
  threshold: (t: string) => `Every question must reach P(yes) ${t}`,
  /** R2: the probability names its outcome, beside the threshold. */
  lastCheck: (p: string, t: string) => `Lowest P(yes) ${p} · threshold ${t}`,
  questions: 'Questions',
  questionsUnknown: 'This Biorouter service does not list the questions.',
  lastError: (error: string) => `The last check decided nothing: ${error}`,
  holds: (titles: string, many: boolean) =>
    `${titles} ${many ? 'wait' : 'waits'} until this gate passes.`,
  laterPhase: 'a later phase',
  details: 'Details',
  facts: {
    threshold: 'Threshold',
    lastCheck: 'Last check',
    attempts: 'Attempts',
    model: 'Model',
    judgedBy: 'Judged by',
    probabilities: 'Probabilities',
    setBy: 'Set by',
  },
  chatDefault: "This chat's decision model, or its own model when it has none",
  uncalibrated: 'Accepts uncalibrated probabilities',
  /** R2-7: the banner's one line; why it waits is in the tip beside it. */
  waitingForYou: 'Waiting for you.',
  waitingWhy: 'Why it waits',
  needsPerson:
    'Only you can move this phase on now: pass it, skip it, give its gate its attempts again, or change or remove the gate.',
  blockedByErrors:
    'The gate could not be evaluated several times in a row. The agent can ask it again once the model that judges it can be reached, or you can decide here.',
  markedBlocked:
    'The agent marked this phase blocked, so the gate was not asked. The agent can ask it by completing the phase, or you can decide here.',
  standingPass:
    'You passed this phase. It completes without its gate once its working subagents finish and the agent completes it.',
  standingSkip:
    'You skipped this phase. It is skipped without its gate once its working subagents finish and the agent completes it.',
  closed: 'The phase is closed, so its gate holds nothing now.',
  noGate: 'This phase has no gate. The agent can complete it without one.',
  gone: 'This phase is no longer in the plan.',
  pass: 'Pass this phase',
  passTip: 'Counts the phase as passed, as if its gate had passed.',
  skip: 'Skip this phase',
  skipTip: 'Lets the agent move on without this phase. It is marked skipped.',
  more: 'More gate actions',
  retry: 'Try again',
  edit: 'Edit gate',
  remove: 'Remove gate…',
  add: 'Add gate',
  history: 'Decision history',
  working: 'Working',
  confirmRemove: {
    title: 'Remove this gate?',
    message: (phase: string) =>
      `Its questions and attempts are discarded, and the agent can complete ${phase} without a check. You can add a new gate afterwards.`,
    confirm: 'Remove gate',
    cancel: 'Cancel',
  },
  editor: {
    replaces: 'Saving makes this gate yours and starts its attempts over.',
    questions: 'Questions',
    questionsHelp:
      "A decision model answers each question from the phase's hand-off and its subagents' results, or the chat's own model when the chat has none. The phase completes only when every answer reaches the threshold.",
    question: (n: number) => `Question ${n}`,
    addQuestion: 'Add a question',
    removeQuestion: (n: number) => `Remove question ${n}`,
    questionLimit: `A gate asks at most ${GATE_QUESTIONS_MAX} questions.`,
    threshold: 'Threshold',
    thresholdHelp: 'The probability of yes each question must reach.',
    /** R2: the helper names its outcome, in words. */
    thresholdRule: (t: string) => `Passes when every question's P(yes) is at or above ${t}.`,
    thresholdLow:
      'Below 0.50 the gate passes answers the model leans against. That is your call to make.',
    moreOptions: 'More options',
    moreSummary: (attempts: number, model: string) =>
      `${attempts} ${attempts === 1 ? 'attempt' : 'attempts'} · ${model}`,
    attempts: 'Attempts',
    attemptsHelp: 'Checks that may fail before the phase is blocked and waits for you.',
    model: 'Model',
    modelHelp: 'A private chat can use only a private model.',
    modelsFailed:
      'Could not load the decision models. The chat’s decision model can still be used.',
    allowUncalibrated: 'Accept uncalibrated probabilities',
    allowUncalibratedHelp:
      'For a model that writes its own number instead of giving token probabilities.',
    save: 'Save gate',
    saving: 'Saving',
    cancel: 'Cancel',
    /** A refusal that names a field inside More options says which, in words. */
    refusalField: {
      model: 'Model',
      attempts: 'Attempts',
      uncalibrated: 'Probabilities',
    },
  },
};

const copy = PHASE_GATE_COPY;

/** The gate's state in a few words, as the dialog and the side panel's chip say it. */
export function phaseGateStatus(
  phase: Pick<PhaseSnapshot, 'status'>,
  gate: PhaseGateSnapshot
): string {
  const closed = isClosedPhase(phase);
  if (closed && gate.personDecision === 'pass') return copy.status.passedByYou;
  if (closed && gate.personDecision === 'skip') return copy.status.skippedByYou;
  // R2-6: a closed phase reads as its gate did, in the side panel's words ("Passed"),
  // never a word of its own; "Completed" is left only for a gate never asked.
  if (closed) {
    if (phase.status === 'skipped') return copy.status.skipped;
    if (gate.lastPassed === true) return copy.status.passed;
    if (gate.lastPassed === false) return copy.status.failed;
    return copy.status.completed;
  }
  if (gate.personDecision === 'pass') return copy.status.passStands;
  if (gate.personDecision === 'skip') return copy.status.skipStands;
  if (gate.needsPerson) return copy.status.needsYou;
  if (gate.blocked) return copy.status.blocked;
  if (gate.lastPassed === true) return copy.status.passed;
  if (gate.lastPassed === false) return copy.status.failed;
  return copy.status.pending;
}

/**
 * Whether the dialog needs the person now, so Pass is its primary button: the gate
 * waits for them ("Blocked: needs you"), or errors blocked it with no attempt left.
 * In every other state the footer has no primary: the safe course is to close.
 */
export function gateNeedsPerson(phase: Pick<PhaseSnapshot, 'status'>, gate: PhaseGateSnapshot) {
  if (isClosedPhase(phase) || gate.personDecision !== null) return false;
  return gate.needsPerson || (gate.blocked && gate.attempts >= gate.maxAttempts);
}

/** `Write`, `Write and Review`, `Write, Review and Ship`. */
function joinTitles(titles: string[]): string {
  if (titles.length <= 1) return titles[0] ?? '';
  return `${titles.slice(0, -1).join(', ')} and ${titles[titles.length - 1]}`;
}

/**
 * A gate's model as a person reads it (R3): `tev1:0.8b · this computer`, or "This
 * chat's decision model" when it names none. `raw` is the snapshot's own spelling, shown
 * only when it cannot be read as a model at all.
 */
function gateModelText(model: DecisionModelRef | null, raw: string | null): string {
  if (model) return decisionModelDisplay(model, null) ?? copy.chatDefault;
  return raw?.trim() || copy.chatDefault;
}

const menuPortalTarget = typeof document === 'undefined' ? undefined : document.body;

type RefusalField = 'model' | 'attempts' | 'uncalibrated';

/**
 * The field inside More options a refusal names, by the daemon's words, so the
 * editor can open it and focus the field. `null` for a refusal about anything else.
 */
export function refusalField(message: string): RefusalField | null {
  const text = message.toLowerCase();
  if (/uncalibrated|calibrat/.test(text)) return 'uncalibrated';
  if (/attempt/.test(text)) return 'attempts';
  if (/\bmodel\b/.test(text)) return 'model';
  return null;
}

/** `true` when the gate already uses a value inside More options. */
function usesMoreOptions(draft: PhaseGateDraft): boolean {
  return draft.maxAttempts !== GATE_DEFAULT_ATTEMPTS || !!draft.model || draft.allowUncalibrated;
}

function FieldLabel({
  id,
  htmlFor,
  label,
  help,
}: {
  id?: string;
  htmlFor?: string;
  label: string;
  help?: string | null;
}) {
  const Tag = htmlFor ? 'label' : 'span';
  return (
    <span className="br-gate-field__label">
      <Tag id={id} htmlFor={htmlFor} className="text-label text-text-default">
        {label}
      </Tag>
      {help ? <InfoTip label={label} help={help} /> : null}
    </span>
  );
}

function GateEditor({
  draft,
  onChange,
  errors,
  models,
  modelsFailed,
  chatTier,
  chatRefusal,
  defaultModel,
  disabled,
  replacing,
  scaleFor,
  opened,
  moreOpen,
  onMoreOpenChange,
  focusField,
  onFocused,
}: {
  draft: PhaseGateDraft;
  onChange: (next: PhaseGateDraft) => void;
  errors: ReturnType<typeof validateGateDraft>;
  models: readonly DecisionModelEntry[];
  modelsFailed: boolean;
  chatTier: DecisionTier | null;
  chatRefusal: string | null;
  /** The chat's stored default decision model, named on the default row; `null` when unknown or unset. */
  defaultModel: DecisionModelRef | null;
  disabled: boolean;
  replacing: boolean;
  /**
   * The threshold the gate had when the edit began. The scale is built from it,
   * not from the draft, so a threshold that is no stop (0.333, set from the CLI)
   * stays one for the whole edit, and moving off it does not rebuild the scale
   * under the thumb.
   */
  scaleFor: number;
  /**
   * The model the gate had when the edit began (`null` for none, or a new gate).
   * Its row stands for it exactly, adapter included (`gateModelPick`), so picking
   * that row again, or going back to it, changes nothing the person did not choose.
   */
  opened: DecisionModelRef | null;
  /** More options is controlled, so a refusal can open it before focusing a field. */
  moreOpen: boolean;
  onMoreOpenChange: (open: boolean) => void;
  focusField: RefusalField | null;
  onFocused: () => void;
}) {
  const base = useId();
  const steps = gateThresholdSteps(scaleFor);
  const options: DecisionModelOption[] = decisionModelOptions({
    models,
    chatTier,
    chatRefusal,
    defaultModel,
  });
  const selectedKey = decisionModelKey(draft.model);
  if (draft.model && !options.some((option) => option.value === selectedKey)) {
    const name = decisionModelDisplay(draft.model, null) ?? selectedKey;
    options.push({
      value: selectedKey,
      label: name,
      name,
      secondary: null,
      isPrivate: false,
      model: draft.model,
      reason: null,
    });
  }
  const selected = options.find((option) => option.value === selectedKey) ?? options[0];
  const attemptsRef = useRef<HTMLDivElement | null>(null);
  const uncalibratedRef = useRef<HTMLLabelElement | null>(null);

  // A refusal named a field inside More options: it is open now, so focus the field.
  useEffect(() => {
    if (!focusField || !moreOpen) return;
    const target =
      focusField === 'model'
        ? document.getElementById(`${base}-model`)
        : focusField === 'attempts'
          ? (attemptsRef.current?.querySelector<HTMLElement>(
              '[role="radio"][aria-checked="true"]'
            ) ?? attemptsRef.current?.querySelector<HTMLElement>('[role="radio"]'))
          : uncalibratedRef.current?.querySelector<HTMLElement>('input');
    target?.focus();
    onFocused();
  }, [focusField, moreOpen, base, onFocused]);

  const setQuestion = (index: number, question: string) =>
    onChange({
      ...draft,
      questions: draft.questions.map((entry, i) => (i === index ? { ...entry, question } : entry)),
    });

  return (
    <div className="br-gate-editor" data-testid="phase-gate-editor">
      {replacing ? <p className="br-gate-note">{copy.editor.replaces}</p> : null}
      <div className="br-gate-field">
        <FieldLabel
          id={`${base}-questions`}
          label={copy.editor.questions}
          help={copy.editor.questionsHelp}
        />
        <ol className="br-monitors__options" aria-labelledby={`${base}-questions`}>
          {draft.questions.map((entry, index) => (
            <li key={entry.name} className="flex min-w-0 flex-col gap-1">
              <span className="br-monitors__option">
                <Textarea
                  rows={2}
                  aria-label={copy.editor.question(index + 1)}
                  aria-invalid={errors.question?.[index] ? true : undefined}
                  value={entry.question}
                  disabled={disabled}
                  data-testid={`phase-gate-question-${index}`}
                  onChange={(event) => setQuestion(index, event.target.value)}
                />
                <Button
                  type="button"
                  variant="ghost"
                  size="xs"
                  shape="round"
                  disabled={disabled || draft.questions.length <= 1}
                  aria-label={copy.editor.removeQuestion(index + 1)}
                  onClick={() =>
                    onChange({ ...draft, questions: draft.questions.filter((_, i) => i !== index) })
                  }
                >
                  <X aria-hidden="true" />
                </Button>
              </span>
              {errors.question?.[index] ? (
                <p className="br-monitors__error" role="alert">
                  {errors.question[index]}
                </p>
              ) : null}
            </li>
          ))}
        </ol>
        {errors.questions ? (
          <p className="br-monitors__error" role="alert">
            {errors.questions}
          </p>
        ) : null}
        <div>
          <Button
            type="button"
            variant="ghost"
            size="sm"
            disabled={disabled || draft.questions.length >= GATE_QUESTIONS_MAX}
            data-testid="phase-gate-add-question"
            onClick={() =>
              onChange({
                ...draft,
                questions: [
                  ...draft.questions,
                  {
                    name: gateQuestionName(draft.questions.map((entry) => entry.name)),
                    question: '',
                  },
                ],
              })
            }
          >
            <Plus aria-hidden="true" />
            {copy.editor.addQuestion}
          </Button>
          {draft.questions.length >= GATE_QUESTIONS_MAX ? (
            <p className="br-gate-note">{copy.editor.questionLimit}</p>
          ) : null}
        </div>
      </div>
      <div className="br-gate-field">
        <FieldLabel
          id={`${base}-threshold`}
          label={copy.editor.threshold}
          help={copy.editor.thresholdHelp}
        />
        <div className="br-monitors__threshold">
          <Slider
            aria-labelledby={`${base}-threshold`}
            aria-describedby={`${base}-threshold-help`}
            steps={steps}
            value={gateThresholdValue(draft.threshold, steps)}
            disabled={disabled}
            data-testid="phase-gate-threshold"
            className="min-w-0 flex-1"
            onValueChange={(value) => onChange({ ...draft, threshold: Number(value) })}
          />
          <span
            className="br-monitors__threshold-value"
            aria-hidden="true"
            data-testid="phase-gate-threshold-value"
          >
            {formatProbability(draft.threshold)}
          </span>
        </div>
        <p
          className="br-gate-note"
          id={`${base}-threshold-help`}
          data-testid="phase-gate-threshold-help"
        >
          {draft.threshold < 0.5
            ? copy.editor.thresholdLow
            : copy.editor.thresholdRule(formatProbability(draft.threshold))}
        </p>
      </div>
      <Disclosure
        label={copy.editor.moreOptions}
        summary={copy.editor.moreSummary(draft.maxAttempts, gateModelText(draft.model, null))}
        open={moreOpen}
        onOpenChange={onMoreOpenChange}
      >
        <div className="br-gate-editor" data-testid="phase-gate-more-options">
          <div className="br-gate-field">
            <FieldLabel
              id={`${base}-attempts`}
              label={copy.editor.attempts}
              help={copy.editor.attemptsHelp}
            />
            <div ref={attemptsRef}>
              <SegmentedControl
                aria-labelledby={`${base}-attempts`}
                value={String(draft.maxAttempts)}
                disabled={disabled}
                data-testid="phase-gate-attempts"
                options={Array.from({ length: GATE_ATTEMPTS_MAX }, (_, i) => ({
                  value: String(i + 1),
                  label: String(i + 1),
                }))}
                onValueChange={(value) => onChange({ ...draft, maxAttempts: Number(value) })}
              />
            </div>
          </div>
          <div className="br-gate-field" data-testid="phase-gate-model">
            <FieldLabel
              htmlFor={`${base}-model`}
              label={copy.editor.model}
              help={
                chatTier === 'private'
                  ? copy.editor.modelHelp
                  : anyPublicModelRefused(models, chatTier)
                    ? TIER_EFFECT_COPY.publicRefusedInfo
                    : null
              }
            />
            <Select
              inputId={`${base}-model`}
              options={options}
              value={selected}
              isSearchable={false}
              isDisabled={disabled}
              menuPlacement="auto"
              menuPortalTarget={menuPortalTarget}
              isOptionDisabled={isDecisionModelOptionDisabled}
              formatOptionLabel={formatDecisionModelOption}
              onChange={(raw: unknown) =>
                onChange({
                  ...draft,
                  model: gateModelPick((raw as DecisionModelOption | null)?.model ?? null, opened),
                })
              }
            />
            {modelsFailed ? (
              <p className="br-gate-note" data-testid="phase-gate-models-error">
                {copy.editor.modelsFailed}
              </p>
            ) : null}
          </div>
          <span className="br-gate-field__label">
            <label
              className="br-dh-check"
              data-testid="phase-gate-uncalibrated"
              ref={uncalibratedRef}
            >
              <Checkbox
                checked={draft.allowUncalibrated}
                disabled={disabled}
                onChange={(event) =>
                  onChange({ ...draft, allowUncalibrated: event.target.checked })
                }
              />
              <span>{copy.editor.allowUncalibrated}</span>
            </label>
            <InfoTip
              label={copy.editor.allowUncalibrated}
              help={copy.editor.allowUncalibratedHelp}
            />
          </span>
        </div>
      </Disclosure>
    </div>
  );
}

function Fact({ label, children }: { label: string; children: ReactNode }) {
  return (
    <div className="br-decision-fact">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

/** What the gate is and where it stands: the dialog's read-only half. */
function GateFacts({
  phase,
  gate,
  phases,
}: {
  phase: PhaseSnapshot;
  gate: PhaseGateSnapshot;
  phases: readonly PhaseSnapshot[];
}) {
  const closed = isClosedPhase(phase);
  const status = phaseGateStatus(phase, gate);
  const notPassed = status === copy.status.failed;
  const Icon = GATE_STATE_ICONS[phaseGateState(gate)];
  const threshold = formatProbability(gate.threshold);
  // R6: a held phase id that matches no title reads "a later phase", never the id.
  const holds = gate.holds.map(
    (id) => phases.find((candidate) => candidate.id === id)?.title ?? copy.laterPhase
  );
  const model = parseGateModel(gate.model);
  const exhausted = gate.attempts >= gate.maxAttempts && gate.lastPassed === false && gate.blocked;
  // Why the open phase waits for the person, when it does and nothing of theirs stands.
  const needsNote =
    !closed && gate.needsPerson && gate.personDecision === null
      ? exhausted
        ? copy.needsPerson
        : gate.blocked
          ? copy.blockedByErrors
          : copy.markedBlocked
      : null;
  const standingNote =
    !closed && gate.personDecision === 'pass'
      ? copy.standingPass
      : !closed && gate.personDecision === 'skip'
        ? copy.standingSkip
        : null;
  // The one number that explains a not-passed or blocked gate is up front.
  const lastCheck =
    gate.lastMinP !== null ? copy.lastCheck(formatProbability(gate.lastMinP), threshold) : null;
  const lastCheckUpFront = lastCheck !== null && !closed && (notPassed || gate.blocked);
  return (
    <div className="br-gate-view" data-testid="phase-gate-facts">
      <div className="br-gate-status">
        <p className="br-gate-status__line">
          <Icon
            aria-hidden="true"
            className="br-gate-status__glyph"
            data-state={phaseGateState(gate)}
          />
          <span data-testid="phase-gate-status">
            {notPassed ? `${status} · ${copy.attempts(gate.attempts, gate.maxAttempts)}` : status}
          </span>
          {!closed ? <InfoTip label={copy.statusHelpLabel} help={copy.statusHelp} /> : null}
        </p>
        {lastCheckUpFront ? (
          <p className="br-gate-note" data-testid="phase-gate-last-check">
            {lastCheck}
          </p>
        ) : null}
      </div>
      {needsNote ? (
        // R2-7: one line up front; the full reason (and what each choice does) in a tip.
        <Note tone="warning" role="status" testId="phase-gate-needs-person">
          <span className="br-gate-waiting">
            {copy.waitingForYou}
            <InfoTip label={copy.waitingWhy} help={needsNote} />
          </span>
        </Note>
      ) : null}
      {standingNote ? (
        <Note role="status" testId="phase-gate-standing-note">
          {standingNote}
        </Note>
      ) : null}
      {closed ? <p className="br-gate-note">{copy.closed}</p> : null}
      {gate.lastError ? (
        <p className="br-gate-note" data-testid="phase-gate-last-error">
          {copy.lastError(gate.lastError)}
        </p>
      ) : null}
      <section aria-label={copy.questions} className="br-gate-field">
        <h3 className="text-label text-text-default">{copy.questions}</h3>
        {gate.questions.length > 0 ? (
          <ul className="br-gate-questions" data-testid="phase-gate-questions">
            {gate.questions.map(({ name, question }) => (
              <li key={name}>{question}</li>
            ))}
          </ul>
        ) : (
          <p className="br-gate-note">{copy.questionsUnknown}</p>
        )}
      </section>
      {!closed && holds.length > 0 ? (
        <p className="br-gate-note" data-testid="phase-gate-holds">
          {copy.holds(joinTitles(holds), holds.length > 1)}
        </p>
      ) : null}
      <Disclosure label={copy.details}>
        <dl className="br-decision-facts" data-testid="phase-gate-details">
          <Fact label={copy.facts.threshold}>{copy.threshold(threshold)}</Fact>
          {lastCheck && !lastCheckUpFront ? (
            <Fact label={copy.facts.lastCheck}>{lastCheck}</Fact>
          ) : null}
          <Fact label={copy.facts.attempts}>
            <span data-testid="phase-gate-attempts-used">
              {[
                copy.attemptsFact(gate.attempts, gate.maxAttempts),
                gate.retriesGranted > 0 ? copy.retried(gate.retriesGranted) : null,
              ]
                .filter(Boolean)
                .join(' · ')}
            </span>
          </Fact>
          <Fact label={copy.facts.model}>
            <span data-testid="phase-gate-model-line">{gateModelText(model, gate.model)}</span>
          </Fact>
          {gate.lastJudge ? (
            <Fact label={copy.facts.judgedBy}>
              <span data-testid="phase-gate-judge">{gateJudgeWords(gate.lastJudge)}</span>
              {gate.lastJudge.reason ? (
                <span className="block" data-testid="phase-gate-judge-reason">
                  {gate.lastJudge.reason}
                </span>
              ) : null}
            </Fact>
          ) : null}
          {gate.allowUncalibrated ? (
            <Fact label={copy.facts.probabilities}>{copy.uncalibrated}</Fact>
          ) : null}
          <Fact label={copy.facts.setBy}>
            <span data-testid="phase-gate-owner">{copy.owner[gate.owner]}</span>
          </Fact>
        </dl>
      </Disclosure>
    </div>
  );
}

/**
 * The gate's `⋯` in the footer: the page header's overflow recipe (`PageHeaderMenu`),
 * a ghost round button with a label tooltip, kept while open (R8), and the shared
 * `RowActionItem` rows, so the destructive look and the separator are the shared ones.
 * Not `RowActions`: its cluster is invisible until a list row hosts a hover.
 */
function GateMenu({ items, disabled }: { items: RowActionItem[]; disabled: boolean }) {
  const [open, setOpen] = useState(false);
  const hint = useSurfaceTriggerHint(open);
  return (
    <DropdownMenu open={open} onOpenChange={setOpen}>
      <Tooltip {...hint.tooltip}>
        <TooltipTrigger asChild {...hint.trigger}>
          <DropdownMenuTrigger asChild>
            <Button
              type="button"
              variant="ghost"
              shape="round"
              aria-label={copy.more}
              disabled={disabled}
              data-testid="phase-gate-more"
              className="br-surface-trigger br-gate-footer__menu"
            >
              <MoreHorizontal aria-hidden />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        <TooltipContent>{copy.more}</TooltipContent>
      </Tooltip>
      <DropdownMenuContent align="start">
        <RowActionMenuItems items={items} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/** A footer button with its label tooltip saying what it does. */
function TipButton({ tip, children, ...props }: ComponentProps<typeof Button> & { tip: string }) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        {/* `data-variant` says which button is the footer's primary, to tests and probes. */}
        <Button type="button" data-variant={props.variant ?? 'default'} {...props}>
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent>{tip}</TooltipContent>
    </Tooltip>
  );
}

export interface PhaseGateDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  sessionId: string;
  /**
   * The phase as the side panel last read it, with its gate when it has one. A
   * phase without a gate opens the editor, to add one.
   */
  phase: PhaseSnapshot | null;
  /** The plan's phases, for the titles of the phases a gate holds. */
  phases?: readonly PhaseSnapshot[];
  /** The chat's tier: a private chat refuses a public model. `null` when not known. */
  chatTier?: DecisionTier | null;
  /** Open the decision history (the side panel), filtered. The dialog closes first. */
  onOpenHistory?: (filter: DecisionHistoryFilter) => void;
  /** After a write the daemon accepted: the plan it answered with, `null` when it sent none. */
  onChanged?: (plan: OrchestrationSnapshot | null) => void;
  /** For tests: the catalog, instead of `GET /decisions/models`. */
  models?: DecisionModelEntry[];
  /** For tests, with `models`: the catalog's `sessionUnavailableReason`. */
  chatRefusal?: string | null;
  /**
   * The chat's default decision model (what a gate with no model asks), instead of
   * reading `GET /sessions/{id}/decision-settings` when the editor opens.
   */
  defaultModel?: DecisionModelRef | null;
}

type Busy = PhaseGateChoice | 'remove' | 'save' | null;

/**
 * A phase gate, and the person's controls over it (item C of the decision
 * supervision work; F-4, C-7, C-8). Up front: where the gate stands (with an InfoTip
 * saying what Pass and Skip do), the last check for a gate that did not pass, the
 * notes that say what the person can do, the questions and what the gate holds;
 * the threshold, attempts, model and who set it under Details. The footer holds
 * Skip this phase and Pass this phase (primary only when the person is needed) and a
 * `⋯` with Try again, Edit gate, Decision history and Remove gate… (last,
 * destructive, confirmed). A phase without a gate opens on the editor ("Add gate"),
 * which edits in place with one Cancel.
 *
 * Focus rests on the dialog box after any open, so Enter passes nothing until the
 * person moves to a button.
 *
 * Every gate binds the agent, whoever set it: the agent cannot change, skip or
 * remove it, and a gate that failed every attempt waits for the person. The
 * writes go through `utils/phaseGates.ts` with the person's proof; a daemon that
 * holds no user-action key (`biorouter serve`) checks only that the chat is in
 * reach, so the dialog works the same in a browser. A refusal (400, 403, 404,
 * 409, 422) is shown in the daemon's words, inline.
 *
 * The editor's threshold runs from 0.05 to 0.99: below 0.50 is allowed, because
 * the agent's floor is the agent's, and a person may decide a gate should pass
 * answers the model leans against.
 */
export function PhaseGateDialog({
  open,
  onOpenChange,
  sessionId,
  phase: givenPhase,
  phases: givenPhases,
  chatTier = null,
  onOpenHistory,
  onChanged,
  models: givenModels,
  chatRefusal: givenChatRefusal,
  defaultModel: givenDefaultModel,
}: PhaseGateDialogProps) {
  // The phase as the last write answered it, until the caller reads a newer one.
  const [answered, setAnswered] = useState<{
    phase: PhaseSnapshot | null;
    phases: readonly PhaseSnapshot[];
  } | null>(null);
  const [editing, setEditingDraft] = useState<PhaseGateDraft | null>(null);
  const [scaleFor, setScaleFor] = useState(GATE_DEFAULT_THRESHOLD);
  const [openedModel, setOpenedModel] = useState<DecisionModelRef | null>(null);
  const [moreOpen, setMoreOpen] = useState(false);
  const [focusField, setFocusField] = useState<RefusalField | null>(null);
  // Starting an edit fixes the threshold scale for it (`GateEditor`'s `scaleFor`)
  // and the model it began with (`opened`), and opens More options when the gate
  // already uses a value inside it.
  const startEditing = (draft: PhaseGateDraft | null) => {
    if (draft) {
      setScaleFor(draft.threshold);
      setOpenedModel(draft.model);
      setMoreOpen(usesMoreOptions(draft));
    }
    setFocusField(null);
    setAsking(null);
    setEditingDraft(draft);
  };
  const [showErrors, setShowErrors] = useState(false);
  const [busy, setBusy] = useState<Busy>(null);
  const [outcome, setOutcome] = useState<{ message: string; applied: boolean } | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  /** DR-33: the daemon's question for a set it would not take unanswered ("This chat will be public: …"). */
  const [asking, setAsking] = useState<PublicDecisionConfirmation | null>(null);
  /** What the last set did to the chat's tier, in one line and a tip. */
  const [effectNote, setEffectNote] = useState<{ line: string; info: string | null } | null>(null);
  const [confirmingRemove, setConfirmingRemove] = useState(false);
  const [fetchedModels, setFetchedModels] = useState<DecisionModelEntry[]>([]);
  const [fetchedRefusal, setFetchedRefusal] = useState<string | null>(null);
  const [modelsFailed, setModelsFailed] = useState(false);
  const [fetchedDefaultModel, setFetchedDefaultModel] = useState<DecisionModelRef | null>(null);

  const phase = answered ? answered.phase : givenPhase;
  const phases = answered ? answered.phases : (givenPhases ?? (givenPhase ? [givenPhase] : []));
  const gate = phase?.gate ?? null;
  const models = givenModels ?? fetchedModels;
  const chatRefusal = givenModels ? (givenChatRefusal ?? null) : fetchedRefusal;
  const defaultModel = givenDefaultModel !== undefined ? givenDefaultModel : fetchedDefaultModel;
  const phaseId = phase?.id ?? givenPhase?.id ?? '';

  // Each open starts from what the side panel read; a phase with no gate opens the
  // editor. Done while rendering the open (not in an effect), so the dialog's first
  // frame, and its open-focus decision, already show the right body.
  const [seenOpen, setSeenOpen] = useState(false);
  if (open !== seenOpen) {
    setSeenOpen(open);
    if (open) {
      setAnswered(null);
      setOutcome(null);
      setRefusal(null);
      setEffectNote(null);
      setShowErrors(false);
      setBusy(null);
      setConfirmingRemove(false);
      startEditing(
        givenPhase && !givenPhase.gate && !isClosedPhase(givenPhase) ? newGateDraft() : null
      );
    }
  }

  // A newer read of the plan replaces what the last write answered.
  useEffect(() => {
    setAnswered(null);
  }, [givenPhase]);

  // The editor's model list, read when the editor opens, named for this chat so
  // each model carries the daemon's own reason this chat may not use it.
  const editorOpen = open && editing !== null;
  useEffect(() => {
    if (!editorOpen || givenModels) return;
    const controller = new AbortController();
    setModelsFailed(false);
    fetchDecisionCatalog(controller.signal, sessionId || undefined)
      .then((catalog) => {
        if (controller.signal.aborted) return;
        setFetchedModels(catalog.models);
        setFetchedRefusal(catalog.sessionUnavailableReason);
      })
      .catch(() => {
        if (!controller.signal.aborted) setModelsFailed(true);
      });
    return () => controller.abort();
  }, [editorOpen, givenModels, sessionId]);

  // The chat's default decision model, so the default row says what it resolves
  // to. Best effort: an unreadable answer leaves the row as plain "Default model".
  const readDefaultModel = givenDefaultModel === undefined;
  useEffect(() => {
    if (!editorOpen || !readDefaultModel || !sessionId) return;
    const controller = new AbortController();
    fetchDecisionSettings(sessionId, controller.signal)
      .then(({ data }) => {
        if (!controller.signal.aborted) setFetchedDefaultModel(data?.defaultModel ?? null);
      })
      .catch(() => {
        if (!controller.signal.aborted) setFetchedDefaultModel(null);
      });
    return () => controller.abort();
  }, [editorOpen, readDefaultModel, sessionId]);

  const settle = (result: PhaseGateResult) => {
    if (!result.ok) {
      // Asked, not refused: the line goes above Save, which then answers it.
      if (result.confirmation) setAsking(result.confirmation);
      else setRefusal(result.message);
      return false;
    }
    setRefusal(null);
    setAsking(null);
    setOutcome({ message: result.message, applied: result.applied });
    setEffectNote(tierEffectNote(result.tierEffect ?? null, result.tierEffectLine ?? null));
    if (result.plan) {
      setAnswered({
        phase: result.plan.phases.find((candidate) => candidate.id === phaseId) ?? null,
        phases: result.plan.phases,
      });
    }
    onChanged?.(result.plan);
    return true;
  };

  const run = async (action: Exclude<Busy, null>, request: () => Promise<PhaseGateResult>) => {
    if (busy || !phaseId) return false;
    setBusy(action);
    setOutcome(null);
    setRefusal(null);
    setEffectNote(null);
    try {
      return settle(await request());
    } finally {
      setBusy(null);
    }
  };

  const decide = (choice: PhaseGateChoice) =>
    void run(choice, () => decidePhaseGate(sessionId, phaseId, choice));

  const draftErrors = editing ? validateGateDraft(editing) : {};
  // DR-33: the gate's model (or the chat's default, for a gate with none) is one the
  // daemon marked as making this chat public. Said above Save, which then answers it.
  const gateModelRef = editing ? (editing.model ?? defaultModel) : null;
  const loweringEntry = gateModelRef
    ? models.find(
        (entry) =>
          entry.provider === gateModelRef.provider &&
          entry.model === gateModelRef.model &&
          entry.chatEffect === 'makesPublic'
      )
    : undefined;
  const gateEffect = asking
    ? { line: asking.message, info: asking.info }
    : loweringEntry
      ? {
          line: TIER_EFFECT_COPY.willBePublic(publicModelsSentence([loweringEntry])),
          info: TIER_EFFECT_COPY.publicInfo(null),
        }
      : null;
  const save = async () => {
    if (!editing) return;
    if (gateDraftHasErrors(draftErrors)) {
      setShowErrors(true);
      return;
    }
    // The line above Save said the chat will be public: Save is the person's answer.
    const acceptPublicDecisionModels = gateEffect !== null;
    const saved = await run('save', () =>
      setPhaseGate(sessionId, phaseId, editing, { acceptPublicDecisionModels })
    );
    if (saved) {
      startEditing(null);
      setShowErrors(false);
    }
  };

  // A refusal that names a field inside More options opens it and focuses the field,
  // because a closed Disclosure unmounts its content: it can never point at a field the
  // person cannot see.
  const refusedField = editing && refusal ? refusalField(refusal) : null;
  useEffect(() => {
    if (!refusedField) return;
    setMoreOpen(true);
    setFocusField(refusedField);
  }, [refusedField, refusal]);

  const closed = phase ? isClosedPhase(phase) : false;
  const working = busy !== null;
  const adding = editing !== null && !gate;
  const primaryPass = phase && gate ? gateNeedsPerson(phase, gate) : false;

  const showHistory = () => {
    onOpenChange(false);
    onOpenHistory?.({ origin: 'gate', originId: phaseId });
  };

  const menuItems: RowActionItem[] = gate
    ? [
        ...(gate.attempts > 0 || gate.blocked
          ? [{ label: copy.retry, onSelect: () => decide('retry'), testId: 'phase-gate-retry' }]
          : []),
        {
          label: copy.edit,
          testId: 'phase-gate-edit',
          onSelect: () => {
            setOutcome(null);
            setRefusal(null);
            setShowErrors(false);
            startEditing(gateDraftFrom(gate));
          },
        },
        ...(onOpenHistory
          ? [{ label: copy.history, onSelect: showHistory, testId: 'phase-gate-history' }]
          : []),
        { kind: 'separator' as const },
        {
          label: copy.remove,
          destructive: true,
          testId: 'phase-gate-remove',
          onSelect: () => setConfirmingRemove(true),
        },
      ]
    : [];

  const footer = !phase ? undefined : editing ? (
    <>
      <Button
        type="button"
        variant="ghost"
        disabled={working}
        onClick={() => {
          setShowErrors(false);
          setRefusal(null);
          // Adding: Cancel leaves with nothing set. Editing: back to the gate.
          if (gate) startEditing(null);
          else onOpenChange(false);
        }}
      >
        {copy.editor.cancel}
      </Button>
      <Button
        type="button"
        disabled={working}
        aria-busy={busy === 'save' || undefined}
        data-testid="phase-gate-save"
        onClick={() => void save()}
      >
        {busy === 'save'
          ? copy.editor.saving
          : asking
            ? TIER_EFFECT_COPY.confirmSave
            : copy.editor.save}
      </Button>
    </>
  ) : gate && !closed ? (
    <>
      <GateMenu items={menuItems} disabled={working} />
      {/* The two decisions wrap TOGETHER onto the next line at a narrow width. */}
      <span className="br-gate-footer__actions">
        {/* Quiet (R2-7, R7): Pass is the one decision the footer leads with. */}
        <TipButton
          tip={copy.skipTip}
          variant="ghost"
          disabled={working}
          aria-busy={busy === 'skip' || undefined}
          data-testid="phase-gate-skip"
          onClick={() => decide('skip')}
        >
          {busy === 'skip' ? copy.working : copy.skip}
        </TipButton>
        <TipButton
          tip={copy.passTip}
          variant={primaryPass ? 'default' : 'secondary'}
          disabled={working}
          aria-busy={busy === 'pass' || undefined}
          data-testid="phase-gate-pass"
          onClick={() => decide('pass')}
        >
          {busy === 'pass' ? copy.working : copy.pass}
        </TipButton>
      </span>
    </>
  ) : closed ? (
    onOpenHistory ? (
      <Button
        type="button"
        variant="secondary"
        disabled={working}
        data-testid="phase-gate-history"
        onClick={showHistory}
      >
        {copy.history}
      </Button>
    ) : undefined
  ) : (
    <Button
      type="button"
      data-variant="default"
      disabled={working}
      data-testid="phase-gate-add"
      onClick={() => {
        setOutcome(null);
        setRefusal(null);
        startEditing(newGateDraft());
      }}
    >
      {copy.add}
    </Button>
  );

  const fieldWord = refusedField ? copy.editor.refusalField[refusedField] : null;

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!working) onOpenChange(next);
      }}
      size="md"
      purpose={working ? 'required' : editing ? 'form' : 'info'}
      title={adding ? copy.addTitle : editing ? copy.editTitle : copy.title}
      subtitle={phase?.title ?? givenPhase?.title}
      footer={footer}
      scrollBody
      bodyClassName="flex flex-col gap-3 py-3"
      testId="phase-gate-dialog"
      // C-8: the first button overrides a gate, so a KEY open rests on the box too (a
      // pointer open already does). The editor keeps the default: its first stop is a
      // textarea.
      onOpenAutoFocus={
        editing
          ? undefined
          : (event) => {
              event.preventDefault();
              const box =
                event.currentTarget instanceof HTMLElement
                  ? event.currentTarget
                  : event.target instanceof HTMLElement
                    ? event.target
                    : null;
              box?.focus({ preventScroll: true });
            }
      }
    >
      {!phase ? (
        <p className="br-gate-note" data-testid="phase-gate-gone">
          {copy.gone}
        </p>
      ) : editing ? (
        <GateEditor
          draft={editing}
          onChange={setEditingDraft}
          errors={showErrors ? draftErrors : {}}
          models={models}
          modelsFailed={modelsFailed}
          chatTier={chatTier}
          chatRefusal={chatRefusal}
          defaultModel={defaultModel}
          disabled={working}
          replacing={!!gate}
          scaleFor={scaleFor}
          opened={openedModel}
          moreOpen={moreOpen}
          onMoreOpenChange={setMoreOpen}
          focusField={focusField}
          onFocused={() => setFocusField(null)}
        />
      ) : gate ? (
        <GateFacts phase={phase} gate={gate} phases={phases} />
      ) : (
        <p className="br-gate-note" data-testid="phase-gate-none">
          {copy.noGate}
        </p>
      )}
      {editing && phase && gateEffect ? (
        <TierEffectLine
          line={gateEffect.line}
          info={gateEffect.info}
          infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
          testId="phase-gate-tier-effect"
        />
      ) : null}
      {effectNote && !editing ? (
        <TierEffectLine
          line={effectNote.line}
          info={effectNote.info}
          infoLabel={TIER_EFFECT_COPY.publicInfoLabel}
          testId="phase-gate-tier-effect-saved"
        />
      ) : null}
      {outcome ? (
        <Note
          tone={outcome.applied ? 'success' : 'neutral'}
          role="status"
          testId={outcome.applied ? 'phase-gate-applied' : 'phase-gate-standing'}
        >
          {outcome.message}
        </Note>
      ) : null}
      {refusal ? (
        <Note tone="danger" role="alert" testId="phase-gate-refusal">
          {fieldWord ? `${fieldWord}: ${refusal}` : refusal}
        </Note>
      ) : null}
      {phase && gate ? (
        <ConfirmationModal
          isOpen={confirmingRemove}
          title={copy.confirmRemove.title}
          message={copy.confirmRemove.message(phase.title)}
          confirmLabel={copy.confirmRemove.confirm}
          cancelLabel={copy.confirmRemove.cancel}
          confirmVariant="destructive"
          isSubmitting={busy === 'remove'}
          onCancel={() => setConfirmingRemove(false)}
          onConfirm={() => {
            void run('remove', () => removePhaseGate(sessionId, phaseId)).finally(() =>
              setConfirmingRemove(false)
            );
          }}
        />
      ) : null}
    </ModalShell>
  );
}

export default PhaseGateDialog;
