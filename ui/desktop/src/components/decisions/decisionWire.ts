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
  formatProbability,
  probability,
  type Decision,
  type DecisionModelRef,
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

/** A source of decision calls, in the words a marker line uses. */
export function originWord(origin: DecisionTurnMarkerItem['origin']): string {
  switch (origin) {
    case 'monitor':
      return 'monitor';
    case 'tool':
      return 'decide';
    case 'gate':
      return 'gate';
    case 'chat':
      return 'decision chat';
    case 'adhoc':
      return 'ad hoc';
  }
}

/**
 * One origin's line in a turn marker: "monitor Form reached 18 checks, last P
 * 0.31", "decide 600 chunks, 41 passed, 2 errors", "gate Tests pass 2 checks, 1
 * passed". Counts only, never the calls themselves.
 */
export function markerItemText(item: DecisionTurnMarkerItem): string {
  const name = item.label && item.origin !== 'tool' ? ` ${item.label}` : '';
  const head = `${originWord(item.origin)}${name}`;
  const parts = [
    item.origin === 'tool' ? pluralize(item.count, 'chunk') : pluralize(item.count, 'check'),
  ];
  if (item.origin === 'monitor') {
    if (item.lastPassProbability !== null) {
      parts.push(`last P ${formatProbability(item.lastPassProbability)}`);
    }
  } else if (item.passed > 0 || item.failed > 0) {
    parts.push(`${item.passed.toLocaleString('en-US')} passed`);
  }
  if (item.errors > 0) parts.push(pluralize(item.errors, 'error'));
  if (item.fired) parts.push('fired');
  return `${head} ${parts.join(', ')}`;
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
}

const optionalCount = (value: unknown): number | null => {
  const n = finite(value);
  return n !== null && n >= 0 ? Math.floor(n) : null;
};

function questionSummaryOf(value: unknown): DecideQuestionSummary | null {
  const question = record(value);
  const name = text(question?.name);
  if (!question || !name) return null;
  const counts = record(question.answerCounts);
  return {
    name,
    kind: text(question.type) ?? 'yes_no',
    threshold: probability(question.threshold),
    decided: optionalCount(question.decided),
    passed: optionalCount(question.passed),
    yes: optionalCount(question.yes),
    review: optionalCount(question.review),
    maxP: probability(question.maxP),
    answerCounts: counts
      ? Object.entries(counts).flatMap(([answer, n]) => {
          const parsed = optionalCount(n);
          return parsed === null ? [] : [{ answer, count: parsed }];
        })
      : [],
  };
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
        return {
          name,
          instructions: typeof question?.instructions === 'string' ? question.instructions : '',
          kind: typeof question?.type === 'string' ? question.type : 'yes_no',
          threshold: probability(question?.threshold),
        };
      })
    : [];
  return { chunks, questions };
}
