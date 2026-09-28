import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AUTO_READ_DWELL_MS,
  AUTO_READ_MIN_INTERVAL_MS,
  useAutoMarkRead,
  type AutoMarkReadInput,
  type AutoReadMemory,
} from './useAutoMarkRead';

/**
 * The automatic mark-read's own gate, apart from the timeline (QA M7): the channel is marked read
 * up to the newest message that has been on screen, measured, for the whole dwell, and never past
 * it. It used to mark the newest loaded message whenever the bottom was in view, so a busy channel
 * opened at its newest lost every unread mark a second later.
 */

beforeEach(() => {
  vi.useFakeTimers();
  vi.spyOn(document, 'hasFocus').mockReturnValue(true);
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function input(overrides: Partial<AutoMarkReadInput> = {}): AutoMarkReadInput {
  return {
    channelId: 'channel-1',
    latestSequence: 's9',
    readPosition: 's1',
    unread: 8,
    seen: () => 's9',
    enabled: true,
    markRead: vi.fn(async () => {}),
    memory: { current: new Map() as AutoReadMemory },
    ...overrides,
  };
}

describe('useAutoMarkRead', () => {
  it('marks read up to the message on screen once it has been there for the dwell', () => {
    const options = input();
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS - 1);
    });
    expect(options.markRead).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(1);
    });
    expect(options.markRead).toHaveBeenCalledWith('channel-1', 's9');
  });

  it('marks only as far as the reader has read, and follows them down', () => {
    let onScreen: string | null = 's4';
    const options = input({ seen: () => onScreen });
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    // Up to s4, not the newest loaded (s9): s5 to s9 have not been on screen.
    expect(options.markRead).toHaveBeenCalledTimes(1);
    expect(options.markRead).toHaveBeenCalledWith('channel-1', 's4');

    onScreen = 's7';
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_MIN_INTERVAL_MS + AUTO_READ_DWELL_MS);
    });
    expect(options.markRead).toHaveBeenLastCalledWith('channel-1', 's7');
  });

  it('marks nothing for a message that was not on screen for the whole dwell', () => {
    const passing = ['s3', 's5', 's7', 's9'];
    let look = 0;
    const options = input({ seen: () => passing[Math.min(look++, passing.length - 1)] });
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 3);
    });
    // Scrolling past: each look finds a different message, until the reader stops at s9.
    expect(options.markRead).not.toHaveBeenCalled();
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(options.markRead).toHaveBeenCalledWith('channel-1', 's9');
  });

  it('marks nothing while nothing unread is on screen, or once it is disabled', () => {
    const seen = vi.fn((): string | null => null);
    const options = input({ seen });
    const { rerender } = renderHook((props: AutoMarkReadInput) => useAutoMarkRead(props), {
      initialProps: options,
    });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 3);
    });
    const looks = seen.mock.calls.length;
    expect(looks).toBeGreaterThan(0);
    rerender({ ...options, enabled: false });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 5);
    });
    expect(seen.mock.calls.length).toBe(looks);
    expect(options.markRead).not.toHaveBeenCalled();
  });

  it('never marks the read position itself again', () => {
    const options = input({ seen: () => 's1' });
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 3);
    });
    expect(options.markRead).not.toHaveBeenCalled();
  });
});
