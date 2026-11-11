import {
  AlertCircle,
  AlertTriangle,
  ArrowRight,
  Check,
  CircleIcon,
  Eye,
  Flag,
  Lock,
  X,
} from '../icons/app-icons';
import type { EntityIcon } from '../icons/entity-icons';
import {
  decisionModelLabel,
  formatProbability,
  outcomeLabel,
  readableAnswer,
  type DecisionHistoryFilter,
  type DecisionModelRef,
  type DecisionOrigin,
  type DecisionOutcome,
  type DecisionRecord,
  type DecisionTier,
  type DecisionTierEffect,
  type PhaseGateState,
  type Question,
  type Threshold,
} from '../../utils/sessionDecisions';
import { isBrowserSurface } from '../../utils/surface';

/**
 * The words every decision surface uses (the history, the `decide` card, the turn
 * marker, the gate dialog, the Monitors dialog, the decision composer and the chat
 * summary), in one module so they cannot drift. Pure functions over the parsed
 * wire shapes in `utils/sessionDecisions.ts`; nothing here fetches or renders.
 *
 * The rules they carry (live QA 2026-11-10, R1 to R6):
 * - R1: a monitor check is Fired or Quiet, a gate Passed or Not passed, and a
 *   decide, decision chat or ad hoc call has no verdict word: its answer is it.
 * - R2: a probability names its outcome (`P(no) 0.88`), beside an answer that
 *   answer's own; `≥` only inside a score's watched level.
 * - R3: a model is its alias plus where it runs, never a provider key or slug.
 * - R6: no question name, batch id, turn id or raw origin id in a title.
 */

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

const TIME = new Intl.DateTimeFormat(undefined, {
  hour: 'numeric',
  minute: '2-digit',
  second: '2-digit',
});
const DAY_TIME = new Intl.DateTimeFormat(undefined, {
  month: 'short',
  day: 'numeric',
  hour: 'numeric',
  minute: '2-digit',
});

/** Seconds today (calls come fast), the day and minute before. `''` for a bad time. */
export function formatDecisionTime(iso: string, now = new Date()): string {
  const date = new Date(iso);
  if (!Number.isFinite(date.getTime())) return '';
  return date.toDateString() === now.toDateString() ? TIME.format(date) : DAY_TIME.format(date);
}

// ---------------------------------------------------------------------------
// Models (R3)
// ---------------------------------------------------------------------------

export const DECISION_PLACE_WORDS = {
  here: 'this computer',
  /** A `biorouter serve` browser: the model runs where the daemon does, not here. */
  hereInBrowser: 'the computer Biorouter runs on',
  baseten: 'Baseten',
  ollama: 'Ollama',
} as const;

/**
 * Where a decision model runs, in words, or `null` when the provider is not one a
 * person knows by name (its key is never printed). A local model is "this computer"
 * ("the computer Biorouter runs on" in a `biorouter serve` browser); a hosted
 * Baseten one "Baseten"; an Ollama model that is not local (a remote host, which
 * reads as Public) "Ollama".
 *
 * `local` wins when the caller knows it (the catalog says so); otherwise an Ollama
 * model is local unless its tier is `public`.
 */
export function decisionModelPlace(
  provider: string,
  where: { local?: boolean | null; tier?: DecisionTier | null; browser?: boolean } = {}
): string | null {
  const browser = where.browser ?? isBrowserSurface();
  const here = browser ? DECISION_PLACE_WORDS.hereInBrowser : DECISION_PLACE_WORDS.here;
  if (where.local === true) return here;
  if (provider === 'baseten_decision') return DECISION_PLACE_WORDS.baseten;
  if (provider === 'ollama_decision') {
    if (where.local === false || where.tier === 'public') return DECISION_PLACE_WORDS.ollama;
    return here;
  }
  return null;
}

/**
 * `joey-1 · Baseten`, `tev1:0.8b · this computer`, `tev1:0.8b · Ollama` for a remote
 * Ollama host (`tier === 'public'`); the alias alone for any other provider.
 * `null` without a model.
 */
export function decisionModelDisplay(
  ref: Pick<DecisionModelRef, 'provider' | 'model'> | null | undefined,
  tier?: DecisionTier | null,
  browser?: boolean
): string | null {
  const alias = decisionModelLabel(ref);
  if (!ref || !alias) return null;
  const place = decisionModelPlace(ref.provider, { tier, browser });
  return place ? `${alias} · ${place}` : alias;
}

