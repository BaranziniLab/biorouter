import { userActionHeaders } from '../../../utils/userAction';
import { useEffect, useState } from 'react';
import {
  getUsageReport,
  getUsageSummary,
  UsageReportRow,
  UsageSummaryResponse,
} from '../../../api';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SettingSection } from '../../ui/setting-row';
import { Skeleton } from '../../ui/skeleton';
import { settingsUsageCopy as copy } from '../../usage/copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { fillCalendarDays, UsagePanel } from './UsagePanel';

/**
 * Settings > App > Usage: fetches the month-to-date summary and the matching
 * day- and model-grouped reports, and hands them to the pure
 * {@link UsagePanel}. Fetch failures remain visible and retryable instead of
 * making the entire settings section disappear. The section's explanation is
 * its InfoTip; there is no paragraph under the label.
 *
 * `showCosts` (Settings > App > Show costs) drops the dollar figures and keeps the tokens and
 * turns, in the same row; the section never unmounts on that switch, so nothing below it moves
 * under the pointer (vision V4-02).
 */
export default function UsageSection({ showCosts = true }: { showCosts?: boolean }) {
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
        const headers = await userActionHeaders();
        const [summaryRes, dayRes, modelRes] = await Promise.all([
          getUsageSummary<true>({ headers, throwOnError: true }),
          getUsageReport<true>({
            headers,
            query: { from, to: now, group: 'day' },
            throwOnError: true,
          }),
          getUsageReport<true>({
            headers,
            query: { from, to: now, group: 'model' },
            throwOnError: true,
          }),
        ]);
        if (cancelled) return;
        setSummary(summaryRes.data);
        setDayRows(fillCalendarDays(dayRes.data.rows, currentTime));
        setModelRows(modelRes.data.rows);
      } catch (error) {
        if (cancelled) return;
        console.error('Failed to load usage:', error);
        setLoadError(copy.loadError);
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
    <SettingSection
      id={SETTINGS_SECTION_IDS.usage}
      title={copy.sectionTitle}
      help={showCosts ? copy.sectionHelp : copy.sectionHelpNoCost}
    >
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
              variant="secondary"
              onClick={() => setRetryVersion((version) => version + 1)}
            >
              {copy.retry}
            </Button>
          }
        >
          {loadError}
          {summary ? copy.loadErrorStale : ''}
        </Note>
      )}

      {loading && !summary ? (
        <Skeleton className="h-10 w-full" data-testid="usage-loading" />
      ) : summary ? (
        <UsagePanel
          summary={summary}
          dayRows={dayRows}
          modelRows={modelRows}
          showCosts={showCosts}
        />
      ) : null}
      {loading && summary && (
        <p className="px-3 text-supporting text-text-muted" role="status">
          {copy.refreshing}
        </p>
      )}
    </SettingSection>
  );
}
