import {
  decidePhaseGate as apiDecidePhaseGate,
  deletePhaseGate,
  putPhaseGate,
} from '../api/sdk.gen';
import type { PhaseGateSpec, PersonGateChoice } from '../api/types.gen';
import type { SliderStep } from '../components/ui/slider';
import {
  decisionModelRefToWire,
  formatProbability,
  parsePublicDecisionConfirmation,
  parseTierEffect,
  refusalText,
  type DecisionAdapter,
  type DecisionModelRef,
  type DecisionTierEffect,
  type PhaseGateSnapshot,
  type PublicDecisionConfirmation,
} from './sessionDecisions';
import { parseOrchestrationSnapshot, type OrchestrationSnapshot } from './sessionSubagents';
import { userActionHeaders } from './userAction';

/**
 * The person's controls over a phase gate: set one, remove one, and pass, skip or
 * retry one (`PUT` / `DELETE /sessions/{id}/phase-gates/{phase}` and
 * `POST .../decision`, `routes/decisions.rs`).
 *
 * Every request goes through the generated client with the person's proof
 * (`userActionHeaders()`): a daemon that holds a user-action key refuses a gate
 * write without it, and a keyless one (`biorouter serve`) asks only that the chat
 * is within reach. Nothing here throws for a refusal: the daemon's sentence comes
 * back as the result, because the dialog shows it word for word.
 */

/** `MAX_GATE_QUESTIONS` in `orchestration_plan.rs`. */
export const GATE_QUESTIONS_MAX = 8;
/** `MAX_GATE_ATTEMPTS`. */
export const GATE_ATTEMPTS_MAX = 5;
/** `default_gate_attempts()`. */
export const GATE_DEFAULT_ATTEMPTS = 3;
/** `DEFAULT_GATE_THRESHOLD`. */
export const GATE_DEFAULT_THRESHOLD = 0.8;
/**
 * The slider's range. The daemon takes any probability from 0 to 1 from the
 * person (the agent's 0.50 floor is the agent's alone), and the dialog offers
 * 0.05 to 0.99 by 0.01: a gate at 0 or 1 asks nothing a person would want asked.
 */
export const GATE_THRESHOLD_MIN = 0.05;
export const GATE_THRESHOLD_MAX = 0.99;
/** `MAX_QUESTION_NAME_CHARS` and `MAX_INSTRUCTIONS_CHARS` in `decision/types.rs`. */
const QUESTION_NAME_MAX = 64;
export const GATE_QUESTION_MAX_CHARS = 8_000;

/** The decision a person gives a gate (`PersonGateChoice`). */
export type PhaseGateChoice = PersonGateChoice;

/** What a gate write did: the daemon's sentence either way. */
export type PhaseGateResult =
  | {
      ok: true;
      /** What changed and what happens next, in the daemon's words. */
      message: string;
      /**
       * `false` when a pass or skip was recorded but stands until the phase's
       * working subagents finish ("Your pass stands ..."). Always `true` for a set,
       * a removal or a retry.
       */
      applied: boolean;
      /** The plan after the write, as this caller may see it; `null` when it could not be read back. */
      plan: OrchestrationSnapshot | null;
      /** What a set did to the chat's tier (DR-33); `null` when the answer does not say. */
      tierEffect?: DecisionTierEffect | null;
      /** The daemon's line for it, or `null` when there is nothing to say. */
      tierEffectLine?: string | null;
    }
  | {
      ok: false;
      /** The HTTP status, or `null` when no answer came. */
      status: number | null;
      /** The daemon's sentence, or one of ours when it sent none. */
      message: string;
      /**
       * DR-33: the gate's model would make the chat public and the person has not
       * said so yet (409 `needsPublicDecisionConfirmation`). Nothing was changed;
       * ask, then set again with `acceptPublicDecisionModels`.
       */
      confirmation?: PublicDecisionConfirmation;
    };

export const PHASE_GATE_REQUEST_COPY = {
  unreachable: 'Could not reach Biorouter. Nothing was changed.',
  refused: 'Biorouter refused the change. Nothing was changed.',
  failed: (status: number) => `Could not change the gate (status ${status}). Nothing was changed.`,
  unreadable: 'Biorouter answered, but its answer could not be read.',
};

type ClientResult = { data?: unknown; error?: unknown; response?: Response };

function settle(result: ClientResult): PhaseGateResult {
  const status = result.response?.status ?? null;
  if (result.error !== undefined || result.data === undefined) {
    const confirmation = status === 409 ? parsePublicDecisionConfirmation(result.error) : null;
    if (confirmation) {
      return { ok: false, status, message: confirmation.message, confirmation };
    }
    const sentence = refusalText(result.error);
    return {
      ok: false,
      status,
      message:
        sentence ??
        (status === 401 || status === 403
          ? PHASE_GATE_REQUEST_COPY.refused
          : status === null
            ? PHASE_GATE_REQUEST_COPY.unreachable
            : PHASE_GATE_REQUEST_COPY.failed(status)),
    };
  }
  const body =
    result.data !== null && typeof result.data === 'object'
      ? (result.data as Record<string, unknown>)
      : null;
  const message = typeof body?.message === 'string' ? body.message.trim() : '';
  if (!body || !message) {
    return { ok: false, status, message: PHASE_GATE_REQUEST_COPY.unreadable };
  }
  return {
    ok: true,
    message,
    applied: body.applied !== false,
    plan:
      body.plan === null || body.plan === undefined ? null : parseOrchestrationSnapshot(body.plan),
    ...parseTierEffect(body),
  };
}

