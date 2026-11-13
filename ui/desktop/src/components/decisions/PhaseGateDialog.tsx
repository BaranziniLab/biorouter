import { useEffect, useId, useState } from 'react';
import { ModalShell } from '../ModalShell';
import { Badge } from '../ui/badge';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { Note } from '../ui/note';
import { SegmentedControl } from '../ui/segmented-control';
import { Select } from '../ui/Select';
import { Slider } from '../ui/slider';
import { Textarea } from '../ui/textarea';
import { Plus, X } from '../icons/app-icons';
import {
  decisionModelLabel,
  fetchDecisionCatalog,
  formatProbability,
  type DecisionHistoryFilter,
  type DecisionModelEntry,
  type DecisionModelRef,
  type DecisionTier,
  type GateOwner,
  type PhaseGateSnapshot,
} from '../../utils/sessionDecisions';
import {
  GATE_ATTEMPTS_MAX,
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
import './decisionsPanel.css';

export const PHASE_GATE_COPY = {
  title: 'Phase gate',
  addTitle: 'Add a phase gate',
  owner: {
    agent: 'Set by the agent',
    person: 'Set by you',
    workflow: 'Set by the workflow',
  } as Record<GateOwner, string>,
  status: {
    needsYou: 'Blocked: needs you',
    blocked: 'Blocked',
    passed: 'Passed',
    failed: 'Failed',
    pending: 'Not asked yet',
    completed: 'Completed',
    skipped: 'Skipped',
    passedByYou: 'Passed by you',
    skippedByYou: 'Skipped by you',
    passStands: 'Your pass stands',
    skipStands: 'Your skip stands',
  },
  attempts: (used: number, max: number) =>
    `${used} of ${max} ${max === 1 ? 'attempt' : 'attempts'} used`,
  retried: (n: number) =>
    n === 1 ? 'You gave it its attempts again once.' : `You gave it its attempts again ${n} times.`,
  threshold: (t: string) => `Every question must reach P(yes) ${t}.`,
  questions: 'Questions',
  questionsUnknown: 'This Biorouter service does not list the questions.',
  lastCheck: (p: string, t: string) => `Lowest P(yes) at the last check: ${p}, needs ${t}.`,
  lastError: (error: string) => `The last check decided nothing: ${error}`,
  holds: (titles: string) => `Holds ${titles} until it passes or you pass or skip it.`,
  model: (label: string) => `Decision model: ${label}`,
  chatDefault: "the chat's decision model",
  uncalibrated: 'Accepts uncalibrated probabilities.',
  needsPerson:
    'Only you can move this phase on now: pass it, skip it, give its gate its attempts again, or change or remove the gate.',
  blockedByErrors:
    'The gate could not be evaluated several times in a row. The agent can ask it again once a decision model this chat may use is reachable, or you can decide here.',
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
  skip: 'Skip this phase',
  retry: 'Try again',
  edit: 'Edit gate',
  remove: 'Remove gate',
  add: 'Add gate',
  history: 'Show history',
  working: 'Working',
  editor: {
    about:
      'A decision model answers these yes/no questions from the phase’s hand-off and its subagents’ results. The phase completes only when every answer reaches the threshold, and the phases after it wait until then.',
    replaces: 'Saving makes this gate yours and starts its attempts over.',
    questions: 'Questions',
    question: (n: number) => `Question ${n}`,
    addQuestion: 'Add a question',
    removeQuestion: (n: number) => `Remove question ${n}`,
    questionLimit: `A gate asks at most ${GATE_QUESTIONS_MAX} questions.`,
    threshold: 'Threshold',
    thresholdHelp: 'The P(yes) every question must reach.',
    thresholdLow:
      'Below 0.50 the gate passes answers the decision model leans against. That is your call to make.',
    attempts: 'Attempts',
    attemptsHelp: 'Failed checks allowed before the phase is blocked and waits for you.',
    model: 'Model',
    modelHelp: 'Only a private model may read a private chat.',
    modelsFailed:
      'Could not load the decision models. The chat’s decision model can still be used.',
    allowUncalibrated: 'Accept uncalibrated probabilities',
    allowUncalibratedHelp:
      'For a model that writes its own number instead of giving token probabilities.',
    save: 'Save gate',
    saving: 'Saving',
    cancel: 'Cancel',
    private: 'Private',
    public: 'Public',
    local: 'this computer',
    publicRefused: 'This chat is private, so a public model may not read it.',
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
  if (closed) return phase.status === 'skipped' ? copy.status.skipped : copy.status.completed;
  if (gate.personDecision === 'pass') return copy.status.passStands;
  if (gate.personDecision === 'skip') return copy.status.skipStands;
  if (gate.needsPerson) return copy.status.needsYou;
  if (gate.blocked) return copy.status.blocked;
  if (gate.lastPassed === true) return copy.status.passed;
  if (gate.lastPassed === false) return copy.status.failed;
  return copy.status.pending;
}

/** `Write and Review`, `Write, Review and Ship`. */
function joinTitles(titles: string[]): string {
  if (titles.length <= 1) return titles[0] ?? '';
  return `${titles.slice(0, -1).join(', ')} and ${titles[titles.length - 1]}`;
}

interface ModelOption {
  value: string;
  label: string;
  model: DecisionModelRef | null;
  reason: string | null;
}

const menuPortalTarget = typeof document === 'undefined' ? undefined : document.body;

function modelKey(model: Pick<DecisionModelRef, 'provider' | 'model'> | null): string {
  return model ? `${model.provider}/${model.model}` : '';
}

function GateEditor({
  draft,
  onChange,
  errors,
  models,
  modelsFailed,
  chatTier,
  chatRefusal,
  disabled,
  replacing,
  scaleFor,
  opened,
}: {
  draft: PhaseGateDraft;
  onChange: (next: PhaseGateDraft) => void;
  errors: ReturnType<typeof validateGateDraft>;
  models: readonly DecisionModelEntry[];
  modelsFailed: boolean;
  chatTier: DecisionTier | null;
  chatRefusal: string | null;
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
}) {
  const base = useId();
  const steps = gateThresholdSteps(scaleFor);
  const options: ModelOption[] = [
    { value: '', label: copy.chatDefault, model: null, reason: chatRefusal },
    ...models.map((entry) => ({
      value: modelKey(entry),
      label: [
        entry.displayName,
        entry.tier === 'private'
          ? copy.editor.private
          : entry.tier === 'public'
            ? copy.editor.public
            : null,
        entry.local ? copy.editor.local : null,
      ]
        .filter(Boolean)
        .join(' · '),
      model: { provider: entry.provider, model: entry.model, adapter: null },
      // The daemon's own reason first (a tier, an institution, a Crew grant).
      reason:
        entry.unavailableReason ??
        (chatTier === 'private' && entry.tier === 'public' ? copy.editor.publicRefused : null),
    })),
  ];
  const selectedKey = modelKey(draft.model);
  if (draft.model && !options.some((option) => option.value === selectedKey)) {
    options.push({
      value: selectedKey,
      label: decisionModelLabel(draft.model) ?? selectedKey,
      model: draft.model,
      reason: null,
    });
  }
  const selected = options.find((option) => option.value === selectedKey) ?? options[0];
  const setQuestion = (index: number, question: string) =>
    onChange({
      ...draft,
      questions: draft.questions.map((entry, i) => (i === index ? { ...entry, question } : entry)),
    });

  return (
    // A filled box, so the fields in it rest one step back: a fill never sits on a fill.
    <div className="br-monitors__editor br-fill-surface" data-testid="phase-gate-editor">
      <p className="br-monitors__note">{copy.editor.about}</p>
      {replacing ? <p className="br-monitors__note">{copy.editor.replaces}</p> : null}
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-questions`}>
          {copy.editor.questions}
        </span>
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
            <p className="br-monitors__note">{copy.editor.questionLimit}</p>
          ) : null}
        </div>
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-threshold`}>
          {copy.editor.threshold}
        </span>
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
        <p className="br-monitors__note" id={`${base}-threshold-help`}>
          {draft.threshold < 0.5 ? copy.editor.thresholdLow : copy.editor.thresholdHelp}
        </p>
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5">
        <span className="text-label text-text-default" id={`${base}-attempts`}>
          {copy.editor.attempts}
        </span>
        <SegmentedControl
          aria-labelledby={`${base}-attempts`}
          aria-describedby={`${base}-attempts-help`}
          value={String(draft.maxAttempts)}
          disabled={disabled}
          data-testid="phase-gate-attempts"
          options={Array.from({ length: GATE_ATTEMPTS_MAX }, (_, i) => ({
            value: String(i + 1),
            label: String(i + 1),
          }))}
          onValueChange={(value) => onChange({ ...draft, maxAttempts: Number(value) })}
        />
        <p className="br-monitors__note" id={`${base}-attempts-help`}>
          {copy.editor.attemptsHelp}
        </p>
      </div>
      <div className="br-form-field flex min-w-0 flex-col gap-1.5" data-testid="phase-gate-model">
        <label htmlFor={`${base}-model`} className="text-label text-text-default">
          {copy.editor.model}
        </label>
        <Select
          inputId={`${base}-model`}
          options={options}
          value={selected}
          isSearchable={false}
          isDisabled={disabled}
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
            onChange({
              ...draft,
              model: gateModelPick((raw as ModelOption | null)?.model ?? null, opened),
            })
          }
        />
        {modelsFailed ? (
          <p className="br-monitors__note" data-testid="phase-gate-models-error">
            {copy.editor.modelsFailed}
          </p>
        ) : chatTier === 'private' ? (
          <p className="br-monitors__note">{copy.editor.modelHelp}</p>
        ) : null}
      </div>
      <label className="br-dh-check" data-testid="phase-gate-uncalibrated">
        <Checkbox
          checked={draft.allowUncalibrated}
          disabled={disabled}
          onChange={(event) => onChange({ ...draft, allowUncalibrated: event.target.checked })}
        />
        <span>{copy.editor.allowUncalibrated}</span>
      </label>
      <p className="br-monitors__note">{copy.editor.allowUncalibratedHelp}</p>
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
  const threshold = formatProbability(gate.threshold);
  const holds = gate.holds.map(
    (id) => phases.find((candidate) => candidate.id === id)?.title ?? id
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
  return (
    <div className="br-monitors" data-testid="phase-gate-facts">
      <span className="flex flex-wrap items-center gap-2">
        <Badge data-testid="phase-gate-owner">{copy.owner[gate.owner]}</Badge>
        <span className="br-monitors__value" data-testid="phase-gate-status">
          {status}
        </span>
      </span>
      {needsNote ? (
        <Note tone="warning" role="status" testId="phase-gate-needs-person">
          {needsNote}
        </Note>
      ) : null}
      {standingNote ? (
        <Note role="status" testId="phase-gate-standing-note">
          {standingNote}
        </Note>
      ) : null}
      {closed ? <p className="br-monitors__note">{copy.closed}</p> : null}
      <p className="br-monitors__note" data-testid="phase-gate-attempts-used">
        {[
          copy.threshold(threshold),
          copy.attempts(gate.attempts, gate.maxAttempts),
          gate.retriesGranted > 0 ? copy.retried(gate.retriesGranted) : null,
        ]
          .filter(Boolean)
          .join(' ')}
      </p>
      <section aria-label={copy.questions} className="flex min-w-0 flex-col gap-1.5">
        <h3 className="text-label text-text-default">{copy.questions}</h3>
        {gate.questions.length > 0 ? (
          <ol className="br-monitors__list" data-testid="phase-gate-questions">
            {gate.questions.map(({ name, question }) => (
              <li key={name} className="br-monitors__item br-fill-surface">
                <span className="br-monitors__item-text">
                  <span className="br-monitors__value">{question}</span>
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="br-monitors__note">{copy.questionsUnknown}</p>
        )}
      </section>
      {gate.lastMinP !== null ? (
        <p className="br-monitors__note" data-testid="phase-gate-last-check">
          {copy.lastCheck(formatProbability(gate.lastMinP), threshold)}
        </p>
      ) : null}
      {gate.lastError ? (
        <p className="br-monitors__note" data-testid="phase-gate-last-error">
          {copy.lastError(gate.lastError)}
        </p>
      ) : null}
      {holds.length > 0 ? (
        <p className="br-monitors__note" data-testid="phase-gate-holds">
          {copy.holds(joinTitles(holds))}
        </p>
      ) : null}
      <p className="br-monitors__note" data-testid="phase-gate-model-line">
        {copy.model(model ? (decisionModelLabel(model) ?? gate.model ?? '') : copy.chatDefault)}
        {gate.allowUncalibrated ? ` ${copy.uncalibrated}` : ''}
      </p>
    </div>
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
}

type Busy = PhaseGateChoice | 'remove' | 'save' | null;

/**
 * A phase gate, and the person's controls over it (item C of the decision
 * supervision work): who set it, where it stands, what it asks and what it
 * holds, then Pass this phase, Skip this phase, Try again, Edit gate, Remove
 * gate and Show history. A phase without a gate opens on the editor ("Add
 * gate").
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
}: PhaseGateDialogProps) {
  // The phase as the last write answered it, until the caller reads a newer one.
  const [answered, setAnswered] = useState<{
    phase: PhaseSnapshot | null;
    phases: readonly PhaseSnapshot[];
  } | null>(null);
  const [editing, setEditingDraft] = useState<PhaseGateDraft | null>(null);
  const [scaleFor, setScaleFor] = useState(GATE_DEFAULT_THRESHOLD);
  const [openedModel, setOpenedModel] = useState<DecisionModelRef | null>(null);
  // Starting an edit fixes the threshold scale for it (`GateEditor`'s `scaleFor`)
  // and the model it began with (`opened`).
  const startEditing = (draft: PhaseGateDraft | null) => {
    if (draft) {
      setScaleFor(draft.threshold);
      setOpenedModel(draft.model);
    }
    setEditingDraft(draft);
  };
  const [showErrors, setShowErrors] = useState(false);
  const [busy, setBusy] = useState<Busy>(null);
  const [outcome, setOutcome] = useState<{ message: string; applied: boolean } | null>(null);
  const [refusal, setRefusal] = useState<string | null>(null);
  const [fetchedModels, setFetchedModels] = useState<DecisionModelEntry[]>([]);
  const [fetchedRefusal, setFetchedRefusal] = useState<string | null>(null);
  const [modelsFailed, setModelsFailed] = useState(false);

  const phase = answered ? answered.phase : givenPhase;
  const phases = answered ? answered.phases : (givenPhases ?? (givenPhase ? [givenPhase] : []));
  const gate = phase?.gate ?? null;
  const models = givenModels ?? fetchedModels;
  const chatRefusal = givenModels ? (givenChatRefusal ?? null) : fetchedRefusal;
  const phaseId = phase?.id ?? givenPhase?.id ?? '';

  // Each open starts from what the side panel read; a phase with no gate opens the editor.
  useEffect(() => {
    if (!open) return;
    setAnswered(null);
    setOutcome(null);
    setRefusal(null);
    setShowErrors(false);
    setBusy(null);
    startEditing(
      givenPhase && !givenPhase.gate && !isClosedPhase(givenPhase) ? newGateDraft() : null
    );
    // Only on open: a poll that re-reads the plan must not throw away a draft.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

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

  const settle = (result: PhaseGateResult) => {
    if (!result.ok) {
      setRefusal(result.message);
      return false;
    }
    setRefusal(null);
    setOutcome({ message: result.message, applied: result.applied });
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
    try {
      return settle(await request());
    } finally {
      setBusy(null);
    }
  };

  const decide = (choice: PhaseGateChoice) =>
    void run(choice, () => decidePhaseGate(sessionId, phaseId, choice));

  const draftErrors = editing ? validateGateDraft(editing) : {};
  const save = async () => {
    if (!editing) return;
    if (gateDraftHasErrors(draftErrors)) {
      setShowErrors(true);
      return;
    }
    const saved = await run('save', () => setPhaseGate(sessionId, phaseId, editing));
    if (saved) {
      startEditing(null);
      setShowErrors(false);
    }
  };

  const closed = phase ? isClosedPhase(phase) : false;
  const working = busy !== null;
  const adding = editing !== null && !gate;

  const showHistory = () => {
    onOpenChange(false);
    onOpenHistory?.({ origin: 'gate', originId: phaseId });
  };

  // Viewing a gate there is nothing to save, so no footer: the dialog's own × closes it.
  const footer = editing ? (
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
        {busy === 'save' ? copy.editor.saving : copy.editor.save}
      </Button>
    </>
  ) : undefined;

  return (
    <ModalShell
      open={open}
      onOpenChange={(next) => {
        if (!working) onOpenChange(next);
      }}
      size="lg"
      purpose={working ? 'required' : editing ? 'form' : 'info'}
      title={adding ? copy.addTitle : copy.title}
      subtitle={phase?.title ?? givenPhase?.title}
      footer={footer}
      scrollBody
      testId="phase-gate-dialog"
    >
      <div className="br-monitors">
        {!phase ? (
          <p className="br-monitors__note" data-testid="phase-gate-gone">
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
            disabled={working}
            replacing={!!gate}
            scaleFor={scaleFor}
            opened={openedModel}
          />
        ) : gate ? (
          <GateFacts phase={phase} gate={gate} phases={phases} />
        ) : (
          <p className="br-monitors__note" data-testid="phase-gate-none">
            {copy.noGate}
          </p>
        )}
        {outcome ? (
          <p
            className="br-monitors__value"
            role="status"
            data-testid={outcome.applied ? 'phase-gate-applied' : 'phase-gate-standing'}
          >
            {outcome.message}
          </p>
        ) : null}
        {refusal ? (
          <p className="br-monitors__error" role="alert" data-testid="phase-gate-refusal">
            {refusal}
          </p>
        ) : null}
        {phase && !editing ? (
          <div className="flex flex-wrap gap-2" data-testid="phase-gate-actions">
            {gate && !closed ? (
              <>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={working}
                  aria-busy={busy === 'pass' || undefined}
                  data-testid="phase-gate-pass"
                  onClick={() => decide('pass')}
                >
                  {busy === 'pass' ? copy.working : copy.pass}
                </Button>
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  disabled={working}
                  aria-busy={busy === 'skip' || undefined}
                  data-testid="phase-gate-skip"
                  onClick={() => decide('skip')}
                >
                  {busy === 'skip' ? copy.working : copy.skip}
                </Button>
                {gate.attempts > 0 || gate.blocked ? (
                  <Button
                    type="button"
                    variant="secondary"
                    size="sm"
                    disabled={working}
                    aria-busy={busy === 'retry' || undefined}
                    data-testid="phase-gate-retry"
                    onClick={() => decide('retry')}
                  >
                    {busy === 'retry' ? copy.working : copy.retry}
                  </Button>
                ) : null}
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={working}
                  data-testid="phase-gate-edit"
                  onClick={() => {
                    setOutcome(null);
                    setRefusal(null);
                    setShowErrors(false);
                    startEditing(gateDraftFrom(gate));
                  }}
                >
                  {copy.edit}
                </Button>
                <Button
                  type="button"
                  variant="ghost"
                  size="sm"
                  disabled={working}
                  aria-busy={busy === 'remove' || undefined}
                  data-testid="phase-gate-remove"
                  onClick={() => void run('remove', () => removePhaseGate(sessionId, phaseId))}
                >
                  {busy === 'remove' ? copy.working : copy.remove}
                </Button>
              </>
            ) : null}
            {!gate && !closed ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
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
            ) : null}
            {gate || closed ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                disabled={working}
                data-testid="phase-gate-history"
                onClick={showHistory}
              >
                {copy.history}
              </Button>
            ) : null}
          </div>
        ) : null}
      </div>
    </ModalShell>
  );
}

export default PhaseGateDialog;
