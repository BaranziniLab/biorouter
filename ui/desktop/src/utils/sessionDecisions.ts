import type { Message } from '../api';
import {
  getDecisionInstall,
  getDecisionSettings,
  getSessionDecision,
  getSessionDecisionSummary,
  listDecisionModels,
  listSessionDecisions,
  postDecisionInstall,
  putDecisionSettings,
} from '../api/sdk.gen';
import type { ListSessionDecisionsData, PutDecisionSettingsData } from '../api/types.gen';
import { decisionModelName } from './usageStats';
import { userActionHeaders } from './userAction';

/**
 * Decision models, as the renderer reads them: the decision log's rows and
 * aggregates, a chat's decision settings (its monitors), and the catalog of
 * decision models. Shared by the chat summary's Decisions section, the decision
 * history view, the Monitors dialog (`components/decisions/`) and the chat's
 * decision cards and turn markers.
 *
 * The shapes mirror the daemon's contract one to one, camelCase on the wire:
 * `DecisionRecord`, `DecisionDetail`, `DecisionPage`, `DecisionAggregate`,
 * `DecisionTurnSummary`, `SessionDecisionSummary` and `DecisionTurnMarker` in
 * `crates/biorouter/src/decision/log.rs`; `Decision`, `Question`, `Threshold`
 * and `DecisionModelRef` in `decision/types.rs`; `DecisionSettings` and
 * `MonitorCriterion` in `decision/settings.rs`; `DecisionModelEntry` in
 * `decision/model.rs`. Rust's enums are externally tagged there
 * (`{"binary":{}}`, `{"yes":0.8}`), so the parsers turn them into tagged unions
 * here and the `*ToWire` functions turn them back.
 *
 * The parsers are defensive in one direction only, as `sessionSubagents.ts` is:
 * a value this build does not know degrades to a neutral one rather than
 * dropping what a person should see. A row with no id is the exception, because
 * nothing could open it.
 *
 * ⚠ The owner's rule (decision calls can number in the thousands): no surface
 * draws one row per call except the decision history view. Everything else reads
 * the aggregates.
 */

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export const DECISION_ORIGINS = ['chat', 'monitor', 'tool', 'gate', 'adhoc'] as const;
export type DecisionOrigin = (typeof DECISION_ORIGINS)[number];

export const DECISION_OUTCOMES = ['passed', 'failed', 'decided', 'error', 'refused'] as const;
export type DecisionOutcome = (typeof DECISION_OUTCOMES)[number];

export const DECISION_KIND_TAGS = ['binary', 'choice', 'score'] as const;
export type DecisionKindTag = (typeof DECISION_KIND_TAGS)[number];

export const DECISION_ADAPTERS = ['system_one', 'logprob', 'verbalized'] as const;
export type DecisionAdapter = (typeof DECISION_ADAPTERS)[number];

/** A decision model's tier, or a chat's. `null` where the wire carried neither. */
export type DecisionTier = 'public' | 'private';

export interface DecisionOption {
  name: string;
  /** What the model reads for this option; `null` sends the name itself. */
  description: string | null;
}

export type DecisionKind =
  | { type: 'binary' }
  | { type: 'choice'; options: DecisionOption[] }
  | { type: 'score'; levels: string[] };

/**
 * A question's pass rule. `below` is the daemon's `Threshold::Below` wrap
 * (`{"below":{"yes":0.3}}`): the rule holds when the probability falls below `p`,
 * the exact complement of the same rule without it. A below monitor's logged
 * question carries one, so a card or the history reads `P(yes) < 0.30`.
 */
export type Threshold =
  | { type: 'yes'; p: number; below?: true }
  | { type: 'no'; p: number; below?: true }
  | { type: 'option'; name: string; p: number; below?: true }
  | { type: 'at_least_level'; level: number; p: number; below?: true };

export interface Question {
  instructions: string;
  kind: DecisionKind;
  threshold: Threshold | null;
}

/** A question with the name it was asked under (a map key on the wire). */
export interface NamedQuestion extends Question {
  name: string;
}

export type DecisionAnswer =
  | { type: 'binary'; value: boolean }
  | { type: 'choice'; option: string }
  | { type: 'score'; level: number; label: string; expected: number };

export type ProbabilitySource =
  | { kind: 'native' }
  | { kind: 'logprob'; mass: number }
  | { kind: 'verbalized'; reason: string | null };

export interface DecisionModelRef {
  provider: string;
  model: string;
  adapter: DecisionAdapter | null;
}

export interface DecisionProbability {
  /** `yes` / `no`, an option's name, or a score level's index (`"0"`, `"1"`, ...). */
  key: string;
  p: number;
}

export interface Decision {
  /** The question's name. */
  question: string;
  answer: DecisionAnswer | null;
  /** In the order the daemon sent them (binary: yes then no; levels lowest first). */
  probabilities: DecisionProbability[];
  /** How concentrated the distribution is (0 uniform, 1 certain). Not P(answer). */
  confidence: number | null;
  passed: boolean | null;
  passProbability: number | null;
  source: ProbabilitySource;
  calibrated: boolean;
  model: DecisionModelRef | null;
  tier: DecisionTier | null;
  requestId: string | null;
  latencyMs: number | null;
  inputTokens: number | null;
  outputTokens: number | null;
  raw: unknown;
}

export interface DecisionRecord {
  id: string;
  sessionId: string | null;
  createdAt: string;
  origin: DecisionOrigin;
  originId: string | null;
  label: string | null;
  turnId: string | null;
  batchId: string | null;
  chunkIndex: number | null;
  chunkId: string | null;
  model: DecisionModelRef | null;
  servedModel: string | null;
  tier: DecisionTier | null;
  calibrated: boolean | null;
  questions: NamedQuestion[];
  decisions: Decision[];
  outcome: DecisionOutcome;
  contextPreview: string;
  contextBytes: number;
  contextTruncated: boolean;
  error: string | null;
  latencyMs: number | null;
  inputTokens: number | null;
  outputTokens: number | null;
  privacyEnforced: boolean;
}

export interface ContextMessage {
  role: string;
  content: string;
}

export type DecisionContext =
  | { type: 'text'; text: string }
  | { type: 'json'; value: unknown }
  | { type: 'messages'; messages: ContextMessage[] };

export interface DecisionDetail {
  record: DecisionRecord;
  context: DecisionContext | null;
  /** The route's answer as it came, for the history view's raw JSON. */
  raw?: unknown;
}

export interface DecisionPage {
  /** Newest first. */
  records: DecisionRecord[];
  nextCursor: string | null;
  /** Rows matching the filter, across every page. */
  total: number;
}

export interface DecisionAggregate {
  origin: DecisionOrigin;
  originId: string | null;
  label: string;
  count: number;
  passed: number;
  failed: number;
  errors: number;
  lastAt: string;
  /** Readable: `yes`, `rrms`, `level 2`. */
  lastAnswer: string | null;
  lastPassProbability: number | null;
  model: DecisionModelRef | null;
}

export interface DecisionTurnSummary {
  turnId: string;
  origin: DecisionOrigin;
  originId: string | null;
  label: string;
  count: number;
  passed: number;
  failed: number;
  errors: number;
  firstAt: string;
  lastAt: string;
  lastPassProbability: number | null;
}

export interface SessionDecisionSummary {
  total: number;
  aggregates: DecisionAggregate[];
  /** Newest turn first. */
  recentTurns: DecisionTurnSummary[];
}

export interface DecisionTurnMarkerItem {
  origin: DecisionOrigin;
  originId: string | null;
  label: string;
  count: number;
  passed: number;
  failed: number;
  errors: number;
  lastPassProbability: number | null;
  fired: boolean;
}

export interface DecisionTurnMarker {
  turnId: string;
  items: DecisionTurnMarkerItem[];
}

export const MONITOR_TIMINGS = ['post_iteration', 'turn_end'] as const;
export type MonitorTiming = (typeof MONITOR_TIMINGS)[number];

export const MONITOR_ACTIONS = ['stop', 'notify', 'steer'] as const;
export type MonitorAction = (typeof MONITOR_ACTIONS)[number];

export const MONITOR_ERROR_POLICIES = ['notify', 'pause'] as const;
export type MonitorErrorPolicy = (typeof MONITOR_ERROR_POLICIES)[number];

/** `MonitorDirection` in `settings.rs`: which side of its threshold a monitor fires on. */
export const MONITOR_DIRECTIONS = ['at_or_above', 'below'] as const;
export type MonitorDirection = (typeof MONITOR_DIRECTIONS)[number];

/** `MonitorOwner` in `settings.rs`: who set a monitor. */
export const MONITOR_OWNERS = ['person', 'agent'] as const;
export type MonitorOwner = (typeof MONITOR_OWNERS)[number];

