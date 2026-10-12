import { act, renderHook } from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import {
  recall,
  remember,
  rememberIfUnknown,
  resetLastKnownForTests,
  useLastKnown,
} from './lastKnown';

/**
 * The store behind "a later visit starts from the last answer" (vision round 8, V4-R7-2). The
 * rules are small and each one is the difference between a control that paints at rest and one
 * that paints a lie, so each has its own case.
 */
describe('lastKnown', () => {
  it('knows nothing until something has read it, and that is not a value', () => {
    expect(recall('approvalMode')).toBeUndefined();
    const { result } = renderHook(() => useLastKnown<string>('approvalMode'));
    expect(result.current[0]).toBeUndefined();
  });

  it('keeps null as an answer: "nothing saved" is known, "not read yet" is not', () => {
    remember('maxTurns', null);
    expect(recall('maxTurns')).toBeNull();
    const { result } = renderHook(() => useLastKnown<number | null>('maxTurns'));
    expect(result.current[0]).toBeNull();
  });

  it('keeps false as an answer too', () => {
    remember('wakelock', false);
    const { result } = renderHook(() => useLastKnown<boolean>('wakelock'));
    expect(result.current[0]).toBe(false);
  });

  it('never stores undefined, which would read as not known', () => {
    remember('wakelock', undefined);
    expect(recall('wakelock')).toBeUndefined();
    rememberIfUnknown('wakelock', undefined);
    rememberIfUnknown('wakelock', true);
    expect(recall('wakelock')).toBe(true);
  });

  it('seeds a new mount with the last answer, in the first render', () => {
    remember('menuBarIcon', false);
    const seen: Array<boolean | undefined> = [];
    renderHook(() => {
      const [value] = useLastKnown<boolean>('menuBarIcon');
      seen.push(value);
    });
    expect(seen[0]).toBe(false);
  });

  it('writes a change through, so the next visit starts from it', () => {
    const first = renderHook(() => useLastKnown<boolean>('dockIcon'));
    act(() => first.result.current[1](true));
    expect(first.result.current[0]).toBe(true);
    first.unmount();
    const next = renderHook(() => useLastKnown<boolean>('dockIcon'));
    expect(next.result.current[0]).toBe(true);
  });

  it('lets a warm-up fill only what nothing has answered, so a flip in flight is not undone', () => {
    rememberIfUnknown('wakelock', true);
    expect(recall('wakelock')).toBe(true);
    // The person switched it off while the warm-up's read was still out.
    remember('wakelock', false);
    rememberIfUnknown('wakelock', true);
    expect(recall('wakelock')).toBe(false);
  });

  it('keeps one answer per setting', () => {
    remember('menuBarIcon', true);
    remember('dockIcon', false);
    expect(recall('menuBarIcon')).toBe(true);
    expect(recall('dockIcon')).toBe(false);
    expect(recall('wakelock')).toBeUndefined();
  });

  it('forgets everything on reset, as a fresh window would', () => {
    remember('spellcheck', false);
    resetLastKnownForTests();
    expect(recall('spellcheck')).toBeUndefined();
  });
});
