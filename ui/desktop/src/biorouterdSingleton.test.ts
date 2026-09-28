import { describe, it, expect, beforeEach, vi } from 'vitest';
import type { BiorouterdResult, StartBiorouterdOptions } from './biorouterd';
import {
  createDaemonReattachController,
  getSharedBackend,
  resetSharedBackend,
  isSharedDaemonEnabled,
  type DaemonReattachDeps,
  type DaemonRestartChoice,
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
    const deps = {
      ask: vi.fn<DaemonReattachDeps['ask']>(async () => 'reconnect'),
      reportFailure: vi.fn<DaemonReattachDeps['reportFailure']>(async () => 'close'),
      reconnect: vi.fn<DaemonReattachDeps['reconnect']>(async () => undefined),
      restart: vi.fn<DaemonReattachDeps['restart']>(),
      ...overrides,
      broadcast,
    };
    return { deps, reattach: createDaemonReattachController(deps) };
  };

  it('asks once, and reattaches only after the person chose Reconnect', async () => {
    const answer = deferred<DaemonRestartChoice>();
    const { deps, reattach } = controller({ ask: vi.fn(() => answer.promise) });
    const first = reattach.lost();
    const second = reattach.lost();
    expect(deps.ask).toHaveBeenCalledTimes(1);
    expect(deps.reconnect).not.toHaveBeenCalled();
    expect(reattach.state()).toBe('lost');
    answer.resolve('reconnect');
    await expect(first).resolves.toBe(true);
    await expect(second).resolves.toBe(true);
    expect(deps.reconnect).toHaveBeenCalledTimes(1);
    expect(reattach.state()).toBe('attached');
    expect(deps.broadcast.mock.calls.map(([state]) => state)).toEqual([
      'lost',
      'reconnecting',
      'attached',
    ]);
  });

  it('never reattaches by itself: Not Now leaves it lost, and asks no more until asked to', async () => {
    const { deps, reattach } = controller({ ask: vi.fn(async () => 'later' as const) });
    await expect(reattach.lost()).resolves.toBe(false);
    await expect(reattach.lost()).resolves.toBe(false);
    expect(deps.ask).toHaveBeenCalledTimes(1);
    expect(deps.reconnect).not.toHaveBeenCalled();
    expect(reattach.state()).toBe('lost');
    // A new window asks again.
    await reattach.lost({ ask: true });
    expect(deps.ask).toHaveBeenCalledTimes(2);
  });

  it('quits and reopens when the person chooses that', async () => {
    const { deps, reattach } = controller({ ask: vi.fn(async () => 'restart' as const) });
    await expect(reattach.lost()).resolves.toBe(false);
    expect(deps.restart).toHaveBeenCalledTimes(1);
    expect(deps.reconnect).not.toHaveBeenCalled();
  });

  it('says why a reconnect failed, stays lost, and offers to quit and reopen', async () => {
    const { deps, reattach } = controller({
      reconnect: vi.fn(async () => {
        throw new Error('Daemon attachment cancelled.');
      }),
      reportFailure: vi.fn(async () => 'restart' as const),
    });
    await expect(reattach.lost()).resolves.toBe(false);
    expect(deps.reportFailure).toHaveBeenCalledWith('Daemon attachment cancelled.');
    expect(deps.restart).toHaveBeenCalledTimes(1);
    expect(reattach.state()).toBe('lost');
  });

  it('runs one reconnect at a time from the sidebar, without asking again', async () => {
    const done = deferred<void>();
    const { deps, reattach } = controller({
      ask: vi.fn(async () => 'later' as const),
      reconnect: vi.fn(() => done.promise),
    });
    await reattach.lost();
    const first = reattach.reconnect();
    const second = reattach.reconnect();
    expect(reattach.state()).toBe('reconnecting');
    // A loss reported meanwhile joins the reconnect instead of asking.
    const during = reattach.lost();
    expect(deps.ask).toHaveBeenCalledTimes(1);
    done.resolve();
    await expect(Promise.all([first, second, during])).resolves.toEqual([true, true, true]);
    expect(deps.reconnect).toHaveBeenCalledTimes(1);
    expect(reattach.state()).toBe('attached');
  });

  it('is attached again, with nothing asked, when the lost instance answers by itself', async () => {
    const { deps, reattach } = controller({ ask: vi.fn(async () => 'later' as const) });
    await reattach.lost();
    reattach.answered();
    expect(reattach.state()).toBe('attached');
    expect(deps.reconnect).not.toHaveBeenCalled();
    expect(deps.broadcast.mock.calls.map(([state]) => state)).toEqual(['lost', 'attached']);
    // A reconnect while attached does nothing.
    await expect(reattach.reconnect()).resolves.toBe(true);
    expect(deps.reconnect).not.toHaveBeenCalled();
  });
});
