// @vitest-environment node
import type { App } from 'electron';
import { EventEmitter } from 'node:events';
import type { PathLike, Stats } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * The shared daemon needs no secret from anyone. The app mints a user-action key for a daemon it
 * starts and saves it in the private key file; for a daemon that is already running it reads that
 * file. A daemon it cannot use (no saved key, a refused key, another version) is stopped and
 * replaced. No path here may ever reach a native secret prompt.
 */
const mocks = vi.hoisted(() => {
  class DaemonKeyRefusedError extends Error {}
  return {
    DaemonKeyRefusedError,
    spawn: vi.fn(),
    discover: vi.fn(),
    verify: vi.fn(),
    createProxy: vi.fn(),
    daemonVersion: vi.fn(),
    readKey: vi.fn(),
    writeKey: vi.fn(),
    removeKey: vi.fn(),
    verifyAccess: vi.fn(),
    stop: vi.fn(),
    prompt: vi.fn(),
    logInfo: vi.fn(),
    logError: vi.fn(),
    logWarn: vi.fn(),
    logDebug: vi.fn(),
  };
});

const isBiorouterdPath = (value: PathLike): boolean => {
  const name = value.toString();
  return name.endsWith('biorouterd') || name.endsWith('biorouterd.exe');
};

vi.mock('node:fs', async (importOriginal) => {
  const actual = await importOriginal<typeof import('node:fs')>();
  const existsSync = (value: PathLike): boolean =>
    isBiorouterdPath(value) ? true : actual.existsSync(value);
  const statSync = (value: PathLike): Stats =>
    isBiorouterdPath(value) ? ({ isFile: () => true } as unknown as Stats) : actual.statSync(value);
  const mocked = { ...actual, existsSync, statSync };
  return { ...mocked, default: mocked };
});

vi.mock('child_process', async (importOriginal) => {
  const actual = await importOriginal<typeof import('child_process')>();
  const mocked = { ...actual, spawn: mocks.spawn };
  return { ...mocked, default: mocked };
});

vi.mock('./utils/logger', () => ({
  default: {
    info: mocks.logInfo,
    error: mocks.logError,
    warn: mocks.logWarn,
    debug: mocks.logDebug,
  },
}));

// A tripwire: nothing on the daemon path may ask a person for a secret.
vi.mock('./nativeSecretPrompt', () => ({
  promptNativeSecret: mocks.prompt,
  closeNativeSecretPrompt: vi.fn(),
}));

vi.mock('./biorouterdSingleton', () => ({ isSharedDaemonEnabled: () => true }));
vi.mock('./daemonRuntime', () => ({
  createDaemonProxy: mocks.createProxy,
  daemonLossOf: (error: { code?: string; syscall?: string } | null) =>
    error?.syscall === 'connect' && ['ENOENT', 'ECONNREFUSED'].includes(error.code ?? '')
      ? 'gone'
      : undefined,
  daemonVersion: mocks.daemonVersion,
  DaemonKeyRefusedError: mocks.DaemonKeyRefusedError,
  discoverDaemonRuntime: mocks.discover,
  generateUserActionKey: () => 'k'.repeat(64),
  readUserActionKey: mocks.readKey,
  removeUserActionKey: mocks.removeKey,
  stopProfileDaemon: mocks.stop,
  verifyDaemonRuntime: mocks.verify,
  verifyHumanAuthorizedAccess: mocks.verifyAccess,
  writeUserActionKey: mocks.writeKey,
}));

import { startBiorouterd } from './biorouterd';

const APP_VERSION = '1.92.1';
const MINTED = 'k'.repeat(64);
const SAVED = 'a'.repeat(64);
const digest = (value: string) => createHash('sha256').update(value).digest('hex') + '\n';

const runtime = {
  version: 1 as const,
  profile_id: '11111111-1111-4111-8111-111111111111',
  instance_id: '22222222-2222-4222-8222-222222222222',
  pid: 88,
  endpoint: { kind: 'unix' as const, path: '/private/tmp/biorouterd.sock' },
  api_secret: 'daemon-secret-123456',
  user_action_installed: true,
};
const fresh = {
  ...runtime,
  instance_id: '33333333-3333-4333-8333-333333333333',
  pid: 99,
  api_secret: 'daemon-secret-fresh-123456',
};

