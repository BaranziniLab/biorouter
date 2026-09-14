import { act, render } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

let cachedList: Array<{ id: string; privacy_tier?: string }> | null = null;
const listeners = new Set<() => void>();
const preloadSessionList = vi.fn();

vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => cachedList,
  subscribeSessionList: (listener: () => void) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
  preloadSessionList: () => preloadSessionList(),
}));

import { useSessionListTiers } from './useSessionListTiers';

/** Every map the hook returned, one per render, in order. */
let renders: Array<Record<string, string>> = [];
function Probe() {
  renders.push(useSessionListTiers());
  return null;
}

describe('useSessionListTiers', () => {
  beforeEach(() => {
    cachedList = null;
    listeners.clear();
    preloadSessionList.mockClear();
    renders = [];
  });

  /**
   * The shell remounts on every route change (Settings → a chat), while the
   * list cache is module state that survives it. If the map is only filled in
   * an effect, the first paint of every remount draws every tab without a tier
   * — Public until 2026-09-14, a dimmed not-yet-known glyph after it.
   */
  it('has the cached tiers on the very first render, before any effect runs', () => {
    cachedList = [
      { id: 'public-chat', privacy_tier: 'public' },
      { id: 'private-chat', privacy_tier: 'private' },
    ];
    render(<Probe />);
    expect(renders[0]).toEqual({ 'public-chat': 'public', 'private-chat': 'private' });
  });

  it('leaves out an id the list does not carry, rather than calling it public', () => {
    cachedList = [{ id: 'listed', privacy_tier: 'public' }, { id: 'no-tier' }];
    render(<Probe />);
    const last = renders[renders.length - 1];
    expect(last).toEqual({ listed: 'public' });
    expect('no-tier' in last).toBe(false);
    expect('a-subagent' in last).toBe(false);
  });

  it('warms a cold cache and follows it when it lands', () => {
    render(<Probe />);
    expect(preloadSessionList).toHaveBeenCalledTimes(1);
    expect(renders[renders.length - 1]).toEqual({});

    cachedList = [{ id: 'late', privacy_tier: 'private' }];
    act(() => listeners.forEach((listener) => listener()));
    expect(renders[renders.length - 1]).toEqual({ late: 'private' });
  });

  // Identity, not a render count: React may render a component once more on a
  // same-value update before it bails out, and what the strips depend on is
  // that the map they receive is the same object.
  it('keeps the same map when a list change touched no tier', () => {
    cachedList = [{ id: 'a', privacy_tier: 'public' }];
    render(<Probe />);
    const before = renders[renders.length - 1];
    cachedList = [{ id: 'a', privacy_tier: 'public' }];
    act(() => listeners.forEach((listener) => listener()));
    expect(renders[renders.length - 1]).toBe(before);
  });
});
