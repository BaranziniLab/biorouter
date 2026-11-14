import { postDictionaryObservations } from '../api';
import { userActionHeaders } from './userAction';
/**
 * Where dictated text sits in a composer while the person edits around it, and
 * what their edits to it were (DD-58, `docs/dictation/dictionary.md`, "Desktop
 * and `serve` ledger").
 *
 * Pure functions over strings, no React and no DOM: the composer maps each
 * dictated span through every change of its prose with {@link mapDictatedSpan},
 * and {@link correctionCandidates} reads an edited span once, when the message
 * is sent. The same single region diff, {@link locateEdit}, also moves the
 * insertion anchor of a dictation that is still recording
 * (`utils/dictatedText.ts` `mapAnchor`).
 *
 * The ledger lives in renderer memory only. Nothing here is persisted, logged or
 * attached to a message, and the only thing that ever leaves the composer is a
 * short `from`/`to` pair, which the daemon's `recognize/learn.rs` checks again.
 */

/** One region of change between two versions of a text. */
export interface TextEdit {
  /** Where the change starts, the same in both versions. */
  start: number;
  /** Where the changed region ends in the text before the edit. */
  endBefore: number;
  /** Where it ends in the text after the edit. */
  endAfter: number;
}

/**
 * The one region that differs between `before` and `after`: the longest common
 * prefix and suffix are cut off, and whatever is left in the middle is the edit.
 *
 * A change inside a run of repeated characters can be placed at several
 * positions ("aa" to "aaa" may be an insertion at 0, 1 or 2). `caretAfter`, the
 * caret position once the edit happened, picks the one the person made: the
 * region is moved left, while that keeps both texts equal outside it, until it
 * ends at the caret. Without a hint the region sits as far right as it can.
 *
 * `null` when the two texts are equal.
 */
export function locateEdit(before: string, after: string, caretAfter?: number): TextEdit | null {
  if (before === after) return null;
  const shortest = Math.min(before.length, after.length);
  let prefix = 0;
  while (prefix < shortest && before.charCodeAt(prefix) === after.charCodeAt(prefix)) prefix++;
  let suffix = 0;
  while (
    suffix < shortest - prefix &&
    before.charCodeAt(before.length - 1 - suffix) === after.charCodeAt(after.length - 1 - suffix)
  ) {
    suffix++;
  }
  let start = prefix;
  let endBefore = before.length - suffix;
  let endAfter = after.length - suffix;

  if (caretAfter !== undefined && caretAfter >= 0 && caretAfter < endAfter) {
    const shift = Math.min(endAfter - caretAfter, start);
    if (
      shift > 0 &&
      before.slice(endBefore - shift, endBefore) === before.slice(start - shift, start) &&
      after.slice(endAfter - shift, endAfter) === after.slice(start - shift, start)
    ) {
      start -= shift;
      endBefore -= shift;
      endAfter -= shift;
    }
  }
  return { start, endBefore, endAfter };
}

/** How a position moves when an edit lands exactly on it. */
export type Assoc = 'before' | 'after';

/**
 * A position in the text before `edit`, mapped into the text after it. A
 * position inside the replaced region goes to the region's start (`before`) or
 * end (`after`); a pure insertion exactly at the position moves it only with
 * `after`.
 */
export function mapPosition(position: number, edit: TextEdit, assoc: Assoc): number {
  const delta = edit.endAfter - edit.endBefore;
  if (position < edit.start) return position;
  if (position > edit.endBefore) return position + delta;
  if (edit.start === edit.endBefore) {
    // A pure insertion at `position`.
    return assoc === 'after' ? edit.endAfter : position;
  }
  if (position === edit.start) return position;
  if (position === edit.endBefore) return edit.endAfter;
  return assoc === 'after' ? edit.endAfter : edit.start;
}

/** The transcript's record of what correction changed, as the daemon reports it. */
export interface TranscriptChange {
  start: number;
  end: number;
  from: string;
  to: string;
  stage: string;
  entry_id?: string;
}

export type DictatedSpanState = 'pristine' | 'edited' | 'tainted' | 'undone';

/**
 * One dictation's text in a composer. `start` and `end` follow the prose as it
 * changes; `current` is the text now between them.
 */
export interface DictatedSpan {
  /** The stream id of the dictation. */
  recordId: string;
  start: number;
  end: number;
  current: string;
  /** The transcript's `raw_text`. */
  raw: string;
  /** The text that was inserted, with the spaces the splice added. */
  inserted: string;
  /** What the selection held before the insert replaced it ('' for a caret). */
  replaced: string;
  changes: TranscriptChange[];
  state: DictatedSpanState;
  model: string;
  /** The dictionary listing's ETag at insert time; opaque, never a store wide counter. */
  dictionaryRevision: string;
  /** For the private draft flag (DD-71). */
  biasScope: 'public' | 'private';
}

