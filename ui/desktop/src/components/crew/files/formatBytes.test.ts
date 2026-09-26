import { describe, expect, it } from 'vitest';
import { formatBytes } from './formatBytes';

describe('formatBytes', () => {
  it.each([
    [0, '0 bytes'],
    [1, '1 byte'],
    [103, '103 bytes'],
    [1023, '1023 bytes'],
    [1024, '1 KB'],
    [1536, '1.5 KB'],
    [10 * 1024 - 1, '10 KB'],
    [55 * 1024, '55 KB'],
    [56_320, '55 KB'],
    [1024 * 1024 - 1, '1 MB'],
    [1.5 * 1024 * 1024, '1.5 MB'],
    [734 * 1024 * 1024, '734 MB'],
    [1024 ** 3, '1 GB'],
    [2.25 * 1024 ** 3, '2.3 GB'],
    [5 * 1024 ** 4, '5 TB'],
  ])('%d bytes reads as %s', (bytes, expected) => {
    expect(formatBytes(bytes)).toBe(expected);
  });

  it('never reads 1024 of a unit', () => {
    for (let bytes = 1000 * 1024; bytes < 1025 * 1024; bytes += 97) {
      expect(formatBytes(bytes)).not.toMatch(/^1024 /);
    }
  });

  it('returns nothing for a size that is not a size', () => {
    expect(formatBytes(-1)).toBe('');
    expect(formatBytes(Number.NaN)).toBe('');
    expect(formatBytes(Number.POSITIVE_INFINITY)).toBe('');
  });
});
