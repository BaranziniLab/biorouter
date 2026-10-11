import { useState } from 'react';
import type { UsageReportRow, UsageSummaryResponse, UsageTotals } from '../../../api';
import {
  billedTokens,
  cacheTokens as combinedCacheTokens,
  knownBilledTokens,
} from '../../../utils/usageAccounting';
import { formatTokensCompact } from '../../../utils/usageStats';
import { ModalShell } from '../../ModalShell';
import { Button } from '../../ui/button';
import { InfoTip } from '../../ui/info-tip';
import { Progress } from '../../ui/progress';
import { SettingRow } from '../../ui/setting-row';
import { StatGrid, type StatItem } from '../../usage/StatGrid';
import { settingsUsageCopy as copy } from '../../usage/copy';
import '../../usage/usage.css';

/**
 * Settings > App > Usage (spec 3.9): the BILLING view of the month, which
 * counts deleted chats and subagent turns (issue #224), unlike Home's activity
 * card. One row ("This month", its figures, Details), and a 640px dialog with
 * the shared StatGrid, flat budget rows and two hairline tables. No icon tiles,
 * no boxes inside boxes, explanations in InfoTips.
 */

function emptyDayRow(date: string): UsageReportRow {
  return {
    date,
    modelId: null,
    provider: null,
    inputTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    cacheReadTokens: 0,
    cacheCreationTokens: 0,
    turns: 0,
    cost: 0,
    hasUnpriced: false,
    costExcludesCache: false,
  };
}

/** The report omits inactive dates; restore them so recency remains a calendar sequence. */
export function fillCalendarDays(rows: UsageReportRow[], through: Date): UsageReportRow[] {
  if (rows.length === 0) return [];

  const year = through.getFullYear();
  const month = through.getMonth() + 1;
  const monthPrefix = `${year}-${String(month).padStart(2, '0')}`;
  const rowsByDate = new Map(
    rows.filter((row) => row.date?.startsWith(monthPrefix)).map((row) => [row.date, row])
  );

  return Array.from({ length: through.getDate() }, (_, index) => {
    const date = `${monthPrefix}-${String(index + 1).padStart(2, '0')}`;
    return rowsByDate.get(date) ?? emptyDayRow(date);
  });
}

export interface UsagePanelProps {
  summary: UsageSummaryResponse;
  dayRows: UsageReportRow[];
  modelRows: UsageReportRow[];
  /**
   * Whether dollar figures are drawn (Settings > App > Show costs). Off drops every cost and
   * keeps tokens and turns, so the row and the report stay where they are instead of the whole
   * section unmounting and moving every row below it (vision V4-02). Default on.
   */
  showCosts?: boolean;
}

export function formatTokens(n: number | null | undefined): string {
  return typeof n === 'number' && Number.isFinite(n) && n >= 0
    ? n.toLocaleString('en-US')
    : 'Not recorded';
}

export function formatCost(cost: number | null | undefined): string {
  if (cost === null || cost === undefined || !Number.isFinite(cost) || cost < 0) {
    return 'Unavailable';
  }
  if (cost > 0 && cost < 0.01) return '<$0.01';
  return `$${cost.toFixed(2)}`;
}

export function formatCostEstimate(cost: number | null | undefined): string {
  return formatCost(cost);
}

export function formatUsageDate(date: string | null | undefined): string {
  const match = date?.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  if (!match) return date ?? 'Unknown date';
  const [, year, month, day] = match;
  return new Date(Number(year), Number(month) - 1, Number(day)).toLocaleDateString('en-US', {
    weekday: 'short',
    month: 'short',
    day: 'numeric',
  });
}

/** `2026-07` → `July 2026`; anything else as given. */
export function formatUsageMonth(month: string): string {
  const match = month.match(/^(\d{4})-(\d{2})$/);
  if (!match) return month;
  return new Date(Number(match[1]), Number(match[2]) - 1, 1).toLocaleDateString('en-US', {
    month: 'long',
    year: 'numeric',
  });
}

export function modelLabel(row: Pick<UsageReportRow, 'modelId' | 'provider'>): string {
  if (row.modelId && row.provider) return `${row.provider}/${row.modelId}`;
  if (row.modelId) return row.modelId;
  return 'unknown';
}

export function cacheTokens(
  row: Pick<UsageReportRow, 'cacheReadTokens' | 'cacheCreationTokens'>
): number | null {
  return combinedCacheTokens(row);
}

