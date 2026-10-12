import { getUsageReport, getUsageSummary } from '../../../api';
import { userActionHeaders } from '../../../utils/userAction';
import { fillCalendarDays } from './UsagePanel';
import { readUsageSnapshot, rememberUsageSnapshot, type UsageSnapshot } from './usageSnapshot';

/** What one month-to-date read returns: the three payloads the Usage section keeps. */
export type UsageRead = Pick<UsageSnapshot, 'summary' | 'dayRows' | 'modelRows'>;

/**
 * Read the month to date: the billing summary and the day- and model-grouped reports for the
 * calendar month the read is made in. Every completed read is remembered (`usageSnapshot.ts`), so
 * the next mount of the Usage section starts from it, whoever asked for this one.
 */
export async function loadUsage(): Promise<UsageRead> {
  const readAt = new Date();
  const now = Math.floor(readAt.getTime() / 1000);
  const from = Math.floor(new Date(readAt.getFullYear(), readAt.getMonth(), 1).getTime() / 1000);
  const headers = await userActionHeaders();
  const [summaryRes, dayRes, modelRes] = await Promise.all([
    getUsageSummary<true>({ headers, throwOnError: true }),
    getUsageReport<true>({ headers, query: { from, to: now, group: 'day' }, throwOnError: true }),
    getUsageReport<true>({ headers, query: { from, to: now, group: 'model' }, throwOnError: true }),
  ]);
  const read: UsageRead = {
    summary: summaryRes.data,
    dayRows: fillCalendarDays(dayRes.data.rows, readAt),
    modelRows: modelRes.data.rows,
  };
  rememberUsageSnapshot(readAt, read);
  return read;
}

/**
 * Ask for this month's usage while Settings opens, so the first visit to the App tab in a
 * process has a read waiting and draws the Usage row at rest too (vision V4-R7-3). Read-only and
 * never louder than the tab: a failed read is dropped, and the tab reads again and says what it
 * always said. Does nothing when this month has already been read.
 */
export function warmUsageSnapshot(): void {
  if (readUsageSnapshot()) return;
  void loadUsage().catch(() => {});
}