const makeApp = () =>
  ({
    isPackaged: false,
    getVersion: () => APP_VERSION,
    on: vi.fn(),
    once: vi.fn(),
    removeListener: vi.fn(),
  }) as unknown as App;

class FakeChild extends EventEmitter {
  pid: number | undefined;
  exitCode: number | null = null;
  signalCode: string | null = null;
  stdin = { write: vi.fn(), end: vi.fn() };
  stdout = Object.assign(new EventEmitter(), { destroy: vi.fn() });
  stderr = Object.assign(new EventEmitter(), { destroy: vi.fn() });
  unref = vi.fn();
  kill = vi.fn((signal?: string) => {
    if (signal === 'SIGINT') queueMicrotask(() => this.emit('exit', null, 'SIGINT'));
    return true;
  });

  constructor(pid: number | undefined) {
    super();
    this.pid = pid;
  }
}

const makeChild = (pid: number | undefined) => new FakeChild(pid);

const successfulProxy = () => {
  const proxy = { baseUrl: 'http://127.0.0.1:4555', close: vi.fn() };
  mocks.createProxy.mockResolvedValue(proxy);
  return proxy;
};

const successfulFetch = () => {
  const fetchMock = vi.fn().mockResolvedValue({ ok: true, body: { cancel: vi.fn() } });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
};

const start = (app = makeApp()) =>
  startBiorouterd({
    app,
    serverSecret: 'server-secret',
    userActionKey: 'renderer-proof',
    dir: process.cwd(),
  });

/** A running daemon that is replaced, then a fresh child that publishes itself. */
const expectReplaced = async (reason: RegExp) => {
  const child = makeChild(fresh.pid);
  mocks.spawn.mockReturnValue(child);
  mocks.discover.mockReturnValueOnce(runtime).mockReturnValue(fresh);
  successfulProxy();
  successfulFetch();

  await start();

  expect(mocks.stop).toHaveBeenCalledWith(runtime);
  expect(mocks.logWarn).toHaveBeenCalledWith(expect.stringMatching(reason));
  expect(mocks.spawn).toHaveBeenCalledTimes(1);
  expect(mocks.stop.mock.invocationCallOrder[0]).toBeLessThan(
    mocks.spawn.mock.invocationCallOrder[0]
  );
  expect(child.stdin.write).toHaveBeenCalledWith(digest(MINTED));
  expect(mocks.writeKey).toHaveBeenCalledWith(fresh, MINTED);
  expect(mocks.createProxy).toHaveBeenCalledWith(
    fresh,
    'server-secret',
    'renderer-proof',
    MINTED,
    undefined
  );
};

