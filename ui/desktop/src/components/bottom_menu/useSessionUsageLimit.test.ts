import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { SessionUsageLimitResponse } from '../../api';
import { USAGE_LIMITS_CHANGED_EVENT } from '../usage/usageLimits';
import { useSessionUsageLimit } from './useSessionUsageLimit';

/**
 * A composer a limit holds cannot send, and the limits-changed announcement is said on one window
 * only, so a limit raised in another Biorouter window reaches it when its own window comes back
 * to the person: on focus, or when it becomes visible.
 */
const mocks = vi.hoisted(() => ({
  getLimit: vi.fn(),
  getSummary: vi.fn(),
  putLimit: vi.fn(),
  headers: vi.fn(async () => ({ 'X-User-Action': 'proof' })),
  browser: false,
}));
vi.mock('../../api', () => ({
  getSessionUsageLimit: mocks.getLimit,
  getUsageSummary: mocks.getSummary,
  putSessionUsageLimit: mocks.putLimit,
}));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../utils/surface', () => ({ isBrowserSurface: () => mocks.browser }));

function limitState(paused: boolean): SessionUsageLimitResponse {
  return {
    billingSessionId: 'chat-a',
    chatDollarLimitInert: false,
    chatOverride: null,
    defaults: { maxTokens: null, maxUsd: null },
    limit: { maxTokens: 1_000, maxUsd: null, source: 'default' },
    monthDollarLimitInert: false,
    monthly: { maxTokens: null, maxUsd: null },
    paused,
    pausedByEstimate: false,
    nextRequest: null,
    reason: paused ? 'chat' : null,
    used: {
      chatTokens: paused ? 1_000 : 10,
      chatUsd: null,
      chatUsdComplete: true,
      month: '2026-11',
      monthTokens: 1_000,
      monthUsd: null,
      monthUsdComplete: true,
    },
    warn: false,
  };
}

type Visibility = 'visible' | 'hidden';
let visibility: Visibility = 'visible';

beforeEach(() => {
  mocks.getLimit.mockReset();
  mocks.getSummary.mockReset();
  mocks.headers.mockClear();
  mocks.browser = false;
  visibility = 'visible';
  Object.defineProperty(document, 'visibilityState', {
    configurable: true,
    get: () => visibility,
  });
});

afterEach(() => {
  // The jsdom prototype getter answers again.
  delete (document as { visibilityState?: unknown }).visibilityState;
});

const focusWindow = () => act(() => void window.dispatchEvent(new Event('focus')));
const becomeVisible = (state: Visibility) =>
  act(() => {
    visibility = state;
    document.dispatchEvent(new Event('visibilitychange'));
  });

describe('a composer held by a limit', () => {
  it('reads the limit again when its window regains focus, and lets Send go once it was raised', async () => {
    mocks.getLimit.mockResolvedValueOnce({ data: limitState(true) });
    const { result } = renderHook(() => useSessionUsageLimit('chat-a'));
    await waitFor(() => expect(result.current.state?.paused).toBe(true));
    expect(mocks.getLimit).toHaveBeenCalledTimes(1);

    // Raised in another window: this one hears nothing until the person comes back to it.
    mocks.getLimit.mockResolvedValueOnce({ data: limitState(false) });
    await focusWindow();
    await waitFor(() => expect(result.current.state?.paused).toBe(false));
    expect(mocks.getLimit).toHaveBeenCalledTimes(2);
    expect(mocks.getLimit).toHaveBeenLastCalledWith(
      expect.objectContaining({ path: { session_id: 'chat-a' } })
    );

    // No longer held: focus is not a reason to read.
    await focusWindow();
    expect(mocks.getLimit).toHaveBeenCalledTimes(2);
  });

  it('reads again when its window becomes visible, not when it is hidden', async () => {
    mocks.getLimit.mockResolvedValue({ data: limitState(true) });
    const { result } = renderHook(() => useSessionUsageLimit('chat-a'));
    await waitFor(() => expect(result.current.state?.paused).toBe(true));
    expect(mocks.getLimit).toHaveBeenCalledTimes(1);

    await becomeVisible('hidden');
    expect(mocks.getLimit).toHaveBeenCalledTimes(1);
    await becomeVisible('visible');
    await waitFor(() => expect(mocks.getLimit).toHaveBeenCalledTimes(2));
  });

  it('does not read on focus while nothing holds the chat', async () => {
    mocks.getLimit.mockResolvedValue({ data: limitState(false) });
    const { result } = renderHook(() => useSessionUsageLimit('chat-a'));
    await waitFor(() => expect(result.current.state?.paused).toBe(false));
    await focusWindow();
    await becomeVisible('visible');
    expect(mocks.getLimit).toHaveBeenCalledTimes(1);
  });

  it('with no chat yet, reads the month again on focus while the monthly limit is reached', async () => {
    mocks.getSummary.mockResolvedValueOnce({ data: { monthlyReached: true, monthlyNear: true } });
    const { result } = renderHook(() => useSessionUsageLimit(null));
    await waitFor(() => expect(result.current.month?.reached).toBe(true));
    expect(mocks.getSummary).toHaveBeenCalledTimes(1);

    mocks.getSummary.mockResolvedValueOnce({ data: { monthlyReached: false, monthlyNear: false } });
    await focusWindow();
    await waitFor(() => expect(result.current.month?.reached).toBe(false));
    expect(mocks.getSummary).toHaveBeenCalledTimes(2);
    expect(mocks.getLimit).not.toHaveBeenCalled();
  });
});

/**
 * SD-8: a `biorouter serve` daemon refuses `GET /usage/summary` to everyone (it holds no key to
 * check the user-action proof against), so Home in a browser asks nothing about the month: no
 * request that can only be refused, and nothing logged. Send is not held there; the daemon still
 * refuses a request at the limit and stores the notice.
 */
describe('a composer with no chat on a browser surface', () => {
  const flush = () =>
    act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 0));
    });

  it('never reads the month, on mount, on a limits change, on focus or on reload, and logs nothing', async () => {
    mocks.browser = true;
    mocks.getSummary.mockResolvedValue({
      error: { message: 'forbidden' },
      response: { status: 403 },
    });
    const error = vi.spyOn(console, 'error').mockImplementation(() => {});
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const { result } = renderHook(() => useSessionUsageLimit(null));
      await flush();

      window.dispatchEvent(new CustomEvent(USAGE_LIMITS_CHANGED_EVENT));
      await focusWindow();
      await becomeVisible('visible');
      act(() => result.current.reload());
      await flush();

      expect(mocks.getSummary).not.toHaveBeenCalled();
      expect(mocks.headers).not.toHaveBeenCalled();
      expect(mocks.getLimit).not.toHaveBeenCalled();
      expect(result.current.month).toBeNull();
      expect(error).not.toHaveBeenCalled();
      expect(warn).not.toHaveBeenCalled();
    } finally {
      error.mockRestore();
      warn.mockRestore();
    }
  });

  it('on the desktop still reads the month', async () => {
    mocks.getSummary.mockResolvedValue({ data: { monthlyReached: false, monthlyNear: true } });
    const { result } = renderHook(() => useSessionUsageLimit(null));
    await waitFor(() => expect(result.current.month).toEqual({ reached: false, near: true }));
    expect(mocks.getSummary).toHaveBeenCalledTimes(1);
    expect(mocks.getSummary).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'proof' } })
    );
  });
});