export interface MonitorWindow {
  entries: number;
  maxChars: number;
  toolCalls: boolean;
}

export interface MonitorCriterion {
  id: string;
  name: string;
  question: Question;
  model: DecisionModelRef | null;
  timing: MonitorTiming;
  window: MonitorWindow;
  /**
   * The probability P(wanted answer) is compared with: it fires at or above it,
   * or below it when `direction` is `below`.
   */
  threshold: number;
  /**
   * The answer whose probability is compared ({@link monitorThreshold}): `yes` (or
   * `null`) or `no` for a yes/no question, an option's name for a choice, and the
   * lowest level that counts for a score, by its label or its index (`"0"`).
   */
  option: string | null;
  consecutive: number;
  /** For an at-or-above monitor: after firing, re-arm only once P drops below this. */
  rearmBelow: number | null;
  action: MonitorAction;
  onError: MonitorErrorPolicy;
  maxEvalsPerTurn: number;
  enabled: boolean;
  /*
   * The fields below were added with the daemon's monitor owners and directions.
   * The parser always sets them; they are optional so a monitor written before
   * them still type-checks, and absent means the default the daemon reads.
   *
   * ⚠ Load-bearing: the parser keeps only the keys it knows, and Save writes back
   * what it parsed. A key missing here would be erased by one desktop Save, so a
   * CLI-made below monitor would come back at-or-above and an agent's monitor
   * would come back the person's.
   */
  /** Which side of `threshold` fires it. Absent: `at_or_above`. */
  direction?: MonitorDirection;
  /** For a `below` monitor: after firing, re-arm only once P rises to this or above. */
  rearmAbove?: number | null;
  /** Who set it. Absent: the person. */
  setBy?: MonitorOwner;
  /** For an agent's monitor: the chat whose agent set it (the subagent's parent). */
  setBySession?: string | null;
  /** For a copy of a parent chat's monitor that also watches its subagents: that chat. */
  inheritedFrom?: string | null;
  /** A person's monitor copied onto every subagent this chat starts. Absent: `false`. */
  applyToSubagents?: boolean;
}

export interface DecisionSettings {
  defaultModel: DecisionModelRef | null;
  monitors: MonitorCriterion[];
}

export interface DecisionModelEntry {
  provider: string;
  model: string;
  displayName: string;
  contextTokens: number | null;
  kinds: DecisionKindTag[];
  tier: DecisionTier | null;
  adapter: DecisionAdapter | null;
  local: boolean;
  /** Why the model cannot be used now, when the daemon says so. */
  unavailableReason: string | null;
}

/** `GateOwner` in `orchestration_plan.rs`: who set a phase gate. */
export const GATE_OWNERS = ['agent', 'person', 'workflow'] as const;
export type GateOwner = (typeof GATE_OWNERS)[number];

/** `PersonGateAction`: the pass or skip a person gave a gate. */
export const PERSON_GATE_ACTIONS = ['pass', 'skip'] as const;
export type PersonGateAction = (typeof PERSON_GATE_ACTIONS)[number];

/**
 * A phase gate as the side panel draws it (`PhaseGateSnapshot` in `orchestration_plan.rs`).
 *
 * Every field after `blocked` came with the person's gate controls. An older
 * daemon sends none of them, and they then read as an agent's gate with nothing
 * more to say: no questions listed, nothing held, no decision of the person's.
 */
export interface PhaseGateSnapshot {
  threshold: number;
  questionCount: number;
  attempts: number;
  maxAttempts: number;
  lastPassed: boolean | null;
  lastMinP: number | null;
  blocked: boolean;
  /** Who set the gate. Absent on the wire: the agent. */
  owner: GateOwner;
  /** The questions, by name, in the order the gate asks them. */
  questions: { name: string; question: string }[];
  /** The decision model the gate names, as `provider/model`; `null` uses the chat's. */
  model: string | null;
  allowUncalibrated: boolean;
  /** Why the last evaluation decided nothing, when it did not. */
  lastError: string | null;
  /** The person's pass or skip: one that stands, or the one that closed the phase. */
  personDecision: PersonGateAction | null;
  /** How many times the person gave the gate its attempts again. */
  retriesGranted: number;
  /** The open phase is stuck and waits for the person. */
  needsPerson: boolean;
  /** Ids of the phases this gate holds now. */
  holds: string[];
}

/** A phase gate's state, as every surface that draws one names it. */
export type PhaseGateState = 'blocked' | 'passed' | 'failed' | 'pending' | 'skipped';

/**
 * The ONE rule for a gate's state: the phase row's chip and the Decisions
 * section's gate row both read it, so the two never name one gate differently.
 * The person's pass or skip is the gate's state from then on, standing or not;
 * then a phase that waits for the person (out of attempts, or marked blocked)
 * reads as blocked; then the gate's own last verdict.
 */
export function phaseGateState(gate: PhaseGateSnapshot): PhaseGateState {
  if (gate.personDecision === 'pass') return 'passed';
  if (gate.personDecision === 'skip') return 'skipped';
  if (gate.blocked || gate.needsPerson) return 'blocked';
  if (gate.lastPassed === true) return 'passed';
  if (gate.lastPassed === false) return 'failed';
  return 'pending';
}

/** Where an install stands (`DecisionInstallState` in `routes/decisions.rs`). */
export const DECISION_INSTALL_STATES = ['downloading', 'verifying', 'installed', 'failed'] as const;
export type DecisionInstallState = (typeof DECISION_INSTALL_STATES)[number];

/** Where the configured Ollama is, as far as installing into it goes (`OllamaPlaceState`). */
export const OLLAMA_PLACES = [
  'running',
  'not_running',
  'not_this_computer',
  'invalid_host',
] as const;
export type OllamaPlace = (typeof OLLAMA_PLACES)[number];

/** One decision model Biorouter installs into the Ollama on this computer. */
export interface InstallableDecisionModel {
  /** The name Ollama knows it by (`tev1:0.8b`). */
  model: string;
  displayName: string;
  /** The download, roughly (`about 1.5 GB`). */
  approxSize: string;
  note: string;
  isDefault: boolean;
  /** Listed already by the Ollama on this computer as a decision model it loads. */
  installed: boolean;
}

/** The install that is running, or the last one (kept until the next one starts). */
export interface DecisionInstallJob {
  model: string;
  state: DecisionInstallState;
  completedBytes: number;
  totalBytes: number;
  /** Ollama's last step, in its words. */
  status: string | null;
  /** Why the install failed, once it has. */
  error: string | null;
}

/** `GET /decisions/models/install`: what can be installed, where, and how it is going. */
export interface DecisionInstallStatus {
  /** The default first. */
  offers: InstallableDecisionModel[];
  ollama: {
    baseUrl: string | null;
    state: OllamaPlace;
    version: string | null;
    /** Why nothing can be installed there now, as a sentence; `null` while it runs here. */
    message: string | null;
  };
  job: DecisionInstallJob | null;
}

// ---------------------------------------------------------------------------
// Limits the daemon enforces (`settings.rs`), mirrored for inline validation
// ---------------------------------------------------------------------------

export const MAX_MONITORS = 8;
export const MONITOR_NAME_MAX = 60;
/** `MAX_WINDOW_ENTRIES` (= `window::MAX_WINDOW_MESSAGES`): newest messages kept. */
export const MONITOR_WINDOW_ENTRIES_MAX = 24;
/** `MAX_WINDOW_CHARS` (= `window::MAX_WINDOW_CHARS`): the most a window delivers. */
export const MONITOR_WINDOW_CHARS_MAX = 6_000;
export const MONITOR_CONSECUTIVE_MAX = 5;
export const MONITOR_MAX_EVALS_MAX = 200;
/** `MAX_PAGE` in `log.rs`. */
export const DECISION_PAGE_MAX = 200;

export const DEFAULT_MONITOR_WINDOW: MonitorWindow = {
  entries: 24,
  maxChars: 6000,
  toolCalls: true,
};

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

function record(value: unknown): Record<string, unknown> | undefined {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined;
}

