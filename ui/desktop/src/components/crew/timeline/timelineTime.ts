import { timelineCopy } from './copy';

/**
 * Dates and times as the timeline shows them. Pure, so the grouping and its
 * tests share one definition of "the same day" and "Yesterday".
 *
 * The broker writes Unix SECONDS; anything at or above 1e12 is taken as
 * milliseconds already, which is the rule the old view used, so a message a
 * newer daemon stamps in milliseconds still lands on the right day.
 *
 * Formatting is `en-US`, as `utils/timeUtils.ts` does for chat, so a transcript
 * and a channel read the same and tests are deterministic. The calendar itself
 * is the viewer's local one.
 */

const LOCALE = 'en-US';

export function messageTime(createdAt: number): Date {
  const value = Number.isFinite(createdAt) ? createdAt : 0;
  return new Date(value < 1e12 ? value * 1000 : value);
}

/** Local calendar day, as a sortable key (`2026-09-22`). */
export function dayKey(date: Date): string {
  const month = String(date.getMonth() + 1).padStart(2, '0');
  const day = String(date.getDate()).padStart(2, '0');
  return `${date.getFullYear()}-${month}-${day}`;
}

function startOfDay(date: Date): Date {
  return new Date(date.getFullYear(), date.getMonth(), date.getDate());
}

/**
 * The day divider's words: Today, Yesterday, "Monday, September 22" in the
 * current year, "September 22, 2025" in any other.
 */
export function dayLabel(date: Date, now: Date): string {
  const days = Math.round(
    (startOfDay(now).getTime() - startOfDay(date).getTime()) / (24 * 60 * 60 * 1000)
  );
  if (days === 0) return timelineCopy.today;
  if (days === 1) return timelineCopy.yesterday;
  if (date.getFullYear() === now.getFullYear()) {
    return date.toLocaleDateString(LOCALE, { weekday: 'long', month: 'long', day: 'numeric' });
  }
  return date.toLocaleDateString(LOCALE, { month: 'long', day: 'numeric', year: 'numeric' });
}

/** "10:02 AM". */
export function shortTime(date: Date): string {
  return date.toLocaleTimeString(LOCALE, { hour: 'numeric', minute: '2-digit', hour12: true });
}

/**
 * "10:02", for a continuation row's 44px gutter, where "10:02 AM" does not fit
 * on one line. The group's head, a few rows up, carries the full time.
 */
export function gutterTime(date: Date): string {
  return date
    .toLocaleTimeString(LOCALE, { hour: 'numeric', minute: '2-digit', hour12: true })
    .replace(/\s?[AP]M$/i, '');
}

/** "Tuesday, September 22, 2026 at 10:02 AM", for the time's tooltip. */
export function fullDateTime(date: Date): string {
  const day = date.toLocaleDateString(LOCALE, {
    weekday: 'long',
    month: 'long',
    day: 'numeric',
    year: 'numeric',
  });
  return `${day} at ${shortTime(date)}`;
}

/** The machine-readable form for `<time dateTime>`, or undefined for an unusable stamp. */
export function isoTime(date: Date): string | undefined {
  return Number.isNaN(date.getTime()) ? undefined : date.toISOString();
}
