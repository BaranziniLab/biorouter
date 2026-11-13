import { describe, expect, it } from 'vitest';
import cases from './formatUsdCases.json';
import { formatUsd, formatUsdPair } from './formatUsd';

// The daemon runs the same table (`dollars_print_as_the_desktop_prints_them` in
// crates/biorouter/src/agents/usage_limit.rs), so both print one amount one way.
describe('formatUsd', () => {
  it.each(cases.single as [number, string][])('prints %s as %s', (amount, expected) => {
    expect(formatUsd(amount)).toBe(expected);
  });

  it('prints nothing for a value that is not a number', () => {
    expect(formatUsd(Number.NaN)).toBe('$0.00');
    expect(formatUsd(Number.POSITIVE_INFINITY)).toBe('$0.00');
  });
});

describe('formatUsdPair', () => {
  it.each(cases.pairs as [number, number, string, string][])(
    'prints %s and %s apart unless they are equal',
    (a, b, expectedA, expectedB) => {
      expect(formatUsdPair(a, b)).toEqual([expectedA, expectedB]);
    }
  );
});