// ---------------------------------------------------------------------------
// What a decision model does to the chat's tier (DR-33)
// ---------------------------------------------------------------------------

/** "A, B and C" (and a lone "A"). */
export function joinWithAnd(parts: readonly string[]): string {
  if (parts.length <= 1) return parts.join('');
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
}

/** A public model a line names, with what the catalog says about where it runs. */
export type PublicModelMention = Pick<DecisionModelRef, 'provider' | 'model'> & {
  tier?: DecisionTier | null;
  local?: boolean | null;
};

/**
 * `joey-1 runs on Baseten`, `joey-1 and kim-2 run on Baseten`, `joey-1 runs on
 * Baseten and tev1:0.8b runs on Ollama`: each place once, its models before it,
 * aliases only (R3). Repeats are named once.
 */
export function publicModelsSentence(
  models: readonly PublicModelMention[],
  browser?: boolean
): string {
  const byPlace = new Map<string, string[]>();
  for (const model of models) {
    const alias = decisionModelLabel(model);
    if (!alias) continue;
    const place =
      decisionModelPlace(model.provider, {
        tier: model.tier ?? 'public',
        local: model.local ?? null,
        browser,
      }) ?? '';
    const names = byPlace.get(place) ?? [];
    if (!names.includes(alias)) names.push(alias);
    byPlace.set(place, names);
  }
  return joinWithAnd(
    [...byPlace.entries()].map(([place, names]) => {
      const verb = names.length > 1 ? 'run' : 'runs';
      return place ? `${joinWithAnd(names)} ${verb} on ${place}` : joinWithAnd(names);
    })
  );
}

/** `joey-1 · Baseten and tev1:0.8b · Ollama` (R3), each model once. */
export function publicModelsList(models: readonly PublicModelMention[], browser?: boolean): string {
  const names: string[] = [];
  for (const model of models) {
    const name = decisionModelDisplay(model, model.tier ?? 'public', browser);
    if (name && !names.includes(name)) names.push(name);
  }
  return joinWithAnd(names);
}

/**
 * The words for a chat's tier as its decision models set it (design §5): one short
 * line up front for the consequence, the detail in an InfoTip. Shared by the
 * Monitors dialog, the gate dialog, both composers' Send, a workflow's start, the
 * chat's privacy pill and the model chip, so no two say it differently.
 */
export const TIER_EFFECT_COPY = {
  /** A picker row's secondary text, for a model whose choice makes the chat public. */
  makesPublic: 'Makes this chat public',
  willBePublic: (sentence: string) => `This chat will be public: ${sentence}.`,
  staysPublic: (sentence: string) => `This chat stays public: ${sentence}.`,
  /** The tip beside "This chat will be public". `chatModel` is the chat's own model, when known. */
  publicInfo: (chatModel: string | null) =>
    `A chat is as private as its least private model, so ${
      chatModel ?? 'its language model'
    } keeps answering but the chat can't use private extensions, knowledge bases or chats. Choose a model on this computer to keep it private.`,
  publicInfoLabel: 'Why this chat will be public',
  becomesPrivate: 'This chat becomes private at your next message.',
  becomesPrivateInfo:
    "Public models can't be used in it after that unless you make it public from History.",
  /** The tip of a public row this chat can no longer use. */
  publicRefusedInfo:
    'Start a new chat and choose this model before the first message, or make this chat public from History.',
  /** The question a Send, a Save or a start asks when the daemon asks for the person's answer. */
  confirmTitle: 'Make this chat public?',
  confirmSend: 'Send',
  confirmSave: 'Make it public',
  confirmCancel: 'Cancel',
  workflowTitle: 'Start this workflow?',
  workflowQuestion: (models: string) =>
    `This workflow uses ${models}, so this chat will be public.`,
  workflowConfirm: 'Start it',
  /** The model chip's extra line on a chat its decision models made public. */
  chipLine: 'This chat is public because of its decision models.',
  /** The model switch, on a chat its decision models keep public. */
  switchStaysPublic: (models: string) =>
    `This chat stays public: its decision models include ${models}.`,
  /** The model switch, on a subagent its public parent's ceiling keeps public. */
  switchStaysPublicParent: "This chat stays public: it is part of a public chat's work.",
  /** The declassify dialog: the public monitors the declassification turns off (design §3.11). */
  declassifyMonitorsOff: (names: readonly string[], models: string) =>
    `${names.length > 1 ? 'Monitors' : 'Monitor'} ${joinWithAnd(
      names.map((name) => `"${name}"`)
    )} ${names.length > 1 ? 'use' : 'uses'} ${models} and will be turned off.`,
  declassifyDefaultCleared: (model: string) =>
    `The default decision model ${model} will be cleared.`,
  declassifyInfo: 'After this, a public decision model you choose can read the whole chat.',
  declassifyInfoLabel: 'Decision models after this',
  /** The model switch, on a chat whose public decision models will stop. */
  switchPublicModelsStop: (models: string) => `Its public decision models (${models}) will stop.`,
} as const;

