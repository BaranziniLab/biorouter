import { act, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  AUTO_READ_DWELL_MS,
  useAutoMarkRead,
  type AutoMarkReadInput,
  type AutoReadMemory,
} from './useAutoMarkRead';

/**
 * The automatic mark-read's own gate, apart from the timeline: the scroll area's
 * cached "at the bottom" is not enough; the newest message must be on screen,
 * measured, when the dwell ends.
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
    latestSequence: 's2',
    readPosition: 's1',
    unread: 1,
    atBottom: true,
    enabled: true,
    markRead: vi.fn(async () => {}),
    memory: { current: new Map() as AutoReadMemory },
    ...overrides,
  };
}

describe('useAutoMarkRead', () => {
  it('marks read after the dwell when nothing measures the bottom (the cached verdict alone)', () => {
    const options = input();
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(options.markRead).toHaveBeenCalledWith('channel-1', 's2');
  });

  it('does not mark read while the newest message is off screen, however long the dwell', () => {
    let onScreen = false;
    const options = input({ isAtBottom: () => onScreen });
    renderHook(() => useAutoMarkRead(options));
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 10);
    });
    expect(options.markRead).not.toHaveBeenCalled();

    // The reader reaches the newest message without the cached verdict changing.
    onScreen = true;
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS);
    });
    expect(options.markRead).toHaveBeenCalledTimes(1);
    expect(options.markRead).toHaveBeenCalledWith('channel-1', 's2');
  });

  it('stops looking once the view is no longer at the bottom or the hook is disabled', () => {
    const isAtBottom = vi.fn(() => false);
    const options = input({ isAtBottom });
    const { rerender } = renderHook((props: AutoMarkReadInput) => useAutoMarkRead(props), {
      initialProps: options,
    });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 3);
    });
    const looks = isAtBottom.mock.calls.length;
    expect(looks).toBeGreaterThan(0);
    rerender({ ...options, enabled: false });
    act(() => {
      vi.advanceTimersByTime(AUTO_READ_DWELL_MS * 5);
    });
    expect(isAtBottom.mock.calls.length).toBe(looks);
    expect(options.markRead).not.toHaveBeenCalled();
  });
});
