import { useEffect, useState } from 'react';
import {
  getUsageReport,
  getUsageSummary,
  UsageReportRow,
  UsageSummaryResponse,
} from '../../../api';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Skeleton } from '../../ui/skeleton';
import { fillCalendarDays, UsagePanel } from './UsagePanel';

/**
 * Settings-embedded Usage surface: fetches the month-to-date summary and the
 * matching day- and model-grouped reports, and hands them to the pure
 * {@link UsagePanel}. Fetch failures remain visible and retryable instead of
 * making the entire settings section disappear.
 */
export default function UsageSection() {
  const [summary, setSummary] = useState<UsageSummaryResponse | null>(null);
  const [dayRows, setDayRows] = useState<UsageReportRow[]>([]);
  const [modelRows, setModelRows] = useState<UsageReportRow[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [retryVersion, setRetryVersion] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      setLoadError(null);
      try {
        const currentTime = new Date();
        const now = Math.floor(currentTime.getTime() / 1000);
        const from = Math.floor(
          new Date(currentTime.getFullYear(), currentTime.getMonth(), 1).getTime() / 1000
        );
        const [summaryRes, dayRes, modelRes] = await Promise.all([
          getUsageSummary<true>({ throwOnError: true }),
          getUsageReport<true>({ query: { from, to: now, group: 'day' }, throwOnError: true }),
          getUsageReport<true>({ query: { from, to: now, group: 'model' }, throwOnError: true }),
        ]);
        if (cancelled) return;
        setSummary(summaryRes.data);
        setDayRows(fillCalendarDays(dayRes.data.rows, currentTime));
        setModelRows(modelRes.data.rows);
      } catch (error) {
        if (cancelled) return;
        console.error('Failed to load usage:', error);
        setLoadError('Usage data could not be loaded. Check the backend connection and try again.');
      } finally {
        if (!cancelled) setLoading(false);
      }
    };
    load();
    return () => {
      cancelled = true;
    };
  }, [retryVersion]);

  return (
    <div className="biorouter-settings-section">
      <div className="biorouter-settings-section-header">
        <h2 className="mb-1 text-caps text-text-muted">Usage</h2>
        <p className="text-supporting text-text-muted">
          Billed tokens and estimated cost for the current month, grouped by day and model.
        </p>
      </div>

      {loadError && (
        <Note
          tone="neutral"
          role="alert"
          testId="usage-load-error"
          className="mb-2"
          action={
            <Button
              type="button"
              size="sm"
              variant="outline"
              onClick={() => setRetryVersion((version) => version + 1)}
            >
              Retry
            </Button>
          }
        >
          {loadError}
          {summary ? ' Showing the last loaded month-to-date report.' : ''}
        </Note>
      )}

      {loading && !summary ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-4 w-40" />
          <Skeleton className="h-16 w-full" />
          <Skeleton className="h-24 w-full" />
        </div>
      ) : summary ? (
        <UsagePanel summary={summary} dayRows={dayRows} modelRows={modelRows} />
      ) : null}
      {loading && summary && (
        <p className="text-supporting text-text-muted" role="status">
          Refreshing usage…
        </p>
      )}
    </div>
  );
}