/**
 * What a write's answer said about the chat's tier, worth a line: the daemon's own
 * line, and the tip that goes with it. `null` for an answer with nothing to say.
 */
export function tierEffectNote(
  effect: DecisionTierEffect | null,
  line: string | null,
  chatModelName: string | null = null
): { line: string; info: string | null } | null {
  switch (effect) {
    case 'becomesPublic':
    case 'staysPublic':
      return line ? { line, info: TIER_EFFECT_COPY.publicInfo(chatModelName) } : null;
    case 'becomesPrivateAtNextMessage':
      return {
        line: line ?? TIER_EFFECT_COPY.becomesPrivate,
        info: TIER_EFFECT_COPY.becomesPrivateInfo,
      };
    default:
      return null;
  }
}

/**
 * What declassifying a chat does to its decision settings (design §3.11), in the
 * dialog's words: each enabled monitor on a public model is turned off, and a public
 * default is cleared. `null` when the chat names no public decision model. The tiers
 * are the catalog's; a model the catalog does not list is not named, because its
 * tier is not known here (the daemon turns it off if it is public).
 */
export function declassifyDecisionLines(
  settings: {
    defaultModel: Pick<DecisionModelRef, 'provider' | 'model'> | null;
    monitors: readonly {
      name: string;
      enabled: boolean;
      model: Pick<DecisionModelRef, 'provider' | 'model'> | null;
    }[];
  } | null,
  models: readonly { provider: string; model: string; tier: DecisionTier | null }[]
): string[] {
  if (!settings) return [];
  const isPublic = (ref: Pick<DecisionModelRef, 'provider' | 'model'> | null) =>
    !!ref &&
    models.some(
      (entry) =>
        entry.provider === ref.provider && entry.model === ref.model && entry.tier === 'public'
    );
  const off = settings.monitors.filter((monitor) => monitor.enabled && isPublic(monitor.model));
  const lines: string[] = [];
  if (off.length > 0) {
    lines.push(
      TIER_EFFECT_COPY.declassifyMonitorsOff(
        off.map((monitor) => monitor.name),
        publicModelsList(off.map((monitor) => monitor.model!))
      )
    );
  }
  if (isPublic(settings.defaultModel)) {
    lines.push(
      TIER_EFFECT_COPY.declassifyDefaultCleared(publicModelsList([settings.defaultModel!]))
    );
  }
  return lines;
}

/** `PrivacyBasis` (`GET /sessions/{id}`): why a chat on a private model is public. */
export interface PrivacyBasisWords {
  source: 'default' | 'monitor' | 'gate' | 'phaseMonitor' | 'parent';
  /** The monitor's name, the phase's title, or the parent chat's id. */
  label: string;
  /** The model's own name (`joey-1`); empty for a parent that recorded none. */
  model: string;
  /** Where it runs, in words (`Baseten`). */
  place: string;
}

/** `monitor "Done" uses joey-1 · Baseten`, or `null` for a parent's ceiling. */
function basisReason(basis: PrivacyBasisWords): string | null {
  const model = [basis.model, basis.place].filter(Boolean).join(' · ');
  switch (basis.source) {
    case 'monitor':
      return `monitor "${basis.label}" uses ${model}`;
    case 'default':
      return `its default decision model is ${model}`;
    case 'gate':
      return `the gate on phase "${basis.label}" uses ${model}`;
    case 'phaseMonitor':
      return `a monitor on phase "${basis.label}" uses ${model}`;
    case 'parent':
      return null;
  }
}

