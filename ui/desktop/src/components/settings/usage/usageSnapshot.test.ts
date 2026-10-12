import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { beforeEach, describe, expect, it } from 'vitest';
import type { UsageSummaryResponse } from '../../../api';
import {
  DATA_RESET_EVENT,
  forgetUsageSnapshot,
  readUsageSnapshot,
  rememberUsageSnapshot,
  usageMonthKey,
} from './usageSnapshot';

const summary = { month: '2026-10' } as unknown as UsageSummaryResponse;
const data = { summary, dayRows: [], modelRows: [] };

beforeEach(() => forgetUsageSnapshot());

describe('usageMonthKey', () => {
  it('is the local calendar month, zero padded', () => {
    expect(usageMonthKey(new Date(2026, 0, 31))).toBe('2026-01');
    expect(usageMonthKey(new Date(2026, 9, 1))).toBe('2026-10');
    expect(usageMonthKey(new Date(2026, 11, 31, 23, 59))).toBe('2026-12');
  });
});

describe('the remembered usage read', () => {
  it('is empty until a read completes', () => {
    expect(readUsageSnapshot(new Date(2026, 9, 11))).toBeNull();
  });

  it('is handed back for the month it was read in, and the same object each time', () => {
    rememberUsageSnapshot(new Date(2026, 9, 3), data);
    const first = readUsageSnapshot(new Date(2026, 9, 28));
    expect(first?.summary).toBe(summary);
    expect(readUsageSnapshot(new Date(2026, 9, 29))).toBe(first);
  });

  it('is not handed back in another month, nor in the same month of another year', () => {
    rememberUsageSnapshot(new Date(2026, 9, 31, 23, 59), data);
    expect(readUsageSnapshot(new Date(2026, 10, 1))).toBeNull();
    expect(readUsageSnapshot(new Date(2027, 9, 15))).toBeNull();
  });

  it('keeps only the newest read', () => {
    const later = { month: '2026-10-later' } as unknown as UsageSummaryResponse;
    rememberUsageSnapshot(new Date(2026, 9, 3), data);
    rememberUsageSnapshot(new Date(2026, 9, 4), { ...data, summary: later });
    expect(readUsageSnapshot(new Date(2026, 9, 5))?.summary).toBe(later);
  });

  it('is forgotten on request', () => {
    rememberUsageSnapshot(new Date(2026, 9, 3), data);
    forgetUsageSnapshot();
    expect(readUsageSnapshot(new Date(2026, 9, 3))).toBeNull();
  });

  it('is forgotten when ResetPanel announces any completed reset', () => {
    for (const categories of [['history'], ['knowledge'], ['history', 'knowledge']]) {
      rememberUsageSnapshot(new Date(2026, 9, 3), data);
      window.dispatchEvent(new CustomEvent(DATA_RESET_EVENT, { detail: { categories } }));
      expect(readUsageSnapshot(new Date(2026, 9, 3))).toBeNull();
    }
  });

  it('listens under the event name ResetPanel dispatches', () => {
    // The two never import each other, so a rename on either side would pass every other test.
    const here = dirname(fileURLToPath(import.meta.url));
    const reset = readFileSync(resolve(here, '../app/ResetPanel.tsx'), 'utf8');
    expect(reset).toContain(`new CustomEvent('${DATA_RESET_EVENT}'`);
  });
});
