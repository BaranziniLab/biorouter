import { renderHook } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import type { ScrollAreaHandle } from '../ui/scroll-area';
import { settleTranscriptScroll, useTranscriptSettle } from './useTranscriptFollow';

/**
 * What the live transcript does when its list settles (WS-TRANSCRIPT-1, after
 * V2-R7-2): the end while the reader follows, nothing once they scrolled up,
 * the first settle included. The scroll itself is measured in Chromium in
 * `transcriptSettle.browser.test.tsx`.
 */
const handle = (isFollowing: boolean) => {
  const scrollToBottom = vi.fn();
  const scroll = { isFollowing, scrollToBottom };
  const ref = { current: scroll as unknown as ScrollAreaHandle };
  return { scroll, scrollToBottom, ref };
};

describe('settleTranscriptScroll', () => {
  it('lands the first settle on the end at once while the reader follows', () => {
    const { scroll, scrollToBottom } = handle(true);
    settleTranscriptScroll(scroll, true);
    expect(scrollToBottom.mock.calls).toEqual([['auto']]);
  });

  it('leaves a reader who scrolled up where they are, on the first settle too', () => {
    const { scroll, scrollToBottom } = handle(false);
    settleTranscriptScroll(scroll, true);
    settleTranscriptScroll(scroll, false);
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('scrolls a later settle smoothly while the reader follows', () => {
    const { scroll, scrollToBottom } = handle(true);
    settleTranscriptScroll(scroll, false);
    expect(scrollToBottom.mock.calls).toEqual([[]]);
  });

  it('does nothing while a late row is held below the fold (V2-R9-2)', () => {
    const { scroll, scrollToBottom } = handle(true);
    settleTranscriptScroll(scroll, true, true);
    settleTranscriptScroll(scroll, false, true);
    expect(scrollToBottom).not.toHaveBeenCalled();
  });

  it('does nothing with no scroll area', () => {
    expect(() => settleTranscriptScroll(null, true)).not.toThrow();
    expect(() => settleTranscriptScroll(undefined, false)).not.toThrow();
  });
});

describe('useTranscriptSettle', () => {
  it('counts the first settle at the first one that has messages', () => {
    const { ref, scrollToBottom } = handle(true);
    const { result, rerender } = renderHook(({ count }) => useTranscriptSettle(ref, count), {
      initialProps: { count: 0 },
    });
    result.current();
    expect(scrollToBottom.mock.calls).toEqual([[]]);
    rerender({ count: 12 });
    result.current();
    result.current();
    expect(scrollToBottom.mock.calls).toEqual([[], ['auto'], []]);
  });

  it('spends the first settle on a reader who scrolled up, so their return is followed smoothly', () => {
    const { ref, scroll, scrollToBottom } = handle(false);
    const { result } = renderHook(() => useTranscriptSettle(ref, 60));
    result.current();
    expect(scrollToBottom).not.toHaveBeenCalled();
    scroll.isFollowing = true;
    result.current();
    expect(scrollToBottom.mock.calls).toEqual([[]]);
  });

  it('asks the follow hook whether a row is held, at settle time', () => {
    const { ref, scrollToBottom } = handle(true);
    let holding = true;
    const { result } = renderHook(() => useTranscriptSettle(ref, 5, () => holding));
    result.current();
    expect(scrollToBottom).not.toHaveBeenCalled();
    holding = false;
    result.current();
    expect(scrollToBottom.mock.calls).toEqual([[]]);
  });

  it('reads the scroll area at settle time, not at render time', () => {
    const { ref, scrollToBottom } = handle(true);
    const { result } = renderHook(() => useTranscriptSettle(ref, 3));
    const later = handle(false);
    ref.current = later.ref.current;
    result.current();
    expect(scrollToBottom).not.toHaveBeenCalled();
    expect(later.scrollToBottom).not.toHaveBeenCalled();
  });
});