/**
 * The privacy pill's InfoTip on a chat a decision model made public: "Public:
 * monitor "Done" uses joey-1 · Baseten. A chat is as private as its least private
 * model, so it can't use private extensions, knowledge bases or chats." For a
 * subagent of a public chat: "Public: it is part of a public chat's work."
 */
export function privacyBasisTip(basis: PrivacyBasisWords): string {
  const reason = basisReason(basis);
  if (!reason) return "Public: it is part of a public chat's work.";
  return `Public: ${reason}. A chat is as private as its least private model, so it can't use private extensions, knowledge bases or chats.`;
}

// ---------------------------------------------------------------------------
// Probabilities and thresholds (R2)
// ---------------------------------------------------------------------------

/** `yes` / `no` for a yes/no answer, the label as written otherwise. */
function probabilityName(label: string): string {
  const lower = label.trim().toLowerCase();
  return lower === 'yes' || lower === 'no' ? lower : label;
}

/**
 * `P(no) 0.88`, `P(rrms) 0.62`, `P(Moderate) 0.40`. `label` is the outcome's display
 * text (`outcomeLabel`); Yes and No are lowercased. `null` without a probability.
 */
export function namedProbability(label: string, p: number | null | undefined): string | null {
  const value = formatProbability(p);
  if (!value) return null;
  return `P(${probabilityName(label)}) ${value}`;
}

/** The outcome a threshold watches, as `P(...)` names it: `yes`, `rrms`, `level ≥ Moderate`. */
function watchedName(threshold: Threshold, question?: Question | null): string {
  switch (threshold.type) {
    case 'yes':
      return 'yes';
    case 'no':
      return 'no';
    case 'option':
      return threshold.name;
    case 'at_least_level':
      return `level ≥ ${outcomeLabel(question ?? null, String(threshold.level))}`;
  }
}

/**
 * The watched outcome's probability, named by the threshold: `P(yes) 0.12`,
 * `P(level ≥ Moderate) 0.40`. What a monitor or gate row prints beside its bar
 * (`passProbability`). `null` without a probability.
 */
export function watchedProbability(
  threshold: Threshold,
  question: Question | null | undefined,
  passP: number | null | undefined
): string | null {
  const value = formatProbability(passP);
  return value ? `P(${watchedName(threshold, question)}) ${value}` : null;
}

/**
 * `P(yes) 0.12 · threshold 0.85`; a `below` rule `P(yes) 0.02 · threshold below 0.30`;
 * `P(rrms) 0.62 · threshold 0.80`; a score `P(level ≥ Moderate) 0.40 · threshold 0.80`
 * (the only `≥`). `null` without a threshold: the line is then left out, never
 * "No threshold". Without a probability only the threshold part is said.
 */
export function thresholdLine(
  threshold: Threshold | null | undefined,
  question: Question | null | undefined,
  passP: number | null | undefined
): string | null {
  if (!threshold) return null;
  const limit = `threshold ${threshold.below ? 'below ' : ''}${formatProbability(threshold.p)}`;
  const watched = watchedProbability(threshold, question, passP);
  return watched ? `${watched} · ${limit}` : limit;
}

/**
 * A rule in the words an editor uses, never `≥` or `<` (Monitors dialog, decision
 * composer, gate editor): `when P(yes) is at or above 0.85`, `when P(yes) is below
 * 0.30`, `when P(rrms) is at or above 0.70`, `when P(mild or a higher level) is at
 * or above 0.85`. The live sentence adds its verb ("Fires …", "Meets the threshold …").
 */
export function ruleInWords(rule: Threshold, question?: Question | null): string {
  let outcome: string;
  switch (rule.type) {
    case 'yes':
      outcome = 'yes';
      break;
    case 'no':
      outcome = 'no';
      break;
    case 'option':
      outcome = rule.name;
      break;
    case 'at_least_level':
      outcome = `${outcomeLabel(question ?? null, String(rule.level))} or a higher level`;
      break;
  }
  const side = rule.below ? 'below' : 'at or above';
  return `when P(${outcome}) is ${side} ${formatProbability(rule.p)}`;
}

export const CALIBRATION_COPY = {
  estimate: "This model's probabilities are its own estimate.",
  estimateTip:
    'The model wrote its number itself instead of reading it from its token probabilities, so treat it as a rough guide.',
  calibrated: "The decision model's own probabilities, not validated on your data.",
  uncalibratedDetail: 'Uncalibrated: the model wrote this number itself.',
} as const;

