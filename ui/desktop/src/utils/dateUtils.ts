import { Session } from '../api';
import { groupByChatDate } from './chatDateBuckets';

export interface DateGroup {
  /** Stable bucket key (`today`, `yesterday`, `week`, `month30`, `2026-08`, `unknown`). */
  key: string;
  /** Sentence-case label: Today, Yesterday, Previous 7 days, Previous 30 days, a month. */
  label: string;
  sessions: Session[];
}

/**
 * Chat history's date groups: the same coarse buckets the sidebar uses
 * (`chatDateBuckets.ts`), so a chat sits under the same heading in both places.
 *
 * This replaced one heading per day in a third format ("Tuesday, October 7"),
 * which made a long history a column of near-identical headings. Buckets come
 * newest first; inside one, sessions keep the order they arrive in (the list
 * route's `updated_at` descending).
 */
export function groupSessionsByDate(sessions: Session[], now: number = Date.now()): DateGroup[] {
  return groupByChatDate(sessions, (session) => session.updated_at, now).map(
    ({ key, label, items }) => ({ key, label, sessions: items })
  );
}

const TIME_FORMAT = new Intl.DateTimeFormat('en-US', { hour: 'numeric', minute: '2-digit' });
const DAY_FORMAT = new Intl.DateTimeFormat('en-US', { month: 'short', day: 'numeric' });
const DAY_YEAR_FORMAT = new Intl.DateTimeFormat('en-US', {
  month: 'short',
  day: 'numeric',
  year: 'numeric',
});

/**
 * A chat's "when" on a row: the time for a chat from today ("3:39 PM"), the day
 * for an older one this year ("Sep 5"), and the day with its year before that
 * ("Sep 5, 2025"). The bucket heading above the row already names the period,
 * so an older row's time of day was noise; the full stamp is in the row's
 * tooltip ({@link formatChatTimestamp}).
 *
 * This replaced a US numeric date with a time ("09/05/2026 10:49 AM") under a
 * "September" heading, beside today's rows in a second format (vision V1-L3).
 * Returns '' for a value that is not a date.
 */
export function formatChatDay(value: string, now: number = Date.now()): string {
  const timestamp = Date.parse(value);
  if (Number.isNaN(timestamp)) return '';
  const date = new Date(timestamp);
  const today = new Date(now);
  if (date.getFullYear() !== today.getFullYear()) return DAY_YEAR_FORMAT.format(date);
  if (date.getMonth() === today.getMonth() && date.getDate() === today.getDate()) {
    return TIME_FORMAT.format(date);
  }
  return DAY_FORMAT.format(date);
}

/**
 * The full stamp, in the same words as {@link formatChatDay}: "Sep 5, 2026,
 * 10:49 AM". For a row's tooltip and a transcript's band. Returns '' for a value
 * that is not a date.
 */
export function formatChatTimestamp(value: string | number): string {
  const timestamp = typeof value === 'number' ? value : Date.parse(value);
  if (Number.isNaN(timestamp)) return '';
  const date = new Date(timestamp);
  return `${DAY_YEAR_FORMAT.format(date)}, ${TIME_FORMAT.format(date)}`;
}
