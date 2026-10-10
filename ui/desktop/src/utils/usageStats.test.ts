import { describe, expect, it } from 'vitest';
import type { ActivityWindow, DailyActivity, UsageReportRow } from '../api';
import {
  DEFAULT_USAGE_RANGE,
  USAGE_RANGES,
  deriveUsageStats,
  expectedHeatmapWeeks,
  favoriteModel,
  formatShare,
  formatTokensCompact,
  heatmapWeeks,
  isUsageRange,
  modelBreakdown,
  rangeStart,
  rangeStartSeconds,
  streakFootnote,
} from './usageStats';

const day = (
  date: string,
  { sessions = 1, tokens = 1000, messages = 4, tokensComplete = true } = {}
): DailyActivity => ({
  date,
  sessions,
  tokens,
  tokensComplete,
  inputTokens: 0,
  outputTokens: 0,
  messages,
  level: 1,
});

function windowOf(overrides: Partial<ActivityWindow> = {}): ActivityWindow {
  return {
    start: '2025-10-04',
    end: '2026-10-09',
    maxSessions: 0,
    maxTokens: 0,
    tokensComplete: true,
    currentStreak: 0,
    longestStreak: 0,
    days: [],
    ...overrides,
  };
}

const row = (
  modelId: string | null,
  totalTokens: number | null,
  turns = 1,
  extra: Partial<UsageReportRow> = {}
): UsageReportRow => ({
  date: null,
  modelId,
  provider: modelId ? 'openai' : null,
  inputTokens: totalTokens ?? 0,
  outputTokens: 0,
  totalTokens,
  cacheReadTokens: 0,
  cacheCreationTokens: 0,
  turns,
  cost: null,
  hasUnpriced: false,
  costExcludesCache: false,
  ...extra,
});

describe('ranges', () => {
  it('offers 1y, 30d and 7d, and never an "All" over a partial window', () => {
    expect(USAGE_RANGES).toEqual(['1y', '30d', '7d']);
    expect(DEFAULT_USAGE_RANGE).toBe('1y');
    expect(isUsageRange('30d')).toBe(true);
    expect(isUsageRange('all')).toBe(false);
    expect(isUsageRange(null)).toBe(false);
  });

  it('counts calendar days ending on the window end, inclusive', () => {
    expect(rangeStart('2026-10-09', '7d')).toBe('2026-10-03');
    expect(rangeStart('2026-10-09', '30d')).toBe('2026-09-10');
    expect(rangeStart('2026-10-09', '1y')).toBe('2025-10-10');
    // Across a month end and a leap day.
    expect(rangeStart('2024-03-02', '7d')).toBe('2024-02-25');
  });

  it('turns the start into local-midnight unix seconds for the report query', () => {
    const expected = Math.floor(new Date(2026, 9, 3).getTime() / 1000);
    expect(rangeStartSeconds('2026-10-09', '7d')).toBe(expected);
  });
});

describe('heatmap columns', () => {
  it('counts one column per week, the first padded back to Sunday', () => {
    // Sunday to Saturday: exactly two weeks.
    expect(heatmapWeeks('2026-03-01', '2026-03-14')).toBe(2);
    // A Saturday start pads six days back, and the ragged last week is a column.
    expect(heatmapWeeks('2025-10-04', '2026-10-09')).toBe(54);
    // A 371-day window that starts on a Sunday is exactly 53 weeks.
    expect(heatmapWeeks('2025-10-05', '2026-10-10')).toBe(53);
  });

  it('predicts the daemon window from today, so the loading grid is as wide as the real one', () => {
    // Fri 2026-10-09: the window starts on a Saturday.
    expect(expectedHeatmapWeeks(371, new Date(2026, 9, 9))).toBe(54);
    // Sat 2026-10-10: it starts on a Sunday.
    expect(expectedHeatmapWeeks(371, new Date(2026, 9, 10))).toBe(53);
    // A window whose end is a Sunday starts on a Monday.
    expect(expectedHeatmapWeeks(371, new Date(2026, 9, 11))).toBe(54);
  });
});