function text(value: unknown): string | null {
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

/** A string kept as sent (context and instructions are not trimmed). */
function rawText(value: unknown): string | null {
  return typeof value === 'string' ? value : null;
}

function count(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

function optionalCount(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0
    ? Math.floor(value)
    : null;
}

function optionalNumber(value: unknown): number | null {
  return typeof value === 'number' && Number.isFinite(value) ? value : null;
}

/** A probability in [0, 1], or `null`. A value just outside (rounding) is clamped. */
export function probability(value: unknown): number | null {
  if (typeof value !== 'number' || !Number.isFinite(value)) return null;
  if (value < -0.001 || value > 1.001) return null;
  return Math.min(1, Math.max(0, value));
}

function optionalBool(value: unknown): boolean | null {
  return typeof value === 'boolean' ? value : null;
}

function oneOf<T extends string>(values: readonly T[], value: unknown, fallback: T): T {
  return values.includes(value as T) ? (value as T) : fallback;
}

function optionalOneOf<T extends string>(values: readonly T[], value: unknown): T | null {
  return values.includes(value as T) ? (value as T) : null;
}

function tier(value: unknown): DecisionTier | null {
  return value === 'public' || value === 'private' ? value : null;
}

/** The single key and value of an externally tagged enum (`{"binary":{}}`), or a bare string. */
function tagged(value: unknown): [string, unknown] | null {
  if (typeof value === 'string') return [value, null];
  const object = record(value);
  if (!object) return null;
  const keys = Object.keys(object);
  return keys.length === 1 ? [keys[0], object[keys[0]]] : null;
}

// ---------------------------------------------------------------------------
// Questions and thresholds
// ---------------------------------------------------------------------------

export function parseDecisionKind(value: unknown): DecisionKind {
  const entry = tagged(value);
  if (entry?.[0] === 'choice') {
    const options = record(record(entry[1])?.options);
    return {
      type: 'choice',
      options: options
        ? Object.entries(options).map(([name, description]) => ({
            name,
            description: text(description),
          }))
        : [],
    };
  }
  if (entry?.[0] === 'score') {
    const levels = record(entry[1])?.levels;
    return {
      type: 'score',
      levels: Array.isArray(levels) ? levels.map((level) => String(level ?? '')) : [],
    };
  }
  return { type: 'binary' };
}

export function parseThreshold(value: unknown): Threshold | null {
  const entry = tagged(value);
  if (!entry) return null;
  const [key, inner] = entry;
  if (key === 'below') {
    // One wrap only: the daemon refuses a `below` inside a `below`.
    const wrapped = parseThreshold(inner);
    return wrapped && !wrapped.below ? { ...wrapped, below: true } : null;
  }
  if (key === 'yes' || key === 'no') {
    const p = probability(inner);
    return p === null ? null : { type: key, p };
  }
  const body = record(inner);
  const p = probability(body?.p);
  if (p === null) return null;
  if (key === 'option') {
    const name = text(body?.name);
    return name ? { type: 'option', name, p } : null;
  }
  if (key === 'at_least_level') {
    const level = optionalCount(body?.level);
    return level === null ? null : { type: 'at_least_level', level, p };
  }
  return null;
}

export function parseQuestion(value: unknown): Question {
  const body = record(value);
  return {
    instructions: rawText(body?.instructions) ?? '',
    kind: parseDecisionKind(body?.kind),
    threshold: parseThreshold(body?.threshold),
  };
}

/** A `name -> Question` map, in the order it was sent. */
export function parseNamedQuestions(value: unknown): NamedQuestion[] {
  const body = record(value);
  if (!body) return [];
  return Object.entries(body).map(([name, question]) => ({ name, ...parseQuestion(question) }));
}

export function decisionKindToWire(kind: DecisionKind): Record<string, unknown> {
  if (kind.type === 'choice') {
    return {
      choice: {
        options: Object.fromEntries(
          kind.options.map((option) => [option.name, option.description ?? null])
        ),
      },
    };
  }
  if (kind.type === 'score') return { score: { levels: [...kind.levels] } };
  return { binary: {} };
}

export function thresholdToWire(threshold: Threshold): Record<string, unknown> {
  if (threshold.below) {
    const { below: _below, ...inner } = threshold;
    return { below: thresholdToWire(inner as Threshold) };
  }
  switch (threshold.type) {
    case 'yes':
      return { yes: threshold.p };
    case 'no':
      return { no: threshold.p };
    case 'option':
      return { option: { name: threshold.name, p: threshold.p } };
    case 'at_least_level':
      return { at_least_level: { level: threshold.level, p: threshold.p } };
  }
}

export function questionToWire(question: Question): Record<string, unknown> {
  return {
    instructions: question.instructions,
    kind: decisionKindToWire(question.kind),
    ...(question.threshold ? { threshold: thresholdToWire(question.threshold) } : {}),
  };
}

// ---------------------------------------------------------------------------
// Decisions and log rows
// ---------------------------------------------------------------------------

export function parseDecisionModelRef(value: unknown): DecisionModelRef | null {
  const body = record(value);
  const provider = text(body?.provider);
  const model = text(body?.model);
  if (!provider || !model) return null;
  return { provider, model, adapter: optionalOneOf(DECISION_ADAPTERS, body?.adapter) };
}

export function decisionModelRefToWire(model: DecisionModelRef): Record<string, unknown> {
  return {
    provider: model.provider,
    model: model.model,
    ...(model.adapter ? { adapter: model.adapter } : {}),
  };
}

function parseAnswer(value: unknown): DecisionAnswer | null {
  const body = record(value);
  if (!body) return null;
  if (body.type === 'binary' && typeof body.value === 'boolean') {
    return { type: 'binary', value: body.value };
  }
  if (body.type === 'choice') {
    const option = rawText(body.option);
    return option === null ? null : { type: 'choice', option };
  }
  if (body.type === 'score') {
    const level = optionalCount(body.level);
    if (level === null) return null;
    return {
      type: 'score',
      level,
      label: rawText(body.label) ?? String(level),
      expected: optionalNumber(body.expected) ?? level,
    };
  }
  return null;
}

function parseSource(value: unknown): ProbabilitySource {
  const body = record(value);
  if (body?.kind === 'logprob') return { kind: 'logprob', mass: probability(body.mass) ?? 0 };
  if (body?.kind === 'verbalized') return { kind: 'verbalized', reason: text(body.reason) };
  if (body?.kind === 'native') return { kind: 'native' };
  // Unknown: say nothing we cannot back up. Treated as uncalibrated everywhere.
  return { kind: 'verbalized', reason: null };
}

function parseProbabilities(value: unknown): DecisionProbability[] {
  const body = record(value);
  if (!body) return [];
  const out: DecisionProbability[] = [];
  for (const [key, p] of Object.entries(body)) {
    const parsed = probability(p);
    if (parsed !== null) out.push({ key, p: parsed });
  }
  return out;
}

export function parseDecision(value: unknown): Decision | null {
  const body = record(value);
  const question = text(body?.question);
  if (!body || !question) return null;
  const usage = record(body.usage);
  return {
    question,
    answer: parseAnswer(body.answer),
    probabilities: parseProbabilities(body.probabilities),
    confidence: probability(body.confidence),
    passed: optionalBool(body.passed),
    passProbability: probability(body.passProbability),
    source: parseSource(body.source),
    calibrated: body.calibrated === true,
    model: parseDecisionModelRef(body.model),
    tier: tier(body.tier),
    requestId: text(body.requestId),
    latencyMs: optionalCount(body.latencyMs),
    inputTokens: optionalCount(usage?.inputTokens),
    outputTokens: optionalCount(usage?.outputTokens),
    raw: body.raw ?? null,
  };
}

function parseDecisions(value: unknown): Decision[] {
  if (!Array.isArray(value)) return [];
  return value.map(parseDecision).filter((entry): entry is Decision => entry !== null);
}

export function parseDecisionRecord(value: unknown): DecisionRecord | null {
  const body = record(value);
  const id = text(body?.id);
  if (!body || !id) return null;
  return {
    id,
    sessionId: text(body.sessionId),
    createdAt: text(body.createdAt) ?? '',
    origin: oneOf(DECISION_ORIGINS, body.origin, 'adhoc'),
    originId: text(body.originId),
    label: text(body.label),
    turnId: text(body.turnId),
    batchId: text(body.batchId),
    chunkIndex: optionalCount(body.chunkIndex),
    chunkId: text(body.chunkId),
    model: parseDecisionModelRef(body.model),
    servedModel: text(body.servedModel),
    tier: tier(body.tier),
    calibrated: optionalBool(body.calibrated),
    questions: parseNamedQuestions(body.questions),
    decisions: parseDecisions(body.decisions),
    // An outcome this build does not know is drawn as an error: never as a pass.
    outcome: oneOf(DECISION_OUTCOMES, body.outcome, 'error'),
    contextPreview: rawText(body.contextPreview) ?? '',
    contextBytes: count(body.contextBytes),
    contextTruncated: body.contextTruncated === true,
    error: text(body.error),
    latencyMs: optionalCount(body.latencyMs),
    inputTokens: optionalCount(body.inputTokens),
    outputTokens: optionalCount(body.outputTokens),
    privacyEnforced: body.privacyEnforced !== false,
  };
}

export function parseDecisionContext(value: unknown): DecisionContext | null {
  const entry = tagged(value);
  if (!entry) return null;
  const [key, inner] = entry;
  if (key === 'text') return { type: 'text', text: rawText(inner) ?? '' };
  if (key === 'json') return { type: 'json', value: inner };
  if (key === 'messages') {
    return {
      type: 'messages',
      messages: (Array.isArray(inner) ? inner : []).flatMap((message) => {
        const body = record(message);
        if (!body) return [];
        return [{ role: text(body.role) ?? 'user', content: rawText(body.content) ?? '' }];
      }),
    };
  }
  return null;
}

export function parseDecisionDetail(value: unknown): DecisionDetail | null {
  const body = record(value);
  const parsed = parseDecisionRecord(body?.record);
  if (!body || !parsed) return null;
  return { record: parsed, context: parseDecisionContext(body.context), raw: value };
}

export function parseDecisionPage(value: unknown): DecisionPage | null {
  const body = record(value);
  if (!body) return null;
  const seen = new Set<string>();
  const records: DecisionRecord[] = [];
  for (const entry of Array.isArray(body.records) ? body.records : []) {
    const row = parseDecisionRecord(entry);
    if (row && !seen.has(row.id)) {
      seen.add(row.id);
      records.push(row);
    }
  }
  return {
    records,
    nextCursor: text(body.nextCursor),
    total: Math.max(count(body.total), records.length),
  };
}

export function parseDecisionAggregate(value: unknown): DecisionAggregate | null {
  const body = record(value);
  if (!body) return null;
  const origin = oneOf(DECISION_ORIGINS, body.origin, 'adhoc');
  return {
    origin,
    originId: text(body.originId),
    label: text(body.label) ?? '',
    count: count(body.count),
    passed: count(body.passed),
    failed: count(body.failed),
    errors: count(body.errors),
    lastAt: text(body.lastAt) ?? '',
    lastAnswer: text(body.lastAnswer),
    lastPassProbability: probability(body.lastPassProbability),
    model: parseDecisionModelRef(body.model),
  };
}

export function parseDecisionTurnSummary(value: unknown): DecisionTurnSummary | null {
  const body = record(value);
  const turnId = text(body?.turnId);
  if (!body || !turnId) return null;
  return {
    turnId,
    origin: oneOf(DECISION_ORIGINS, body.origin, 'adhoc'),
    originId: text(body.originId),
    label: text(body.label) ?? '',
    count: count(body.count),
    passed: count(body.passed),
    failed: count(body.failed),
    errors: count(body.errors),
    firstAt: text(body.firstAt) ?? '',
    lastAt: text(body.lastAt) ?? '',
    lastPassProbability: probability(body.lastPassProbability),
  };
}

export function parseSessionDecisionSummary(value: unknown): SessionDecisionSummary | null {
  const body = record(value);
  if (!body) return null;
  const aggregates = (Array.isArray(body.aggregates) ? body.aggregates : [])
    .map(parseDecisionAggregate)
    .filter((entry): entry is DecisionAggregate => entry !== null);
  const recentTurns = (Array.isArray(body.recentTurns) ? body.recentTurns : [])
    .map(parseDecisionTurnSummary)
    .filter((entry): entry is DecisionTurnSummary => entry !== null);
  const summed = aggregates.reduce((sum, entry) => sum + entry.count, 0);
  return { total: Math.max(count(body.total), summed), aggregates, recentTurns };
}

export function parseDecisionTurnMarker(value: unknown): DecisionTurnMarker | null {
  const body = record(value);
  const turnId = text(body?.turnId);
  if (!body || !turnId) return null;
  const items = (Array.isArray(body.items) ? body.items : []).flatMap(
    (entry): DecisionTurnMarkerItem[] => {
      const item = record(entry);
      if (!item) return [];
      return [
        {
          origin: oneOf(DECISION_ORIGINS, item.origin, 'adhoc'),
          originId: text(item.originId),
          label: text(item.label) ?? '',
          count: count(item.count),
          passed: count(item.passed),
          failed: count(item.failed),
          errors: count(item.errors),
          lastPassProbability: probability(item.lastPassProbability),
          fired: item.fired === true,
        },
      ];
    }
  );
  return { turnId, items };
}

/** A message's `metadata.decisionSummary`, the per-turn marker's data, when it carries one. */
export function messageDecisionMarker(message: Message): DecisionTurnMarker | null {
  const metadata = record((message as { metadata?: unknown }).metadata);
  return metadata ? parseDecisionTurnMarker(metadata.decisionSummary) : null;
}

/** A message's `metadata.decisionId` (a decision chat's reply), when it carries one. */
export function messageDecisionId(message: Message): string | null {
  const metadata = record((message as { metadata?: unknown }).metadata);
  return text(metadata?.decisionId);
}

// ---------------------------------------------------------------------------
// Settings
// ---------------------------------------------------------------------------

function clampInt(value: unknown, low: number, high: number, fallback: number): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) return fallback;
  return Math.min(high, Math.max(low, Math.round(value)));
}

