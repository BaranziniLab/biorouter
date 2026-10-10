import * as React from 'react';
import * as TabsPrimitive from '@radix-ui/react-tabs';

import { getUsageReport, type ActivityWindow, type UsageReportRow } from '../../api';
import { HOME_ACTIVITY_DAYS } from '../../utils/homeInsightsCache';
import { userActionHeaders } from '../../utils/userAction';
import { isBrowserSurface } from '../../utils/surface';
import {
  DEFAULT_USAGE_RANGE,
  USAGE_RANGES,
  deriveUsageStats,
  expectedHeatmapWeeks,
  favoriteModel,
  formatCount,
  formatShare,
  formatTokensCompact,
  isUsageRange,
  modelBreakdown,
  rangeStartSeconds,
  streakFootnote,
  type ModelUsage,
  type UsageRange,
} from '../../utils/usageStats';
import { friendlyModelName } from '../bottom_menu/modelLabel';
import { getModelDisplayName } from '../settings/models/predefinedModelsUtils';
import { InfoTip } from '../ui/info-tip';
import { SegmentedControl } from '../ui/segmented-control';
import { Tabs, TabsList, TabsTrigger } from '../ui/tabs';
import { StatGrid, type StatItem } from '../usage/StatGrid';
import { homeUsageCopy as copy } from '../usage/copy';
import '../usage/usage.css';
import { HeatmapLegend, StreakGlyph, UsageHeatmap, UsageHeatmapLoading } from './UsageHeatmap';

/**
 * HomeUsageCard: Home's usage, in the shape of Claude's and Codex's usage
 * cards (owner message 1, screenshot 4), keeping Biorouter's coral heatmap.
 *
 * One flat bordered card on the chat measure:
 *   - a 36px head: underline Tabs (Overview | Models) on the left, the range
 *     (`1y · 30d · 7d`, a SegmentedControl) on the right, over a hairline;
 *   - six flat tiles (StatGrid) or, on Models, up to four model rows;
 *   - the heatmap, which always shows the whole window and never follows the
 *     range;
 *   - one footnote line: the streak on the left, `Less … More` on the right.
 *
 * Explanations are InfoTips; nothing on the card is a paragraph.
 *
 * Two scopes, never presented as one (issue #224): the tiles and the heatmap
 * count chats that still exist (`/sessions/activity`), while Favorite model and
 * the Models tab read the billing report (`/usage/report`), which includes
 * deleted chats and subagents and says so. The report needs the desktop's
 * user-action proof, so a `biorouter serve` browser (403) gets no Models tab
 * and a Longest streak tile in place of Favorite model.
 */

export const HOME_USAGE_RANGE_STORAGE_KEY = 'biorouter-home-usage-range';

type UsageTab = 'overview' | 'models';

function readStoredRange(): UsageRange {
  try {
    const stored = window.localStorage.getItem(HOME_USAGE_RANGE_STORAGE_KEY);
    return isUsageRange(stored) ? stored : DEFAULT_USAGE_RANGE;
  } catch {
    return DEFAULT_USAGE_RANGE;
  }
}

function storeRange(range: UsageRange) {
  try {
    window.localStorage.setItem(HOME_USAGE_RANGE_STORAGE_KEY, range);
  } catch {
    // Blocked storage: the range still applies for this visit.
  }
}

const RANGE_OPTIONS = USAGE_RANGES.map((range) => ({
  value: range,
  label: copy.ranges[range].label,
  ariaLabel: copy.ranges[range].name,
  testId: `home-usage-range-${range}`,
}));

type ReportState =
  | { status: 'loading' }
  | { status: 'ready'; rows: UsageReportRow[] }
  /** 403 on `serve`, or any failure: the card falls back to activity-only tiles. */
  | { status: 'unavailable' };

/**
 * The model report for one range, cached per window end and range for the
 * card's lifetime. A browser surface never asks (the daemon refuses it).
 */