describe('shared daemon attachment without an approval secret', () => {
  beforeEach(() => {
    for (const mock of [
      mocks.spawn,
      mocks.discover,
      mocks.createProxy,
      mocks.writeKey,
      mocks.removeKey,
      mocks.prompt,
    ])
      mock.mockReset();
    mocks.verify.mockReset().mockResolvedValue(undefined);
    mocks.daemonVersion.mockReset().mockResolvedValue(APP_VERSION);
    mocks.readKey.mockReset().mockResolvedValue(SAVED);
    mocks.verifyAccess.mockReset().mockResolvedValue(undefined);
    mocks.stop.mockReset().mockResolvedValue(undefined);
    mocks.logInfo.mockClear();
    mocks.logError.mockClear();
    mocks.logWarn.mockClear();
    mocks.logDebug.mockClear();
  });

  afterEach(() => {
    // Whatever happened, nobody was asked for a secret.
    expect(mocks.prompt).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it('first launch: mints a key, sends only its digest, saves the key and detaches without killing the daemon', async () => {
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(fresh);
    const proxy = successfulProxy();
    successfulFetch();
    const app = makeApp();
    const result = await start(app);

    expect(child.stdin.write).toHaveBeenCalledTimes(1);
    expect(child.stdin.write).toHaveBeenCalledWith(digest(MINTED));
    expect(child.stdin.write).not.toHaveBeenCalledWith(expect.stringContaining(MINTED));
    expect(child.stdin.end).toHaveBeenCalled();
    expect(mocks.writeKey).toHaveBeenCalledWith(fresh, MINTED);
    expect(mocks.writeKey.mock.invocationCallOrder[0]).toBeLessThan(
      mocks.createProxy.mock.invocationCallOrder[0]
    );
    expect(mocks.createProxy).toHaveBeenCalledWith(
      fresh,
      'server-secret',
      'renderer-proof',
      MINTED,
      undefined
    );
    const spawnOptions = mocks.spawn.mock.calls[0]?.[2] as {
      stdio: string[];
      env: Record<string, string>;
    };
    expect(spawnOptions.stdio).toEqual(['pipe', 'ignore', 'ignore']);
    expect(spawnOptions.env.BIOROUTER_SHARED_DAEMON).toBe('1');
    expect(JSON.stringify(mocks.spawn.mock.calls[0])).not.toContain(MINTED);
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(result.process.kill?.()).toBe(true);
    expect(proxy.close).toHaveBeenCalledTimes(1);
    expect(child.kill).not.toHaveBeenCalled();
    expect(child.unref).toHaveBeenCalledTimes(1);
  });

  it('attaches silently to a running daemon of this version with the saved key', async () => {
    mocks.discover.mockReturnValue(runtime);
    successfulProxy();
    successfulFetch();
    await start();
    expect(mocks.readKey).toHaveBeenCalledWith(runtime);
    expect(mocks.verifyAccess).toHaveBeenCalledWith(runtime, SAVED);
    expect(mocks.createProxy).toHaveBeenCalledWith(
      runtime,
      'server-secret',
      'renderer-proof',
      SAVED,
      undefined
    );
    expect(mocks.spawn).not.toHaveBeenCalled();
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(mocks.writeKey).not.toHaveBeenCalled();
  });

  it('replaces a daemon with no saved key (one an earlier Biorouter started with a typed secret)', async () => {
    mocks.readKey.mockResolvedValue(undefined);
    await expectReplaced(/no saved user-action key/);
    expect(mocks.verifyAccess).not.toHaveBeenCalled();
  });

  it('replaces a daemon that refuses the saved key', async () => {
    mocks.verifyAccess.mockRejectedValue(new mocks.DaemonKeyRefusedError('refused'));
    await expectReplaced(/refused the saved user-action key/);
  });

  it('replaces a daemon of another version, so an update takes effect on the next launch', async () => {
    mocks.daemonVersion.mockResolvedValue('1.92.0');
    await expectReplaced(/version 1\.92\.0 and this app is version 1\.92\.1/);
    expect(mocks.readKey).not.toHaveBeenCalled();
  });

  it('replaces a daemon that was started without any user-action key', async () => {
    mocks.discover.mockReturnValueOnce({ ...runtime, user_action_installed: false });
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.discover.mockReturnValue(fresh);
    successfulProxy();
    successfulFetch();
    await start();
    expect(mocks.stop).toHaveBeenCalledWith({ ...runtime, user_action_installed: false });
    expect(mocks.writeKey).toHaveBeenCalledWith(fresh, MINTED);
  });

  it('does not start a second daemon when the old one could not be stopped', async () => {
    mocks.readKey.mockResolvedValue(undefined);
    mocks.discover.mockReturnValue(runtime);
    mocks.stop.mockRejectedValue(new Error('Biorouter could not stop the old background service'));
    await expect(start()).rejects.toThrow(/could not stop the old background service/);
    expect(mocks.spawn).not.toHaveBeenCalled();
  });

  it('starts a new daemon, stopping nothing, when the descriptor is left over from one that is gone', async () => {
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.verify.mockImplementation(async (target: { instance_id: string }) => {
      if (target.instance_id === runtime.instance_id)
        throw Object.assign(new Error('connect ECONNREFUSED'), {
          code: 'ECONNREFUSED',
          syscall: 'connect',
        });
    });
    // The leftover descriptor is still there while the child starts.
    mocks.discover.mockReturnValueOnce(runtime).mockReturnValueOnce(runtime).mockReturnValue(fresh);
    successfulProxy();
    successfulFetch();
    await start();
    expect(mocks.stop).not.toHaveBeenCalled();
    expect(mocks.writeKey).toHaveBeenCalledTimes(1);
    expect(mocks.writeKey).toHaveBeenCalledWith(fresh, MINTED);
  });

  it('attaches to the daemon another starter won the race with, through its saved key', async () => {
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    let discoveries = 0;
    mocks.discover.mockImplementation(() => {
      discoveries += 1;
      if (discoveries === 1) return undefined;
      // Our child lost the profile lock and exited; the winner published itself.
      child.exitCode = 1;
      return runtime;
    });
    successfulProxy();
    successfulFetch();
    await start();
    expect(mocks.writeKey).not.toHaveBeenCalled();
    expect(mocks.readKey).toHaveBeenCalledWith(runtime);
    expect(mocks.createProxy).toHaveBeenCalledWith(
      runtime,
      'server-secret',
      'renderer-proof',
      SAVED,
      undefined
    );
    expect(child.kill).not.toHaveBeenCalled();
  });

  it('reports a child that exited before publishing and leaves it alone', async () => {
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    let discoveries = 0;
    mocks.discover.mockImplementation(() => {
      discoveries += 1;
      if (discoveries === 2) child.exitCode = 1;
      return undefined;
    });
    await expect(start()).rejects.toThrow(/failed to start/i);
    expect(child.kill).not.toHaveBeenCalled();
    expect(mocks.writeKey).not.toHaveBeenCalled();
  });

  it('reports a spawn failure from a child with no pid without attempting termination', async () => {
    let child: FakeChild | undefined;
    mocks.spawn.mockImplementation(() => {
      child = makeChild(undefined);
      queueMicrotask(() => child?.emit('error', new Error('spawn failed')));
      return child;
    });
    mocks.discover.mockReturnValue(undefined);
    await expect(start()).rejects.toThrow(/failed to start/i);
    expect(child?.pid).toBeUndefined();
    expect(child?.kill).not.toHaveBeenCalled();
  });

  it('removes the saved key and stops its own child when the new daemon refuses the connection', async () => {
    const child = makeChild(fresh.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(fresh);
    const proxy = successfulProxy();
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({ ok: false, status: 403, body: { cancel: vi.fn() } })
    );
    await expect(start()).rejects.toThrow(/refused this app's connection/);
    expect(mocks.writeKey).toHaveBeenCalledWith(fresh, MINTED);
    expect(mocks.removeKey).toHaveBeenCalledWith(fresh);
    expect(proxy.close).toHaveBeenCalledTimes(1);
    expect(child.kill).toHaveBeenCalledWith('SIGINT');
    expect(child.kill).not.toHaveBeenCalledWith('SIGKILL');
  });

  it('escalates an owned child that ignores SIGINT and reports the cleanup deadline', async () => {
    vi.useFakeTimers();
    try {
      const child = makeChild(fresh.pid);
      child.kill.mockImplementation(() => true);
      mocks.spawn.mockReturnValue(child);
      mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(fresh);
      successfulProxy();
      vi.stubGlobal(
        'fetch',
        vi.fn().mockResolvedValue({ ok: false, status: 403, body: { cancel: vi.fn() } })
      );
      const observed = start().catch((error: Error) => error);
      await vi.advanceTimersByTimeAsync(100);
      await vi.advanceTimersByTimeAsync(3000);
      await expect(observed).resolves.toMatchObject({
        message: expect.stringMatching(/cleanup deadline/),
      });
      expect(child.kill).toHaveBeenCalledWith('SIGINT');
      expect(child.kill).toHaveBeenCalledWith('SIGKILL');
    } finally {
      vi.useRealTimers();
    }
  });
});
