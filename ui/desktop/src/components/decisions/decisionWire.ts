/**
 * Decision models in the transcript and the composer: what the chat's own
 * surfaces need beyond the shared decision types (`utils/sessionDecisions.ts`,
 * which mirrors the daemon's contract): which providers are decision providers,
 * the words a per-turn marker uses, the order a card draws its bars in, and the
 * batch summary a `decisions__decide` tool result carries.
 *
 * The owner rule these serve (decision-model PR, addendum A1): decision calls can
 * number in the thousands, so the transcript never draws one row per call. A
 * decision chat's reply is one card, a turn's monitor and tool calls are one
 * marker, and a `decide` batch is one tool row. The decision history view (the
 * artifact panel's `decisions` kind, `decisionHistoryArtifact`) is the only place
 * single calls are listed.
 */
import {
  probability,
  type Decision,
  type DecisionModelRef,
  type DecisionTurnMarker,
  type DecisionTurnMarkerItem,
  type Question,
} from '../../utils/sessionDecisions';

function record(value: unknown): Record<string, unknown> | null {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

const text = (value: unknown): string | null =>
  typeof value === 'string' && value.trim() ? value.trim() : null;

const finite = (value: unknown): number | null =>
  typeof value === 'number' && Number.isFinite(value) ? value : null;

const count = (value: unknown): number => {
  const n = finite(value);
  return n !== null && n >= 0 ? Math.floor(n) : 0;
};

// ---------------------------------------------------------------------------
// Decision providers
// ---------------------------------------------------------------------------

/**
 * The providers whose models are decision models: exactly
 * `DECISION_CHAT_PROVIDERS` in the core crate's `providers/decision/chat.rs`,
 * which a Rust test reads this list against. A chat bound to one has no LLM
 * loop: each message is the state, the chat's question is asked of it, and
 * the reply is one decision card.
 */
export const DECISION_PROVIDER_IDS: ReadonlySet<string> = new Set([
  'baseten_decision',
  'ollama_decision',
]);

export function isDecisionProvider(provider: string | null | undefined): boolean {
  return !!provider && DECISION_PROVIDER_IDS.has(provider);
}

/**
 * The decision provider with no meter: Ollama bills nothing per token, so an
 * unpriced request there costs nothing. An unpriced Baseten request was billed
 * at a rate Biorouter does not know. `is_unmetered_decision_provider` in
 * `providers/decision/chat.rs` makes the same call for the usage report.
 */
export function isUnmeteredDecisionProvider(provider: string | null | undefined): boolean {
  return provider === 'ollama_decision';
}

// ---------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------

/** `1 chunk`, `600 chunks`. */
export function pluralize(n: number, one: string, many = `${one}s`): string {
  return `${n.toLocaleString('en-US')} ${n === 1 ? one : many}`;
}

/** `once`, `twice`, then `3 times`. */
export function times(n: number): string {
  if (n === 1) return 'once';
  if (n === 2) return 'twice';
  return `${n.toLocaleString('en-US')} times`;
}

/** `Shell seen`, `Shell seen and Needs a rest`, `A, B and C`: never names joined with `+`. */
function joinNames(names: string[]): string {
  if (names.length <= 1) return names[0] ?? '';
  return `${names.slice(0, -1).join(', ')} and ${names[names.length - 1]}`;
}

/** The names one marker item stands for (the daemon joins a several-monitor call's with ` + `). */
function itemNames(item: DecisionTurnMarkerItem): string[] {
  return item.label
    .split(' + ')
    .map((name) => name.trim())
    .filter(Boolean);
}

type MarkerKind = 'monitor' | 'tool' | 'gate' | 'chat' | 'adhoc';

/** Ties keep this order: monitors, decide calls, gates, then the rest. */
const KIND_ORDER: Record<MarkerKind, number> = { monitor: 0, tool: 1, gate: 2, chat: 3, adhoc: 4 };

/**
 * How serious what a part or line holds is, most serious first: errors, then a gate
 * that did not pass, then a monitor that fired, then everything else.
 */
function rankOf(kind: MarkerKind, counts: { passed: number; failed: number; errors: number }) {
  if (counts.errors > 0) return 0;
  if (kind === 'gate' && counts.failed > 0) return 1;
  if (kind === 'monitor' && counts.passed > 0) return 2;
  return 3;
}

const withErrors = (text: string, errors: number) =>
  errors > 0 ? `${text}, ${pluralize(errors, 'error')}` : text;

/** A gate's outcome words: `passed`, `not passed`, `passed twice, 1 not passed`. */
function gateOutcome(passed: number, failed: number, several: boolean): string | null {
  if (!several) return passed > 0 ? 'passed' : failed > 0 ? 'not passed' : null;
  const parts: string[] = [];
  if (passed > 0) parts.push(`passed ${times(passed)}`);
  if (failed > 0) parts.push(`${failed.toLocaleString('en-US')} not passed`);
  return parts.length > 0 ? parts.join(', ') : null;
}

export interface MarkerPart {
  kind: MarkerKind;
  text: string;
  /** 0 errors, 1 a gate not passed, 2 a monitor that fired, 3 the rest. */
  rank: number;
}

/**
 * The closed marker line's parts, one per kind of source, counts first and most
 * important first (F-19): `Monitors checked 10 times, none fired`, `1 decide call read
 * 30 chunks`, `Gate checked twice, not passed`, `Decision chat answered 3 times`,
 * `3 ad hoc calls`, each ending `, 2 errors` when it had any. No probability: an
 * item's last probability does not say which outcome it is (R2). A surface shows the
 * first {@link MARKER_PARTS_SHOWN} and then `+N more`, so a not-passed gate or an
 * error is never the part that drops.
 */
export function markerParts(marker: DecisionTurnMarker): MarkerPart[] {
  const groups = new Map<MarkerKind, DecisionTurnMarkerItem[]>();
  for (const item of marker.items) {
    const list = groups.get(item.origin) ?? [];
    list.push(item);
    groups.set(item.origin, list);
  }
  const parts: MarkerPart[] = [];
  for (const [kind, items] of groups) {
    const sum = (key: 'count' | 'passed' | 'failed' | 'errors') =>
      items.reduce((total, item) => total + item[key], 0);
    const counts = { passed: sum('passed'), failed: sum('failed'), errors: sum('errors') };
    const checks = sum('count');
    let text: string;
    switch (kind) {
      case 'monitor': {
        const monitors = items.reduce(
          (total, item) => total + Math.max(1, itemNames(item).length),
          0
        );
        if (monitors > 1) {
          text = `Monitors checked ${times(checks)}, ${
            counts.passed > 0 ? `fired ${times(counts.passed)}` : 'none fired'
          }`;
        } else {
          text = `Monitor checked ${times(checks)}, ${
            counts.passed > 0 ? `fired ${times(counts.passed)}` : 'quiet'
          }`;
        }
        break;
      }
      case 'tool':
        // R3-F3: an item is a question, not a call; the daemon says how many calls
        // made it (an older daemon sent an item per call, so 1 each).
        text = `${pluralize(
          items.reduce((total, item) => total + (item.calls ?? 1), 0),
          'decide call'
        )} read ${pluralize(checks, 'chunk')}`;
        break;
      case 'gate': {
        const several = items.length > 1;
        const outcome = gateOutcome(counts.passed, counts.failed, several);
        text = `${several ? 'Gates' : 'Gate'} checked ${times(checks)}${outcome ? `, ${outcome}` : ''}`;
        break;
      }
      case 'chat':
        text = `Decision chat answered ${times(checks)}`;
        break;
      case 'adhoc':
        text = pluralize(checks, 'ad hoc call');
        break;
    }
    parts.push({ kind, text: withErrors(text, counts.errors), rank: rankOf(kind, counts) });
  }
  return parts.sort((a, b) => a.rank - b.rank || KIND_ORDER[a.kind] - KIND_ORDER[b.kind]);
}

/** How many parts the closed line shows before `+N more`. */
export const MARKER_PARTS_SHOWN = 2;

/**
 * One source's line in the open marker, by its display name: `Done with shell:
 * checked 3 times, fired twice`, `Probe write test: checked twice, not passed`,
 * `Decide call: 30 chunks`, `Decision chat: answered 3 times`, `Ad hoc calls: 3`.
 */
export function markerSourceLine(item: DecisionTurnMarkerItem): string {
  const name = joinNames(itemNames(item));
  let text: string;
  switch (item.origin) {
    case 'monitor':
      text = `${name || 'Monitor'}: checked ${times(item.count)}, ${
        item.passed > 0 ? `fired ${times(item.passed)}` : 'quiet'
      }`;
      break;
    case 'gate': {
      const outcome = gateOutcome(item.passed, item.failed, false);
      text = `${name || 'Phase gate'}: checked ${times(item.count)}${outcome ? `, ${outcome}` : ''}`;
      break;
    }
    case 'tool':
      text = `Decide call: ${pluralize(item.count, 'chunk')}`;
      break;
    case 'chat':
      text = `Decision chat: answered ${times(item.count)}`;
      break;
    case 'adhoc':
      text = `Ad hoc calls: ${item.count.toLocaleString('en-US')}`;
      break;
  }
  return withErrors(text, item.errors);
}

/** The marker's items in the closed line's importance order (ties keep their order). */
export function markerItemsByImportance(marker: DecisionTurnMarker): DecisionTurnMarkerItem[] {
  return marker.items
    .map((item, index) => ({ item, index }))
    .sort(
      (a, b) =>
        rankOf(a.item.origin, a.item) - rankOf(b.item.origin, b.item) ||
        KIND_ORDER[a.item.origin] - KIND_ORDER[b.item.origin] ||
        a.index - b.index
    )
    .map(({ item }) => item);
}

/**
 * The marker's status, first that applies: a decision call failed, a phase gate did
 * not pass, a monitor fired. `null` otherwise.
 */
export function markerStatus(marker: DecisionTurnMarker): 'error' | 'gate' | 'fired' | null {
  if (marker.items.some((item) => item.errors > 0)) return 'error';
  if (marker.items.some((item) => item.origin === 'gate' && item.failed > 0)) return 'gate';
  if (marker.items.some((item) => item.fired || (item.origin === 'monitor' && item.passed > 0)))
    return 'fired';
  return null;
}

/**
 * A card's bars in the order the question lists its outcomes (yes before no,
 * options as the question lists them, levels lowest first), then any key the
 * question does not name, so the bars never follow the order the probabilities
 * happened to arrive in. (An option NAMED like an integer cannot keep its place
 * through JSON at all: an object puts integer-like keys first, in the question
 * as much as in the probabilities.)
 */
export function orderedProbabilities(
  decision: Decision,
  question: Question | undefined
): Array<{ key: string; p: number }> {
  const byKey = new Map(decision.probabilities.map((entry) => [entry.key, entry.p]));
  const wanted =
    question?.kind.type === 'choice'
      ? question.kind.options.map((option) => option.name)
      : question?.kind.type === 'score'
        ? question.kind.levels.map((_, index) => String(index))
        : ['yes', 'no'];
  const ordered: Array<{ key: string; p: number }> = [];
  for (const key of wanted) {
    const p = byKey.get(key);
    if (p !== undefined) {
      ordered.push({ key, p });
      byKey.delete(key);
    }
  }
  return [...ordered, ...[...byKey].map(([key, p]) => ({ key, p }))];
}

// ---------------------------------------------------------------------------
// The `decisions__decide` tool
// ---------------------------------------------------------------------------

export const DECIDE_TOOL_NAME = 'decisions__decide';

/**
 * One question's aggregate over a batch: `QuestionSummary` in
 * `crates/biorouter/src/agents/decisions_extension.rs`, as far as a tool row draws it.
 */
export interface DecideQuestionSummary {
  name: string;
  /** `yes_no`, `choice` or `score`. */
  kind: string;
  threshold: number | null;
  /** Chunks that answered this question. */
  decided: number | null;
  /** Chunks that passed the threshold, when there is one. */
  passed: number | null;
  /** yes_no only: chunks with P(yes) of at least one half. */
  yes: number | null;
  /**
   * Chunks to read before their answer is reported: P of the pass answer from
   * 0.2 up to the larger of the threshold and 0.8, so not a clear no and not a
   * clear pass. Counted for yes_no, and for a choice or score with a threshold;
   * `null` when the rule does not apply.
   */
  review: number | null;
  /** The highest pass probability. */
  maxP: number | null;
  /** choice and score: how many chunks gave each answer, in option order. */
  answerCounts: Array<{ answer: string; count: number }>;
  /**
   * G-3: the answered chunks in four groups that do not overlap and add up to
   * `decided` (`AnswerGroups`), for a question with a pass. `null` from an older
   * daemon, or for a choice or score with no threshold.
   */
  groups: DecideAnswerGroups | null;
  /** A vote of several models (`decide` with `models`): the majority and the agreement. */
  vote: DecideQuestionVote | null;
}

/** `AnswerGroups`: a yes is a pass, a no everything else; review is both middles. */
export interface DecideAnswerGroups {
  yesConfident: number;
  yesReview: number;
  noReview: number;
  noConfident: number;
}

/** `VoteQuestion`, as far as a row draws it. */
export interface DecideQuestionVote {
  /** How many chunks have each answer as their majority, in the daemon's order. */
  majority: Array<{ answer: string; count: number }>;
  /** Chunks whose answers tied. */
  ties: number;
  /** Chunks at least two models answered. */
  compared: number;
  /** Of those, chunks every answering model gave the same answer. */
  agreed: number;
}

/** One model of a vote (`VoteModel`). */
export interface DecideVoteModel {
  /** The name a person reads (an alias for a Baseten model). */
  label: string;
  decided: number;
  errors: number;
  /** Why this model answered nothing, when it did not. */
  error: string | null;
}

/** One distinct error in a batch, and how many chunks it hit (never which ones here). */
export interface DecideErrorGroup {
  message: string;
  chunks: number;
}

/**
 * What one `decide` call did (`DecideSummary`, the call's MCP `structuredContent`):
 * counts, never chunks.
 */
export interface DecideBatchSummary {
  batchId: string;
  chunks: number | null;
  decided: number | null;
  errors: number;
  model: Pick<DecisionModelRef, 'provider' | 'model'> | null;
  tier: 'public' | 'private' | null;
  calibrated: boolean | null;
  elapsedMs: number | null;
  questions: DecideQuestionSummary[];
  errorGroups: DecideErrorGroup[];
  /** The models of a vote, in the order they were asked; `null` for one model's batch. */
  voteModels: DecideVoteModel[] | null;
}

const optionalCount = (value: unknown): number | null => {
  const n = finite(value);
  return n !== null && n >= 0 ? Math.floor(n) : null;
};

/** An ordered `{answer: count}` object as a list, dropping what is not a count. */
function countsOf(value: unknown): Array<{ answer: string; count: number }> {
  const counts = record(value);
  return counts
    ? Object.entries(counts).flatMap(([answer, n]) => {
        const parsed = optionalCount(n);
        return parsed === null ? [] : [{ answer, count: parsed }];
      })
    : [];
}

function groupsOf(value: unknown): DecideAnswerGroups | null {
  const groups = record(value);
  if (!groups) return null;
  const yesConfident = optionalCount(groups.yesConfident);
  const yesReview = optionalCount(groups.yesReview);
  const noReview = optionalCount(groups.noReview);
  const noConfident = optionalCount(groups.noConfident);
  return yesConfident === null || yesReview === null || noReview === null || noConfident === null
    ? null
    : { yesConfident, yesReview, noReview, noConfident };
}

function voteOf(question: Record<string, unknown>): DecideQuestionVote | null {
  if (!record(question.majority)) return null;
  return {
    majority: countsOf(question.majority),
    ties: count(question.ties),
    compared: count(question.compared),
    agreed: count(question.agreed),
  };
}

function questionSummaryOf(value: unknown): DecideQuestionSummary | null {
  const question = record(value);
  const name = text(question?.name);
  if (!question || !name) return null;
  return {
    name,
    kind: text(question.type) ?? 'yes_no',
    threshold: probability(question.threshold),
    decided: optionalCount(question.decided),
    passed: optionalCount(question.passed),
    yes: optionalCount(question.yes),
    review: optionalCount(question.review),
    maxP: probability(question.maxP),
    answerCounts: countsOf(question.answerCounts),
    groups: groupsOf(question.groups),
    vote: voteOf(question),
  };
}

function voteModelsOf(value: unknown): DecideVoteModel[] {
  return Array.isArray(value)
    ? value.flatMap((entry): DecideVoteModel[] => {
        const model = record(entry);
        if (!model) return [];
        const ref = record(model.model);
        const label = text(model.label) ?? text(ref?.model);
        if (!label) return [];
        return [
          {
            label,
            decided: count(model.decided),
            errors: count(model.errors),
            error: text(model.error),
          },
        ];
      })
    : [];
}

function modelOf(value: unknown): DecideBatchSummary['model'] {
  const ref = record(value);
  const provider = text(ref?.provider);
  const model = text(ref?.model);
  return provider && model ? { provider, model } : null;
}

function batchSummaryOf(value: unknown): DecideBatchSummary | null {
  const summary = record(value);
  const batchId = text(summary?.batchId);
  if (!summary || !batchId) return null;
  const questions = Array.isArray(summary.questions)
    ? summary.questions.flatMap((entry) => {
        const parsed = questionSummaryOf(entry);
        return parsed ? [parsed] : [];
      })
    : [];
  const errorGroups = Array.isArray(summary.errorGroups)
    ? summary.errorGroups.flatMap((entry): DecideErrorGroup[] => {
        const group = record(entry);
        const message = text(group?.message);
        if (!group || !message) return [];
        const listed = Array.isArray(group.chunkIds) ? group.chunkIds.length : 0;
        return [{ message, chunks: listed + count(group.more) }];
      })
    : [];
  // G-1: a vote of several models (`VoteSummary`, kind `decide_vote`) has no one model.
  const vote = summary.kind === 'decide_vote' || Array.isArray(summary.models);
  return {
    batchId,
    chunks: optionalCount(summary.chunks),
    decided: optionalCount(summary.decided),
    errors: count(summary.errors),
    model: modelOf(summary.model),
    tier: summary.tier === 'public' || summary.tier === 'private' ? summary.tier : null,
    calibrated: typeof summary.calibrated === 'boolean' ? summary.calibrated : null,
    elapsedMs: optionalCount(summary.elapsedMs),
    questions,
    errorGroups,
    voteModels: vote ? voteModelsOf(summary.models) : null,
  };
}

/** A JSON object written as a whole text block, or inside its one fenced block. */
function jsonIn(body: string): unknown {
  const trimmed = body.trim();
  const fenced = /```(?:json)?\s*([\s\S]*?)```/.exec(trimmed);
  for (const candidate of [trimmed, fenced?.[1]?.trim()]) {
    if (!candidate || !candidate.startsWith('{')) continue;
    try {
      return JSON.parse(candidate);
    } catch {
      // not JSON; try the next shape
    }
  }
  return undefined;
}

/**
 * The batch summary a `decide` result carries: its MCP `structuredContent`
 * (`DecideSummary`), else a text block that is that summary as JSON. `null` when
 * neither is there; the row then counts what it can from the call's own
 * arguments. The result's prose, which lists chunk ids for the model, is never
 * read for the row.
 */
export function parseDecideBatchSummary(toolResult: unknown): DecideBatchSummary | null {
  let result = record(toolResult);
  if (result?.status === 'error') return null;
  if (result?.status === 'success') result = record(result.value);
  if (!result) return null;
  const structured = batchSummaryOf(result.structuredContent);
  if (structured) return structured;
  const content = Array.isArray(result.content) ? result.content : [];
  for (const item of content) {
    const block = record(item);
    if (block?.type !== 'text' || typeof block.text !== 'string') continue;
    const json = jsonIn(block.text);
    const parsed = json === undefined ? null : batchSummaryOf(json);
    if (parsed) return parsed;
  }
  return null;
}

const BINARY_TYPE_WORDS = new Set(['yes_no', 'yesno', 'binary', 'boolean', 'bool', 'noul']);
const CHOICE_TYPE_WORDS = new Set(['choice', 'multiple_choice', 'multi_choice', 'options']);
const SCORE_TYPE_WORDS = new Set(['score', 'rating', 'scale', 'levels']);

/**
 * The type a model-written question names, or the one its fields imply, by the
 * tool's own rule (`question_type` in `agents/decisions_extension.rs`): the word
 * trimmed, lowercased and with spaces, dashes and slashes read as underscores,
 * its synonyms accepted, and with no word, `options` meaning a choice and
 * `levels` a score. A word the tool refuses is returned as it was written.
 */
export function decideQuestionKind(question: Record<string, unknown> | null): string {
  const written = typeof question?.type === 'string' ? question.type.trim() : '';
  const word = written.toLowerCase().replace(/[ \-/]/g, '_');
  if (!word) {
    if (question?.options != null) return 'choice';
    if (question?.levels != null) return 'score';
    return 'yes_no';
  }
  if (BINARY_TYPE_WORDS.has(word)) return 'yes_no';
  if (CHOICE_TYPE_WORDS.has(word)) return 'choice';
  if (SCORE_TYPE_WORDS.has(word)) return 'score';
  return written;
}

/** What a `decide` call's own arguments state: how many chunks, and its questions. */
export function decideArguments(args: Record<string, unknown> | undefined): {
  chunks: number | null;
  questions: Array<{ name: string; instructions: string; kind: string; threshold: number | null }>;
} {
  const chunks = Array.isArray(args?.chunks) ? args.chunks.length : null;
  const raw = record(args?.questions);
  const questions = raw
    ? Object.entries(raw).map(([name, value]) => {
        const question = record(value);
        // `question` is the tool's other name for `instructions` (a serde alias).
        const instructions =
          typeof question?.instructions === 'string'
            ? question.instructions
            : typeof question?.question === 'string'
              ? question.question
              : '';
        return {
          name,
          instructions,
          kind: decideQuestionKind(question),
          threshold: probability(question?.threshold),
        };
      })
    : [];
  return { chunks, questions };
}