async function send(request: (headers: Record<string, string>) => Promise<unknown>) {
  try {
    const headers = await userActionHeaders();
    return settle((await request(headers)) as ClientResult);
  } catch {
    return {
      ok: false,
      status: null,
      message: PHASE_GATE_REQUEST_COPY.unreachable,
    } satisfies PhaseGateResult;
  }
}

/** `PUT /sessions/{id}/phase-gates/{phase}`: the gate becomes the person's, its attempts start over. */
export function setPhaseGate(
  sessionId: string,
  phaseId: string,
  draft: PhaseGateDraft,
  options: { acceptPublicDecisionModels?: boolean } = {}
): Promise<PhaseGateResult> {
  // `acceptPublicDecisionModels` is the person's answer to "This chat will be
  // public: …" (DR-33), sent only after the dialog showed that line.
  const body: PhaseGateSpec & { acceptPublicDecisionModels?: boolean } =
    phaseGateDraftToWire(draft);
  if (options.acceptPublicDecisionModels) body.acceptPublicDecisionModels = true;
  return send((headers) =>
    putPhaseGate({
      path: { session_id: sessionId, phase_id: phaseId },
      body,
      headers,
    })
  );
}

/** `DELETE /sessions/{id}/phase-gates/{phase}`: the gate goes, and the phases after it are released. */
export function removePhaseGate(sessionId: string, phaseId: string): Promise<PhaseGateResult> {
  return send((headers) =>
    deletePhaseGate({ path: { session_id: sessionId, phase_id: phaseId }, headers })
  );
}

/**
 * `POST /sessions/{id}/phase-gates/{phase}/decision`: pass or skip the phase
 * without asking its gate, or give the gate its attempts again (`retry`).
 */
export function decidePhaseGate(
  sessionId: string,
  phaseId: string,
  decision: PhaseGateChoice
): Promise<PhaseGateResult> {
  return send((headers) =>
    apiDecidePhaseGate({
      path: { session_id: sessionId, phase_id: phaseId },
      body: { decision },
      headers,
    })
  );
}

// ---------------------------------------------------------------------------
// The editor's draft
// ---------------------------------------------------------------------------

export interface GateQuestionDraft {
  /** The question's name on the wire (`q1`); kept from the gate being edited. */
  name: string;
  question: string;
}

export interface PhaseGateDraft {
  questions: GateQuestionDraft[];
  threshold: number;
  maxAttempts: number;
  /** `null`: the chat's decision model, else the configured one. */
  model: DecisionModelRef | null;
  allowUncalibrated: boolean;
}

/**
 * `provider/model`, the spelling the snapshot gives a gate's model, as a model
 * reference (`DecisionModelRef::parse`: the model part may hold `/` itself),
 * asked through `adapter` (the snapshot's `modelAdapter`; `null` is the model's
 * own). The label alone does not carry the adapter, so an editor that leaves it
 * out writes the gate back on the model's default adapter.
 */
export function parseGateModel(
  label: string | null | undefined,
  adapter: DecisionAdapter | null = null
): DecisionModelRef | null {
  const text = label?.trim() ?? '';
  const slash = text.indexOf('/');
  if (slash <= 0 || slash === text.length - 1) return null;
  return { provider: text.slice(0, slash), model: text.slice(slash + 1), adapter };
}

/**
 * The model a pick in the editor's list stands for. The row of the model the
 * gate was opened with stands for exactly that model, adapter included, so
 * choosing it again (or going back to it) changes nothing; any other row is
 * that model on its own adapter.
 */
export function gateModelPick(
  picked: DecisionModelRef | null,
  opened: DecisionModelRef | null
): DecisionModelRef | null {
  if (!picked) return null;
  if (opened && opened.provider === picked.provider && opened.model === picked.model) {
    return opened;
  }
  return picked;
}

/** The next free question name: `q1`, `q2`, ... */
export function gateQuestionName(taken: Iterable<string>): string {
  const used = new Set(taken);
  for (let n = 1; ; n += 1) {
    if (!used.has(`q${n}`)) return `q${n}`;
  }
}

/** A new gate as the daemon defaults one: one empty question, 0.80, three attempts. */
export function newGateDraft(): PhaseGateDraft {
  return {
    questions: [{ name: 'q1', question: '' }],
    threshold: GATE_DEFAULT_THRESHOLD,
    maxAttempts: GATE_DEFAULT_ATTEMPTS,
    model: null,
    allowUncalibrated: false,
  };
}

