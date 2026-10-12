import { useEffect, useState } from 'react';
import type { UsageReportRow, UsageSummaryResponse } from '../../../api';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { SettingSection } from '../../ui/setting-row';
import { Skeleton } from '../../ui/skeleton';
import { settingsUsageCopy as copy } from '../../usage/copy';
import { SETTINGS_SECTION_IDS } from '../settingsSections';
import { loadUsage } from './loadUsage';
import { UsagePanel } from './UsagePanel';
import { readUsageSnapshot } from './usageSnapshot';

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
 *
 * The App tab unmounts on a tab switch, so the section seeds itself from the last read of this
 * month (`usageSnapshot.ts`) and draws the row in its first frame; the daemon is still asked, and
 * its answer replaces the seed in place without a status line, because the words are the same
 * and a line that appeared for 100 ms would move everything below it. The skeleton is only for
 * the first read in a process, and `warmUsageSnapshot` (asked while Settings opens) usually has
 * even that one waiting (vision V4-R7-3, principle 9).
 */
export default function UsageSection({ showCosts = true }: { showCosts?: boolean }) {
  const [seed] = useState(readUsageSnapshot);
  const [summary, setSummary] = useState<UsageSummaryResponse | null>(seed?.summary ?? null);
  const [dayRows, setDayRows] = useState<UsageReportRow[]>(seed?.dayRows ?? []);
  const [modelRows, setModelRows] = useState<UsageReportRow[]>(seed?.modelRows ?? []);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [retryVersion, setRetryVersion] = useState(0);

  useEffect(() => {
    let cancelled = false;
    const load = async () => {
      setLoading(true);
      setLoadError(null);
      try {
        const read = await loadUsage();
        if (cancelled) return;
        setSummary(read.summary);
        setDayRows(read.dayRows);
        setModelRows(read.modelRows);
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
      {/* Only a retry the person asked for says it is working; the seeded read stays silent. */}
      {loading && summary && retryVersion > 0 && (
        <p className="px-3 text-supporting text-text-muted" role="status">
          {copy.refreshing}
        </p>
      )}
    </SettingSection>
  );
}