export function parseMonitorCriterion(value: unknown): MonitorCriterion | null {
  const body = record(value);
  const id = text(body?.id);
  if (!body || !id) return null;
  const window = record(body.window);
  return {
    id,
    name: text(body.name) ?? id,
    question: parseQuestion(body.question),
    model: parseDecisionModelRef(body.model),
    timing: oneOf(MONITOR_TIMINGS, body.timing, 'post_iteration'),
    window: {
      entries: clampInt(window?.entries, 1, MONITOR_WINDOW_ENTRIES_MAX, 24),
      maxChars: clampInt(window?.maxChars, 1, MONITOR_WINDOW_CHARS_MAX, 6000),
      toolCalls: window?.toolCalls !== false,
    },
    threshold: probability(body.threshold) ?? 0.8,
    option: text(body.option),
    consecutive: clampInt(body.consecutive, 1, MONITOR_CONSECUTIVE_MAX, 1),
    rearmBelow: probability(body.rearmBelow),
    action: oneOf(MONITOR_ACTIONS, body.action, 'notify'),
    onError: oneOf(MONITOR_ERROR_POLICIES, body.onError, 'notify'),
    maxEvalsPerTurn: clampInt(body.maxEvalsPerTurn, 1, MONITOR_MAX_EVALS_MAX, 50),
    enabled: body.enabled !== false,
    direction: oneOf(MONITOR_DIRECTIONS, body.direction, 'at_or_above'),
    rearmAbove: probability(body.rearmAbove),
    setBy: oneOf(MONITOR_OWNERS, body.setBy, 'person'),
    setBySession: text(body.setBySession),
    inheritedFrom: text(body.inheritedFrom),
    applyToSubagents: body.applyToSubagents === true,
  };
}

export function parseDecisionSettings(value: unknown): DecisionSettings | null {
  let body = record(value);
  // Tolerate a route that wraps the stored value (`{ settings: {...} }`).
  if (body && body.monitors === undefined && record(body.settings)) body = record(body.settings);
  if (!body) return null;
  const seen = new Set<string>();
  const monitors: MonitorCriterion[] = [];
  for (const entry of Array.isArray(body.monitors) ? body.monitors : []) {
    const monitor = parseMonitorCriterion(entry);
    if (monitor && !seen.has(monitor.id)) {
      seen.add(monitor.id);
      monitors.push(monitor);
    }
  }
  return { defaultModel: parseDecisionModelRef(body.defaultModel), monitors };
}

/**
 * A monitor as `PUT /sessions/{id}/decision-settings` takes it. The owner and
 * direction keys are written only when they differ from the daemon's default,
 * as the daemon stores them, so an at-or-above person's monitor is sent exactly
 * as an older daemon (which refuses keys it does not know) takes it.
 */
export function monitorToWire(monitor: MonitorCriterion): Record<string, unknown> {
  return {
    id: monitor.id,
    name: monitor.name,
    question: questionToWire(monitor.question),
    model: monitor.model ? decisionModelRefToWire(monitor.model) : null,
    timing: monitor.timing,
    window: { ...monitor.window },
    threshold: monitor.threshold,
    option: monitor.option,
    consecutive: monitor.consecutive,
    rearmBelow: monitor.rearmBelow,
    action: monitor.action,
    onError: monitor.onError,
    maxEvalsPerTurn: monitor.maxEvalsPerTurn,
    enabled: monitor.enabled,
    ...(monitor.direction === 'below' ? { direction: 'below' } : {}),
    ...(typeof monitor.rearmAbove === 'number' ? { rearmAbove: monitor.rearmAbove } : {}),
    ...(monitor.setBy === 'agent' ? { setBy: 'agent' } : {}),
    ...(monitor.setBySession ? { setBySession: monitor.setBySession } : {}),
    ...(monitor.inheritedFrom ? { inheritedFrom: monitor.inheritedFrom } : {}),
    ...(monitor.applyToSubagents ? { applyToSubagents: true } : {}),
  };
}

