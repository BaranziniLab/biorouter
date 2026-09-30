import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { BiorouterdResult, StartBiorouterdOptions } from './biorouterd';
import {
  createDaemonReattachController,
  getSharedBackend,
  resetSharedBackend,
  isSharedDaemonEnabled,
  DAEMON_RECONNECT_BACKOFF_MS,
  type DaemonReattachDeps,
} from './biorouterdSingleton';

const fakeResult = (baseUrl: string): BiorouterdResult => ({
  baseUrl,
  managed: true,
  workingDir: '/home/tester',
  // A ChildProcess stand-in — the singleton never touches it.
  process: { kill: () => {} } as unknown as BiorouterdResult['process'],
  errorLog: [],
});

const opts = {} as StartBiorouterdOptions;

describe('getSharedBackend', () => {
  beforeEach(() => resetSharedBackend());

  it('starts the backend exactly once across N concurrent calls', async () => {
    const start = vi.fn(async () => fakeResult('http://127.0.0.1:5001'));

    const [a, b, c] = await Promise.all([
      getSharedBackend(start, opts),
      getSharedBackend(start, opts),
      getSharedBackend(start, opts),
    ]);

    expect(start).toHaveBeenCalledTimes(1);
    // All windows get the same daemon result.
    expect(a).toBe(b);
    expect(b).toBe(c);
    expect(a.baseUrl).toBe('http://127.0.0.1:5001');
  });

  it('reuses the same backend across sequential calls', async () => {
    const start = vi.fn(async () => fakeResult('http://127.0.0.1:5002'));

    const first = await getSharedBackend(start, opts);
    const second = await getSharedBackend(start, opts);

    expect(start).toHaveBeenCalledTimes(1);
    expect(first).toBe(second);
  });

  it('re-initializes after reset', async () => {
    const start = vi
      .fn<(o: StartBiorouterdOptions) => Promise<BiorouterdResult>>()
      .mockResolvedValueOnce(fakeResult('http://127.0.0.1:5003'))
      .mockResolvedValueOnce(fakeResult('http://127.0.0.1:5004'));

    const first = await getSharedBackend(start, opts);
    resetSharedBackend();
    const second = await getSharedBackend(start, opts);

    expect(start).toHaveBeenCalledTimes(2);
    expect(first.baseUrl).toBe('http://127.0.0.1:5003');
    expect(second.baseUrl).toBe('http://127.0.0.1:5004');
  });

  it('does not cache a rejected start (a later window can retry)', async () => {
    const start = vi
      .fn<(o: StartBiorouterdOptions) => Promise<BiorouterdResult>>()
      .mockRejectedValueOnce(new Error('spawn failed'))
      .mockResolvedValueOnce(fakeResult('http://127.0.0.1:5005'));

    await expect(getSharedBackend(start, opts)).rejects.toThrow('spawn failed');
    // Retry succeeds because the failed promise was not cached.
    const ok = await getSharedBackend(start, opts);

    expect(start).toHaveBeenCalledTimes(2);
    expect(ok.baseUrl).toBe('http://127.0.0.1:5005');
  });
});

describe('isSharedDaemonEnabled', () => {
  it('defaults to on when unset', () => {
    expect(isSharedDaemonEnabled({})).toBe(true);
  });

  it.each(['0', 'false', 'FALSE', 'off', 'No', ' false '])(
    'is off when BIOROUTER_SHARED_DAEMON=%s',
    (flag) => {
      expect(isSharedDaemonEnabled({ BIOROUTER_SHARED_DAEMON: flag })).toBe(false);
    }
  );

  it.each(['1', 'true', 'on', 'yes', 'anything'])(
    'is on when BIOROUTER_SHARED_DAEMON=%s',
    (flag) => {
      expect(isSharedDaemonEnabled({ BIOROUTER_SHARED_DAEMON: flag })).toBe(true);
    }
  );
});