export function formatBilledTokens(row: UsageReportRow | UsageTotals): string {
  const exact = billedTokens(row);
  if (exact !== null) return formatTokens(exact);
  const knownSubtotal = knownBilledTokens(row);
  return knownSubtotal > 0 ? formatTokens(knownSubtotal) : 'Unavailable';
}

/** The app's one compact token format (`usageStats.formatTokensCompact`), with "Not recorded". */
export function formatCompactTokens(n: number | null | undefined): string {
  if (typeof n !== 'number' || !Number.isFinite(n) || n < 0) return 'Not recorded';
  return formatTokensCompact(n);
}

function billedTokenValue(row: UsageReportRow | UsageTotals): number | null {
  const exact = billedTokens(row);
  if (exact !== null) return exact;
  const knownSubtotal = knownBilledTokens(row);
  return knownSubtotal > 0 ? knownSubtotal : null;
}

function costIsPartial(row: Pick<UsageReportRow, 'hasUnpriced' | 'costExcludesCache'>) {
  return row.hasUnpriced || row.costExcludesCache;
}

function rowHasUnknownCost(
  row: Pick<UsageReportRow | UsageTotals, 'turns' | 'cost' | 'hasUnpriced'>
) {
  return row.hasUnpriced || (row.turns > 0 && formatCost(row.cost) === 'Unavailable');
}

function hasIncompleteTokens(row: UsageReportRow | UsageTotals): boolean {
  return (
    billedTokens(row) === null || row.cacheReadTokens == null || row.cacheCreationTokens == null
  );
}

function hasRecordedCacheUsage(row: UsageReportRow | UsageTotals): boolean {
  return (
    (row.cacheReadTokens != null && row.cacheReadTokens > 0) ||
    (row.cacheCreationTokens != null && row.cacheCreationTokens > 0)
  );
}

function hasKnownCost(row: UsageReportRow | UsageTotals): boolean {
  return row.cost != null && Number.isFinite(row.cost) && row.cost >= 0;
}

function hasUsage(row: UsageReportRow): boolean {
  return row.turns > 0 || knownBilledTokens(row) > 0;
}

function serverPercent(percent: number | null | undefined): number | null {
  return typeof percent === 'number' && Number.isFinite(percent) && percent >= 0 ? percent : null;
}

/** A compact figure with the exact one in its hover title. */
function CompactTokenValue({
  value,
  fallback = 'Not recorded',
}: {
  value: number | null | undefined;
  fallback?: string;
}) {
  if (typeof value !== 'number' || !Number.isFinite(value) || value < 0) {
    return <>{fallback}</>;
  }
  const compact = formatCompactTokens(value);
  const exact = formatTokens(value);
  return <span title={compact === exact ? undefined : exact}>{compact}</span>;
}

/** `500`, or `not recorded` for a bucket the history never kept. */
function cacheFigure(value: number | null | undefined): string {
  const formatted = formatTokens(value);
  return formatted === 'Not recorded' ? 'not recorded' : formatted;
}

function cacheSplit(row: Pick<UsageReportRow, 'cacheReadTokens' | 'cacheCreationTokens'>): string {
  return `Cache read ${cacheFigure(row.cacheReadTokens)}, cache write ${cacheFigure(row.cacheCreationTokens)}`;
}

/**
 * One line, `3M in · 1M out`, so the model table fits the 640px dialog. The
 * cache split is the line's hover title, and is read out to a screen reader.
 */
function TokenFlowCell({ row, showCache }: { row: UsageReportRow; showCache: boolean }) {
  const split = showCache ? cacheSplit(row) : undefined;
  return (
    <span className="whitespace-nowrap" title={split} data-testid="usage-model-token-flow">
      {formatCompactTokens(row.inputTokens)} in · {formatCompactTokens(row.outputTokens)} out
      {split ? <span className="sr-only">, {split}</span> : null}
    </span>
  );
}

function EmptyTableState({ children }: { children: React.ReactNode }) {
  return <p className="py-6 text-center text-supporting text-text-muted">{children}</p>;
}

