/**
 * The **History** tab's rows, free of React and the DOM ("Rows" in `history-design.md` 10.2):
 * the date buckets, each row's muted line and its expanded line, and the sentences a row's
 * controls show when the daemon refuses them before the click.
 */
import type {
  DictationHistoryEntryView,
  DictationHistorySettings,
  DictationHistorySurface,
} from '../../../api';
import { chatDateBucket, type ChatDateBucket } from '../../../utils/chatDateBuckets';
import { formatChatDay, formatChatTimestamp } from '../../../utils/dateUtils';
import { fillMessage } from '../../dictation/copy';
import { DICTIONARY_LABELS, dictionaryMessage, type DictionaryMessageId } from '../copy';

const LABELS = DICTIONARY_LABELS.history;

/** What a row the person acted on while the pointer held the list says in place. */
export type HistorySettledKind = keyof typeof LABELS.settled;

/** One date bucket of dictations, newest first, under the chat lists' own heading words. */
export interface HistoryGroup {
  /** The bucket's key (`today`, `yesterday`, `week`, `month30`, `2026-08`, `unknown`). */
  key: string;
  title: string;
  entries: DictationHistoryEntryView[];
}

/**
 * Group dictations under the shared chat date buckets (`chatDateBuckets.ts`: Today, Yesterday,
 * Previous 7 days, Previous 30 days, then months), so a dictation sits under the same heading
 * words as a chat (one format across sibling surfaces). Entries keep their order inside a bucket.
 *
 * The bucket is asked for once per local calendar day, as `groupSessionsByDate` does: for a day
 * older than 30 the bucket builds a month name, and 5,000 entries would pay that 5,000 times.
 * The result is `groupByChatDate`'s, which `historyRows.test.ts` holds it to.
 */
export function groupDictationsByDate(
  entries: readonly DictationHistoryEntryView[],
  now: number = Date.now()
): HistoryGroup[] {
  const bucketOfDay = new Map<number, ChatDateBucket>();
  const groups = new Map<string, HistoryGroup & { rank: number }>();
  for (const entry of entries) {
    const timestamp = Date.parse(entry.created_at);
    const date = new Date(timestamp);
    // NaN is its own key, so unreadable times share the one 'unknown' bucket.
    const day = date.getFullYear() * 10_000 + date.getMonth() * 100 + date.getDate();
    let bucket = bucketOfDay.get(day);
    if (!bucket) {
      bucket = chatDateBucket(timestamp, now);
      bucketOfDay.set(day, bucket);
    }
    const group = groups.get(bucket.key);
    if (group) group.entries.push(entry);
    else {
      groups.set(bucket.key, {
        key: bucket.key,
        title: bucket.label,
        rank: bucket.rank,
        entries: [entry],
      });
    }
  }
  return [...groups.values()]
    .sort((left, right) => left.rank - right.rank)
    .map(({ key, title, entries: grouped }) => ({ key, title, entries: grouped }));
}

/** The word for where a dictation went: Chat, Crew, Clipboard, Another app, App preview, Terminal. */
export function surfaceWord(surface: DictationHistorySurface): string {
  return LABELS.surfaces[surface] ?? surface;
}

/** The parts of a row's muted line: "3:39 PM · Chat · 14 s", and the time's full stamp. */
export interface HistoryMeta {
  /** `formatChatDay`: the time today, the day this year, the day and year before. */
  time: string;
  /** `formatChatTimestamp`: the time's hover label. */
  stamp: string;
  surface: string;
  length: string;
}

export function historyMeta(
  entry: DictationHistoryEntryView,
  now: number = Date.now()
): HistoryMeta {
  return {
    time: formatChatDay(entry.created_at, now),
    stamp: formatChatTimestamp(entry.created_at),
    surface: surfaceWord(entry.surface),
    length: LABELS.duration(entry.duration_ms),
  };
}

/** The muted line as one string, for a test or an accessible description. */
export function metaLine(entry: DictationHistoryEntryView, now: number = Date.now()): string {
  const meta = historyMeta(entry, now);
  return [meta.time, meta.surface, meta.length].filter(Boolean).join(' · ');
}

/**
 * An expanded row's one muted line, the only place the model is named (complexity hidden by
 * default): "Private · Nov 26, 2026, 3:39 PM · Chat · 14 s · Large v3 Turbo".
 */