describe('createDaemonReattachController (R-1)', () => {
  const deferred = <T>() => {
    let resolve!: (value: T) => void;
    const promise = new Promise<T>((r) => (resolve = r));
    return { promise, resolve };
  };
  const controller = (overrides: Partial<DaemonReattachDeps> = {}) => {
    const broadcast = vi.fn<DaemonReattachDeps['broadcast']>();
    const waits: number[] = [];
    const deps = {
      reportFailure: vi.fn<DaemonReattachDeps['reportFailure']>(async () => 'close'),
      reconnect: vi.fn<DaemonReattachDeps['reconnect']>(async () => undefined),
      restart: vi.fn<DaemonReattachDeps['restart']>(),
      wait: vi.fn(async (ms: number) => {
        waits.push(ms);
      }),
      ...overrides,
      broadcast,
    };
    return { deps, waits, reattach: createDaemonReattachController(deps) };
  };
  const states = (deps: { broadcast: ReturnType<typeof vi.fn> }) =>
    deps.broadcast.mock.calls.map(([state]) => state);

  it('asks nothing: it has no way to, and reconnects on its own', async () => {
    const { deps, reattach } = controller();
    // The dependencies are the whole of what the controller can do: there is no prompt among them.
    expect(Object.keys(deps).sort()).toEqual([
      'broadcast',
      'reconnect',
      'reportFailure',
      'restart',
      'wait',
    ]);
    await expect(reattach.lost()).resolves.toBe(true);
    expect(deps.reconnect).toHaveBeenCalledTimes(1);
    expect(deps.reportFailure).not.toHaveBeenCalled();
    expect(reattach.state()).toBe('attached');
    expect(states(deps)).toEqual(['reconnecting', 'attached']);
  });

  it('runs one reconnect at a time, however many losses are reported meanwhile', async () => {
    const done = deferred<void>();
    const { deps, reattach } = controller({ reconnect: vi.fn(() => done.promise) });
    const first = reattach.lost();
    const second = reattach.lost();
    const fromSidebar = reattach.reconnect();
    expect(reattach.state()).toBe('reconnecting');
    done.resolve();
    await expect(Promise.all([first, second, fromSidebar])).resolves.toEqual([true, true, true]);
    expect(deps.reconnect).toHaveBeenCalledTimes(1);
  });

  it('tries three times with 1 s and 2 s between, then says why once and stays lost', async () => {
    const { deps, waits, reattach } = controller({
      reconnect: vi.fn(async () => {
        throw new Error('The background service stopped answering.');
      }),
    });
    await expect(reattach.lost()).resolves.toBe(false);
    expect(deps.reconnect).toHaveBeenCalledTimes(3);
    expect(waits).toEqual([...DAEMON_RECONNECT_BACKOFF_MS]);
    expect(waits).toEqual([1000, 2000]);
    expect(deps.reportFailure).toHaveBeenCalledTimes(1);
    expect(deps.reportFailure).toHaveBeenCalledWith('The background service stopped answering.');
    expect(reattach.state()).toBe('lost');
    // Only the final failure is broadcast: no window shows a notice while attempts remain.
    expect(states(deps)).toEqual(['reconnecting', 'lost']);
  });

  it('attaches on a later attempt without reporting anything', async () => {
    const reconnect = vi
      .fn<DaemonReattachDeps['reconnect']>()
      .mockRejectedValueOnce(new Error('not yet'))
      .mockResolvedValueOnce(undefined);
    const { deps, reattach } = controller({ reconnect });
    await expect(reattach.lost()).resolves.toBe(true);
    expect(reconnect).toHaveBeenCalledTimes(2);
    expect(deps.reportFailure).not.toHaveBeenCalled();
    expect(reattach.state()).toBe('attached');
  });

  it('quits and reopens when the person chooses that in the failure report', async () => {
    const { deps, reattach } = controller({
      reconnect: vi.fn(async () => {
        throw new Error('nope');
      }),
      reportFailure: vi.fn(async () => 'restart' as const),
    });
    await expect(reattach.lost()).resolves.toBe(false);
    await vi.waitFor(() => expect(deps.restart).toHaveBeenCalledTimes(1));
  });

  it('keeps one failure report on screen at a time', async () => {
    const report = deferred<'restart' | 'close'>();
    const { deps, reattach } = controller({
      reconnect: vi.fn(async () => {
        throw new Error('nope');
      }),
      reportFailure: vi.fn(() => report.promise),
    });
    await reattach.lost();
    await reattach.lost();
    expect(deps.reportFailure).toHaveBeenCalledTimes(1);
    report.resolve('close');
    await report.promise;
    await new Promise((resolve) => setTimeout(resolve, 0));
    await reattach.lost();
    expect(deps.reportFailure).toHaveBeenCalledTimes(2);
  });

  it('Try again from the sidebar is one attempt, and attaches when it works', async () => {
    const reconnect = vi.fn<DaemonReattachDeps['reconnect']>(async () => {
      throw new Error('nope');
    });
    const { deps, reattach } = controller({ reconnect });
    await reattach.lost();
    expect(reconnect).toHaveBeenCalledTimes(3);
    reconnect.mockResolvedValueOnce(undefined);
    await expect(reattach.reconnect()).resolves.toBe(true);
    expect(reconnect).toHaveBeenCalledTimes(4);
    expect(reattach.state()).toBe('attached');
    expect(states(deps)).toEqual(['reconnecting', 'lost', 'reconnecting', 'attached']);
  });

  it('is attached again when the lost instance answers by itself', async () => {
    const { deps, reattach } = controller({
      reconnect: vi.fn(async () => {
        throw new Error('nope');
      }),
    });
    await reattach.lost();
    reattach.answered();
    expect(reattach.state()).toBe('attached');
    expect(states(deps)).toEqual(['reconnecting', 'lost', 'attached']);
    // Try again while attached does nothing.
    await expect(reattach.reconnect()).resolves.toBe(true);
    expect(deps.reconnect).toHaveBeenCalledTimes(3);
  });
});
