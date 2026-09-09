import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { useTransientFlag, useTransientValue } from './useTransientFlag';

beforeEach(() => {
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useTransientFlag', () => {
  it('starts down, raises on demand and lowers itself after the delay', () => {
    const { result } = renderHook(() => useTransientFlag(2000));

    expect(result.current[0]).toBe(false);

    act(() => result.current[1]());
    expect(result.current[0]).toBe(true);

    act(() => vi.advanceTimersByTime(1999));
    expect(result.current[0]).toBe(true);

    act(() => vi.advanceTimersByTime(1));
    expect(result.current[0]).toBe(false);
  });

  /**
   * The second of the two defects this hook exists to close. Every hand-rolled
   * copy re-armed a SECOND timer and left the first one running, so a second
   * copy inside the window was un-flagged by the first copy's countdown —
   * "Copied!" vanished ~1s after the click that produced it.
   */
  it('restarts the countdown on a re-trigger instead of stacking timers', () => {
    const { result } = renderHook(() => useTransientFlag(2000));

    act(() => result.current[1]());
    act(() => vi.advanceTimersByTime(1500));
    act(() => result.current[1]());

    // The first timer's original deadline. It must have been cancelled.
    act(() => vi.advanceTimersByTime(500));
    expect(result.current[0]).toBe(true);
    expect(vi.getTimerCount()).toBe(1);

    act(() => vi.advanceTimersByTime(1500));
    expect(result.current[0]).toBe(false);
  });

  /**
   * The first defect: a modal closed, a toast dismissed or a route changed
   * inside the window left a live callback holding a setter for a dead tree.
   */
  it('cancels its pending timer when the component unmounts', () => {
    const { result, unmount } = renderHook(() => useTransientFlag(2000));

    act(() => result.current[1]());
    expect(vi.getTimerCount()).toBe(1);

    unmount();
    expect(vi.getTimerCount()).toBe(0);

    expect(() => act(() => vi.advanceTimersByTime(5000))).not.toThrow();
  });

  it('can be lowered early, cancelling the timer with it', () => {
    const { result } = renderHook(() => useTransientFlag(2000));

    act(() => result.current[1]());
    act(() => result.current[2]());

    expect(result.current[0]).toBe(false);
    expect(vi.getTimerCount()).toBe(0);
  });
});

describe('useTransientValue', () => {
  it('holds the most recent value and clears it after the delay', () => {
    const { result } = renderHook(() => useTransientValue<string>(2000));

    expect(result.current[0]).toBeNull();

    act(() => result.current[1]('developer'));
    expect(result.current[0]).toBe('developer');

    // A different value replaces the first one AND its countdown, so the value
    // on screen always names the most recent action.
    act(() => vi.advanceTimersByTime(1500));
    act(() => result.current[1]('memory'));
    act(() => vi.advanceTimersByTime(500));
    expect(result.current[0]).toBe('memory');

    act(() => vi.advanceTimersByTime(1500));
    expect(result.current[0]).toBeNull();
  });
});
