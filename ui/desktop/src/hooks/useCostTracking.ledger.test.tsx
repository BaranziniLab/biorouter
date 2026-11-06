import { act, renderHook, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';
import type { ModelUsageRow, Session } from '../api';
import { useCostTracking } from './useCostTracking';

/**
 * Vision V2-R5-5: until a chat's usage ledger answers, the hook used to hand
 * back `[]`, which every reader takes as "no ledger rows" and falls back to the
 * accumulated counters. The summary's Tokens value and the footer cost then
 * showed a figure ("2.2M", "$11.75") that the ledger withdrew a moment later
 * ("N/A", no cost). `ledgerSettled` is what lets a reader wait.
 */
const mocks = vi.hoisted(() => ({
  getSessionUsage: vi.fn(),
  pricing: vi.fn(),
  headers: vi.fn(),
}));
vi.mock('../api', () => ({ getSessionUsage: mocks.getSessionUsage }));
vi.mock('../utils/pricing', () => ({ fetchModelPricing: mocks.pricing }));
vi.mock('../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const usageRow: ModelUsageRow = {
  provider: 'anthropic',
  modelId: 'claude-test',
  inputTokens: 100,
  outputTokens: 20,
  cacheReadTokens: null,
  cacheCreationTokens: null,
  totalTokens: null,
  turns: 2,
};

const chat = (id: string, total = 120) =>
  ({ id, accumulated_total_tokens: total }) as unknown as Session;

/** A usage request the test answers by hand. */
function deferred() {
  let resolve!: (value: { data: { models: ModelUsageRow[] } }) => void;
  const promise = new Promise<{ data: { models: ModelUsageRow[] } }>((r) => (resolve = r));
  return { promise, answer: (models: ModelUsageRow[]) => resolve({ data: { models } }) };
}

beforeEach(() => {
  mocks.getSessionUsage.mockReset();
  mocks.pricing.mockReset().mockResolvedValue(null);
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

describe('useCostTracking: a reader can tell a pending ledger from an empty one', () => {
  it('is not settled, and holds no rows, while the usage request is pending', async () => {
    const usage = deferred();
    mocks.getSessionUsage.mockReturnValue(usage.promise);
    const { result } = renderHook(() => useCostTracking({ session: chat('a') }));

    expect(result.current.ledgerSettled).toBe(false);
    expect(result.current.modelRows).toEqual([]);
    // Sent after the proof is in hand, and still pending after it was sent.
    await waitFor(() =>
      expect(mocks.getSessionUsage).toHaveBeenCalledWith(
        expect.objectContaining({
          path: { session_id: 'a' },
          headers: { 'X-User-Action': 'synthetic-proof' },
        })
      )
    );
    expect(result.current.ledgerSettled).toBe(false);
    expect(result.current.modelRows).toEqual([]);

    await act(async () => usage.answer([usageRow]));
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));
    expect(result.current.modelRows).toHaveLength(1);
    expect(result.current.modelRows[0]).toMatchObject({ model: 'claude-test', totalTokens: null });
  });

  it('settles on an answer with no rows, and on a failed request', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [] } });
    const empty = renderHook(() => useCostTracking({ session: chat('a') }));
    await waitFor(() => expect(empty.result.current.ledgerSettled).toBe(true));
    expect(empty.result.current.modelRows).toEqual([]);

    mocks.getSessionUsage.mockRejectedValueOnce(new Error('offline'));
    const failed = renderHook(() => useCostTracking({ session: chat('b') }));
    await waitFor(() => expect(failed.result.current.ledgerSettled).toBe(true));
    expect(failed.result.current.modelRows).toEqual([]);
  });

  it('is settled with no chat, and asks for nothing', () => {
    const { result } = renderHook(() => useCostTracking({ session: undefined }));
    expect(result.current.ledgerSettled).toBe(true);
    expect(result.current.modelRows).toEqual([]);
    expect(mocks.getSessionUsage).not.toHaveBeenCalled();
  });

  it('never lets one chat’s ledger stand in for another’s', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [usageRow] } });
    const { result, rerender } = renderHook(({ session }) => useCostTracking({ session }), {
      initialProps: { session: chat('a') },
    });
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));

    const usage = deferred();
    mocks.getSessionUsage.mockReturnValue(usage.promise);
    rerender({ session: chat('b') });
    expect(result.current.ledgerSettled).toBe(false);
    expect(result.current.modelRows).toEqual([]);

    await act(async () => usage.answer([]));
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));
  });

  it('keeps the answer it has while a turn refreshes the same chat', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [usageRow] } });
    const { result, rerender } = renderHook(({ session }) => useCostTracking({ session }), {
      initialProps: { session: chat('a', 120) },
    });
    await waitFor(() => expect(result.current.modelRows).toHaveLength(1));

    const usage = deferred();
    mocks.getSessionUsage.mockReturnValue(usage.promise);
    rerender({ session: chat('a', 400) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(2));
    // Still settled, still the rows it had: a refresh is not a withdrawal.
    expect(result.current.ledgerSettled).toBe(true);
    expect(result.current.modelRows).toHaveLength(1);

    await act(async () => usage.answer([usageRow, { ...usageRow, modelId: 'other' }]));
    await waitFor(() => expect(result.current.modelRows).toHaveLength(2));
  });
});