export function decisionSettingsToWire(settings: DecisionSettings): Record<string, unknown> {
  return {
    defaultModel: settings.defaultModel ? decisionModelRefToWire(settings.defaultModel) : null,
    monitors: settings.monitors.map(monitorToWire),
  };
}

export function parseDecisionModelEntry(value: unknown): DecisionModelEntry | null {
  const body = record(value);
  const provider = text(body?.provider);
  const model = text(body?.model);
  if (!body || !provider || !model) return null;
  const kinds = Array.isArray(body.kinds)
    ? body.kinds.filter((kind): kind is DecisionKindTag =>
        DECISION_KIND_TAGS.includes(kind as DecisionKindTag)
      )
    : [];
  return {
    provider,
    model,
    displayName: text(body.displayName) ?? model,
    contextTokens: optionalCount(body.contextTokens),
    kinds: kinds.length > 0 ? kinds : ['binary'],
    tier: tier(body.tier),
    adapter: optionalOneOf(DECISION_ADAPTERS, body.adapter),
    local: body.local === true,
    unavailableReason: text(body.unavailableReason ?? body.unavailable_reason),
  };
}

/** `GET /decisions/models`: a bare list, or one wrapped as `{ models: [...] }`. */
export function parseDecisionModelEntries(value: unknown): DecisionModelEntry[] | null {
  const list = Array.isArray(value) ? value : record(value)?.models;
  if (!Array.isArray(list)) return null;
  return list
    .map(parseDecisionModelEntry)
    .filter((entry): entry is DecisionModelEntry => entry !== null);
}

/** `GET /decisions/models` for one chat: its models, and what rules out every one. */
export interface DecisionCatalog {
  models: DecisionModelEntry[];
  /**
   * Why the chat may use no decision model at all, whichever is named or chosen
   * for it (a Crew grant), or `null`. Every model's `unavailableReason` says the
   * same; this one also covers a choice that names no model, such as a monitor
   * left on the chat's decision model.
   */
  sessionUnavailableReason: string | null;
}

export function parseDecisionCatalog(value: unknown): DecisionCatalog | null {
  const models = parseDecisionModelEntries(value);
  if (!models) return null;
  const body = Array.isArray(value) ? null : record(value);
  return {
    models,
    sessionUnavailableReason: text(
      body?.sessionUnavailableReason ?? body?.session_unavailable_reason
    ),
  };
}

/**
 * A phase's gate, read tolerantly: every field the person's gate controls added
 * (owner, questions, model, ...) may be missing (an older daemon) or of a shape
 * this build does not know, and then reads as its neutral value rather than
 * dropping the gate. Only a gate without a threshold is no gate.
 */
export function parsePhaseGateSnapshot(value: unknown): PhaseGateSnapshot | null {
  const body = record(value);
  if (!body) return null;
  const threshold = probability(body.threshold);
  if (threshold === null) return null;
  const questions = Object.entries(record(body.questions) ?? {}).flatMap(([name, question]) => {
    const asked = rawText(question);
    return name.trim() && asked !== null ? [{ name, question: asked }] : [];
  });
  return {
    threshold,
    // An older daemon sends only the count; a newer one sends both, and they agree.
    questionCount: Math.max(count(body.questionCount), questions.length),
    attempts: count(body.attempts),
    maxAttempts: Math.max(1, count(body.maxAttempts) || 3),
    lastPassed: optionalBool(body.lastPassed),
    lastMinP: probability(body.lastMinP),
    blocked: body.blocked === true,
    owner: oneOf(GATE_OWNERS, body.owner, 'agent'),
    questions,
    model: text(body.model),
    allowUncalibrated: body.allowUncalibrated === true,
    lastError: text(body.lastError),
    personDecision: optionalOneOf(PERSON_GATE_ACTIONS, body.personDecision),
    retriesGranted: count(body.retriesGranted),
    needsPerson: body.needsPerson === true,
    holds: Array.isArray(body.holds)
      ? body.holds.flatMap((id) => {
          const phase = text(id);
          return phase ? [phase] : [];
        })
      : [],
  };
}

// ---------------------------------------------------------------------------
// Installing a local decision model (`GET`/`POST /decisions/models/install`)
// ---------------------------------------------------------------------------

function parseInstallableModel(value: unknown): InstallableDecisionModel | null {
  const body = record(value);
  const model = text(body?.model);
  if (!body || !model) return null;
  return {
    model,
    displayName: text(body.displayName) ?? model,
    approxSize: text(body.approxSize) ?? '',
    note: text(body.note) ?? '',
    isDefault: body.isDefault === true,
    installed: body.installed === true,
  };
}

function parseInstallJob(value: unknown): DecisionInstallJob | null {
  const body = record(value);
  const model = text(body?.model);
  if (!body || !model) return null;
  return {
    model,
    // A state this build does not know is drawn as failed, never as installed.
    state: oneOf(DECISION_INSTALL_STATES, body.state, 'failed'),
    completedBytes: count(body.completedBytes),
    totalBytes: count(body.totalBytes),
    status: text(body.status),
    error: text(body.error),
  };
}

export function parseDecisionInstallStatus(value: unknown): DecisionInstallStatus | null {
  const body = record(value);
  const ollama = record(body?.ollama);
  if (!body || !ollama) return null;
  return {
    offers: (Array.isArray(body.offers) ? body.offers : [])
      .map(parseInstallableModel)
      .filter((entry): entry is InstallableDecisionModel => entry !== null),
    ollama: {
      baseUrl: text(ollama.baseUrl),
      // A place this build does not know offers no install: its sentence is shown.
      state: oneOf(OLLAMA_PLACES, ollama.state, 'invalid_host'),
      version: text(ollama.version),
      message: text(ollama.message),
    },
    job: parseInstallJob(body.job),
  };
}

/** `true` while an install is downloading or checking the model it pulled. */
export function installRunning(job: DecisionInstallJob | null | undefined): boolean {
  return job?.state === 'downloading' || job?.state === 'verifying';
}

// ---------------------------------------------------------------------------
// Readable answers and figures
// ---------------------------------------------------------------------------

/** The option's display text: a choice's name, a level's label, `Yes` / `No`. */
export function outcomeLabel(question: Question | null | undefined, key: string): string {
  if (question?.kind.type === 'score') {
    const index = Number(key);
    const label = Number.isInteger(index) ? question.kind.levels[index] : undefined;
    return label?.trim() ? label : `Level ${key}`;
  }
  if (key === 'yes' && (!question || question.kind.type === 'binary')) return 'Yes';
  if (key === 'no' && (!question || question.kind.type === 'binary')) return 'No';
  return key;
}

/** The key of `probabilities` an answer names: `yes` / `no`, the option, the level index. */
export function answerKey(answer: DecisionAnswer | null): string | null {
  if (!answer) return null;
  if (answer.type === 'binary') return answer.value ? 'yes' : 'no';
  if (answer.type === 'choice') return answer.option;
  return String(answer.level);
}

/** `Yes`, `rrms`, `Moderate`: the answer a person reads. */
export function readableAnswer(decision: Decision, question?: Question | null): string {
  const answer = decision.answer;
  if (!answer) return 'No answer';
  if (answer.type === 'binary') return answer.value ? 'Yes' : 'No';
  if (answer.type === 'choice') return answer.option;
  return answer.label?.trim() ? answer.label : outcomeLabel(question ?? null, String(answer.level));
}

/** The probability of the answer the decision gave, P(answer), or `null`. */
export function answerProbability(decision: Decision): number | null {
  const key = answerKey(decision.answer);
  if (key === null) return null;
  return decision.probabilities.find((entry) => entry.key === key)?.p ?? null;
}

/**
 * The threshold marker's place on one option's bar, when the question's
 * threshold is on that option: P(yes) for `yes`, P(no) for `no`, P(option).
 * A score's at-least-level threshold acts on a SUM of levels, so it names no
 * single bar and returns `null` here (`thresholdSummary` says it in words).
 * A `below` rule marks the same place: only the side that passes differs.
 */
export function thresholdOnOption(threshold: Threshold | null, key: string): number | null {
  if (!threshold) return null;
  if (threshold.type === 'yes') return key === 'yes' ? threshold.p : null;
  if (threshold.type === 'no') return key === 'no' ? threshold.p : null;
  if (threshold.type === 'option') return key === threshold.name ? threshold.p : null;
  return null;
}

/**
 * `P(yes) ≥ 0.80`, `P(rrms) ≥ 0.90`, `P(level ≥ Moderate) ≥ 0.80`; a `below` rule
 * says `<`: `P(yes) < 0.30`, `P(level ≥ Moderate) < 0.80`.
 */
export function thresholdSummary(threshold: Threshold, question?: Question | null): string {
  const p = `${threshold.below ? '<' : '≥'} ${formatProbability(threshold.p)}`;
  switch (threshold.type) {
    case 'yes':
      return `P(yes) ${p}`;
    case 'no':
      return `P(no) ${p}`;
    case 'option':
      return `P(${threshold.name}) ${p}`;
    case 'at_least_level':
      return `P(level ≥ ${outcomeLabel(question ?? null, String(threshold.level))}) ${p}`;
  }
}

