import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';

const mockLlamacppStatus = vi.fn();

vi.mock('../../../api', () => ({
  llamacppStatus: (...args: unknown[]) => mockLlamacppStatus(...args),
}));

import {
  llamaServerStore,
  resetLlamaServerStoreForTests,
  LLAMA_SERVER_POLL_INTERVAL_MS,
  LLAMA_SERVER_OPERATION_TIMEOUT_MS,
} from './llamaServerStore';

const statusResponse = (sidecar: Record<string, unknown>) => ({
  data: {
    sidecar: {
      state: 'starting',
      warmed: false,
      build: 'test',
      detail: null,
      model: 'qwen3.8-27b',
      ...sidecar,
    },
    catalog: [],
    system: {
      os: 'macos',
      total_memory_gib: 64,
      accelerator_memory_gib: 64,
      accelerator_memory_kind: 'apple_unified',
      default_context_size: 131072,
      model_cache_dir: '/tmp/models',
      model_cache_layout: 'test',
    },
  },
});

const advanceTicks = async (ticks: number) => {
  await vi.advanceTimersByTimeAsync(ticks * LLAMA_SERVER_POLL_INTERVAL_MS);
};

describe('llamaServerStore', () => {
  beforeEach(() => {
    vi.useFakeTimers();
    resetLlamaServerStoreForTests();
    mockLlamacppStatus.mockReset();
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'starting' }));
  });

  afterEach(() => {
    resetLlamaServerStoreForTests();
    vi.useRealTimers();
  });

  it('polls while an operation is in flight, even with zero subscribers', async () => {
    llamaServerStore.beginOperation('install', 'qwen3.8-27b', 'Preparing install...');

    await advanceTicks(3);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(3);
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({
      kind: 'install',
      model: 'qwen3.8-27b',
    });
    expect(llamaServerStore.getSnapshot().status).not.toBeNull();
  });

  it('keeps polling after the only subscriber unsubscribes (unmount)', async () => {
    const listener = vi.fn();
    const unsubscribe = llamaServerStore.subscribe(listener);
    llamaServerStore.beginOperation('install', 'qwen3.8-27b');

    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);

    unsubscribe();
    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(4);
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();
  });

  it('two subscribers share a single poll interval', async () => {
    const a = vi.fn();
    const b = vi.fn();
    llamaServerStore.subscribe(a);
    llamaServerStore.subscribe(b);
    llamaServerStore.beginOperation('install', 'qwen3.8-27b');

    await advanceTicks(1);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(1);
    expect(a).toHaveBeenCalled();
    expect(b).toHaveBeenCalled();
  });

  it('a late subscriber immediately reads the latest snapshot', async () => {
    llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'starting', detail: 'downloading 42%' })
    );
    await advanceTicks(1);

    // No prior subscription needed: getSnapshot reflects the poll results.
    const snap = llamaServerStore.getSnapshot();
    expect(snap.status?.sidecar.detail).toBe('downloading 42%');
    expect(snap.operation?.message).toBe('downloading 42%');
  });

  it('install operations stop polling once ready with the matching model', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    const ready = llamaServerStore.waitForReady('qwen3.8-27b', opId);

    await advanceTicks(1);
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'ready', model: 'qwen3.8-27b' }));
    await advanceTicks(1);

    await expect(ready).resolves.toBeUndefined();
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    const calls = mockLlamacppStatus.mock.calls.length;
    await advanceTicks(3);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(calls);
  });

  it('does not treat ready-with-other-model as terminal', async () => {
    llamaServerStore.beginOperation('install', 'gemma4-31b');
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'ready', model: 'qwen3.8-27b' }));

    await advanceTicks(2);
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);
  });

  it('start operations keep polling past ready until endOperation', async () => {
    const opId = llamaServerStore.beginOperation('start', 'qwen3.8-27b');
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'ready', model: 'qwen3.8-27b' }));

    await advanceTicks(2);
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);

    expect(llamaServerStore.endOperation(opId)).toBe(true);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);
  });

  it('stops polling, rejects waiters, and retains lastError on sidecar error', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    const ready = llamaServerStore.waitForReady('qwen3.8-27b', opId);
    const rejection = expect(ready).rejects.toThrow('download failed: boom');

    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'error', detail: 'download failed: boom' })
    );
    await advanceTicks(1);

    await rejection;
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    expect(llamaServerStore.getSnapshot().lastError).toMatchObject({
      opId,
      kind: 'install',
      model: 'qwen3.8-27b',
      message: 'download failed: boom',
    });

    const calls = mockLlamacppStatus.mock.calls.length;
    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(calls);
  });

  it('claimErrorToast grants the terminal error exactly once, to the right op', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'error', detail: 'boom' }));
    await advanceTicks(1);

    expect(llamaServerStore.claimErrorToast(opId + 1)).toBe(false);
    expect(llamaServerStore.claimErrorToast(opId)).toBe(true);
    expect(llamaServerStore.claimErrorToast(opId)).toBe(false);

    // A new operation clears the retained error.
    llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    expect(llamaServerStore.getSnapshot().lastError).toBeNull();
    expect(llamaServerStore.claimErrorToast(opId)).toBe(false);
  });

  it('times out after 60 minutes and stops polling', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    const ready = llamaServerStore.waitForReady('qwen3.8-27b', opId);
    const rejection = expect(ready).rejects.toThrow('Timed out waiting for the local model');

    await vi.advanceTimersByTimeAsync(
      LLAMA_SERVER_OPERATION_TIMEOUT_MS + 2 * LLAMA_SERVER_POLL_INTERVAL_MS
    );

    await rejection;
    expect(llamaServerStore.getSnapshot().operation).toBeNull();

    const calls = mockLlamacppStatus.mock.calls.length;
    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(calls);
  });

  it('times out even when every status request fails', async () => {
    mockLlamacppStatus.mockRejectedValue(new Error('connection refused'));
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b');
    const ready = llamaServerStore.waitForReady('qwen3.8-27b', opId);
    const rejection = expect(ready).rejects.toThrow('Timed out waiting for the local model');

    await vi.advanceTimersByTimeAsync(
      LLAMA_SERVER_OPERATION_TIMEOUT_MS + 2 * LLAMA_SERVER_POLL_INTERVAL_MS
    );

    await rejection;
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
  });

  it('times out a warm-up hung in the ready state (ready branch cannot mask the deadline)', async () => {
    const opId = llamaServerStore.beginOperation('warmup', 'qwen3.8-27b');
    mockLlamacppStatus.mockResolvedValue(statusResponse({ state: 'ready', model: 'qwen3.8-27b' }));

    // Ready-with-matching-model is NOT terminal for warm-up ops: they poll on
    // while the driving warm-up HTTP call runs...
    await advanceTicks(3);
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();

    // ...but the independent deadline still fires even though every tick
    // takes the ready branch (the old code returned before its timeout check).
    await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    expect(llamaServerStore.getSnapshot().lastError).toMatchObject({
      opId,
      kind: 'warmup',
      message: expect.stringContaining('Timed out'),
    });

    const calls = mockLlamacppStatus.mock.calls.length;
    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(calls);
  });

  it('times out poll-less operations (stalled Ollama pull cannot stay busy forever)', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b', 'Preparing install...', {
      poll: false,
    });

    await vi.advanceTimersByTimeAsync(LLAMA_SERVER_OPERATION_TIMEOUT_MS - 1);
    expect(llamaServerStore.getSnapshot().operation).not.toBeNull();

    await vi.advanceTimersByTimeAsync(2);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    expect(llamaServerStore.getSnapshot().lastError).toMatchObject({
      opId,
      message: expect.stringContaining('Timed out'),
    });
    expect(mockLlamacppStatus).not.toHaveBeenCalled();
  });

  it('a stale status response from op A cannot update or terminate op B', async () => {
    // Op A's first tick hangs on a deferred status request.
    let resolveStale!: (value: unknown) => void;
    mockLlamacppStatus.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveStale = resolve;
        })
    );
    llamaServerStore.beginOperation('install', 'model-a');
    await advanceTicks(1);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(1);

    // Op B supersedes A while A's request is still in flight.
    mockLlamacppStatus.mockResolvedValue(
      statusResponse({ state: 'starting', model: 'model-b', detail: 'b downloading' })
    );
    const opB = llamaServerStore.beginOperation('install', 'model-b');
    await advanceTicks(1);
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({
      id: opB,
      model: 'model-b',
      message: 'b downloading',
    });

    // A's stale response finally lands with a terminal error — it must be
    // discarded wholesale: op B stays alive, untouched, with no lastError.
    resolveStale(statusResponse({ state: 'error', model: 'model-a', detail: 'a exploded' }));
    await vi.advanceTimersByTimeAsync(0);

    const snap = llamaServerStore.getSnapshot();
    expect(snap.operation).toMatchObject({ id: opB, model: 'model-b', message: 'b downloading' });
    expect(snap.lastError).toBeNull();
    expect(snap.status?.sidecar.detail).toBe('b downloading');

    // B's poll loop is still running.
    const calls = mockLlamacppStatus.mock.calls.length;
    await advanceTicks(1);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(calls + 1);

    llamaServerStore.endOperation(opB);
  });

  it("a superseded caller's endOperation is a no-op for the replacement op", async () => {
    const opA = llamaServerStore.beginOperation('start', 'model-a');
    const opB = llamaServerStore.beginOperation('start', 'model-b');

    // The superseded flow's finally block fires late — it must not stop op B.
    expect(llamaServerStore.endOperation(opA)).toBe(false);
    expect(llamaServerStore.getSnapshot().operation).toMatchObject({ id: opB, model: 'model-b' });

    await advanceTicks(2);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);

    expect(llamaServerStore.endOperation(opB)).toBe(true);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
    // Ending twice is also a no-op.
    expect(llamaServerStore.endOperation(opB)).toBe(false);
  });

  it('skips interval ticks while a status request is still in flight', async () => {
    let resolveSlow!: (value: unknown) => void;
    mockLlamacppStatus.mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveSlow = resolve;
        })
    );
    llamaServerStore.beginOperation('install', 'qwen3.8-27b');

    // Three interval firings, one hung request: no overlapping calls.
    await advanceTicks(3);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(1);

    // Once the slow request settles, polling resumes normally.
    resolveSlow(statusResponse({ state: 'starting' }));
    await advanceTicks(1);
    expect(mockLlamacppStatus).toHaveBeenCalledTimes(2);
  });

  it('waitForReady rejects when no operation is polling', async () => {
    await expect(llamaServerStore.waitForReady('qwen3.8-27b', 1)).rejects.toThrow(
      'No Llama Server operation is polling'
    );
  });

  it('waitForReady rejects for a superseded operation id', async () => {
    const opA = llamaServerStore.beginOperation('install', 'model-a');
    llamaServerStore.beginOperation('install', 'model-b');

    await expect(llamaServerStore.waitForReady('model-a', opA)).rejects.toThrow(
      'No Llama Server operation is polling'
    );
  });

  it('poll-less operations (Ollama pull) track scoped messages and never poll', async () => {
    const opId = llamaServerStore.beginOperation('install', 'qwen3.8-27b', 'Preparing install...', {
      poll: false,
    });
    llamaServerStore.setOperationMessage(opId, 'pulling manifest 10%');
    // A stale (superseded) caller's message is ignored.
    llamaServerStore.setOperationMessage(opId - 1, 'stale message');

    await advanceTicks(3);
    expect(mockLlamacppStatus).not.toHaveBeenCalled();
    expect(llamaServerStore.getSnapshot().operation?.message).toBe('pulling manifest 10%');

    llamaServerStore.endOperation(opId);
    expect(llamaServerStore.getSnapshot().operation).toBeNull();
  });
});