/** A composer keeps at most this many spans; the oldest goes first. */
export const MAX_DICTATED_SPANS = 16;

/** Add a span, keeping at most {@link MAX_DICTATED_SPANS}. */
export function recordDictatedSpan(
  spans: readonly DictatedSpan[],
  span: DictatedSpan
): DictatedSpan[] {
  const next = [...spans, span];
  return next.length > MAX_DICTATED_SPANS ? next.slice(next.length - MAX_DICTATED_SPANS) : next;
}

/**
 * Move a span through one change of the prose.
 *
 * - A change outside the span shifts it.
 * - A change inside it (or touching only its edges from inside) marks it
 *   `edited`, and `current` is read from the new prose.
 * - A change that covers the whole span and more, or straddles one of its
 *   edges, cannot be told apart from a programmatic replacement, so the span is
 *   `tainted` and nothing is learned from it.
 * - A change that puts back exactly what the insert replaced, where the span
 *   was, is an undo: the span is `undone` and collapses.
 *
 * `tainted` and `undone` are final.
 */
export function mapDictatedSpan(
  span: DictatedSpan,
  before: string,
  after: string,
  caretAfter?: number
): DictatedSpan {
  if (span.state === 'tainted' || span.state === 'undone') return span;
  const edit = locateEdit(before, after, caretAfter);
  if (!edit) return span;

  // An undo puts back exactly what the insert replaced, wherever the diff
  // happens to place the change inside a run of equal characters.
  if (
    span.current === span.inserted &&
    after === before.slice(0, span.start) + span.replaced + before.slice(span.end)
  ) {
    return {
      ...span,
      end: span.start + span.replaced.length,
      current: span.replaced,
      state: 'undone',
    };
  }

  const touchesInside =
    edit.start < span.end && edit.endBefore > span.start
      ? true
      : edit.start === edit.endBefore && edit.start > span.start && edit.start < span.end;

  if (!touchesInside) {
    // Wholly before or after the span; an insertion exactly at an edge stays outside.
    const start = mapPosition(span.start, edit, 'after');
    const end = mapPosition(span.end, edit, 'before');
    return { ...span, start, end: Math.max(start, end) };
  }

  if (edit.start < span.start || edit.endBefore > span.end) {
    // Covers the span and more, or straddles an edge: part of the change is the
    // span's and part is not, as with a programmatic replacement of the prose.
    return { ...span, state: 'tainted' };
  }

  const end = span.end + (edit.endAfter - edit.endBefore);
  const current = after.slice(span.start, end);
  return {
    ...span,
    end,
    current,
    state: current === span.inserted ? span.state : 'edited',
  };
}

/** Map every span through one change of the prose. */
export function mapDictatedSpans(
  spans: readonly DictatedSpan[],
  before: string,
  after: string,
  caretAfter?: number
): DictatedSpan[] {
  return spans.map((span) => mapDictatedSpan(span, before, after, caretAfter));
}

/** A pair the composer may offer the daemon when the message is sent. */
export interface CorrectionObservation {
  record_id: string;
  kind: 'correction' | 'rejected';
  from: string;
  to: string;
  entry_id?: string;
}

const words = (text: string): string[] =>
  text
    .split(/\s+/)
    .map((word) => word.replace(/^[^\p{L}\p{N},]+|[^\p{L}\p{N},]+$/gu, ''))
    .filter(Boolean);

/**
 * The longest common subsequence of two word lists, as index pairs.
 * Dictations are short, so the quadratic table is fine; a list longer than
 * 400 words is not aligned at all and yields no pairs.
 */
function alignWords(a: readonly string[], b: readonly string[]): Array<[number, number]> | null {
  if (a.length > 400 || b.length > 400) return null;
  const table: number[][] = Array.from({ length: a.length + 1 }, () =>
    new Array<number>(b.length + 1).fill(0)
  );
  for (let i = a.length - 1; i >= 0; i--) {
    for (let j = b.length - 1; j >= 0; j--) {
      table[i][j] =
        a[i] === b[j] ? table[i + 1][j + 1] + 1 : Math.max(table[i + 1][j], table[i][j + 1]);
    }
  }
  const pairs: Array<[number, number]> = [];
  let i = 0;
  let j = 0;
  while (i < a.length && j < b.length) {
    if (a[i] === b[j]) {
      pairs.push([i, j]);
      i++;
      j++;
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      i++;
    } else {
      j++;
    }
  }
  return pairs;
}