// ---------------------------------------------------------------------------
// Verdicts and record lines (R1, R6)
// ---------------------------------------------------------------------------

/**
 * A monitor `Fired` / `Quiet`, a phase gate `Passed` / `Not passed`, `Error` and
 * `Refused` for every origin. Decide, decision chat and ad hoc calls have no verdict
 * word (their answer is the verdict): `null`, as for any outcome a source has no
 * word for.
 */
export function verdictWord(origin: DecisionOrigin, outcome: DecisionOutcome): string | null {
  if (outcome === 'error') return 'Error';
  if (outcome === 'refused') return 'Refused';
  if (origin === 'monitor') {
    if (outcome === 'passed') return 'Fired';
    if (outcome === 'failed') return 'Quiet';
  }
  if (origin === 'gate') {
    if (outcome === 'passed') return 'Passed';
    if (outcome === 'failed') return 'Not passed';
  }
  return null;
}

/**
 * The names a monitor call asked under, from its label: the daemon joins several
 * monitors' names with ` + ` in one request's label (`PlannedRequest::label`).
 */
function monitorNames(record: DecisionRecord): string[] {
  const label = record.label?.trim();
  if (!label) return [];
  return label
    .split(' + ')
    .map((name) => name.trim())
    .filter(Boolean);
}

/** How many monitors one call asked: its questions, decisions or names, the most of them. */
function monitorCount(record: DecisionRecord): number {
  return Math.max(record.questions.length, record.decisions.length, monitorNames(record).length);
}

function isSeveralMonitors(record: DecisionRecord): boolean {
  return record.origin === 'monitor' && monitorCount(record) > 1;
}

/**
 * The record's title: a monitor by name; a call that asked several monitors
 * `2 monitors`; a gate by its phase title; a decide chunk by the chunk id the
 * agent gave (`note-30`), else `Chunk 30`; `Decision chat`; `Ad hoc call`. Never a
 * question name, batch id or turn id, and never names joined with `+`.
 */
export function recordTitle(record: DecisionRecord): string {
  const label = record.label?.trim();
  switch (record.origin) {
    case 'monitor':
      if (isSeveralMonitors(record)) return `${monitorCount(record)} monitors`;
      return label || 'Monitor';
    case 'gate':
      return label || 'Phase gate';
    case 'tool': {
      const chunk = record.chunkId?.trim();
      if (chunk) return chunk;
      if (record.chunkIndex !== null) return `Chunk ${record.chunkIndex + 1}`;
      return 'Decide call';
    }
    case 'chat':
      return 'Decision chat';
    case 'adhoc':
      return 'Ad hoc call';
  }
}

/**
 * The row's muted second line: the time; for a several-monitor call the time and
 * the names, `1:31:02 PM · Shell seen, Needs a rest`.
 */
export function recordSubtitle(record: DecisionRecord, now = new Date()): string {
  const time = formatDecisionTime(record.createdAt, now);
  if (!isSeveralMonitors(record)) return time;
  const names = monitorNames(record);
  return [time, names.join(', ')].filter(Boolean).join(' · ');
}

/** Whether a one-question monitor or gate call passed: the outcome, else its decision. */
function passedOf(record: DecisionRecord): boolean | null {
  if (record.outcome === 'passed') return true;
  if (record.outcome === 'failed') return false;
  return record.decisions[0]?.passed ?? null;
}

/**
 * The row's right side: monitor `Fired` / `Quiet`; several monitors `1 of 2 fired` /
 * `Quiet`; gate `Passed` / `Not passed`; decide, chat and ad hoc the first answer as
 * `readableAnswer` gives it, with ` +1` when more questions were asked; `Error` /
 * `Refused`.
 */
export function recordVerdict(record: DecisionRecord): string {
  if (record.outcome === 'error') return 'Error';
  if (record.outcome === 'refused') return 'Refused';
  if (record.origin === 'monitor' && isSeveralMonitors(record)) {
    const fired = record.decisions.filter((decision) => decision.passed === true).length;
    return fired > 0 ? `${fired} of ${monitorCount(record)} fired` : 'Quiet';
  }
  if (record.origin === 'monitor') return passedOf(record) ? 'Fired' : 'Quiet';
  if (record.origin === 'gate') return passedOf(record) ? 'Passed' : 'Not passed';
  const first = record.decisions[0];
  if (!first) return 'No answer';
  const question = record.questions.find((entry) => entry.name === first.question);
  const more = record.decisions.length - 1;
  return `${readableAnswer(first, question)}${more > 0 ? ` +${more}` : ''}`;
}

