import type { ActivityWindow, UsageReportRow } from '../api';
import { billedTokens, knownBilledTokens } from './usageAccounting';

/**
 * The arithmetic behind Home's usage card, free of React and the DOM (the
 * `messageClamp.ts` rule: a number you can only check by rendering a component
 * is one nobody re-checks).
 *
 * Two scopes, never mixed (issue #224):
 * - `ActivityWindow` (`GET /sessions/activity`) is the ACTIVITY view: chats that
 *   still exist, started per local day. It feeds the tiles and the heatmap.
 * - `UsageReportRow` (`GET /usage/report`) is the BILLING view: it includes
 *   deleted chats and subagent turns. It feeds Favorite model and the Models
 *   tab only, which say so in their help.
 *
 * The range is a frontend filter over one 371-day window (owner decision 6.4):
 * `1y` is the last 365 days, so it is never labelled "All".
 */

export type UsageRange = '1y' | '30d' | '7d';

/** Display order, left to right. */
export const USAGE_RANGES: readonly UsageRange[] = ['1y', '30d', '7d'];

export const DEFAULT_USAGE_RANGE: UsageRange = '1y';

/** Calendar days in each range, today included. */
export const USAGE_RANGE_DAYS: Readonly<Record<UsageRange, number>> = {
  '1y': 365,
  '30d': 30,
  '7d': 7,
};

export function isUsageRange(value: unknown): value is UsageRange {
  return typeof value === 'string' && (USAGE_RANGES as readonly string[]).includes(value);
}

/** Local calendar day, `YYYY-MM-DD`, matching the server's `date('now','localtime')`. */
export function isoDay(date: Date): string {
  const y = date.getFullYear();
  const m = String(date.getMonth() + 1).padStart(2, '0');
  const d = String(date.getDate()).padStart(2, '0');
  return `${y}-${m}-${d}`;
}

/** Local midnight of a `YYYY-MM-DD` day. */
export function parseIsoDay(day: string): Date {
  const [y, m, d] = day.split('-').map(Number);
  return new Date(y, m - 1, d);
}

/** The first day of `range`, inclusive, ending on `end` (a `YYYY-MM-DD` day). */
export function rangeStart(end: string, range: UsageRange): string {
  const start = parseIsoDay(end);
  start.setDate(start.getDate() - (USAGE_RANGE_DAYS[range] - 1));
  return isoDay(start);
}

/** `rangeStart` as unix seconds at local midnight, for `/usage/report?from=`. */
export function rangeStartSeconds(end: string, range: UsageRange): number {
  return Math.floor(parseIsoDay(rangeStart(end, range)).getTime() / 1000);
}

const DAY_MS = 86_400_000;

/**
 * Columns the heatmap draws for a window: one per week, the first padded back
 * to Sunday. A 371-day window is 53 columns only when it starts on a Sunday and
 * 54 on every other weekday, so the count is computed and never written down.
 */
export function heatmapWeeks(start: string, end: string): number {
  const first = parseIsoDay(start);
  first.setDate(first.getDate() - first.getDay());
  const total = Math.round((parseIsoDay(end).getTime() - first.getTime()) / DAY_MS) + 1;
  return Math.ceil(total / 7);
}

/**
 * The columns the daemon's `days`-day window ending on `today` will fill. The
 * loading grid draws this many, so the card keeps its width and height when the
 * real heatmap replaces it.
 */
export function expectedHeatmapWeeks(days: number, today: Date = new Date()): number {
  const end = isoDay(today);
  const start = parseIsoDay(end);
  start.setDate(start.getDate() - (days - 1));
  return heatmapWeeks(isoDay(start), end);
}

export interface UsageStats {
  /** Chats started in range. */
  chats: number;
  /**
   * Messages in range. A lower bound: the daemon drops a day that has messages
   * but no new chat and no billed tokens (`build_activity_window`).
   */
  messages: number;
  /** Billed tokens in range; a known subtotal when `tokensComplete` is false. */
  tokens: number;
  /** False when any active day in range lacks full token records. */
  tokensComplete: boolean;
  /** No trustworthy total: the sum is 0 and some day is incomplete. */
  tokensUnavailable: boolean;
  /** Days in range with a new chat or billed tokens (the server's definition). */
  activeDays: number;
  /** The range's length in days, the denominator of `activeDays`. */
  rangeDays: number;
  currentStreak: number;
  /** Longest run inside the fetched window only, not all time. */
  longestStreak: number;
}

export function deriveUsageStats(window: ActivityWindow, range: UsageRange): UsageStats {
  const start = rangeStart(window.end, range);
  let chats = 0;
  let messages = 0;
  let tokens = 0;
  let activeDays = 0;
  let tokensComplete = true;
  for (const day of window.days) {
    // ISO days compare correctly as strings.
    if (day.date < start || day.date > window.end) continue;
    chats += day.sessions;
    messages += day.messages;
    tokens += day.tokens;
    activeDays += 1;
    if (!day.tokensComplete) tokensComplete = false;
  }
  return {
    chats,
    messages,
    tokens,
    tokensComplete,
    tokensUnavailable: !tokensComplete && tokens === 0,
    activeDays,
    rangeDays: USAGE_RANGE_DAYS[range],
    currentStreak: window.currentStreak,
    longestStreak: window.longestStreak,
  };
}

const COMPACT = new Intl.NumberFormat('en-US', { notation: 'compact', maximumFractionDigits: 1 });
const FULL = new Intl.NumberFormat('en-US');