describe('deriveUsageStats', () => {
  const window = windowOf({
    currentStreak: 2,
    longestStreak: 9,
    days: [
      day('2025-10-09', { sessions: 9, tokens: 9 }), // outside even 1y
      day('2025-12-01', { sessions: 2, tokens: 200, messages: 10 }),
      day('2026-09-15', { sessions: 1, tokens: 50, messages: 3 }),
      day('2026-10-03', { sessions: 3, tokens: 300, messages: 7 }),
      day('2026-10-09', { sessions: 1, tokens: 100, messages: 2 }),
    ],
  });

  it('sums chats, messages and tokens inside the range only', () => {
    expect(deriveUsageStats(window, '7d')).toMatchObject({
      chats: 4,
      messages: 9,
      tokens: 400,
      activeDays: 2,
      rangeDays: 7,
      tokensComplete: true,
      tokensUnavailable: false,
    });
    expect(deriveUsageStats(window, '30d')).toMatchObject({ chats: 5, tokens: 450, activeDays: 3 });
    expect(deriveUsageStats(window, '1y')).toMatchObject({
      chats: 7,
      tokens: 650,
      activeDays: 4,
      rangeDays: 365,
    });
  });

  it('carries the streaks through unchanged', () => {
    expect(deriveUsageStats(window, '7d')).toMatchObject({ currentStreak: 2, longestStreak: 9 });
  });

  it('marks the total a lower bound when any day in range is incomplete', () => {
    const partial = windowOf({
      days: [day('2026-10-08', { tokens: 40, tokensComplete: false }), day('2026-10-09')],
    });
    expect(deriveUsageStats(partial, '7d')).toMatchObject({
      tokens: 1040,
      tokensComplete: false,
      tokensUnavailable: false,
    });
  });

  it('ignores an incomplete day outside the range', () => {
    const partial = windowOf({
      days: [day('2026-01-01', { tokensComplete: false }), day('2026-10-09')],
    });
    expect(deriveUsageStats(partial, '7d').tokensComplete).toBe(true);
    expect(deriveUsageStats(partial, '1y').tokensComplete).toBe(false);
  });

  it('calls a zero total over incomplete records unavailable, not zero', () => {
    const unknown = windowOf({ days: [day('2026-10-09', { tokens: 0, tokensComplete: false })] });
    expect(deriveUsageStats(unknown, '7d')).toMatchObject({
      tokens: 0,
      tokensUnavailable: true,
    });
  });

  it('answers zeros for an empty window', () => {
    expect(deriveUsageStats(windowOf(), '30d')).toMatchObject({
      chats: 0,
      messages: 0,
      tokens: 0,
      activeDays: 0,
      tokensComplete: true,
      tokensUnavailable: false,
    });
  });
});

describe('formatTokensCompact', () => {
  it('keeps one fraction digit and spells out small numbers', () => {
    expect(formatTokensCompact(87_900_000_000)).toBe('87.9B');
    expect(formatTokensCompact(29_350_000)).toBe('29.4M');
    expect(formatTokensCompact(128_402)).toBe('128.4K');
    expect(formatTokensCompact(3_000_000)).toBe('3M');
    expect(formatTokensCompact(9_999)).toBe('9,999');
    expect(formatTokensCompact(0)).toBe('0');
  });
});

describe('streakFootnote', () => {
  it('says the current streak and the longest', () => {
    expect(streakFootnote(3, 16)).toBe('3 day streak · longest 16 days');
    expect(streakFootnote(1, 1)).toBe('1 day streak · longest 1 day');
  });

  it('says only the longest when there is no current streak', () => {
    expect(streakFootnote(0, 16)).toBe('Longest streak 16 days');
    expect(streakFootnote(0, 1)).toBe('Longest streak 1 day');
  });

  it('says nothing when both are zero', () => {
    expect(streakFootnote(0, 0)).toBe('');
  });
});

describe('favoriteModel', () => {
  it('picks the model with the most tokens, then the most turns', () => {
    expect(favoriteModel([row('a', 10), row('b', 30), row('c', 20)])?.modelId).toBe('b');
    expect(favoriteModel([row('a', 10, 1), row('b', 10, 4)])?.modelId).toBe('b');
  });

  it('falls back to the known subtotal when the total is uncertified', () => {
    expect(
      favoriteModel([row('a', 10), row('b', null, 1, { inputTokens: 50, outputTokens: 5 })])
        ?.modelId
    ).toBe('b');
  });

  it('lets the unattributed bucket win only when it is the only model', () => {
    expect(favoriteModel([row(null, 500), row('a', 10)])?.modelId).toBe('a');
    expect(favoriteModel([row(null, 500)])?.modelId).toBeNull();
  });

  it('answers null when nothing was used', () => {
    expect(favoriteModel([])).toBeNull();
    expect(favoriteModel([row('a', 0, 0)])).toBeNull();
  });
});

describe('modelBreakdown', () => {
  it('lists up to four models by tokens with their shares', () => {
    const models = modelBreakdown([row('a', 100), row('b', 300), row('c', 0, 0)]);
    expect(models.map((m) => m.modelId)).toEqual(['b', 'a']);
    expect(models.map((m) => m.share)).toEqual([0.75, 0.25]);
    expect(models.every((m) => !m.other)).toBe(true);
  });

  it('folds everything past the top three into one Other row', () => {
    const models = modelBreakdown([
      row('a', 50),
      row('b', 40),
      row('c', 30),
      row('d', 20, 2),
      row('e', 10, 3),
    ]);
    expect(models).toHaveLength(4);
    expect(models.slice(0, 3).map((m) => m.modelId)).toEqual(['a', 'b', 'c']);
    expect(models[3]).toMatchObject({ other: true, modelId: null, tokens: 30, turns: 5 });
    expect(models.reduce((sum, m) => sum + m.share, 0)).toBeCloseTo(1);
    expect(new Set(models.map((m) => m.key)).size).toBe(4);
  });

  it('is empty when no model was used', () => {
    expect(modelBreakdown([])).toEqual([]);
  });
});

describe('formatShare', () => {
  it('rounds to whole percents and keeps a sliver visible', () => {
    expect(formatShare(0.625)).toBe('63%');
    expect(formatShare(0.001)).toBe('<1%');
    expect(formatShare(0)).toBe('0%');
  });
});
