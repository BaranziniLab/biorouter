import { describe, expect, it } from 'vitest';
import {
  DEFAULT_AUTO_COMPACT_MAX_TOKENS,
  effectiveAutoCompactTrigger,
  savedAutoCompactCeiling,
} from './autoCompactTrigger';

describe('the auto-compact ceiling, read the way the daemon reads it', () => {
  it.each([
    [undefined, DEFAULT_AUTO_COMPACT_MAX_TOKENS],
    [null, DEFAULT_AUTO_COMPACT_MAX_TOKENS],
    [0.8, DEFAULT_AUTO_COMPACT_MAX_TOKENS],
    ['many', DEFAULT_AUTO_COMPACT_MAX_TOKENS],
    [-5, DEFAULT_AUTO_COMPACT_MAX_TOKENS],
    [0, null],
    ['0', null],
    [300_000, 300_000],
    [' 150000 ', 150_000],
  ])('a saved %s is %s', (saved, expected) => {
    expect(savedAutoCompactCeiling(saved)).toBe(expected);
  });
});

describe('the effective trigger', () => {
  it('is the ceiling on a window over 250k tokens', () => {
    const trigger = effectiveAutoCompactTrigger(80, 1_050_000, 200_000)!;
    expect(trigger.byCeiling).toBe(true);
    expect(trigger.tokens).toBe(200_000);
    expect(trigger.pct).toBeCloseTo(19.05, 1);
  });

  it('is the share on a window the ceiling does not reach', () => {
    expect(effectiveAutoCompactTrigger(80, 200_000, 200_000)).toEqual({
      pct: 80,
      tokens: 160_000,
      byCeiling: false,
    });
  });

  it('is the share with no ceiling, and nothing when compaction is off', () => {
    expect(effectiveAutoCompactTrigger(80, 1_050_000, null)?.byCeiling).toBe(false);
    expect(effectiveAutoCompactTrigger(null, 1_050_000, 200_000)).toBeNull();
    expect(effectiveAutoCompactTrigger(80, 0, 200_000)).toBeNull();
  });
});