/** `230,115`. */
export function formatCount(value: number): string {
  return FULL.format(value);
}

/** `87.9B`, `29.4M`, `128.4K`; the full number below 10,000. */
export function formatTokensCompact(value: number): string {
  if (!Number.isFinite(value) || value < 0) return '0';
  if (value < 10_000) return FULL.format(value);
  return COMPACT.format(value);
}

function days(count: number): string {
  return count === 1 ? '1 day' : `${formatCount(count)} days`;
}

/**
 * The footnote under the heatmap: `3 day streak · longest 16 days`, or
 * `Longest streak 16 days` when there is no current streak, or nothing at all.
 */
export function streakFootnote(currentStreak: number, longestStreak: number): string {
  const longest = Math.max(longestStreak, currentStreak);
  if (currentStreak <= 0 && longest <= 0) return '';
  if (currentStreak <= 0) return `Longest streak ${days(longest)}`;
  return `${formatCount(currentStreak)} day streak · longest ${days(longest)}`;
}

/** A report row's tokens: the certified total, else the known subtotal. */
export function reportRowTokens(row: UsageReportRow): number {
  return billedTokens(row) ?? knownBilledTokens(row);
}

export interface ModelUsage {
  /** Stable React key. */
  key: string;
  /** Null for `Other` and for the unrecorded row. */
  modelId: string | null;
  provider: string | null;
  /** True for the `Other` row that folds the named models past the top rows. */
  other: boolean;
  /**
   * True for the one row holding usage saved without a model name. It is
   * always last and carries no share: it says nothing about which models were
   * used.
   */
  unrecorded: boolean;
  tokens: number;
  turns: number;
  /**
   * Fraction of the NAMED models' tokens in range, 0 to 1; null on the
   * unrecorded row.
   */
  share: number | null;
}

function hasUsage(row: UsageReportRow): boolean {
  return reportRowTokens(row) > 0 || row.turns > 0;
}

function byUsage(a: UsageReportRow, b: UsageReportRow): number {
  return reportRowTokens(b) - reportRowTokens(a) || b.turns - a.turns;
}

/**
 * The model with the most tokens in range (ties: more turns). The unattributed
 * bucket wins only when it is the only model there is.
 */
export function favoriteModel(rows: readonly UsageReportRow[]): UsageReportRow | null {
  const used = rows.filter(hasUsage).sort(byUsage);
  return used.find((row) => row.modelId) ?? used[0] ?? null;
}

/** At most this many rows in the Models tab. */
export const MODEL_ROWS_MAX = 4;

function sumTokens(rows: readonly UsageReportRow[]): number {
  return rows.reduce((sum, row) => sum + reportRowTokens(row), 0);
}

function sumTurns(rows: readonly UsageReportRow[]): number {
  return rows.reduce((sum, row) => sum + row.turns, 0);
}

/**
 * The Models tab, which agrees with Favorite model about the top row.
 *
 * Named models come first, by tokens, and their shares are of the named
 * models' tokens only. Usage saved without a model name (the report's
 * unattributed bucket, often most of an older history) is folded into ONE
 * `unrecorded` row, ranked last whatever its size and given no share: leading
 * the list with it, as the tab used to, made Overview and Models disagree
 * about the top model and dwarfed every real bar.
 *
 * At most four rows: when the named models do not fit beside the unrecorded
 * row, the top ones stay and one `Other` row holds the rest.
 */
export function modelBreakdown(rows: readonly UsageReportRow[]): ModelUsage[] {
  const used = rows.filter(hasUsage);
  const named = used.filter((row) => row.modelId).sort(byUsage);
  const unrecorded = used.filter((row) => !row.modelId);
  const namedTotal = sumTokens(named);
  const share = (tokens: number) => (namedTotal > 0 ? tokens / namedTotal : 0);
  const toUsage = (row: UsageReportRow): ModelUsage => {
    const tokens = reportRowTokens(row);
    return {
      key: `${row.provider ?? ''}/${row.modelId}`,
      modelId: row.modelId ?? null,
      provider: row.provider ?? null,
      other: false,
      unrecorded: false,
      tokens,
      turns: row.turns,
      share: share(tokens),
    };
  };

  const slots = MODEL_ROWS_MAX - (unrecorded.length > 0 ? 1 : 0);
  let models: ModelUsage[];
  if (named.length <= slots) {
    models = named.map(toUsage);
  } else {
    const rest = named.slice(slots - 1);
    const restTokens = sumTokens(rest);
    models = [
      ...named.slice(0, slots - 1).map(toUsage),
      {
        key: 'other',
        modelId: null,
        provider: null,
        other: true,
        unrecorded: false,
        tokens: restTokens,
        turns: sumTurns(rest),
        share: share(restTokens),
      },
    ];
  }

  if (unrecorded.length > 0) {
    models.push({
      key: 'unrecorded',
      modelId: null,
      provider: null,
      other: false,
      unrecorded: true,
      tokens: sumTokens(unrecorded),
      turns: sumTurns(unrecorded),
      share: null,
    });
  }
  return models;
}

/** `62%`; `<1%` for a sliver that would otherwise round to nothing. */
export function formatShare(share: number): string {
  if (!(share > 0)) return '0%';
  const percent = Math.round(share * 100);
  return percent < 1 ? '<1%' : `${percent}%`;
}