export function UsagePanel({ summary, dayRows, modelRows, showCosts = true }: UsagePanelProps) {
  const [reportOpen, setReportOpen] = useState(false);
  const month = summary.monthToDate;
  const monthTokens = billedTokenValue(month);

  return (
    <div data-testid="usage-panel">
      <SettingRow
        label={copy.rowLabel}
        help={showCosts ? copy.rowHelp(summary.month) : copy.rowHelpNoCost(summary.month)}
        value={
          <span className="tabular-nums">
            <span title={formatBilledTokens(month)}>
              {monthTokens === null ? formatBilledTokens(month) : formatCompactTokens(monthTokens)}
            </span>{' '}
            tokens
            {showCosts && (
              <>
                {' · '}
                <span>{formatCostEstimate(month.cost)}</span>
              </>
            )}
            {' · '}
            <span>{month.turns.toLocaleString('en-US')}</span>{' '}
            {month.turns === 1 ? 'turn' : 'turns'}
          </span>
        }
      >
        <Button
          type="button"
          size="sm"
          variant="secondary"
          onClick={() => setReportOpen(true)}
          aria-label={copy.detailsName}
        >
          {copy.details}
        </Button>
      </SettingRow>

      <ModalShell
        open={reportOpen}
        onOpenChange={setReportOpen}
        size="lg"
        title={copy.dialogTitle(formatUsageMonth(summary.month))}
        scrollBody
      >
        <div className="py-4" data-testid="usage-report-dialog">
          <UsageReport
            summary={summary}
            dayRows={dayRows}
            modelRows={modelRows}
            showCosts={showCosts}
          />
        </div>
      </ModalShell>
    </div>
  );
}

/** A caps label over a hairline table: no card, no icon tile, no description. */
function ReportSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-2">
      <h3 className="text-caps text-text-muted">{title}</h3>
      {children}
    </section>
  );
}

const TH = 'px-3 py-2 text-supporting font-medium text-text-muted';
const TD = 'border-t border-border-subtle px-3 py-2';

