import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { Session } from '../../api';
import { listedChatRow, useLoadingChatRow } from './useLoadingChatRow';

/**
 * Vision V2-R5-6: while a chat's transcript loads, BaseChat holds no `session`.
 * This hook is what it knows instead: the listing's row at once, the row itself
 * when it lands, and nothing at all once the load is over.
 */
const mocks = vi.hoisted(() => ({
  getSession: vi.fn(),
  headers: vi.fn(),
  rows: null as Session[] | null,
  listeners: new Set<() => void>(),
}));
vi.mock('../../api', () => ({ getSession: mocks.getSession }));
vi.mock('../../utils/userAction', () => ({ userActionHeaders: mocks.headers }));
vi.mock('../../utils/sessionListCache', () => ({
  getCachedSessionList: () => mocks.rows,
  subscribeSessionList: (listener: () => void) => {
    mocks.listeners.add(listener);
    return () => mocks.listeners.delete(listener);
  },
}));

const row = (id: string, over: Partial<Session> = {}) =>
  ({
    id,
    name: `chat ${id}`,
    message_count: 12,
    provider_name: 'anthropic',
    model_config: { model_name: 'claude-opus-4-5' },
    total_tokens: 64_600,
    ...over,
  }) as unknown as Session;

function publishList(rows: Session[]) {
  mocks.rows = rows;
  act(() => {
    for (const listener of mocks.listeners) listener();
  });
}

beforeEach(() => {
  mocks.rows = null;
  mocks.listeners.clear();
  mocks.getSession.mockReset().mockReturnValue(new Promise(() => {}));
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

describe('listedChatRow', () => {
  it('finds the chat in a listing, and says nothing before one is read', () => {
    expect(listedChatRow(null, 'a')).toBeUndefined();
    expect(listedChatRow([row('b')], 'a')).toBeUndefined();
    expect(listedChatRow([row('b'), row('a')], 'a')?.id).toBe('a');
  });
});

describe('useLoadingChatRow', () => {
  it('answers from a warm listing in the first render', () => {
    mocks.rows = [row('b'), row('a')];
    const { result } = renderHook(() => useLoadingChatRow('a', true));
    expect(result.current?.id).toBe('a');
    expect(result.current?.message_count).toBe(12);
  });

  it('reads the row alone, with the proof, and prefers it to the listing', async () => {
    mocks.rows = [row('a', { model_config: { model_name: 'stale' } as Session['model_config'] })];
    mocks.getSession.mockResolvedValue({ data: row('a') });
    const { result } = renderHook(() => useLoadingChatRow('a', true));
    expect(result.current?.model_config?.model_name).toBe('stale');
    await waitFor(() => expect(result.current?.model_config?.model_name).toBe('claude-opus-4-5'));
    expect(mocks.getSession).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'a' },
        query: { metadata_only: true },
        headers: { 'X-User-Action': 'synthetic-proof' },
      })
    );
  });

  it('picks up a listing that arrives while the chat is still loading', () => {
    const { result } = renderHook(() => useLoadingChatRow('a', true));
    expect(result.current).toBeUndefined();
    publishList([row('a')]);
    expect(result.current?.id).toBe('a');
  });

  it('ignores an answer for another chat, and a failed read', async () => {
    mocks.getSession.mockResolvedValueOnce({ data: row('other') });
    const first = renderHook(() => useLoadingChatRow('a', true));
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    expect(first.result.current).toBeUndefined();

    mocks.getSession.mockRejectedValueOnce(new Error('offline'));
    const second = renderHook(() => useLoadingChatRow('b', true));
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(2));
    expect(second.result.current).toBeUndefined();
  });

  it('is nothing once the load is over, and reads nothing for a chat that is not loading', async () => {
    mocks.rows = [row('a')];
    mocks.getSession.mockResolvedValue({ data: row('a') });
    const { result, rerender } = renderHook(({ loading }) => useLoadingChatRow('a', loading), {
      initialProps: { loading: true },
    });
    await waitFor(() => expect(mocks.getSession).toHaveBeenCalledTimes(1));
    rerender({ loading: false });
    expect(result.current).toBeUndefined();
    // No longer subscribed: a later listing does not reach a loaded chat.
    expect(mocks.listeners.size).toBe(0);

    const idle = renderHook(() => useLoadingChatRow('c', false));
    expect(idle.result.current).toBeUndefined();
    // A new chat (no id) never asks either.
    const blank = renderHook(() => useLoadingChatRow('', true));
    expect(blank.result.current).toBeUndefined();
    expect(mocks.getSession).toHaveBeenCalledTimes(1);
  });
});