export function expandedLine(entry: DictationHistoryEntryView): string {
  return [
    LABELS.tier[entry.tier],
    formatChatTimestamp(entry.created_at),
    surfaceWord(entry.surface),
    LABELS.duration(entry.duration_ms),
    entry.model_name?.trim() || entry.model,
  ]
    .filter(Boolean)
    .join(' · ');
}

/**
 * The warning **Make public…** asks: the stronger one when the dictation used private dictionary
 * terms (its text may contain them), which the daemon tells only a caller that may list private
 * entries.
 */
export function historyPublicizeWarning(entry: DictationHistoryEntryView): DictionaryMessageId {
  return entry.private_terms ? 'MSG-HISTORY-PUBLICIZE-WARN-TERMS' : 'MSG-HISTORY-PUBLICIZE-WARN';
}

/** MSG-HISTORY-HOST-TIER as the catalog writes it, both placeholders unfilled. */
const HOST_TIER_RAW = dictionaryMessage('MSG-HISTORY-HOST-TIER');

/**
 * The reason a tier change is refused before the click, for this row: the daemon's sentence,
 * with MSG-HISTORY-HOST-TIER's verb and id filled for the change this row would make (the daemon
 * names neither, so its refusal tells nothing about any entry).
 */
export function tierChangeReason(
  reason: string | null | undefined,
  entry: DictationHistoryEntryView
): string | null {
  const given = reason?.trim();
  if (!given) return null;
  const verb = entry.tier === 'private' ? 'publicize' : 'privatize';
  const hostPublicize = dictionaryMessage('MSG-HISTORY-HOST-TIER', { verb: 'publicize' });
  if (given === hostPublicize || given === HOST_TIER_RAW) {
    return dictionaryMessage('MSG-HISTORY-HOST-TIER', { verb, id: entry.id });
  }
  return fillMessage(given, { id: entry.id });
}

/**
 * Whether **Turn on** may be pressed, and why not: turning the history on is a raise
 * (`authority.raise_settings`), refused before the click with the daemon's sentence, its
 * `<option>` filled as the terminal spells it. `null` settings (not read yet) leave it disabled
 * with no reason.
 */
export function turnOnGate(settings: DictationHistorySettings | null): {
  disabled: boolean;
  reason: string | null;
} {
  const authority = settings?.authority;
  if (!settings || !authority) return { disabled: true, reason: null };
  if (settings.keep_history || authority.raise_settings.allowed) {
    return { disabled: false, reason: null };
  }
  const reason = authority.raise_settings.reason?.trim();
  return { disabled: true, reason: reason ? fillMessage(reason, { option: '--keep on' }) : null };
}

/** An answer the daemon gives as `{allowed, reason}`. */
export interface HistoryAllowance {
  allowed: boolean;
  reason?: string | null;
}

/**
 * Managed `dictation.history` from the window's dictation status (`policy.history`, which a
 * daemon from before the history does not send): `null` when absent, so nothing is refused.
 */
export function historyPolicyOf(
  status: { policy?: object | null } | null | undefined
): HistoryAllowance | null {
  const policy = status?.policy as { history?: unknown } | null | undefined;
  const history = policy?.history as { allowed?: unknown; reason?: unknown } | null | undefined;
  if (!history || typeof history.allowed !== 'boolean') return null;
  return {
    allowed: history.allowed,
    reason: typeof history.reason === 'string' ? history.reason : null,
  };
}

/** At most this many characters may become a term (MSG-TERM-TOO-LONG). */
const MAX_TERM_CHARS = 64;

/**
 * The text selected inside `element`, when it is one line of at most 64 characters, which is
 * what **Add to dictionary…** prefills; '' otherwise.
 */
export function selectionWithin(element: Element | null): string {
  if (!element || typeof window === 'undefined') return '';
  const selection = window.getSelection?.();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return '';
  const range = selection.getRangeAt(0);
  if (!element.contains(range.commonAncestorContainer)) return '';
  const text = selection.toString().trim();
  if (!text || /[\r\n]/.test(text) || Array.from(text).length > MAX_TERM_CHARS) return '';
  return text;
}

/** Whether any text is selected inside `element` (a click that ends a selection must not toggle). */
export function hasSelectionWithin(element: Element | null): boolean {
  if (!element || typeof window === 'undefined') return false;
  const selection = window.getSelection?.();
  if (!selection || selection.isCollapsed || selection.rangeCount === 0) return false;
  return element.contains(selection.getRangeAt(0).commonAncestorContainer);
}