function useModelReport(end: string, range: UsageRange): ReportState {
  const browser = React.useMemo(() => isBrowserSurface(), []);
  const cache = React.useRef(new Map<string, UsageReportRow[]>());
  const [failed, setFailed] = React.useState(false);
  const [, setVersion] = React.useState(0);
  const key = `${end}:${range}`;

  React.useEffect(() => {
    if (browser || failed || cache.current.has(key)) return;
    let cancelled = false;
    void (async () => {
      try {
        const headers = await userActionHeaders();
        const response = await getUsageReport<true>({
          headers,
          query: { from: rangeStartSeconds(end, range), group: 'model' },
          throwOnError: true,
        });
        cache.current.set(key, response.data.rows);
        if (!cancelled) setVersion((version) => version + 1);
      } catch (error) {
        console.error('Failed to load model usage:', error);
        if (!cancelled) setFailed(true);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [browser, failed, key, end, range]);

  if (browser || failed) return { status: 'unavailable' };
  const rows = cache.current.get(key);
  return rows ? { status: 'ready', rows } : { status: 'loading' };
}

/**
 * A model's name as the composer's model chip says it (principle 10): the
 * configured alias when there is one, and the provider's release-date stamp
 * off (`gpt-5.5-2026-04-24` reads `gpt-5.5`). The full `provider/modelId`
 * stays in the tooltip.
 */
function modelName(modelId: string | null | undefined): string {
  return modelId ? friendlyModelName(getModelDisplayName(modelId)) : copy.notRecorded;
}

function modelTitle(row: { modelId?: string | null; provider?: string | null }) {
  if (!row.modelId) return undefined;
  return row.provider ? `${row.provider}/${row.modelId}` : row.modelId;
}

function OverviewTiles({
  activity,
  range,
  report,
}: {
  activity: ActivityWindow;
  range: UsageRange;
  report: ReportState;
}) {
  const stats = React.useMemo(() => deriveUsageStats(activity, range), [activity, range]);

  const tokensItem: StatItem = stats.tokensUnavailable
    ? {
        key: 'tokens',
        label: copy.tokens,
        value: copy.tokensUnavailable,
        help: copy.tokensUnavailableHelp,
      }
    : {
        key: 'tokens',
        label: copy.tokens,
        value: formatTokensCompact(stats.tokens),
        title: stats.tokens >= 10_000 ? formatCount(stats.tokens) : undefined,
        help: stats.tokensComplete ? undefined : copy.tokensPartialHelp,
      };

  let lastItem: StatItem;
  if (report.status === 'unavailable') {
    lastItem = {
      key: 'longest-streak',
      label: copy.longestStreak,
      value: formatCount(stats.longestStreak),
      suffix: copy.dayUnit(stats.longestStreak),
    };
  } else if (report.status === 'loading') {
    lastItem = { key: 'favorite-model', label: copy.favoriteModel, value: copy.pending };
  } else {
    const favorite = favoriteModel(report.rows);
    const name = favorite ? modelName(favorite.modelId) : copy.none;
    lastItem = {
      key: 'favorite-model',
      label: copy.favoriteModel,
      value: name,
      title: favorite ? (modelTitle(favorite) ?? name) : undefined,
    };
  }

  const items: StatItem[] = [
    { key: 'chats', label: copy.chats, value: formatCount(stats.chats) },
    {
      key: 'messages',
      label: copy.messages,
      value: formatCount(stats.messages),
      help: copy.messagesHelp,
    },
    tokensItem,
    {
      key: 'active-days',
      label: copy.activeDays,
      value: formatCount(stats.activeDays),
      suffix: `/${stats.rangeDays}`,
    },
    {
      key: 'streak',
      label: copy.streak,
      value: formatCount(stats.currentStreak),
      suffix: copy.dayUnit(stats.currentStreak),
      help: copy.streakHelp(stats.longestStreak),
    },
    lastItem,
  ];

  return <StatGrid items={items} animateKey={range} data-testid="home-usage-tiles" />;
}

function ModelRow({ model }: { model: ModelUsage }) {
  if (model.unrecorded) {
    // Last, muted and without a share: it says how much usage carries no model
    // name, not which model it was.
    return (
      <li className="br-usage-model" data-testid="home-usage-model" data-unrecorded="">
        <span className="br-usage-model-label">
          <span className="br-usage-model-name text-body text-text-muted">{copy.notRecorded}</span>
          <InfoTip label={copy.notRecorded} help={copy.notRecordedHelp} />
        </span>
        <span aria-hidden="true" />
        <span
          className="br-usage-model-tokens text-body text-text-muted"
          title={model.tokens >= 10_000 ? formatCount(model.tokens) : undefined}
        >
          {formatTokensCompact(model.tokens)}
        </span>
        <span aria-hidden="true" />
      </li>
    );
  }
  const name = model.other ? copy.other : modelName(model.modelId);
  const share = model.share ?? 0;
  const width = `${Math.max(0, Math.min(100, share * 100))}%`;
  return (
    <li className="br-usage-model" data-testid="home-usage-model">
      <span className="br-usage-model-name text-body text-text-default" title={modelTitle(model)}>
        {name}
      </span>
      <span className="br-usage-model-bar" aria-hidden="true">
        <span style={{ width }} />
      </span>
      <span
        className="br-usage-model-tokens text-body text-text-default"
        title={model.tokens >= 10_000 ? formatCount(model.tokens) : undefined}
      >
        {formatTokensCompact(model.tokens)}
      </span>
      <span className="br-usage-model-share text-supporting text-text-muted">
        {formatShare(share)}
      </span>
    </li>
  );
}

function ModelsPanel({ report }: { report: ReportState }) {
  if (report.status !== 'ready') {
    return (
      <div className="br-usage-empty" aria-busy="true">
        <span className="text-supporting text-text-muted">{copy.pending}</span>
      </div>
    );
  }
  const models = modelBreakdown(report.rows);
  if (models.length === 0) {
    return (
      <div className="br-usage-empty">
        <p className="text-supporting text-text-muted">{copy.modelsEmpty}</p>
      </div>
    );
  }
  return (
    <ul className="br-usage-models" aria-label={copy.models}>
      {models.map((model) => (
        <ModelRow key={model.key} model={model} />
      ))}
    </ul>
  );
}

function UsageFootnote({ activity }: { activity: ActivityWindow }) {
  const streak = streakFootnote(activity.currentStreak, activity.longestStreak);
  return (
    <div className="br-usage-card-foot text-supporting text-text-muted">
      {streak ? (
        <span className="br-usage-streak" data-testid="home-usage-streak">
          {activity.currentStreak > 0 ? <StreakGlyph /> : null}
          <span>{streak}</span>
        </span>
      ) : (
        <span />
      )}
      <HeatmapLegend />
    </div>
  );
}

export function HomeUsageCard({ window: activity }: { window: ActivityWindow }) {
  const [range, setRange] = React.useState<UsageRange>(readStoredRange);
  const [tab, setTab] = React.useState<UsageTab>('overview');
  // Panels fade in only once the person has switched tabs; the first is at rest.
  const [tabSwitched, setTabSwitched] = React.useState(false);
  const report = useModelReport(activity.end, range);
  const modelsAvailable = report.status !== 'unavailable';
  const activeTab: UsageTab = modelsAvailable ? tab : 'overview';

  const onRangeChange = (next: UsageRange) => {
    setRange(next);
    storeRange(next);
  };

  return (
    <section aria-label={copy.region} className="br-usage-card" data-testid="home-usage-card">
      <Tabs
        value={activeTab}
        onValueChange={(next) => {
          setTab(next as UsageTab);
          setTabSwitched(true);
        }}
      >
        <div className="br-usage-card-head">
          <div className="br-usage-card-tabs">
            <TabsList className="border-b-0" aria-label={copy.tabsLabel}>
              <TabsTrigger value="overview">{copy.overview}</TabsTrigger>
              {modelsAvailable ? <TabsTrigger value="models">{copy.models}</TabsTrigger> : null}
            </TabsList>
            {modelsAvailable ? <InfoTip label={copy.models} help={copy.modelsHelp} /> : null}
          </div>
          <SegmentedControl
            className="br-usage-card-range"
            aria-label={copy.rangeLabel}
            options={RANGE_OPTIONS}
            value={range}
            onValueChange={onRangeChange}
            data-testid="home-usage-range"
          />
        </div>

        <div className="br-usage-card-body">
          <div className="br-usage-panels">
            {/* `tabIndex={-1}`: Radix makes every panel a Tab stop, and these have
                nothing to scroll and no ring to show for it. The InfoTips inside
                are their own stops. */}
            <TabsPrimitive.Content
              value="overview"
              tabIndex={-1}
              className="br-usage-panel"
              data-animate={tabSwitched ? '' : undefined}
            >
              <OverviewTiles activity={activity} range={range} report={report} />
            </TabsPrimitive.Content>
            {modelsAvailable ? (
              <TabsPrimitive.Content
                value="models"
                tabIndex={-1}
                className="br-usage-panel"
                data-animate={tabSwitched ? '' : undefined}
              >
                <ModelsPanel report={report} />
              </TabsPrimitive.Content>
            ) : null}
          </div>

          <div className="br-usage-card-heatmap">
            <UsageHeatmap window={activity} />
          </div>
          <UsageFootnote activity={activity} />
        </div>
      </Tabs>
    </section>
  );
}

/**
 * The card while the activity window loads: the same shell, the triggers
 * disabled, six tile skeletons and the grid's skeleton. No visible text; the
 * status is named for assistive tech.
 */
export function HomeUsageCardLoading() {
  const weeks = React.useMemo(() => expectedHeatmapWeeks(HOME_ACTIVITY_DAYS), []);
  return (
    <section aria-label={copy.region} className="br-usage-card" data-testid="home-usage-card">
      <Tabs value="overview">
        <div className="br-usage-card-head">
          <div className="br-usage-card-tabs">
            <TabsList className="border-b-0" aria-label={copy.tabsLabel}>
              <TabsTrigger value="overview" disabled>
                {copy.overview}
              </TabsTrigger>
              <TabsTrigger value="models" disabled>
                {copy.models}
              </TabsTrigger>
            </TabsList>
          </div>
          <SegmentedControl
            className="br-usage-card-range"
            aria-label={copy.rangeLabel}
            options={RANGE_OPTIONS}
            value={DEFAULT_USAGE_RANGE}
            onValueChange={() => undefined}
            disabled
          />
        </div>
        <div className="br-usage-card-body" role="status" aria-label={copy.loading}>
          <div className="br-usage-panels" aria-hidden="true">
            <div className="br-stat-grid-frame">
              <div className="br-stat-grid">
                {Array.from({ length: 6 }, (_, index) => (
                  <div key={index} className="br-stat" data-testid="home-usage-tile-skeleton">
                    <span className="br-usage-skeleton biorouter-heatmap-loading-line h-3 w-16" />
                    <span className="br-usage-skeleton biorouter-heatmap-loading-line mt-2 h-6 w-20" />
                  </div>
                ))}
              </div>
            </div>
          </div>
          <div className="br-usage-card-heatmap">
            <UsageHeatmapLoading weeks={weeks} />
          </div>
          <div className="br-usage-card-foot" aria-hidden="true">
            <span />
            <span className="br-usage-skeleton biorouter-heatmap-loading-line h-3 w-24" />
          </div>
        </div>
      </Tabs>
    </section>
  );
}