// ---------------------------------------------------------------------------
// Glyphs (R1: told apart by shape)
// ---------------------------------------------------------------------------

export type RecordGlyphState =
  | 'monitorFired'
  | 'monitorQuiet'
  | 'gatePassed'
  | 'gateNotPassed'
  | 'answer'
  | 'error'
  | 'refused';

/**
 * A record's glyph by SHAPE: a raised flag for a fired monitor, an open eye for a
 * quiet one (never a cross), a check and a cross for a gate, a plain ring for an
 * answer whatever a threshold said, a warning for an error and a lock for a
 * refusal. The shapes `DECISION_STATE_ICONS` in `summary/SummaryDecisions.tsx`
 * draws for the same states.
 */
export const RECORD_GLYPHS: Record<RecordGlyphState, EntityIcon> = {
  monitorFired: Flag,
  monitorQuiet: Eye,
  gatePassed: Check,
  gateNotPassed: X,
  answer: CircleIcon,
  error: AlertTriangle,
  refused: Lock,
};

/** The ink each glyph wears (a `data-tone` for the surface's CSS). */
export const RECORD_GLYPH_TONES: Record<
  RecordGlyphState,
  'accent' | 'muted' | 'success' | 'warning'
> = {
  monitorFired: 'accent',
  monitorQuiet: 'muted',
  gatePassed: 'success',
  gateNotPassed: 'accent',
  answer: 'muted',
  error: 'warning',
  refused: 'warning',
};

/** Which of {@link RECORD_GLYPHS} a record draws. */
export function recordGlyphState(record: DecisionRecord): RecordGlyphState {
  if (record.outcome === 'error') return 'error';
  if (record.outcome === 'refused') return 'refused';
  if (record.origin === 'monitor') {
    const fired = isSeveralMonitors(record)
      ? record.decisions.some((decision) => decision.passed === true)
      : passedOf(record) === true;
    return fired ? 'monitorFired' : 'monitorQuiet';
  }
  if (record.origin === 'gate') return passedOf(record) ? 'gatePassed' : 'gateNotPassed';
  return 'answer';
}

/**
 * The gate dialog's status glyphs, the same shapes: passed `Check`, not passed `X`,
 * blocked and needs you `AlertCircle`, not asked yet `CircleIcon`, skipped `ArrowRight`.
 */
export const GATE_STATE_ICONS: Record<PhaseGateState, EntityIcon> = {
  passed: Check,
  failed: X,
  blocked: AlertCircle,
  pending: CircleIcon,
  skipped: ArrowRight,
};

// ---------------------------------------------------------------------------
// The history's Source and Outcome filters (history spec, section 1.2)
// ---------------------------------------------------------------------------

export const FILTER_CHIP_COPY = {
  source: 'Source',
  outcome: 'Outcome',
  anySource: 'Any source',
  anyOutcome: 'Any outcome',
  errorOrRefused: 'Error or refused',
  /** An engaged chip: `Source: Monitors`, `Outcome: Fired`. */
  engaged: (noun: string, value: string) => `${noun}: ${value}`,
} as const;

export interface SourceFilterOption {
  /** `null` for Any source. */
  origin: DecisionOrigin | null;
  label: string;
}

/** Every Source row, in the order the chip lists them (Any source first). */
export const SOURCE_FILTER_OPTIONS: readonly SourceFilterOption[] = [
  { origin: null, label: FILTER_CHIP_COPY.anySource },
  { origin: 'monitor', label: 'Monitors' },
  { origin: 'tool', label: 'Decide calls' },
  { origin: 'gate', label: 'Phase gates' },
  { origin: 'chat', label: 'Decision chat' },
  { origin: 'adhoc', label: 'Ad hoc calls' },
];

/**
 * The Source rows a chat offers: Any source, then only the sources it has (from the
 * summary's aggregates), plus the current one when it is not among them, in
 * {@link SOURCE_FILTER_OPTIONS} order.
 */
