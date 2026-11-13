import { describe, expect, it } from 'vitest';
import {
  parseDecisionSettings,
  parseSessionDecisionSummary,
  type DecisionSettings,
} from '../../utils/sessionDecisions';
import { decisionsHeadline, monitorIdsOf, summaryDecisionRows } from './summaryDecisionRows';

/** Each reshaping rule of the summary spec's section 2.1 (A-1). */

const settings = (ids: string[], off: string[] = []): DecisionSettings =>
  parseDecisionSettings({
    defaultModel: null,
    monitors: ids.map((id) => ({
      id,
      name: `Monitor ${id}`,
      question: { instructions: `Is ${id} true?`, kind: { binary: {} } },
      threshold: 0.8,
      action: 'notify',
      enabled: !off.includes(id),
    })),
  })!;

const aggregate = (
  origin: string,
  originId: string | null,
  count: number,
  over: Record<string, unknown> = {}
) => ({
  origin,
  originId,
  label: originId ?? origin,
  count,
  passed: 0,
  failed: count,
  errors: 0,
  lastAt: '2026-11-04T10:00:00Z',
  lastAnswer: 'no',
  lastPassProbability: 0.2,
  ...over,
});

const summary = (aggregates: Record<string, unknown>[]) =>
  parseSessionDecisionSummary({
    total: aggregates.reduce((sum, a) => sum + (a.count as number), 0),
    aggregates,
    recentTurns: [],
  })!;

describe('monitorIdsOf', () => {
  it('splits a combined call into its monitors', () => {
    expect(monitorIdsOf('a+b')).toEqual(['a', 'b']);
    expect(monitorIdsOf('a')).toEqual(['a']);
    expect(monitorIdsOf(null)).toEqual([]);
  });
});

describe('summaryDecisionRows', () => {
  it('draws one row per monitor that exists now, in settings order, checked or not', () => {
    const rows = summaryDecisionRows(
      summary([aggregate('monitor', 'b', 3)]),
      settings(['c', 'b', 'a'])
    );
    expect(rows.map((row) => row.key)).toEqual(['monitor:c', 'monitor:b', 'monitor:a']);
    expect(rows.map((row) => row.state)).toEqual(['pending', 'quiet', 'pending']);
    expect(rows.map((row) => row.label)).toEqual(['Monitor c', 'Monitor b', 'Monitor a']);
  });

  it('counts a combined call on each monitor it asked, without its answer', () => {
    const rows = summaryDecisionRows(
      summary([
        aggregate('monitor', 'a', 2),
        // A combined record whose probability is over the threshold: it cannot
        // say which monitor that belongs to, so it fires neither.
        aggregate('monitor', 'a+b', 5, {
          lastPassProbability: 0.99,
          lastAt: '2026-11-05T00:00:00Z',
        }),
      ]),
      settings(['a', 'b'])
    );
    expect(rows.map((row) => [row.key, row.count, row.state])).toEqual([
      ['monitor:a', 7, 'quiet'],
      ['monitor:b', 5, 'quiet'],
    ]);
    expect(rows.some((row) => row.label.includes('+'))).toBe(false);
  });

  it('carries a combined call’s errors to each monitor', () => {
    const rows = summaryDecisionRows(
      summary([aggregate('monitor', 'a+b', 2, { errors: 2, lastPassProbability: null })]),
      settings(['a', 'b'])
    );
    expect(rows.map((row) => [row.key, row.errors, row.state])).toEqual([
      ['monitor:a', 2, 'error'],
      ['monitor:b', 2, 'error'],
    ]);
  });

  it('fires a monitor the newest marker names, combined or not', () => {
    const rows = summaryDecisionRows(
      summary([aggregate('monitor', 'a+b', 1)]),
      settings(['a', 'b', 'c']),
      {
        fired: new Set(['a+b']),
      }
    );
    expect(rows.map((row) => row.state)).toEqual(['fired', 'fired', 'pending']);
  });

  it('leaves out a removed monitor, alone or in a combined call', () => {
    const rows = summaryDecisionRows(
      summary([aggregate('monitor', 'gone', 4), aggregate('monitor', 'a+gone', 2)]),
      settings(['a'])
    );
    expect(rows.map((row) => [row.key, row.count])).toEqual([['monitor:a', 2]]);
  });

  it('draws no gate rows', () => {
    const rows = summaryDecisionRows(summary([aggregate('gate', 'verify', 4)]), settings([]));
    expect(rows).toEqual([]);
  });

  it('puts every decide batch in one row, then the decision chat, after the monitors', () => {
    const rows = summaryDecisionRows(
      summary([
        aggregate('chat', null, 2),
        aggregate('tool', 'call-1', 3),
        aggregate('tool', 'call-2', 4),
        aggregate('monitor', 'a', 1),
      ]),
      settings(['a'])
    );
    expect(rows.map((row) => [row.key, row.count, row.state])).toEqual([
      ['monitor:a', 1, 'quiet'],
      ['tool', 7, 'done'],
      ['chat', 2, 'done'],
    ]);
  });

  it('says a switched-off monitor is off', () => {
    const rows = summaryDecisionRows(
      summary([aggregate('monitor', 'a', 1)]),
      settings(['a'], ['a'])
    );
    expect(rows[0].state).toBe('off');
  });
});

describe('decisionsHeadline', () => {
  const rows = (states: string[], origin = 'monitor') =>
    states.map((state, index) => ({
      key: `${origin}:${index}`,
      origin: origin as 'monitor',
      originId: String(index),
      label: String(index),
      count: 2,
      errors: state === 'error' ? 1 : 0,
      state: state as 'quiet',
    }));

  it('follows section 2.2, first that applies', () => {
    expect(decisionsHeadline(rows(['fired', 'quiet']))).toEqual({
      text: '1 fired',
      spoken: '1 monitor fired',
      openByDefault: true,
    });
    expect(decisionsHeadline(rows(['fired', 'error'])).text).toBe('1 fired · 1 error');
    expect(decisionsHeadline(rows(['error', 'quiet']))).toEqual({
      text: '1 error',
      spoken: '1 error',
      openByDefault: true,
    });
    expect(decisionsHeadline(rows(['quiet', 'pending']))).toEqual({
      text: 'None fired',
      spoken: 'No monitor fired',
      openByDefault: false,
    });
    expect(decisionsHeadline(rows(['done', 'done'], 'tool'))).toEqual({
      text: '4 checks',
      spoken: '4 decision checks',
      openByDefault: false,
    });
  });
});
