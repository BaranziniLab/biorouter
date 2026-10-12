import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import {
  SUMMARY_RAIL_DEFAULT_PREFERENCE,
  SUMMARY_RAIL_STORAGE_KEY,
  readSummaryRailPreference,
  resetSummaryRailPreferenceForTests,
  setSummaryRailPreference,
  useSummaryRailPreference,
  writeSummaryRailPreference,
} from './summaryRailPreference';

function memoryStorage(initial: Record<string, string> = {}) {
  const data = new Map(Object.entries(initial));
  return {
    getItem: (key: string) => data.get(key) ?? null,
    setItem: (key: string, value: string) => void data.set(key, value),
    data,
  };
}

const throwing = {
  getItem: () => {
    throw new Error('blocked');
  },
  setItem: () => {
    throw new Error('full');
  },
};

beforeEach(() => {
  window.localStorage.clear();
  resetSummaryRailPreferenceForTests();
});
afterEach(() => {
  window.localStorage.clear();
  resetSummaryRailPreferenceForTests();
});

describe('the summary rail preference', () => {
  it('defaults to open', () => {
    expect(SUMMARY_RAIL_DEFAULT_PREFERENCE).toBe('open');
    expect(readSummaryRailPreference(memoryStorage())).toBe('open');
    expect(readSummaryRailPreference(null)).toBe('open');
  });

  it('reads an invalid stored value as the default', () => {
    for (const value of ['', 'OPEN', 'nope', '1']) {
      expect(readSummaryRailPreference(memoryStorage({ [SUMMARY_RAIL_STORAGE_KEY]: value }))).toBe(
        'open'
      );
    }
    expect(readSummaryRailPreference(memoryStorage({ [SUMMARY_RAIL_STORAGE_KEY]: 'closed' }))).toBe(
      'closed'
    );
  });

  it('reads a throwing storage as the default and swallows its writes', () => {
    expect(readSummaryRailPreference(throwing)).toBe('open');
    expect(() => writeSummaryRailPreference('closed', throwing)).not.toThrow();
  });

  it('writes under its key', () => {
    const storage = memoryStorage();
    writeSummaryRailPreference('closed', storage);
    expect(storage.data.get(SUMMARY_RAIL_STORAGE_KEY)).toBe('closed');
  });

  it('gives two subscribers the same answer after a write, and persists it', () => {
    const first = renderHook(() => useSummaryRailPreference());
    const second = renderHook(() => useSummaryRailPreference());
    expect(first.result.current[0]).toBe('open');
    act(() => first.result.current[1]('closed'));
    expect(first.result.current[0]).toBe('closed');
    expect(second.result.current[0]).toBe('closed');
    expect(window.localStorage.getItem(SUMMARY_RAIL_STORAGE_KEY)).toBe('closed');
    act(() => setSummaryRailPreference('open'));
    expect(second.result.current[0]).toBe('open');
  });

  it('follows a change another window made', () => {
    const { result } = renderHook(() => useSummaryRailPreference());
    act(() => {
      window.dispatchEvent(
        new StorageEvent('storage', { key: SUMMARY_RAIL_STORAGE_KEY, newValue: 'closed' })
      );
    });
    expect(result.current[0]).toBe('closed');
    // A cleared store (key null) returns to the default.
    act(() => {
      window.dispatchEvent(new StorageEvent('storage', { key: null, newValue: null }));
    });
    expect(result.current[0]).toBe('open');
    // Another key is not ours.
    act(() => {
      window.dispatchEvent(new StorageEvent('storage', { key: 'other', newValue: 'closed' }));
    });
    expect(result.current[0]).toBe('open');
  });
});
