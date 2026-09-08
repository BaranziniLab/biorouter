import { type ReactNode, useState } from 'react';
import type { UsageReportRow, UsageSummaryResponse, UsageTotals } from '../../../api';
import {
  billedTokens,
  cacheTokens as combinedCacheTokens,
  knownBilledTokens,
} from '../../../utils/usageAccounting';
import { Button } from '../../ui/button';
import { Badge } from '../../ui/badge';
import { Progress } from '../../ui/progress';
import { Activity, Calendar, Clock, Database, Info, Layers } from '../../icons/app-icons';
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from '../../ui/dialog';

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

export function formatCompactTokens(n: number | null | undefined): string {
  if (typeof n !== 'number' || !Number.isFinite(n) || n < 0) return 'Not recorded';
  if (n < 10_000) return formatTokens(n);
  return new Intl.NumberFormat('en-US', {
    notation: 'compact',
    maximumFractionDigits: 2,
  }).format(n);
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

function SummaryMetric({
  icon,
  label,
  value,
  detail,
}: {
  icon: ReactNode;
  label: string;
  value: string;
  detail: string;
}) {
  return (
    <div className="flex min-w-0 items-start gap-3 rounded-element border border-border-subtle bg-background-default p-3">
      <span className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-element bg-heat-0 text-text-accent">
        {icon}
      </span>
      <div className="min-w-0">
        <p className="text-caps text-text-subtle">{label}</p>
        <p className="mt-0.5 truncate text-subheading leading-tight text-text-default tabular-nums">
          {value}
        </p>
        <p className="mt-1 text-supporting leading-tight text-text-muted">{detail}</p>
      </div>
    </div>
  );
}

function SectionCard({
  icon,
  title,
  description,
  children,
}: {
  icon: ReactNode;
  title: string;
  description: string;
  children: ReactNode;
}) {
  return (
    <section className="overflow-hidden rounded-container border border-border-subtle bg-background-card">
      <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border-subtle bg-background-muted/40 px-4 py-3">
        <div className="flex min-w-0 items-center gap-3">
          <span className="flex h-8 w-8 flex-shrink-0 items-center justify-center rounded-element border border-border-subtle bg-background-default text-text-accent">
            {icon}
          </span>
          <div className="min-w-0">
            <h3 className="text-sm font-medium text-text-default">{title}</h3>
            <p className="mt-0.5 text-supporting text-text-muted">{description}</p>
          </div>
        </div>
      </div>
      {children}
    </section>
  );
}

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

function TokenFlowCell({ row, showCache }: { row: UsageReportRow; showCache: boolean }) {
  const items = [
    { label: 'Fresh in', value: row.inputTokens },
    ...(showCache
      ? [
          { label: 'Cache read', value: row.cacheReadTokens },
          { label: 'Cache write', value: row.cacheCreationTokens },
        ]
      : []),
    { label: 'Out', value: row.outputTokens },
  ];

  return (
    <div
      className={`grid overflow-hidden rounded-element border border-border-subtle bg-background-muted/30 ${showCache ? 'grid-cols-4' : 'grid-cols-2'}`}
      data-testid="usage-model-token-flow"
    >
      {items.map((item, index) => (
        <div
          key={item.label}
          className={`min-w-0 px-2 py-1.5 ${index > 0 ? 'border-l border-border-subtle' : ''}`}
        >
          <p className="whitespace-nowrap text-caps text-text-subtle">{item.label}</p>
          <p className="mt-0.5 truncate text-supporting font-medium text-text-default tabular-nums">
            <CompactTokenValue value={item.value} />
          </p>
        </div>
      ))}
    </div>
  );
}

function EmptyTableState({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <div className="flex flex-col items-center justify-center gap-2 px-4 py-8 text-center text-body text-text-muted">
      <span className="flex h-9 w-9 items-center justify-center rounded-element bg-background-muted text-text-subtle">
        {icon}
      </span>
      <p>{children}</p>
    </div>
  );
}

export function UsagePanel({ summary, dayRows, modelRows }: UsagePanelProps) {
  const [reportOpen, setReportOpen] = useState(false);
  const month = summary.monthToDate;
  const monthTokens = billedTokenValue(month);

  return (
    <div data-testid="usage-panel">
      {/* A settings ROW, not a card. A bordered `--background-card` strip was
          the only boxed ground on the App tab, and the rows either side of it
          bring no fill of their own. */}
      <div className="biorouter-settings-list">
        <div className="biorouter-settings-row flex flex-col gap-3 px-3 py-2.5 sm:flex-row sm:items-center sm:justify-between">
          <div className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-2">
            <Badge tone="accent" variant="chip" className="tabular-nums">
              {summary.month}
            </Badge>
            <dl className="flex min-w-0 flex-wrap items-center gap-x-4 gap-y-1 text-supporting">
              <div className="flex items-baseline gap-1.5">
                <dt className="text-text-muted">Tokens</dt>
                <dd
                  className="font-medium text-text-default tabular-nums"
                  title={formatBilledTokens(month)}
                >
                  {monthTokens === null
                    ? formatBilledTokens(month)
                    : formatCompactTokens(monthTokens)}
                </dd>
              </div>
              <div className="flex items-baseline gap-1.5">
                <dt className="text-text-muted">Est. cost</dt>
                <dd className="font-medium text-text-default tabular-nums">
                  {formatCostEstimate(month.cost)}
                </dd>
              </div>
              <div className="flex items-baseline gap-1.5">
                <dt className="text-text-muted">Turns</dt>
                <dd className="font-medium text-text-default tabular-nums">
                  {month.turns.toLocaleString('en-US')}
                </dd>
              </div>
            </dl>
          </div>
          <Button
            type="button"
            size="sm"
            variant="secondary"
            onClick={() => setReportOpen(true)}
            aria-label="Open detailed usage report"
          >
            View report
          </Button>
        </div>
      </div>

      <Dialog open={reportOpen} onOpenChange={setReportOpen}>
        <DialogContent
          className="grid max-h-[min(820px,calc(100vh-2rem))] w-[calc(100vw-2rem)] grid-rows-[auto_minmax(0,1fr)] gap-0 overflow-hidden p-0 sm:max-w-[1040px]"
          data-testid="usage-report-dialog"
        >
          <DialogHeader className="mb-0 border-b border-border-subtle bg-background-muted/40 px-6 py-5">
            <div className="mb-2 flex items-center gap-2">
              <span className="flex h-8 w-8 items-center justify-center rounded-element border border-border-subtle bg-background-default text-text-accent">
                <Activity className="h-4 w-4" />
              </span>
              <Badge tone="accent" variant="chip" className="tabular-nums">
                {summary.month}
              </Badge>
            </div>
            <DialogTitle>Usage report</DialogTitle>
            <DialogDescription>
              Billed token activity, estimated cost, and model attribution for this month.
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 overflow-y-auto px-6 py-5">
            <UsageReport summary={summary} dayRows={dayRows} modelRows={modelRows} />
          </div>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export function UsageReport({ summary, dayRows, modelRows }: UsagePanelProps) {
  const mtd = summary.monthToDate;
  const anyUnpriced =
    rowHasUnknownCost(mtd) || dayRows.some(rowHasUnknownCost) || modelRows.some(rowHasUnknownCost);
  const showMtdCache = hasRecordedCacheUsage(mtd);
  const showModelCache = modelRows.some(hasRecordedCacheUsage);
  const showDayCost = dayRows.some((row) => hasUsage(row) && hasKnownCost(row));
  const showModelCost = modelRows.some(hasKnownCost);
  const anyIncompleteTokens =
    hasIncompleteTokens(mtd) ||
    dayRows.some(hasIncompleteTokens) ||
    modelRows.some(hasIncompleteTokens);
  const anyCostExcludesCache =
    mtd.costExcludesCache ||
    dayRows.some((row) => row.costExcludesCache) ||
    modelRows.some((row) => row.costExcludesCache);
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

  return (
    <div className="flex flex-col gap-5" data-testid="usage-report">
      <section
        className="overflow-hidden rounded-container border border-border-subtle bg-background-card"
        data-testid="usage-summary-card"
      >
        <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border-subtle bg-background-muted/40 px-4 py-3">
          <div>
            <h3 className="text-sm font-medium text-text-default">Month to date</h3>
            <p className="mt-0.5 text-supporting text-text-muted">
              Billed usage recorded across all chats
            </p>
          </div>
          <Badge tone="accent" variant="chip" className="tabular-nums">
            {summary.month}
          </Badge>
        </div>

        <div className="grid grid-cols-1 gap-2.5 p-3 sm:grid-cols-3">
          <SummaryMetric
            icon={<Database className="h-4 w-4" />}
            label="Billed tokens"
            value={formatBilledTokens(mtd)}
            detail={
              hasIncompleteTokens(mtd) ? 'Known conservative subtotal' : 'Recorded this month'
            }
          />
          <SummaryMetric
            icon={<Activity className="h-4 w-4" />}
            label="Estimated cost"
            value={formatCostEstimate(mtd.cost)}
            detail={mtdCostPartial ? 'Known conservative subtotal' : 'Based on stored pricing'}
          />
          <SummaryMetric
            icon={<Clock className="h-4 w-4" />}
            label="Turns"
            value={mtd.turns.toLocaleString('en-US')}
            detail="Completed model turns"
          />
        </div>

        {showMtdCache && (
          <div
            className="flex flex-wrap gap-2 border-t border-border-subtle px-3 py-2.5"
            data-testid="usage-mtd-cache"
          >
            <Badge tone="neutral" variant="chip" className="gap-1.5 font-normal tabular-nums">
              <span className="text-text-subtle">Cache read</span>
              <span className="text-text-default">{formatTokens(mtd.cacheReadTokens)}</span>
            </Badge>
            <Badge tone="neutral" variant="chip" className="gap-1.5 font-normal tabular-nums">
              <span className="text-text-subtle">Cache write</span>
              <span className="text-text-default">{formatTokens(mtd.cacheCreationTokens)}</span>
            </Badge>
          </div>
        )}

        {(summary.monthlyTokenLimit != null || summary.monthlyDollarLimit != null) && (
          <div className="grid grid-cols-1 gap-2.5 border-t border-border-subtle bg-background-muted/20 p-3 md:grid-cols-2">
            {summary.monthlyTokenLimit != null && (
              <UsageGauge
                testid="usage-gauge-tokens"
                label="Token budget"
                used={formatBilledTokens(mtd)}
                limit={formatTokens(summary.monthlyTokenLimit)}
                percent={tokenPercent}
                unavailableReason={tokenUnavailableReason}
              />
            )}
            {summary.monthlyDollarLimit != null && (
              <UsageGauge
                testid="usage-gauge-dollars"
                label="Dollar budget"
                used={formatCostEstimate(mtd.cost)}
                limit={`$${summary.monthlyDollarLimit.toFixed(2)}`}
                percent={dollarPercent}
                unavailableReason={dollarUnavailableReason}
              />
            )}
          </div>
        )}
      </section>

      <SectionCard
        icon={<Calendar className="h-4 w-4" />}
        title="By day"
        description="Calendar activity for the current month"
      >
        {dayRows.length === 0 ? (
          <EmptyTableState icon={<Calendar className="h-4 w-4" />}>
            No usage this month.
          </EmptyTableState>
        ) : (
          <div className="w-full overflow-x-auto" data-testid="usage-day-table-wrap">
            <table
              className="w-full min-w-[520px] border-collapse text-body"
              data-testid="usage-day-table"
            >
              <colgroup>
                <col className="w-[36%]" />
                <col className="w-[18%]" />
                <col className={showDayCost ? 'w-[26%]' : 'w-[46%]'} />
                {showDayCost && <col className="w-[20%]" />}
              </colgroup>
              <thead>
                <tr className="text-left text-label text-text-muted">
                  <th className="px-3 py-2">Day</th>
                  <th className="px-3 py-2 text-right">Turns</th>
                  <th className="px-3 py-2 text-right">Billed</th>
                  {showDayCost && <th className="px-3 py-2 text-right">Cost</th>}
                </tr>
              </thead>
              <tbody>
                {orderedDayRows.map((row) => (
                  <tr
                    key={row.date}
                    className={`tint-interactive transition-colors ${hasUsage(row) ? 'text-text-default' : 'text-text-subtle'}`}
                  >
                    <td className="border-t border-border-subtle px-3 py-2 font-medium">
                      <time dateTime={row.date ?? undefined} title={row.date ?? undefined}>
                        {formatUsageDate(row.date)}
                      </time>
                    </td>
                    <td className="border-t border-border-subtle px-3 py-2 text-right text-text-muted tabular-nums">
                      {row.turns.toLocaleString('en-US')}
                    </td>
                    <td className="border-t border-border-subtle px-3 py-2 text-right font-medium tabular-nums">
                      <CompactTokenValue
                        value={billedTokenValue(row)}
                        fallback={formatBilledTokens(row)}
                      />
                    </td>
                    {showDayCost && (
                      <td className="border-t border-border-subtle px-3 py-2 text-right text-text-muted tabular-nums">
                        {formatCostEstimate(row.cost)}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </SectionCard>

      <SectionCard
        icon={<Layers className="h-4 w-4" />}
        title="By model"
        description="Provider attribution and the token flow behind each billed total"
      >
        {modelRows.length === 0 ? (
          <EmptyTableState icon={<Layers className="h-4 w-4" />}>
            No usage this month.
          </EmptyTableState>
        ) : (
          <div className="w-full overflow-x-auto" data-testid="usage-model-table-wrap">
            <table
              className={`w-full border-collapse text-body ${showModelCache ? 'min-w-[900px]' : 'min-w-[720px]'}`}
              data-testid="usage-model-table"
            >
              <colgroup>
                <col className="w-[27%]" />
                <col className="w-[9%]" />
                <col className={showModelCache ? 'w-[40%]' : 'w-[36%]'} />
                <col className={showModelCost ? 'w-[13%]' : 'w-[22%]'} />
                {showModelCost && <col className="w-[11%]" />}
              </colgroup>
              <thead>
                <tr className="text-left text-label text-text-muted">
                  <th className="px-3 py-2">Model</th>
                  <th className="px-3 py-2 text-right">Turns</th>
                  <th className="px-3 py-2">Token flow</th>
                  <th className="px-3 py-2 text-right">Billed</th>
                  {showModelCost && <th className="px-3 py-2 text-right">Cost</th>}
                </tr>
              </thead>
              <tbody>
                {modelRows.map((row, index) => (
                  <tr
                    key={`${modelLabel(row)}-${index}`}
                    className="tint-interactive text-text-default transition-colors"
                  >
                    <td className="border-t border-border-subtle px-3 py-2">
                      <div className="min-w-0" title={modelLabel(row)}>
                        <p className="truncate font-mono text-supporting font-medium">
                          {row.modelId ?? 'unknown'}
                        </p>
                        <p className="mt-1 truncate text-caps text-text-subtle">
                          {row.provider ?? 'Unattributed provider'}
                        </p>
                      </div>
                    </td>
                    <td className="border-t border-border-subtle px-3 py-2 text-right text-text-muted tabular-nums">
                      {row.turns.toLocaleString('en-US')}
                    </td>
                    <td className="border-t border-border-subtle px-3 py-2">
                      <TokenFlowCell row={row} showCache={showModelCache} />
                    </td>
                    <td className="border-t border-border-subtle px-3 py-2 text-right font-medium tabular-nums">
                      <CompactTokenValue
                        value={billedTokenValue(row)}
                        fallback={formatBilledTokens(row)}
                      />
                    </td>
                    {showModelCost && (
                      <td className="border-t border-border-subtle px-3 py-2 text-right text-text-muted tabular-nums">
                        {formatCostEstimate(row.cost)}
                      </td>
                    )}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </SectionCard>

      {(anyIncompleteTokens || anyUnpriced || anyCostExcludesCache) && (
        <aside
          className="flex items-start gap-3 rounded-container border border-border-subtle bg-background-muted/40 px-4 py-3"
          aria-label="Usage data notes"
          data-testid="usage-data-notes"
        >
          <span className="mt-0.5 flex h-7 w-7 flex-shrink-0 items-center justify-center rounded-element bg-background-default text-text-muted">
            <Info className="h-3.5 w-3.5" />
          </span>
          <div className="min-w-0">
            <p className="text-label text-text-default">About these totals</p>
            <ul className="mt-1.5 space-y-1 text-supporting leading-relaxed text-text-muted">
              {anyIncompleteTokens && (
                <li data-testid="usage-incomplete-note" role="status">
                  Some historical token details were not recorded. Totals are conservative, and
                  empty cache or cost fields stay hidden.
                </li>
              )}
              {anyUnpriced && (
                <li data-testid="usage-unpriced-note">
                  Some historical usage has no stored model or pricing attribution, so its cost
                  cannot be recovered.
                </li>
              )}
              {anyCostExcludesCache && (
                <li data-testid="usage-cache-excluded-note">
                  Some cache cost is unavailable because cache pricing or historical accounting is
                  incomplete.
                </li>
              )}
            </ul>
          </div>
        </aside>
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

function UsageGauge({ testid, label, used, limit, percent, unavailableReason }: UsageGaugeProps) {
  const pct = percent ?? 0;
  const clamped = Math.max(0, Math.min(100, pct));
  const over = percent != null && percent > 100;
  return (
    <div
      className="rounded-element border border-border-subtle bg-background-default p-3"
      data-testid={testid}
    >
      <div className="flex items-baseline justify-between text-supporting">
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
        <div
          className="mt-1 rounded-element border border-border-subtle bg-background-medium px-2 py-1 text-supporting text-text-muted"
          role="status"
          data-testid={`${testid}-unavailable`}
        >
          {unavailableReason}
        </div>
      ) : (
        // `heat` is the ramp pairing this gauge has always used (a heat-0 track
        // under a heat-3 fill). Past the limit only the FILL turns danger —
        // `track="heat"` pins the ground so the overrun reads as the same gauge
        // rather than as a different control. Both are tones on the one primitive.
        <Progress
          className="mt-1"
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