/** Cheap first checks; `recognize/learn.rs` is the authority and checks every pair again. */
const PAIR_MAX_WORDS = 3;
const PAIR_MAX_CHARS = 64;
const pairSizeFits = (text: string): boolean =>
  text.length > 0 &&
  text.length <= PAIR_MAX_CHARS &&
  !/[\r\n]/.test(text) &&
  words(text).length <= PAIR_MAX_WORDS;

/** At most this many pairs leave one message. */
export const MAX_PAIRS_PER_MESSAGE = 3;

/**
 * The correction pairs one edited span offers: each run of words the person
 * replaced, aligned by a word level LCS between what was inserted and what the
 * span holds now. A span whose edit is a rewrite, more than half of its words
 * changed, offers nothing. A replaced run that puts back the `from` of one of
 * the transcript's own changes is a `rejected` pair for that change.
 */
export function correctionCandidates(span: DictatedSpan): CorrectionObservation[] {
  if (span.state !== 'edited') return [];
  const inserted = words(span.inserted);
  const current = words(span.current);
  if (inserted.length === 0) return [];
  const pairs = alignWords(inserted, current);
  if (!pairs) return [];
  const unchanged = pairs.length;
  const rewritten = inserted.length - unchanged > inserted.length / 2;

  const out: CorrectionObservation[] = [];
  let i = 0;
  let j = 0;
  for (const [pi, pj] of [...pairs, [inserted.length, current.length] as [number, number]]) {
    if (pi > i && pj > j) {
      const from = inserted.slice(i, pi).join(' ');
      const to = current.slice(j, pj).join(' ');
      const reverted = span.changes.find((change) => change.to === from && change.from === to);
      if (reverted && pairSizeFits(reverted.from) && pairSizeFits(reverted.to)) {
        out.push({
          record_id: span.recordId,
          kind: 'rejected',
          from: reverted.from,
          to: reverted.to,
          ...(reverted.entry_id ? { entry_id: reverted.entry_id } : {}),
        });
      } else if (!rewritten && pairSizeFits(from) && pairSizeFits(to) && !from.includes(',')) {
        out.push({ record_id: span.recordId, kind: 'correction', from, to });
      }
    }
    i = pi + 1;
    j = pj + 1;
  }
  return out.slice(0, MAX_PAIRS_PER_MESSAGE);
}

/** Raw spans follow queue objects only in memory; they never become message fields. */
const queuedSpans = new WeakMap<object, DictatedSpan[]>();

export function keepQueuedDictation(message: object, spans: DictatedSpan[]): void {
  if (spans.length) queuedSpans.set(message, spans);
}

export function readQueuedDictation(message: object): DictatedSpan[] {
  return queuedSpans.get(message) ?? [];
}

/** Only an actual field edit can teach; other replacements invalidate intersected spans. */
export function mapComposerDictation(
  spans: readonly DictatedSpan[],
  before: string,
  after: string,
  userEdit: boolean,
  caret?: number
): DictatedSpan[] {
  const mapped = mapDictatedSpans(spans, before, after, caret);
  if (userEdit || before === after) return mapped;
  const edit = locateEdit(before, after);
  if (!edit) return mapped;
  return mapped.map((span, i) => {
    const old = spans[i];
    const touches = edit.start < old.end && edit.endBefore > old.start;
    const insertsInside =
      edit.start === edit.endBefore && edit.start > old.start && edit.start < old.end;
    return touches || insertsInside ? { ...span, state: 'tainted' } : span;
  });
}

export function observationsFromSpans(spans: readonly DictatedSpan[]): CorrectionObservation[] {
  const seen = new Set<string>();
  return spans
    .flatMap(correctionCandidates)
    .filter((pair) => {
      const key = JSON.stringify([pair.record_id, pair.from, pair.to]);
      if (seen.has(key)) return false;
      seen.add(key);
      return true;
    })
    .slice(0, MAX_PAIRS_PER_MESSAGE);
}

/** A failed observation is silent and cannot change whether the send was accepted. */
export function postCorrectionObservations(
  spans: readonly DictatedSpan[],
  sessionId?: string
): void {
  void Promise.resolve()
    .then(async () => {
      const observations = observationsFromSpans(spans);
      if (observations.length === 0) return;
      await postDictionaryObservations({
        body: { session_id: sessionId ?? null, observations },
        headers: await userActionHeaders(),
      });
    })
    .catch(() => {});
}
