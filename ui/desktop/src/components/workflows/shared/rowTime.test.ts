import { describe, expect, it } from 'vitest';
import { fromUnixSeconds, rowTime, rowTimeTitle } from './rowTime';

const NOW = new Date('2026-10-09T12:00:00');

describe('rowTime', () => {
  it('counts up through minutes, hours and days, then names the date', () => {
    expect(rowTime(new Date('2026-10-09T11:59:30'), NOW)).toBe('now');
    expect(rowTime(new Date('2026-10-09T11:55:00'), NOW)).toBe('5m');
    expect(rowTime(new Date('2026-10-09T09:00:00'), NOW)).toBe('3h');
    expect(rowTime(new Date('2026-10-05T12:00:00'), NOW)).toBe('4d');
    expect(rowTime(new Date('2026-09-07T12:00:00'), NOW)).toMatch(/Sep\s7/);
  });

  it('adds the year only when it is not this year', () => {
    expect(rowTime(new Date('2025-03-02T12:00:00'), NOW)).toMatch(/2025/);
    expect(rowTime(new Date('2026-03-02T12:00:00'), NOW)).not.toMatch(/2026/);
  });

  it('says nothing for a missing or unreadable date, and never a negative age', () => {
    expect(rowTime(null, NOW)).toBeNull();
    expect(rowTime(new Date('not a date'), NOW)).toBeNull();
    expect(rowTime(new Date('2026-10-09T12:30:00'), NOW)).toBe('now');
    expect(rowTimeTitle(null)).toBeNull();
  });

  it('reads Unix seconds', () => {
    expect(fromUnixSeconds(0)).toBeNull();
    expect(fromUnixSeconds(1750000000)?.getTime()).toBe(1750000000 * 1000);
  });
});
