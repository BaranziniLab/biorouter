/**
 * Folding a long line of prose into the one line a person needs and the rest.
 *
 * The owner's rule (2026-11-13): no large, solid chunks of always-visible text. A surface shows
 * its first line (the error, the refusal, the state) and the rest moves behind an InfoTip
 * (`components/ui/info-tip.tsx`, two sentences at most) or a `Disclosure` for anything longer.
 * These helpers only cut text at a sentence boundary; every word is kept, in order, between the
 * two halves, so the folded text read back is the text that came in.
 */

/**
 * Characters past which a line of text in the transcript wraps: the 760px column at the 13px
 * secondary size holds about a hundred. Text this short stays whole, because a second line is
 * not yet a chunk.
 */
export const ONE_LINE_CHARS = 100;

/** The InfoTip's ceiling (spec 2.6): two sentences, about 160 characters. */
export const INFO_TIP_MAX_CHARS = 160;

/**
 * A sentence end: a stop (with any closing quote or bracket after it), then space, then what
 * starts a sentence (a capital, a digit, or an opening quote or bracket before one). A stop with
 * no space after it ("0.80", "config.yaml") is never an end.
 */
const SENTENCE_END = /([.!?]["')\]}”’]*)\s+(?=["'([“‘]?[A-Z0-9])/g;

/** A first sentence shorter than this is an abbreviation or a label ("e.g.", "No."), not a sentence. */
const MIN_SENTENCE_CHARS = 12;

/** The text's first sentence and everything after it (empty when it is one sentence). */
export function splitFirstSentence(text: string): { first: string; rest: string } {
  const trimmed = text.trim();
  SENTENCE_END.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = SENTENCE_END.exec(trimmed)) !== null) {
    const end = match.index + match[1].length;
    if (end < MIN_SENTENCE_CHARS) continue;
    const rest = trimmed.slice(match.index + match[0].length).trim();
    if (!rest) break;
    return { first: trimmed.slice(0, end), rest };
  }
  return { first: trimmed, rest: '' };
}

/**
 * The line to show and the rest to fold: the whole text when it fits `limit`, otherwise its
 * first sentence. Text with no sentence boundary is never cut mid-sentence.
 */
export function foldProse(text: string, limit = ONE_LINE_CHARS): { lead: string; rest: string } {
  const trimmed = text.trim();
  if (trimmed.length <= limit) return { lead: trimmed, rest: '' };
  const { first, rest } = splitFirstSentence(trimmed);
  return { lead: first, rest };
}

/** How many sentences the text holds, by the same boundary `splitFirstSentence` uses. */
export function sentenceCount(text: string): number {
  let count = 0;
  let remaining = text.trim();
  while (remaining) {
    count += 1;
    remaining = splitFirstSentence(remaining).rest;
  }
  return count;
}

/** Whether the text may be an InfoTip's help: two sentences at most, about 160 characters. */
export function fitsInfoTip(text: string): boolean {
  const trimmed = text.trim();
  return trimmed.length <= INFO_TIP_MAX_CHARS && sentenceCount(trimmed) <= 2;
}

/**
 * Where each part of a folded text goes: `lead` stays on screen, `tip` (when the rest fits an
 * InfoTip) goes behind the info icon beside it, and `details` (when it does not) goes into a
 * `Disclosure`. At most one of `tip` and `details` is set.
 */
export function foldForHelp(
  text: string,
  limit = ONE_LINE_CHARS
): { lead: string; tip: string; details: string } {
  const { lead, rest } = foldProse(text, limit);
  if (!rest) return { lead, tip: '', details: '' };
  return fitsInfoTip(rest) ? { lead, tip: rest, details: '' } : { lead, tip: '', details: rest };
}