/**
 * The index of the score level a monitor's `option` names: a level's label
 * first, then its index (`"0"` is the lowest), as `MonitorCriterion::pass_threshold`
 * in `settings.rs` reads it. `null` when it names none.
 */
export function scoreLevelIndex(levels: readonly string[], option: string | null): number | null {
  if (option === null) return null;
  const wanted = option.trim();
  const byLabel = levels.findIndex((level) => level.trim() === wanted);
  if (byLabel >= 0) return byLabel;
  if (!/^\d+$/.test(wanted)) return null;
  const index = Number(wanted);
  return index < levels.length ? index : null;
}

/**
 * A monitor's firing rule as the daemon applies it (`MonitorCriterion::pass_threshold`
 * in `settings.rs`): P(yes), or P(no) when a yes/no monitor watches `no`; P(option)
 * for a choice; P(level at or above `option`) for a score. A `below` monitor's
 * rule is wrapped (`below: true`), as the question it sends is. `null` when the
 * option fits no answer of its question, which the daemon refuses to save.
 */
export function monitorThreshold(monitor: MonitorCriterion): Threshold | null {
  const rule = monitorRule(monitor);
  return rule && monitor.direction === 'below' ? { ...rule, below: true } : rule;
}

function monitorRule(monitor: MonitorCriterion): Threshold | null {
  const p = monitor.threshold;
  const option = monitor.option;
  const kind = monitor.question.kind;
  switch (kind.type) {
    case 'binary': {
      const watched = option?.toLowerCase() ?? 'yes';
      if (watched === 'yes') return { type: 'yes', p };
      if (watched === 'no') return { type: 'no', p };
      return null;
    }
    case 'choice':
      return option !== null && kind.options.some((entry) => entry.name === option)
        ? { type: 'option', name: option, p }
        : null;
    case 'score': {
      const level = scoreLevelIndex(kind.levels, option);
      return level === null ? null : { type: 'at_least_level', level, p };
    }
  }
}

/** `THRESHOLD_EPSILON` in `decision/types.rs`: equality belongs to the at-or-above side. */
const THRESHOLD_EPSILON = 1e-9;

/**
 * Whether a monitor fires at probability `p` (`MonitorCriterion::fires_at`): at or
 * above its threshold, or, for a `below` monitor, the exact complement.
 */
export function monitorFiresAt(monitor: MonitorCriterion, p: number): boolean {
  const atOrAbove = p + THRESHOLD_EPSILON >= monitor.threshold;
  return monitor.direction === 'below' ? !atOrAbove : atOrAbove;
}

/** Two decimals: `0.93`. A probability that rounds to 1 or 0 but is not says so: `>0.99`. */
export function formatProbability(p: number | null | undefined): string {
  if (p === null || p === undefined || !Number.isFinite(p)) return '';
  if (p > 0.995 && p < 1) return '>0.99';
  if (p < 0.005 && p > 0) return '<0.01';
  return p.toFixed(2);
}