export function sourceFilterOptionsFor(
  present: Iterable<DecisionOrigin>,
  current?: DecisionOrigin | null
): SourceFilterOption[] {
  const shown = new Set<DecisionOrigin>(present);
  if (current) shown.add(current);
  return SOURCE_FILTER_OPTIONS.filter(
    (option) => option.origin === null || shown.has(option.origin)
  );
}

export type OutcomeFilterKey = 'any' | 'passed' | 'failed' | 'errors';

export interface OutcomeFilterOption {
  key: OutcomeFilterKey;
  label: string;
  /** The filter fields this row sets; values stay the wire's. */
  filter: Pick<DecisionHistoryFilter, 'outcome' | 'errorsOnly'>;
}

const ANY_OUTCOME: OutcomeFilterOption = {
  key: 'any',
  label: FILTER_CHIP_COPY.anyOutcome,
  filter: { outcome: null, errorsOnly: false },
};
const ERRORS: OutcomeFilterOption = {
  key: 'errors',
  label: FILTER_CHIP_COPY.errorOrRefused,
  filter: { outcome: null, errorsOnly: true },
};

/**
 * The Outcome rows for the chosen source: only words the list itself shows (R1).
 * Monitors: Fired, Quiet; phase gates: Passed, Not passed; every source: Error or
 * refused. Decide calls, decision chat, ad hoc calls and Any source offer no
 * threshold word, because their rows show an answer, not a verdict.
 */
export function OUTCOME_FILTER_OPTIONS(origin: DecisionOrigin | null): OutcomeFilterOption[] {
  if (origin === 'monitor' || origin === 'gate') {
    return [
      ANY_OUTCOME,
      {
        key: 'passed',
        label: verdictWord(origin, 'passed') as string,
        filter: { outcome: 'passed', errorsOnly: false },
      },
      {
        key: 'failed',
        label: verdictWord(origin, 'failed') as string,
        filter: { outcome: 'failed', errorsOnly: false },
      },
      ERRORS,
    ];
  }
  return [ANY_OUTCOME, ERRORS];
}

/** The row key a filter's outcome fields stand for, or `null` for an outcome no row names. */
export function outcomeFilterKey(
  filter: Pick<DecisionHistoryFilter, 'outcome' | 'errorsOnly'>
): OutcomeFilterKey | null {
  if (filter.errorsOnly) return 'errors';
  if (!filter.outcome) return 'any';
  if (filter.outcome === 'passed' || filter.outcome === 'failed') return filter.outcome;
  return null;
}

/**
 * What the Outcome chip says is applied: the listed row's label when the source
 * lists it, else the outcome's own word (an outcome that arrived from a link), so a
 * chip can still say what it holds. `listed` tells the caller to add a checked row.
 */
export function outcomeFilterChoice(
  origin: DecisionOrigin | null,
  filter: Pick<DecisionHistoryFilter, 'outcome' | 'errorsOnly'>
): { key: OutcomeFilterKey | null; label: string; listed: boolean } {
  const key = outcomeFilterKey(filter);
  const listed = OUTCOME_FILTER_OPTIONS(origin).find((option) => option.key === key);
  if (listed) return { key, label: listed.label, listed: true };
  const outcome = filter.outcome as DecisionOutcome;
  return { key, label: outcomeOwnWord(origin, outcome), listed: false };
}

/** An outcome's word where its source has none listed: never "Failed" (R1). */
function outcomeOwnWord(origin: DecisionOrigin | null, outcome: DecisionOutcome): string {
  const word = origin ? verdictWord(origin, outcome) : null;
  if (word) return word;
  switch (outcome) {
    case 'passed':
      return 'Passed';
    case 'failed':
      return 'Not passed';
    case 'decided':
      return 'Answered';
    case 'error':
      return 'Error';
    case 'refused':
      return 'Refused';
  }
}

/**
 * Whether the outcome filter still has a row once the Source changes to `origin`;
 * when not, the history resets Outcome to Any outcome.
 */
export function outcomeKeptFor(
  origin: DecisionOrigin | null,
  filter: Pick<DecisionHistoryFilter, 'outcome' | 'errorsOnly'>
): boolean {
  const key = outcomeFilterKey(filter);
  return OUTCOME_FILTER_OPTIONS(origin).some((option) => option.key === key);
}