/**
 * In a live chat the `session` row is the one the chat was opened with: its
 * counters never move, and only the stream's `tokenState` does. Keyed on the row
 * alone, the ledger was read once on open, so after a first reply the footer
 * had no By model list and priced the counters instead, and after later turns
 * it kept the total it had ($0.12 where a reload showed $0.18).
 */
describe('useCostTracking: a live chat’s counters refresh the ledger', () => {
  const live = (input: number, output: number) => ({
    accumulatedInputTokens: input,
    accumulatedOutputTokens: output,
    accumulatedTotalTokens: input + output,
  });
  /** The row a live chat holds: read on open, never refreshed by a turn. */
  const openedRow = chat('a', 0);
  const renderLive = (counters = live(0, 0)) =>
    renderHook(({ liveCounters }) => useCostTracking({ session: openedRow, liveCounters }), {
      initialProps: { liveCounters: counters },
    });

  it('re-reads after each turn, with the chat’s row unchanged and nothing remounted', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [] } });
    const { result, rerender } = renderLive();
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));
    expect(result.current.modelRows).toEqual([]);

    // The first reply recorded its usage.
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [usageRow] } });
    rerender({ liveCounters: live(100, 20) });
    await waitFor(() => expect(result.current.modelRows).toHaveLength(1));
    expect(mocks.getSessionUsage).toHaveBeenCalledTimes(2);

    // A second turn: the new answer replaces the old one, which held meanwhile.
    const usage = deferred();
    mocks.getSessionUsage.mockReturnValue(usage.promise);
    rerender({ liveCounters: live(300, 60) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(3));
    expect(result.current.ledgerSettled).toBe(true);
    expect(result.current.modelRows[0]).toMatchObject({ inputTokens: 100, turns: 2 });
    await act(async () =>
      usage.answer([{ ...usageRow, inputTokens: 300, outputTokens: 60, turns: 4 }])
    );
    await waitFor(() =>
      expect(result.current.modelRows[0]).toMatchObject({ inputTokens: 300, turns: 4 })
    );
    for (const call of mocks.getSessionUsage.mock.calls) {
      expect(call[0]).toMatchObject({
        path: { session_id: 'a' },
        headers: { 'X-User-Action': 'synthetic-proof' },
      });
    }
  });

  it('asks nothing of the frames that only copy the counters it has', async () => {
    mocks.getSessionUsage.mockResolvedValue({ data: { models: [usageRow] } });
    const { result, rerender } = renderLive(live(100, 20));
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));

    // Every streamed token's frame carries a new copy of the same counters.
    for (let token = 0; token < 50; token += 1) rerender({ liveCounters: live(100, 20) });
    await act(async () => {});
    expect(mocks.getSessionUsage).toHaveBeenCalledTimes(1);

    rerender({ liveCounters: live(150, 40) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(2));
  });

  it('never lets a read that a newer one superseded land', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [] } });
    const { result, rerender } = renderLive();
    await waitFor(() => expect(result.current.ledgerSettled).toBe(true));

    const older = deferred();
    const newer = deferred();
    mocks.getSessionUsage.mockReturnValueOnce(older.promise).mockReturnValueOnce(newer.promise);
    rerender({ liveCounters: live(100, 20) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(2));
    rerender({ liveCounters: live(300, 60) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(3));

    await act(async () => newer.answer([usageRow, { ...usageRow, modelId: 'other' }]));
    await waitFor(() => expect(result.current.modelRows).toHaveLength(2));
    // The older read answers last, with what the ledger held before the newer.
    await act(async () => older.answer([usageRow]));
    await act(async () => {});
    expect(result.current.modelRows).toHaveLength(2);
  });

  it('keeps the answer it has when a refresh fails, and settles a first read that fails', async () => {
    mocks.getSessionUsage.mockResolvedValueOnce({ data: { models: [usageRow] } });
    const { result, rerender } = renderLive(live(100, 20));
    await waitFor(() => expect(result.current.modelRows).toHaveLength(1));

    mocks.getSessionUsage.mockRejectedValueOnce(new Error('offline'));
    rerender({ liveCounters: live(150, 40) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(2));
    await act(async () => {});
    expect(result.current.modelRows).toHaveLength(1);

    // A refused read (`throwOnError: false`) is a failure too, not an empty ledger.
    mocks.getSessionUsage.mockResolvedValueOnce({ data: undefined, error: { message: 'no' } });
    rerender({ liveCounters: live(200, 50) });
    await waitFor(() => expect(mocks.getSessionUsage).toHaveBeenCalledTimes(3));
    await act(async () => {});
    expect(result.current.ledgerSettled).toBe(true);
    expect(result.current.modelRows).toHaveLength(1);

    mocks.getSessionUsage.mockResolvedValueOnce({ data: undefined, error: { message: 'no' } });
    const first = renderHook(() => useCostTracking({ session: chat('b') }));
    await waitFor(() => expect(first.result.current.ledgerSettled).toBe(true));
    expect(first.result.current.modelRows).toEqual([]);
  });
});