/** `22 ms`, `1.4 s`. */
export function formatLatency(ms: number | null | undefined): string | null {
  if (ms === null || ms === undefined || !Number.isFinite(ms) || ms < 0) return null;
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(ms < 10_000 ? 1 : 0)} s`;
}

/**
 * A model reference as the person reads it: a Baseten slug by its alias
 * (`joey-1`, as the model picker, the Usage panel and the decision chat's reply
 * name it; `baseten_display_name` in `providers/decision/catalog.rs`), an Ollama
 * tag by its last path segment (`tev1:0.8b`). The provider goes after it in a
 * title. The record keeps the slug.
 */
export function decisionModelLabel(
  model: Pick<DecisionModelRef, 'provider' | 'model'> | null | undefined
): string | null {
  if (!model) return null;
  if (model.provider === 'baseten_decision') return decisionModelName(model.model) || model.model;
  const name = model.model.slice(model.model.lastIndexOf('/') + 1);
  return name || model.model;
}

/** What a probability's source means, in words. Uncalibrated is always said. */
export function calibrationText(source: ProbabilitySource, calibrated: boolean): string {
  if (source.kind === 'verbalized') {
    return source.reason
      ? `Uncalibrated: the model wrote this number itself (${source.reason}).`
      : 'Uncalibrated: the model wrote this number itself.';
  }
  if (!calibrated) {
    return source.kind === 'logprob'
      ? `Uncalibrated: only ${formatProbability(source.mass)} of the probability fell on the options.`
      : 'Uncalibrated.';
  }
  return source.kind === 'logprob'
    ? 'Model probabilities from token log-probabilities, not validated on your data.'
    : "The decision model's own probabilities, not validated on your data.";
}

export const ORIGIN_NAMES: Record<DecisionOrigin, string> = {
  chat: 'Decision chat',
  monitor: 'Monitor',
  tool: 'decide',
  gate: 'Phase gate',
  adhoc: 'Ad hoc',
};

/** The plural used by the history view's Source filter. */
export const ORIGIN_FILTER_NAMES: Record<DecisionOrigin, string> = {
  chat: 'Decision chat',
  monitor: 'Monitors',
  tool: 'decide tool',
  gate: 'Phase gates',
  adhoc: 'Ad hoc',
};

export const OUTCOME_NAMES: Record<DecisionOutcome, string> = {
  passed: 'Passed',
  failed: 'Failed',
  decided: 'Decided',
  error: 'Error',
  refused: 'Refused',
};

/** A row's label: the origin's own (a monitor's name, a phase's title), else its kind. */
export function originLabel(origin: DecisionOrigin, label?: string | null): string {
  const named = label?.trim();
  if (origin === 'tool') return ORIGIN_NAMES.tool;
  return named || ORIGIN_NAMES[origin];
}

// ---------------------------------------------------------------------------
// The summary's rows: one per source, never one per call
// ---------------------------------------------------------------------------

export type DecisionSourceState =
  | 'pending'
  | 'armed'
  | 'fired'
  | 'off'
  | 'error'
  | 'done'
  | 'passed'
  | 'failed'
  | 'blocked'
  | 'skipped';

export interface DecisionSourceRow {
  /** `monitor:<id>`, `tool`, `gate:<phase id>`, `chat`, `adhoc`. */
  key: string;
  origin: DecisionOrigin;
  /** The filter the history view opens with; `null` for the whole origin. */
  originId: string | null;
  label: string;
  count: number;
  passed: number;
  failed: number;
  errors: number;
  lastAt: string | null;
  lastAnswer: string | null;
  lastPassProbability: number | null;
  state: DecisionSourceState;
  /**
   * A gate row's state from the gate's own evaluations alone, before the
   * person's pass or skip and a phase that waits for them: what the row's words
   * (`0.62 < 0.80 · attempt 3 of 3`) say after the person's. `null` on any other
   * row.
   */
  gateVerdict: DecisionSourceState | null;
  /** A monitor's or gate's threshold, for the bar's marker. */
  threshold: number | null;
  /** A gate's attempt line: `attempt 2 of 3`. */
  attempts: { attempts: number; maxAttempts: number } | null;
}

export interface DecisionGateSource {
  phaseId: string;
  title: string;
  gate: PhaseGateSnapshot;
}

function later(a: string | null, b: string | null): boolean {
  if (!b) return false;
  if (!a) return true;
  const left = Date.parse(a);
  const right = Date.parse(b);
  return Number.isFinite(right) && (!Number.isFinite(left) || right > left);
}

const SOURCE_ORDER: readonly DecisionOrigin[] = ['monitor', 'tool', 'gate', 'chat', 'adhoc'];

function sourceKey(origin: DecisionOrigin, originId: string | null): string {
  return origin === 'monitor' || origin === 'gate' ? `${origin}:${originId ?? ''}` : origin;
}

/**
 * The chat summary's Decisions rows: one per monitor (by its id, including the
 * monitors no call has reached yet), ONE for every `decide` batch together, one
 * per phase gate, one for the decision chat and one for ad hoc calls. Never one
 * per call: a chat with 5,000 calls draws as many rows as it has sources.
 *
 * Order: monitors as the settings list them (a removed monitor that still has
 * calls after them), then the decide tool, the gates in plan order, the decision
 * chat, ad hoc. `fired` is the set of monitor ids the newest turn marker says
 * fired.
 */
export function decisionSourceRows(
  summary: SessionDecisionSummary | null,
  settings: DecisionSettings | null,
  options: { gates?: readonly DecisionGateSource[]; fired?: ReadonlySet<string> } = {}
): DecisionSourceRow[] {
  const monitors = settings?.monitors ?? [];
  const gates = options.gates ?? [];
  const fired = options.fired ?? new Set<string>();
  const rows = new Map<string, DecisionSourceRow & { rank: number }>();
  let seen = 0;

  const ensure = (origin: DecisionOrigin, originId: string | null, label: string) => {
    const key = sourceKey(origin, originId);
    let row = rows.get(key);
    if (!row) {
      const monitorIndex = monitors.findIndex((monitor) => monitor.id === originId);
      const gateIndex = gates.findIndex((gate) => gate.phaseId === originId);
      const within =
        origin === 'monitor' && monitorIndex >= 0
          ? monitorIndex
          : origin === 'gate' && gateIndex >= 0
            ? gateIndex
            : 1000 + seen;
      seen += 1;
      row = {
        key,
        origin,
        originId: origin === 'monitor' || origin === 'gate' ? originId : null,
        label,
        count: 0,
        passed: 0,
        failed: 0,
        errors: 0,
        lastAt: null,
        lastAnswer: null,
        lastPassProbability: null,
        state: 'done',
        gateVerdict: null,
        threshold: null,
        attempts: null,
        rank: SOURCE_ORDER.indexOf(origin) * 100_000 + within,
      };
      rows.set(key, row);
    }
    return row;
  };

  for (const monitor of monitors) {
    ensure('monitor', monitor.id, monitor.name).threshold = monitor.threshold;
  }
  for (const gate of gates) ensure('gate', gate.phaseId, gate.title);
  for (const aggregate of summary?.aggregates ?? []) {
    const row = ensure(
      aggregate.origin,
      aggregate.originId,
      originLabel(aggregate.origin, aggregate.label)
    );
    row.count += aggregate.count;
    row.passed += aggregate.passed;
    row.failed += aggregate.failed;
    row.errors += aggregate.errors;
    if (row.lastAt === null || later(row.lastAt, aggregate.lastAt)) {
      row.lastAt = aggregate.lastAt || row.lastAt;
      row.lastAnswer = aggregate.lastAnswer;
      row.lastPassProbability = aggregate.lastPassProbability;
    }
  }

  for (const row of rows.values()) {
    // The last call left no probability while calls have failed: it was one of them.
    const lastFailed = row.count > 0 && row.errors > 0 && row.lastPassProbability === null;
    if (row.origin === 'monitor') {
      const monitor = monitors.find((entry) => entry.id === row.originId);
      if (monitor && !monitor.enabled) row.state = 'off';
      else if (lastFailed) row.state = 'error';
      else if (
        fired.has(row.originId ?? '') ||
        (row.lastPassProbability !== null &&
          // A `below` monitor fires under its threshold: its own rule, not `≥`.
          (monitor
            ? monitorFiresAt(monitor, row.lastPassProbability)
            : row.threshold !== null && row.lastPassProbability >= row.threshold))
      )
        row.state = 'fired';
      else row.state = 'armed';
    } else if (row.origin === 'gate') {
      const gate = gates.find((entry) => entry.phaseId === row.originId)?.gate;
      if (gate) {
        row.threshold = gate.threshold;
        row.attempts = { attempts: gate.attempts, maxAttempts: gate.maxAttempts };
        if (row.lastPassProbability === null) row.lastPassProbability = gate.lastMinP;
      }
      let verdict: DecisionSourceState;
      if (gate?.blocked) verdict = 'blocked';
      else if (gate?.lastPassed === true) verdict = 'passed';
      else if (gate?.lastPassed === false) verdict = 'failed';
      else if (lastFailed) verdict = 'error';
      else if (row.count === 0) verdict = 'pending';
      else verdict = row.failed > 0 ? 'failed' : 'passed';
      row.gateVerdict = verdict;
      // The state is the chip's (`phaseGateState`): a person's pass or skip, and
      // a phase that waits for them, come before the gate's own verdict. Only a
      // gate that has decided nothing yet falls back to what its calls say.
      const shared = gate ? phaseGateState(gate) : 'pending';
      row.state = shared === 'pending' ? verdict : shared;
    } else {
      row.state = lastFailed ? 'error' : 'done';
    }
  }
  return [...rows.values()].sort((a, b) => a.rank - b.rank).map(({ rank: _rank, ...row }) => row);
}

/** The monitor ids the newest turn marker in `messages` says fired, if any did. */
export function firedMonitorIds(messages: readonly Message[]): Set<string> {
  for (let index = messages.length - 1; index >= 0; index -= 1) {
    const marker = messageDecisionMarker(messages[index]);
    if (!marker) continue;
    return new Set(
      marker.items
        .filter((item) => item.origin === 'monitor' && item.fired && item.originId)
        .map((item) => item.originId!)
    );
  }
  return new Set();
}

// ---------------------------------------------------------------------------
// When to refetch
// ---------------------------------------------------------------------------

/** Tools whose answer can add decision rows: a decide batch, or a phase completed through a gate. */
const DECISION_TOOLS = new Set(['decide', 'workspace_orchestrate']);

function isDecisionTool(name: unknown): boolean {
  if (typeof name !== 'string' || !name) return false;
  const segments = name.split('__');
  return DECISION_TOOLS.has(segments[segments.length - 1]);
}

/**
 * A string that changes when a decision-making tool call ANSWERS (a `decide`
 * batch, a `workspace_orchestrate` call that may have run a gate), when a
 * decision notice lands (a turn marker, a monitor that fired) and when a
 * decision chat's reply lands. Streamed prose never moves it, and the same
 * transcript always derives the same string.
 */
export function decisionRevision(messages: readonly Message[]): string {
  const requests = new Set<string>();
  const marks: string[] = [];
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'toolRequest') {
        const call = record(content.toolCall.value);
        if (isDecisionTool(call?.name)) requests.add(content.id);
      }
    }
  }
  for (const message of messages) {
    for (const content of message.content) {
      if (content.type === 'toolResponse' && requests.has(content.id)) marks.push(content.id);
    }
    const marker = messageDecisionMarker(message);
    if (marker) marks.push(`turn:${marker.turnId}:${message.id ?? ''}`);
    const decisionId = messageDecisionId(message);
    if (decisionId) marks.push(`decision:${decisionId}`);
  }
  return JSON.stringify(marks.sort());
}

/** `true` once the transcript holds any decision activity `decisionRevision` sees. */
export function decisionRevisionHasActivity(revision: string): boolean {
  return revision !== '' && revision !== '[]';
}

// ---------------------------------------------------------------------------
// The decision history view's filter, and the artifact that opens it
// ---------------------------------------------------------------------------

export interface DecisionHistoryFilter {
  /**
   * Only this call (`DecisionQuery.decisionId`): a decision card's link, which
   * opens the history on that one row, selected. Any other filter the person
   * changes in the view lets it go.
   */
  decisionId?: string | null;
  origin?: DecisionOrigin | null;
  originId?: string | null;
  turnId?: string | null;
  batchId?: string | null;
  outcome?: DecisionOutcome | null;
  errorsOnly?: boolean;
  minP?: number | null;
  maxP?: number | null;
}

/** The title of the decision history tab. One tab per chat. */
export const DECISION_HISTORY_TITLE = 'Decisions';

/**
 * The artifact side panel source that opens the decision history view, filtered.
 * One tab per chat: opening it again with another filter re-filters that tab
 * (the panel replaces a tab's source when the key matches). `openedAt` makes
 * the same filter applied twice still count as a new request.
 */
export function decisionHistoryArtifact(
  sessionId: string,
  filter: DecisionHistoryFilter = {}
): {
  kind: 'decisions';
  title: string;
  sessionId: string;
  filter: DecisionHistoryFilter;
  openedAt: number;
} {
  return {
    kind: 'decisions',
    title: DECISION_HISTORY_TITLE,
    sessionId,
    filter,
    openedAt: Date.now(),
  };
}

/** The query string `GET /sessions/{id}/decisions` takes, from a filter and a cursor. */
export function decisionQuery(
  filter: DecisionHistoryFilter,
  page: { before?: string | null; limit?: number } = {}
): Record<string, string | number | boolean> {
  const query: Record<string, string | number | boolean> = {};
  if (filter.decisionId) query.decisionId = filter.decisionId;
  if (filter.origin) query.origin = filter.origin;
  if (filter.originId) query.originId = filter.originId;
  if (filter.turnId) query.turnId = filter.turnId;
  if (filter.batchId) query.batchId = filter.batchId;
  if (filter.outcome) query.outcome = filter.outcome;
  if (filter.errorsOnly) query.errorsOnly = true;
  if (typeof filter.minP === 'number' && Number.isFinite(filter.minP)) query.minP = filter.minP;
  if (typeof filter.maxP === 'number' && Number.isFinite(filter.maxP)) query.maxP = filter.maxP;
  if (page.before) query.before = page.before;
  query.limit = Math.min(DECISION_PAGE_MAX, Math.max(1, page.limit ?? 100));
  return query;
}

// ---------------------------------------------------------------------------
// Requests
// ---------------------------------------------------------------------------

/*
 * Through the generated functions (`api/sdk.gen.ts`), which carry the daemon
 * secret's security scheme, and with the person's proof on every request: the
 * routes filter a private chat's decisions through `session_reach`, and without
 * the proof a private chat's rows would silently be refused.
 */

export class DecisionRequestError extends Error {
  status: number | null;
  /** The sentence the daemon gave with its refusal, or `null` when it gave none. */
  reason: string | null;
  constructor(message: string, status: number | null, reason: string | null = null) {
    super(message);
    this.name = 'DecisionRequestError';
    this.status = status;
    this.reason = reason;
  }
}

/** The sentence a refusal carries: a plain-text body, or `message` / `error` in a JSON one. */
export function refusalText(error: unknown): string | null {
  if (typeof error === 'string') return error.trim() || null;
  const body = record(error);
  return text(body?.message) ?? text(body?.error) ?? text(body?.detail);
}

type ClientResult = {
  data?: unknown;
  error?: unknown;
  response?: Response;
};

function settle(result: ClientResult, what: string): { data: unknown; missing: boolean } {
  const status = result.response?.status ?? null;
  if (status === 404) return { data: null, missing: true };
  if (result.error !== undefined || result.data === undefined) {
    const reason = refusalText(result.error);
    throw new DecisionRequestError(reason ?? `${what} unavailable`, status, reason);
  }
  return { data: result.data, missing: false };
}

/**
 * `GET /sessions/{id}/decisions/summary`. `missing` is a daemon that predates the
 * route (404): nothing to show, and nothing Retry could fix.
 */
export async function fetchDecisionSummary(
  sessionId: string,
  signal?: AbortSignal
): Promise<{ data: SessionDecisionSummary | null; missing: boolean }> {
  const headers = await userActionHeaders();
  const result = (await getSessionDecisionSummary({
    path: { session_id: sessionId },
    headers,
    signal,
  })) as ClientResult;
  const { data, missing } = settle(result, 'Decisions');
  if (missing) return { data: null, missing };
  const parsed = parseSessionDecisionSummary(data);
  if (!parsed) throw new DecisionRequestError('Decisions unavailable', null);
  return { data: parsed, missing };
}

export async function fetchDecisionSettings(
  sessionId: string,
  signal?: AbortSignal
): Promise<{ data: DecisionSettings | null; missing: boolean }> {
  const headers = await userActionHeaders();
  const result = (await getDecisionSettings({
    path: { session_id: sessionId },
    headers,
    signal,
  })) as ClientResult;
  const { data, missing } = settle(result, 'Decision settings');
  if (missing) return { data: null, missing };
  const parsed = parseDecisionSettings(data);
  if (!parsed) throw new DecisionRequestError('Decision settings unavailable', null);
  return { data: parsed, missing };
}

/** `PUT /sessions/{id}/decision-settings`. The stored value, or the daemon's refusal as an error. */
export async function saveDecisionSettings(
  sessionId: string,
  settings: DecisionSettings
): Promise<DecisionSettings> {
  const headers = await userActionHeaders();
  const result = (await putDecisionSettings({
    path: { session_id: sessionId },
    // The parsers' own wire writer, which mirrors the Rust enums the generated type spells out.
    body: decisionSettingsToWire(settings) as PutDecisionSettingsData['body'],
    headers,
  })) as ClientResult;
  const status = result.response?.status ?? null;
  if (result.error !== undefined || result.data === undefined) {
    throw new DecisionRequestError(
      refusalText(result.error) ?? `Could not save (${status ?? 'no answer'})`,
      status
    );
  }
  return parseDecisionSettings(result.data) ?? settings;
}

export async function fetchDecisionPage(
  sessionId: string,
  filter: DecisionHistoryFilter,
  page: { before?: string | null; limit?: number } = {},
  signal?: AbortSignal
): Promise<DecisionPage> {
  const headers = await userActionHeaders();
  const result = (await listSessionDecisions({
    path: { session_id: sessionId },
    query: decisionQuery(filter, page) as ListSessionDecisionsData['query'],
    headers,
    signal,
  })) as ClientResult;
  const { data, missing } = settle(result, 'Decisions');
  if (missing) return { records: [], nextCursor: null, total: 0 };
  const parsed = parseDecisionPage(data);
  if (!parsed) throw new DecisionRequestError('Decisions unavailable', null);
  return parsed;
}

export async function fetchDecisionDetail(
  sessionId: string,
  decisionId: string,
  signal?: AbortSignal
): Promise<DecisionDetail | null> {
  const headers = await userActionHeaders();
  const result = (await getSessionDecision({
    path: { session_id: sessionId, decision_id: decisionId },
    headers,
    signal,
  })) as ClientResult;
  const { data, missing } = settle(result, 'Decision');
  if (missing) return null;
  const parsed = parseDecisionDetail(data);
  if (!parsed) throw new DecisionRequestError('Decision unavailable', null);
  return parsed;
}

/**
 * `GET /decisions/models`. Name a chat (`sessionId`) and each model carries the
 * daemon's own reason that chat may not use it (`unavailableReason`: its tier,
 * or an institution the model does not cover); the route then asks
 * `session_reach` like every route that names a chat.
 */
export async function fetchDecisionModels(
  signal?: AbortSignal,
  sessionId?: string
): Promise<DecisionModelEntry[]> {
  return (await fetchDecisionCatalog(signal, sessionId)).models;
}

/**
 * {@link fetchDecisionModels}, with the reason the named chat may use no
 * decision model at all (`sessionUnavailableReason`), for a choice no row
 * stands for: a monitor left on the chat's decision model.
 *
 * `missing` is a daemon that predates the route (404): it lists no models
 * because it cannot, which the caller says rather than showing an empty list as
 * if there were none. A refusal or a failure is thrown as a
 * {@link DecisionRequestError} carrying the status and the daemon's sentence.
 */
export async function fetchDecisionCatalog(
  signal?: AbortSignal,
  sessionId?: string
): Promise<DecisionCatalog & { missing: boolean }> {
  const headers = await userActionHeaders();
  const result = (await listDecisionModels({
    ...(sessionId ? { query: { sessionId } } : {}),
    headers,
    signal,
  })) as ClientResult;
  // The route answers 404 itself, with a sentence, when the named chat is gone;
  // only a bare 404 is a daemon without the route.
  const goneReason = result.response?.status === 404 ? refusalText(result.error) : null;
  if (goneReason) throw new DecisionRequestError(goneReason, 404, goneReason);
  const { data, missing } = settle(result, 'Decision models');
  if (missing) return { models: [], sessionUnavailableReason: null, missing };
  return {
    ...(parseDecisionCatalog(data) ?? { models: [], sessionUnavailableReason: null }),
    missing,
  };
}

/**
 * `GET /decisions/models/install`: the models Biorouter installs, whether each is
 * installed, where the configured Ollama is, and the current or last install.
 * `missing` is a daemon that predates the route (404), which installs nothing.
 */
export async function fetchDecisionInstall(
  signal?: AbortSignal
): Promise<{ data: DecisionInstallStatus | null; missing: boolean }> {
  const headers = await userActionHeaders();
  const result = (await getDecisionInstall({ headers, signal })) as ClientResult;
  const { data, missing } = settle(result, 'Decision model install');
  if (missing) return { data: null, missing };
  const parsed = parseDecisionInstallStatus(data);
  if (!parsed) throw new DecisionRequestError('Decision model install unavailable', null);
  return { data: parsed, missing };
}

/**
 * `POST /decisions/models/install`: start installing `model` (the daemon's
 * default, `tev1:0.8b`, when none is named) into the Ollama on this computer.
 * The answer is the install status to poll from. A refusal is thrown as a
 * {@link DecisionRequestError} carrying the status and the daemon's sentence:
 * 400 for a model Biorouter does not install, 409 while another model installs
 * or when Ollama is on another computer, 503 when Ollama is not running.
 */
export async function startDecisionInstall(model?: string): Promise<DecisionInstallStatus> {
  const headers = await userActionHeaders();
  const result = (await postDecisionInstall({
    body: model ? { model } : {},
    headers,
  })) as ClientResult;
  const status = result.response?.status ?? null;
  if (result.error !== undefined || result.data === undefined) {
    const reason = refusalText(result.error);
    throw new DecisionRequestError(
      reason ?? `Could not start the install (${status ?? 'no answer'})`,
      status,
      reason
    );
  }
  const parsed = parseDecisionInstallStatus(result.data);
  if (!parsed) throw new DecisionRequestError('Decision model install unavailable', status);
  return parsed;
}
