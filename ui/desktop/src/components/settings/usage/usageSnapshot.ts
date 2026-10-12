import type { UsageReportRow, UsageSummaryResponse } from '../../../api';

/**
 * The last month-to-date report Settings > App > Usage read, kept for the life of the process.
 *
 * The App tab unmounts when a person switches to another tab, so every return mounted the Usage
 * row with nothing to draw and showed a 712x40 pulsing skeleton for the 90 to 190 ms the daemon
 * took to answer (vision V4-R7-3, principle 9: a surface mounted by a tab switch appears at
 * rest). The section now seeds its state from here, draws the row in its first frame and swaps
 * the daemon's answer in place; the skeleton is only for the first read in a process.
 *
 * It holds the three payloads the section keeps, exactly as it keeps them, and nothing else: no
 * dollar figure is decided here, and `showCosts` is applied by the panel as before. A snapshot
 * belongs to one calendar month (the client's, the same month `from` is built from), because the
 * row is labelled "This month" and last month's figures must never stand in for it.
 */
export interface UsageSnapshot {
  /** `YYYY-MM` in the client's local calendar, when the read was made. */
  month: string;
  summary: UsageSummaryResponse;
  dayRows: UsageReportRow[];
  modelRows: UsageReportRow[];
}

let snapshot: UsageSnapshot | null = null;
let listening = false;

/**
 * `ResetPanel` announces every completed reset on this window event, after the daemon has
 * emptied what it was asked to. Any reset forgets the snapshot, whichever categories it named:
 * the one that matters is history, and the cost of forgetting on another is a single skeleton on
 * the next visit. It runs before the section remounts, because `ResetPanel` dispatches it in the
 * same task as its `onReset` state update and React renders that update afterwards.
 */
export const DATA_RESET_EVENT = 'biorouter:data-reset';

/** `YYYY-MM` for the client's local calendar month of `date`. */
export const usageMonthKey = (date: Date): string =>
  `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, '0')}`;

/** The remembered read, or `null` when there is none for the month `now` falls in. */
export function readUsageSnapshot(now: Date = new Date()): UsageSnapshot | null {
  return snapshot && snapshot.month === usageMonthKey(now) ? snapshot : null;
}

/** Remember a completed read, made at `readAt`, as the one the next mount seeds from. */
export function rememberUsageSnapshot(
  readAt: Date,
  data: Pick<UsageSnapshot, 'summary' | 'dayRows' | 'modelRows'>
): void {
  snapshot = { month: usageMonthKey(readAt), ...data };
  if (!listening && typeof window !== 'undefined') {
    listening = true;
    window.addEventListener(DATA_RESET_EVENT, forgetUsageSnapshot);
  }
}

/**
 * Drop what was remembered. Resetting history empties the report, and the section remounts to
 * show that: seeding it from the old totals would draw them for one round trip before the zeros.
 * The reset event calls this by itself; nothing needs to remember to.
 */
export function forgetUsageSnapshot(): void {
  snapshot = null;
}