/**
 * The editor's starting point for a gate: what it asks, its threshold, attempts,
 * model and calibration rule. A gate an older daemon describes without its
 * questions starts with one empty question, which Save then asks for.
 */
export function gateDraftFrom(gate: PhaseGateSnapshot | null | undefined): PhaseGateDraft {
  if (!gate) return newGateDraft();
  return {
    questions:
      gate.questions.length > 0
        ? gate.questions.map(({ name, question }) => ({ name, question }))
        : [{ name: 'q1', question: '' }],
    threshold: gate.threshold,
    maxAttempts: Math.min(GATE_ATTEMPTS_MAX, Math.max(1, gate.maxAttempts)),
    // The adapter too: Save replaces the whole gate, so one left out is one changed.
    model: parseGateModel(gate.model, gate.modelAdapter),
    allowUncalibrated: gate.allowUncalibrated,
  };
}

/** `PhaseGateSpec` as `PUT .../phase-gates/{phase}` takes it. */
export function phaseGateDraftToWire(draft: PhaseGateDraft): PhaseGateSpec {
  const questions: Record<string, string> = {};
  for (const { name, question } of draft.questions) questions[name.trim()] = question.trim();
  return {
    questions,
    threshold: draft.threshold,
    maxAttempts: draft.maxAttempts,
    model: draft.model
      ? (decisionModelRefToWire(draft.model) as unknown as PhaseGateSpec['model'])
      : null,
    allowUncalibrated: draft.allowUncalibrated,
  };
}

const THRESHOLD_STOPS = Math.round((GATE_THRESHOLD_MAX - GATE_THRESHOLD_MIN) * 100) + 1;

/** 0.05 to 0.99 by 0.01 (95 stops). */
export const GATE_THRESHOLD_STEPS: SliderStep[] = Array.from(
  { length: THRESHOLD_STOPS },
  (_, i) => {
    const value = (GATE_THRESHOLD_MIN + i / 100).toFixed(2);
    return { value, label: value };
  }
);

/**
 * The stops for a gate whose threshold is `p`: {@link GATE_THRESHOLD_STEPS}, plus
 * `p` as a stop of its own, in order, when it is none of them (a gate set from the
 * CLI at 0.333, or at 1). Snapping it would change a rule the person did not touch.
 */
export function gateThresholdSteps(p: number): SliderStep[] {
  if (!Number.isFinite(p) || GATE_THRESHOLD_STEPS.some((step) => Number(step.value) === p)) {
    return GATE_THRESHOLD_STEPS;
  }
  const own = { value: String(p), label: formatProbability(p) };
  const at = GATE_THRESHOLD_STEPS.findIndex((step) => Number(step.value) > p);
  return at < 0
    ? [...GATE_THRESHOLD_STEPS, own]
    : [...GATE_THRESHOLD_STEPS.slice(0, at), own, ...GATE_THRESHOLD_STEPS.slice(at)];
}

/** The slider value that stands for `p` among `steps` (which hold it: {@link gateThresholdSteps}). */
export function gateThresholdValue(p: number, steps: readonly SliderStep[]): string {
  return steps.find((step) => Number(step.value) === p)?.value ?? steps[0].value;
}

export const GATE_DRAFT_COPY = {
  noQuestion: 'Write at least one yes/no question.',
  tooMany: `A gate asks at most ${GATE_QUESTIONS_MAX} questions.`,
  empty: 'Write the question, or remove it.',
  tooLong: `Keep each question to ${GATE_QUESTION_MAX_CHARS.toLocaleString()} characters.`,
  badName: 'This question has a name the daemon cannot take.',
  sameName: 'Two questions share a name.',
};

export interface GateDraftErrors {
  /** About the list as a whole. */
  questions?: string;
  /** About one question, by its index. */
  question?: Record<number, string>;
}

/** What the daemon would refuse in a draft (`build_gate`), said before Save. */
export function validateGateDraft(draft: PhaseGateDraft): GateDraftErrors {
  const errors: GateDraftErrors = {};
  if (draft.questions.length === 0) errors.questions = GATE_DRAFT_COPY.noQuestion;
  else if (draft.questions.length > GATE_QUESTIONS_MAX) errors.questions = GATE_DRAFT_COPY.tooMany;
  const seen = new Set<string>();
  const each: Record<number, string> = {};
  draft.questions.forEach(({ name, question }, index) => {
    const key = name.trim();
    if (!question.trim()) each[index] = GATE_DRAFT_COPY.empty;
    else if (question.length > GATE_QUESTION_MAX_CHARS) each[index] = GATE_DRAFT_COPY.tooLong;
    else if (!key || key.length > QUESTION_NAME_MAX || !/^[A-Za-z0-9_-]+$/.test(key))
      each[index] = GATE_DRAFT_COPY.badName;
    else if (seen.has(key)) each[index] = GATE_DRAFT_COPY.sameName;
    seen.add(key);
  });
  if (Object.keys(each).length > 0) errors.question = each;
  return errors;
}

export function gateDraftHasErrors(errors: GateDraftErrors): boolean {
  return !!errors.questions || Object.keys(errors.question ?? {}).length > 0;
}