export function UsageReport({ summary, dayRows, modelRows, showCosts = true }: UsagePanelProps) {
  const mtd = summary.monthToDate;
  // With costs hidden nothing about cost is said at all: no figure, no column, no budget and no
  // note about unpriced or partial cost, which would explain a number that is not there.
  const anyUnpriced =
    showCosts &&
    (rowHasUnknownCost(mtd) ||
      dayRows.some(rowHasUnknownCost) ||
      modelRows.some(rowHasUnknownCost));
  const showMtdCache = hasRecordedCacheUsage(mtd);
  const showModelCache = modelRows.some(hasRecordedCacheUsage);
  const showDayCost = showCosts && dayRows.some((row) => hasUsage(row) && hasKnownCost(row));
  const showModelCost = showCosts && modelRows.some(hasKnownCost);
  const showDollarBudget = showCosts && summary.monthlyDollarLimit != null;
  const anyIncompleteTokens =
    hasIncompleteTokens(mtd) ||
    dayRows.some(hasIncompleteTokens) ||
    modelRows.some(hasIncompleteTokens);
  const anyCostExcludesCache =
    showCosts &&
    (mtd.costExcludesCache ||
      dayRows.some((row) => row.costExcludesCache) ||
      modelRows.some((row) => row.costExcludesCache));
  const mtdCostPartial = costIsPartial(mtd);
  const knownMtdCost =
    mtd.cost != null && Number.isFinite(mtd.cost) && mtd.cost >= 0 ? mtd.cost : null;
  const tokenPercent = serverPercent(summary.tokenPercent);
  const dollarPercent = serverPercent(summary.dollarPercent);
  const orderedDayRows = [...dayRows].sort((a, b) => (b.date ?? '').localeCompare(a.date ?? ''));
  const tokenUnavailableReason =
    tokenPercent !== null
      ? null
      : summary.monthlyTokenLimit != null && summary.monthlyTokenLimit <= 0
        ? 'Budget percentage unavailable for a zero token limit.'
        : billedTokens(mtd) === null
          ? 'Budget percentage unavailable because billed token history is incomplete.'
          : 'Budget percentage unavailable.';
  const dollarUnavailableReason =
    dollarPercent !== null
      ? null
      : summary.monthlyDollarLimit != null && summary.monthlyDollarLimit <= 0
        ? 'Budget percentage unavailable for a zero dollar limit.'
        : knownMtdCost === null
          ? 'Budget percentage unavailable because cost is unknown.'
          : mtdCostPartial
            ? 'Budget percentage unavailable because the known cost is only a partial subtotal.'
            : 'Budget percentage unavailable.';

  const mtdTokens = billedTokenValue(mtd);
  const exactMtdTokens = formatBilledTokens(mtd);
  const stats: StatItem[] = [
    {
      key: 'tokens',
      label: copy.tokens,
      value: mtdTokens === null ? exactMtdTokens : formatCompactTokens(mtdTokens),
      title:
        mtdTokens !== null && exactMtdTokens !== formatCompactTokens(mtdTokens)
          ? exactMtdTokens
          : undefined,
      help: showMtdCache
        ? copy.tokensCacheHelp(
            cacheFigure(mtd.cacheReadTokens),
            cacheFigure(mtd.cacheCreationTokens)
          )
        : copy.tokensHelp,
      note: hasIncompleteTokens(mtd) && mtdTokens !== null ? copy.conservative : undefined,
    },
    ...(showCosts
      ? [
          {
            key: 'cost',
            label: copy.cost,
            value: formatCostEstimate(mtd.cost),
            help: copy.costHelp,
            note: mtdCostPartial && knownMtdCost !== null ? copy.conservative : undefined,
          },
        ]
      : []),
    {
      key: 'turns',
      label: copy.turns,
      value: mtd.turns.toLocaleString('en-US'),
      help: copy.turnsHelp,
    },
  ];

  return (
    <div className="flex flex-col gap-6" data-testid="usage-report">
      <StatGrid items={stats} data-testid="usage-summary-stats" />

      {(summary.monthlyTokenLimit != null || showDollarBudget) && (
        <div className="grid grid-cols-1 gap-x-6 gap-y-4 sm:grid-cols-2">
          {summary.monthlyTokenLimit != null && (
            <UsageGauge
              testid="usage-gauge-tokens"
              label={copy.tokenBudget}
              used={formatBilledTokens(mtd)}
              limit={formatTokens(summary.monthlyTokenLimit)}
              percent={tokenPercent}
              unavailableReason={tokenUnavailableReason}
            />
          )}
          {showDollarBudget && summary.monthlyDollarLimit != null && (
            <UsageGauge
              testid="usage-gauge-dollars"
              label={copy.dollarBudget}
              used={formatCostEstimate(mtd.cost)}
              limit={`$${summary.monthlyDollarLimit.toFixed(2)}`}
              percent={dollarPercent}
              unavailableReason={dollarUnavailableReason}
            />
          )}
        </div>
      )}

      <ReportSection title={copy.byDay}>
        {dayRows.length === 0 ? (
          <EmptyTableState>{copy.empty}</EmptyTableState>
        ) : (
          <div
            className="br-usage-table-frame w-full overflow-x-auto"
            data-testid="usage-day-table-wrap"
          >
            <table
              className="w-full table-fixed border-collapse text-secondary"
              data-testid="usage-day-table"
            >
              <colgroup>
                <col className="w-[40%]" />
                <col className="w-[16%]" />
                <col className={showDayCost ? 'w-[24%]' : 'w-[44%]'} />
                {showDayCost && <col className="w-[20%]" />}
              </colgroup>
              <thead>
                <tr className="text-left">
                  <th className={TH}>{copy.day}</th>
                  <th className={`${TH} text-right`}>{copy.turns}</th>
                  <th className={`${TH} text-right`}>{copy.billed}</th>
                  {showDayCost && <th className={`${TH} text-right`}>{copy.costColumn}</th>}
                </tr>
              </thead>
              <tbody>
                {orderedDayRows.map((row) => (
                  <tr
                    key={row.date}
                    className={hasUsage(row) ? 'text-text-default' : 'text-text-subtle'}
                  >
                    <td className={TD}>
                      <time dateTime={row.date ?? undefined} title={row.date ?? undefined}>
                        {formatUsageDate(row.date)}
                      </time>
                    </td>
                    <td className={`${TD} text-right text-text-muted tabular-nums`}>
                      {row.turns.toLocaleString('en-US')}
                    </td>
                    <td className={`${TD} text-right tabular-nums`}>
                      <CompactTokenValue
                        value={billedTokenValue(row)}
                        fallback={formatBilledTokens(row)}
                      />
                    </td>
                    {showDayCost && (
                      <td className={`${TD} text-right text-text-muted tabular-nums`}>
                        {formatCostEstimate(row.cost)}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </ReportSection>

      <ReportSection title={copy.byModel}>
        {modelRows.length === 0 ? (
          <EmptyTableState>{copy.empty}</EmptyTableState>
        ) : (
          <div
            className="br-usage-table-frame w-full overflow-x-auto"
            data-testid="usage-model-table-wrap"
          >
            <table
              className="w-full table-fixed border-collapse text-secondary"
              data-testid="usage-model-table"
            >
              <colgroup>
                <col className={showModelCost ? 'w-[32%]' : 'w-[40%]'} />
                <col className="w-[12%]" />
                <col className="w-[28%]" />
                <col className={showModelCost ? 'w-[14%]' : 'w-[20%]'} />
                {showModelCost && <col className="w-[14%]" />}
              </colgroup>
              <thead>
                <tr className="text-left">
                  <th className={TH}>{copy.model}</th>
                  <th className={`${TH} text-right`}>{copy.turns}</th>
                  <th className={TH}>{copy.tokenFlow}</th>
                  <th className={`${TH} text-right`}>{copy.billed}</th>
                  {showModelCost && <th className={`${TH} text-right`}>{copy.costColumn}</th>}
                </tr>
              </thead>
              <tbody>
                {modelRows.map((row, index) => (
                  <tr key={`${modelLabel(row)}-${index}`} className="text-text-default">
                    <td className={TD}>
                      <div className="min-w-0" title={modelLabel(row)}>
                        <p className="truncate">{row.modelId ?? 'unknown'}</p>
                        <p className="truncate text-supporting text-text-subtle">
                          {row.provider ?? copy.unattributedProvider}
                        </p>
                      </div>
                    </td>
                    <td className={`${TD} text-right text-text-muted tabular-nums`}>
                      {row.turns.toLocaleString('en-US')}
                    </td>
                    <td className={`${TD} tabular-nums text-text-muted`}>
                      <TokenFlowCell row={row} showCache={showModelCache} />
                    </td>
                    <td className={`${TD} text-right tabular-nums`}>
                      <CompactTokenValue
                        value={billedTokenValue(row)}
                        fallback={formatBilledTokens(row)}
                      />
                    </td>
                    {showModelCost && (
                      <td className={`${TD} text-right text-text-muted tabular-nums`}>
                        {formatCostEstimate(row.cost)}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </ReportSection>

      {(anyIncompleteTokens || anyUnpriced || anyCostExcludesCache) && (
        <div
          className="flex items-center text-supporting text-text-muted"
          data-testid="usage-data-notes"
        >
          <span>{copy.estimates}</span>
          <InfoTip label={copy.estimatesLabel} help={copy.estimatesHelp} />
          {/* The specific notes, for a screen reader and for tests; the eye gets
              the one line and its tip. */}
          <ul className="sr-only">
            {anyIncompleteTokens && (
              <li data-testid="usage-incomplete-note">{copy.incompleteNote}</li>
            )}
            {anyUnpriced && <li data-testid="usage-unpriced-note">{copy.unpricedNote}</li>}
            {anyCostExcludesCache && (
              <li data-testid="usage-cache-excluded-note">{copy.cacheExcludedNote}</li>
            )}
          </ul>
        </div>
      )}
    </div>
  );
}

interface UsageGaugeProps {
  testid: string;
  label: string;
  used: string;
  limit: string;
  percent: number | null;
  unavailableReason: string | null;
}

/** A flat budget row: the label and `used / limit (pct)`, the bar under them. */
function UsageGauge({ testid, label, used, limit, percent, unavailableReason }: UsageGaugeProps) {
  const pct = percent ?? 0;
  const clamped = Math.max(0, Math.min(100, pct));
  const over = percent != null && percent > 100;
  return (
    <div className="flex min-w-0 flex-col gap-1.5" data-testid={testid}>
      <div className="flex items-baseline justify-between gap-3 text-secondary">
        <span className="text-text-muted">{label}</span>
        <span className="text-text-default tabular-nums">
          {used} / {limit}
          {percent != null && (
            <span className={`ml-1 ${over ? 'text-text-danger' : 'text-text-muted'}`}>
              ({pct.toFixed(1)}%)
            </span>
          )}
        </span>
      </div>
      {unavailableReason ? (
        <p className="text-supporting text-text-muted" data-testid={`${testid}-unavailable`}>
          {unavailableReason}
        </p>
      ) : (
        // `heat` is the ramp pairing this gauge has always used (a heat-0 track
        // under a heat-3 fill). Past the limit only the FILL turns danger —
        // `track="heat"` pins the ground so the overrun reads as the same gauge
        // rather than as a different control. Both are tones on the one primitive.
        <Progress
          label={label}
          value={clamped}
          tone={over ? 'danger' : 'heat'}
          track="heat"
          fillProps={{ 'data-testid': `${testid}-fill` }}
        />
      )}
    </div>
  );
}
