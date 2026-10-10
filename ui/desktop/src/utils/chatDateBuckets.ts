/**
 * Coarse date buckets for chat lists, shared by the sidebar and History so the
 * two never disagree about where a chat sits.
 *
 * Today, Yesterday, Previous 7 days, Previous 30 days, then the month name
 * ("September"), with the year added outside the current year ("August 2025").
 * It replaces one header per day ("Oct 7", "Oct 6", …) in the sidebar and the
 * long weekday format History used.
 *
 * Days are local calendar days, so a chat from 23:50 yesterday is "Yesterday"
 * at 00:10 today. A timestamp in the future (clock skew between windows and the
 * daemon) counts as today.
 */

export type ChatDateBucketKind = 'today' | 'yesterday' | 'week' | 'month30' | 'month' | 'unknown';

export interface ChatDateBucket {
  /** Stable key, for React keys and tests: `today`, `yesterday`, `week`, `month30`, `2026-08`, `unknown`. */
  key: string;
  kind: ChatDateBucketKind;
  /** Sentence-case label shown above the bucket's rows. */
  label: string;
  /** Sort rank: smaller is newer. Buckets are shown in ascending rank. */
  rank: number;
}

export interface ChatDateGroup<T> {
  key: string;
  label: string;
  items: T[];
}

export const CHAT_DATE_BUCKET_LABELS = {
  today: 'Today',
  yesterday: 'Yesterday',
  week: 'Previous 7 days',
  month30: 'Previous 30 days',
  unknown: 'Older',
} as const;

const DAY_MS = 86_400_000;

function toTimestamp(value: string | number | Date | null | undefined): number {
  if (value instanceof Date) return value.getTime();
  if (typeof value === 'number') return value;
  if (typeof value === 'string') return Date.parse(value);
  return Number.NaN;
}

function localCalendarDay(timestamp: number): number {
  const date = new Date(timestamp);
  return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
}

/** The bucket a timestamp falls in, relative to `now`. */
export function chatDateBucket(
  value: string | number | Date | null | undefined,
  now: number = Date.now()
): ChatDateBucket {
  const timestamp = toTimestamp(value);
  if (Number.isNaN(timestamp)) {
    return {
      key: 'unknown',
      kind: 'unknown',
      label: CHAT_DATE_BUCKET_LABELS.unknown,
      rank: Number.MAX_SAFE_INTEGER,
    };
  }

  const days = Math.max(
    0,
    Math.round((localCalendarDay(now) - localCalendarDay(timestamp)) / DAY_MS)
  );
  if (days === 0)
    return { key: 'today', kind: 'today', label: CHAT_DATE_BUCKET_LABELS.today, rank: 0 };
  if (days === 1) {
    return {
      key: 'yesterday',
      kind: 'yesterday',
      label: CHAT_DATE_BUCKET_LABELS.yesterday,
      rank: 1,
    };
  }
  if (days <= 7) return { key: 'week', kind: 'week', label: CHAT_DATE_BUCKET_LABELS.week, rank: 2 };
  if (days <= 30) {
    return { key: 'month30', kind: 'month30', label: CHAT_DATE_BUCKET_LABELS.month30, rank: 3 };
  }

  const date = new Date(timestamp);
  const current = new Date(now);
  const sameYear = date.getFullYear() === current.getFullYear();
  const label = new Intl.DateTimeFormat('en-US', {
    month: 'long',
    ...(sameYear ? {} : { year: 'numeric' }),
  }).format(date);
  const monthsAgo =
    (current.getFullYear() - date.getFullYear()) * 12 + (current.getMonth() - date.getMonth());
  const month = String(date.getMonth() + 1).padStart(2, '0');
  return {
    key: `${date.getFullYear()}-${month}`,
    kind: 'month',
    label,
    rank: 4 + Math.max(0, monthsAgo),
  };
}

/**
 * Group items into date buckets, newest bucket first. Items keep their input
 * order inside a bucket, so the caller decides the order within a group (by
 * activity, by creation or by name).
 */
export function groupByChatDate<T>(
  items: readonly T[],
  timeOf: (item: T) => string | number | Date | null | undefined,
  now: number = Date.now()
): ChatDateGroup<T>[] {
  const groups = new Map<string, ChatDateGroup<T> & { rank: number }>();
  for (const item of items) {
    const bucket = chatDateBucket(timeOf(item), now);
    const existing = groups.get(bucket.key);
    if (existing) {
      existing.items.push(item);
    } else {
      groups.set(bucket.key, {
        key: bucket.key,
        label: bucket.label,
        rank: bucket.rank,
        items: [item],
      });
    }
  }
  return [...groups.values()]
    .sort((left, right) => left.rank - right.rank)
    .map(({ key, label, items: groupItems }) => ({ key, label, items: groupItems }));
}
