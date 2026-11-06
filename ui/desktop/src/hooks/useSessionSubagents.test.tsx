import { act, renderHook, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { Message } from '../api';
import { wireRow, wireSubagents } from '../utils/sessionSubagents.fixture';
import { SUBAGENT_POLL_MS, useSessionSubagents } from './useSessionSubagents';

const mocks = vi.hoisted(() => ({ get: vi.fn(), headers: vi.fn() }));
vi.mock('../api', () => ({ getSessionSubagents: mocks.get }));
vi.mock('../utils/userAction', () => ({ userActionHeaders: mocks.headers }));

const ok = (body: unknown) => ({ data: body, error: undefined, response: { status: 200 } });

function spawned(id: string, name = 'workspace__subagent'): Message[] {
  return [
    {
      role: 'assistant',
      created: 0,
      metadata: { agentVisible: true, userVisible: true },
      content: [
        { type: 'toolRequest', id, toolCall: { status: 'success', value: { name } } },
        { type: 'toolResponse', id, toolResult: { status: 'success', value: { content: [] } } },
      ],
    },
  ] as Message[];
}

/**
 * Advance fake time in small acts. Inside ONE act, React holds the state a timer
 * sets (and the effect it re-runs) until the act ends, so a poll timer, the read
 * it triggers and the read's own coalescing timer would never chain.
 */
async function advance(ms: number) {
  for (let elapsed = 0; elapsed < ms; elapsed += 50) {
    await act(() => vi.advanceTimersByTimeAsync(Math.min(50, ms - elapsed)));
  }
}

const finished = wireSubagents([wireRow({ status: 'completed', quietSeconds: null })]);
const running = wireSubagents([wireRow({ status: 'running' })]);

beforeEach(() => {
  mocks.get.mockReset().mockResolvedValue(ok(finished));
  mocks.headers.mockReset().mockResolvedValue({ 'X-User-Action': 'synthetic-proof' });
});

afterEach(() => {
  vi.useRealTimers();
});

describe('useSessionSubagents', () => {
  it('reads nothing while the summary is hidden and no turn runs', async () => {
    renderHook(() => useSessionSubagents('a', [], { visible: false, turnRunning: false }));
    await act(() => new Promise((resolve) => setTimeout(resolve, 120)));
    expect(mocks.get).not.toHaveBeenCalled();
  });

  it('reads the route with the user-action proof once the summary is shown', async () => {
    const { result } = renderHook(() =>
      useSessionSubagents('a', [], { visible: true, turnRunning: false })
    );
    expect(result.current.loading).toBe(true);
    await waitFor(() => expect(result.current.data?.subagents).toHaveLength(1));
    expect(result.current.loading).toBe(false);
    expect(mocks.get).toHaveBeenCalledWith(
      expect.objectContaining({
        path: { session_id: 'a' },
        headers: { 'X-User-Action': 'synthetic-proof' },
      })
    );
  });

  it('re-reads when a roster tool answers, and not for prose', async () => {
    const { result, rerender } = renderHook(
      ({ messages }) => useSessionSubagents('a', messages, { visible: true, turnRunning: false }),
      { initialProps: { messages: [] as Message[] } }
    );
    await waitFor(() => expect(result.current.data).not.toBeNull());
    expect(mocks.get).toHaveBeenCalledTimes(1);

    rerender({
      messages: [
        { role: 'assistant', created: 0, content: [{ type: 'text', text: 'Hi' }] },
      ] as unknown as Message[],
    });
    await act(() => new Promise((resolve) => setTimeout(resolve, 120)));
    expect(mocks.get).toHaveBeenCalledTimes(1);

    rerender({ messages: spawned('one') });
    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(2));
    rerender({ messages: [...spawned('one'), ...spawned('two', 'workspace__workspace_watch')] });
    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(3));
  });

  it('polls every 2s while a child is active, and stops once none is', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.get.mockResolvedValue(ok(running));
    const { result } = renderHook(() =>
      useSessionSubagents('a', [], { visible: true, turnRunning: false })
    );
    await advance(100);
    expect(mocks.get).toHaveBeenCalledTimes(1);
    expect(result.current.data?.subagents[0].status).toBe('running');

    await advance(SUBAGENT_POLL_MS + 100);
    expect(mocks.get).toHaveBeenCalledTimes(2);

    mocks.get.mockResolvedValue(ok(finished));
    await advance(SUBAGENT_POLL_MS + 100);
    expect(mocks.get).toHaveBeenCalledTimes(3);
    expect(result.current.data?.subagents[0].status).toBe('completed');

    // Nothing active and no turn: no more reads.
    await advance(SUBAGENT_POLL_MS * 5);
    expect(mocks.get).toHaveBeenCalledTimes(3);
  });

  it('does not re-render the chat for a poll that changed nothing', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.get.mockResolvedValue(ok(running));
    let renders = 0;
    renderHook(() => {
      renders += 1;
      return useSessionSubagents('a', [], { visible: true, turnRunning: false });
    });
    await advance(100);
    const settled = renders;
    await advance(SUBAGENT_POLL_MS * 5 + 100);
    expect(mocks.get.mock.calls.length).toBeGreaterThanOrEqual(6);
    // React may call a component once more before it bails out of an identical
    // state; it never renders once per poll.
    expect(renders).toBeLessThanOrEqual(settled + 1);
    // A changed answer does render.
    mocks.get.mockResolvedValue(ok(wireSubagents([wireRow({ elapsedSeconds: 99 })])));
    const before = renders;
    await advance(SUBAGENT_POLL_MS + 100);
    expect(renders).toBeGreaterThan(before);
  });

  it('does not re-render a hidden summary’s chat for counters that tick, and shows the newest when shown', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    // An active child as the daemon reports it: its run time and quiet time are
    // computed fresh on every read, and its tokens move as it works.
    let tick = 0;
    const ticking = (status = 'running') =>
      ok(
        wireSubagents([
          wireRow({
            status,
            elapsedSeconds: 60 + tick * 2,
            quietSeconds: tick * 2,
            totalTokens: 1000 * tick,
            updatedAt: `2026-11-04T10:0${tick % 10}:00Z`,
          }),
        ])
      );
    mocks.get.mockImplementation(async () => {
      tick += 1;
      return ticking();
    });
    let renders = 0;
    const { result, rerender } = renderHook(
      ({ visible }) => {
        renders += 1;
        return useSessionSubagents('a', [], { visible, turnRunning: false });
      },
      { initialProps: { visible: true } }
    );
    await advance(100);
    expect(result.current.data?.subagents[0].status).toBe('running');
    // Folded away while the child keeps running.
    rerender({ visible: false });
    const settled = renders;
    await advance(SUBAGENT_POLL_MS * 5 + 100);
    expect(mocks.get.mock.calls.length).toBeGreaterThanOrEqual(6);
    expect(renders).toBeLessThanOrEqual(settled + 1);

    // Shown again: the newest read goes up at once, not at the next poll.
    rerender({ visible: true });
    expect(result.current.data?.subagents[0].elapsedSeconds).toBe(60 + tick * 2);
    expect(result.current.data?.subagents[0].totalTokens).toBe(1000 * tick);

    // Visible, every tick draws. Hidden again, a change of state still does.
    const shownRenders = renders;
    await advance(SUBAGENT_POLL_MS + 100);
    expect(renders).toBeGreaterThan(shownRenders);
    rerender({ visible: false });
    mocks.get.mockImplementation(async () => {
      tick += 1;
      return ticking('completed');
    });
    await advance(SUBAGENT_POLL_MS + 100);
    expect(result.current.data?.subagents[0].status).toBe('completed');
  });

  it('keeps polling a background child after the summary is hidden, until it ends', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.get.mockResolvedValue(ok(running));
    const { result, rerender } = renderHook(
      ({ visible }) => useSessionSubagents('a', [], { visible, turnRunning: false }),
      { initialProps: { visible: true } }
    );
    await advance(100);
    rerender({ visible: false });
    await advance(SUBAGENT_POLL_MS + 100);
    expect(mocks.get).toHaveBeenCalledTimes(2);
    mocks.get.mockResolvedValue(ok(finished));
    await advance(SUBAGENT_POLL_MS + 100);
    expect(result.current.data?.subagents[0].status).toBe('completed');
    const reads = mocks.get.mock.calls.length;
    await advance(SUBAGENT_POLL_MS * 5);
    expect(mocks.get).toHaveBeenCalledTimes(reads);
  });

  it('polls while the turn runs only in a chat that has subagents', async () => {
    vi.useFakeTimers({ shouldAdvanceTime: false });
    mocks.get.mockResolvedValue(ok(wireSubagents([])));
    const { rerender } = renderHook(
      ({ messages }) => useSessionSubagents('a', messages, { visible: false, turnRunning: true }),
      { initialProps: { messages: [] as Message[] } }
    );
    await advance(100);
    expect(mocks.get).toHaveBeenCalledTimes(1);
    // A turn with no subagent in sight reads once, then waits for a spawn.
    await advance(SUBAGENT_POLL_MS * 3);
    expect(mocks.get).toHaveBeenCalledTimes(1);

    mocks.get.mockResolvedValue(ok(finished));
    rerender({ messages: spawned('one') });
    await advance(100);
    expect(mocks.get).toHaveBeenCalledTimes(2);
    // The turn is still running and the chat now has subagents: keep reading.
    await advance(SUBAGENT_POLL_MS + 100);
    expect(mocks.get).toHaveBeenCalledTimes(3);
  });

  it('keeps the last good data when a read fails, and recovers on refresh', async () => {
    const { result } = renderHook(() =>
      useSessionSubagents('a', [], { visible: true, turnRunning: false })
    );
    await waitFor(() => expect(result.current.data?.subagents).toHaveLength(1));
    mocks.get.mockResolvedValueOnce({ data: undefined, error: 'boom', response: { status: 500 } });
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.error).toBe(true));
    expect(result.current.data?.subagents).toHaveLength(1);
    act(() => result.current.refresh());
    await waitFor(() => expect(result.current.error).toBe(false));
  });

  it('reads a daemon without the route as no roster, not as an error', async () => {
    mocks.get.mockResolvedValue({ data: undefined, error: 'nope', response: { status: 404 } });
    const { result } = renderHook(() =>
      useSessionSubagents('a', [], { visible: true, turnRunning: false })
    );
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.error).toBe(false);
    expect(result.current.data).toBeNull();
  });

  it('aborts a superseded read and never shows another session’s answer', async () => {
    let finish!: (value: unknown) => void;
    const signals: AbortSignal[] = [];
    mocks.get.mockImplementationOnce(
      ({ signal }: { signal: AbortSignal }) =>
        new Promise((resolve) => {
          signals.push(signal);
          finish = resolve;
        })
    );
    const { result, rerender } = renderHook(
      ({ id }) => useSessionSubagents(id, [], { visible: true, turnRunning: false }),
      { initialProps: { id: 'a' } }
    );
    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(1));
    mocks.get.mockResolvedValueOnce(ok(wireSubagents([], null, 'b')));
    rerender({ id: 'b' });
    expect(result.current.data).toBeNull();
    expect(signals[0].aborted).toBe(true);
    await waitFor(() => expect(mocks.get).toHaveBeenCalledTimes(2));
    await act(async () => finish(ok(running)));
    await waitFor(() => expect(result.current.loading).toBe(false));
    expect(result.current.data?.subagents).toEqual([]);
  });
});
