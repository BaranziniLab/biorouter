// @vitest-environment node
import type { App } from 'electron';
import { EventEmitter } from 'node:events';
import type { PathLike, Stats } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

const mocks = vi.hoisted(() => ({
  spawn: vi.fn(),
  discover: vi.fn(),
  verify: vi.fn(),
  createProxy: vi.fn(),
  logInfo: vi.fn(),
  logError: vi.fn(),
  logWarn: vi.fn(),
  logDebug: vi.fn(),
}));

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

vi.mock('./biorouterdSingleton', () => ({ isSharedDaemonEnabled: () => true }));
vi.mock('./daemonRuntime', () => ({
  createDaemonProxy: mocks.createProxy,
  discoverDaemonRuntime: mocks.discover,
  verifyDaemonRuntime: mocks.verify,
}));

import { startBiorouterd, validateDaemonApprovalSecret } from './biorouterd';

const runtime = {
  version: 1 as const,
  profile_id: '11111111-1111-4111-8111-111111111111',
  instance_id: '22222222-2222-4222-8222-222222222222',
  pid: 88,
  endpoint: { kind: 'unix' as const, path: '/private/tmp/biorouterd.sock' },
  api_secret: 'daemon-secret-123456',
  user_action_installed: true,
};

const makeApp = () =>
  ({
    isPackaged: false,
    on: vi.fn(),
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
  const fetchMock = vi.fn().mockResolvedValue({
    ok: true,
    body: { cancel: vi.fn() },
  });
  vi.stubGlobal('fetch', fetchMock);
  return fetchMock;
};

describe('shared daemon approval-key admission', () => {
  beforeEach(() => {
    mocks.spawn.mockReset();
    mocks.discover.mockReset();
    mocks.verify.mockReset().mockResolvedValue(undefined);
    mocks.createProxy.mockReset();
    mocks.logInfo.mockClear();
    mocks.logError.mockClear();
    mocks.logWarn.mockClear();
    mocks.logDebug.mockClear();
  });

  afterEach(() => vi.unstubAllGlobals());

  it.each([
    ['too short', '!'.repeat(31)],
    ['too long', '!'.repeat(4097)],
    ['space', '!'.repeat(31) + ' '],
    ['unicode', '!'.repeat(31) + 'é'],
    ['newline', '!'.repeat(31) + '\n'],
  ])('rejects an approval secret with %s before any daemon spawn', async (_label, secret) => {
    mocks.discover.mockReturnValue(undefined);
    const app = makeApp();
    await expect(
      startBiorouterd({
        app,
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => secret,
      })
    ).rejects.toThrow(/Approval secret/);
    expect(mocks.spawn).not.toHaveBeenCalled();
  });

  it('refuses a cancelled new-daemon approval prompt before spawn', async () => {
    mocks.discover.mockReturnValue(undefined);
    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => undefined,
      })
    ).rejects.toThrow(/startup cancelled/i);
    expect(mocks.spawn).not.toHaveBeenCalled();
  });

  it('refuses a new shared daemon when no approval callback is provided', async () => {
    mocks.discover.mockReturnValue(undefined);
    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
      })
    ).rejects.toThrow(/startup cancelled/i);
    expect(mocks.spawn).not.toHaveBeenCalled();
  });

  it('writes only the SHA-256 digest for a newly started shared daemon and detaches without killing it', async () => {
    const secret = '!'.repeat(32);
    const child = makeChild(runtime.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(runtime);
    const proxy = successfulProxy();
    successfulFetch();
    const app = makeApp();
    const result = await startBiorouterd({
      app,
      serverSecret: 'server-secret',
      userActionKey: 'renderer-proof',
      dir: process.cwd(),
      requestNewUserActionKey: async () => secret,
    });

    expect(child.stdin.write).toHaveBeenCalledWith(
      createHash('sha256').update(secret).digest('hex') + '\n'
    );
    expect(child.stdin.write).not.toHaveBeenCalledWith(expect.stringContaining(secret));
    expect(mocks.createProxy).toHaveBeenCalledWith(
      runtime,
      'server-secret',
      'renderer-proof',
      secret,
      undefined
    );
    expect((mocks.spawn.mock.calls[0]?.[2] as { stdio: string[] }).stdio).toEqual([
      'pipe',
      'ignore',
      'ignore',
    ]);
    expect(result.process.kill?.()).toBe(true);
    expect(proxy.close).toHaveBeenCalledTimes(1);
    expect(child.kill).not.toHaveBeenCalled();
    expect(child.unref).toHaveBeenCalledTimes(1);
  });

  it('uses the existing-daemon prompt, never the new-daemon prompt, and closes the proxy on refusal', async () => {
    mocks.discover.mockReturnValue(runtime);
    const proxy = successfulProxy();
    const wrongSecret = '!'.repeat(33);
    const fetchMock = vi.fn((_: string, init?: RequestInit) => {
      expect((init?.headers as Record<string, string>)['X-User-Action']).toBe('');
      return Promise.resolve({ ok: false, body: { cancel: vi.fn() } });
    });
    vi.stubGlobal('fetch', fetchMock);
    const requestNew = vi.fn(async () => '!'.repeat(32));
    const requestExisting = vi.fn(async () => wrongSecret);

    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: requestNew,
        requestUserActionKey: requestExisting,
      })
    ).rejects.toThrow(/did not accept human-authorized access/);
    expect(requestNew).not.toHaveBeenCalled();
    expect(requestExisting).toHaveBeenCalledWith({
      profileId: runtime.profile_id,
      instanceId: runtime.instance_id,
      userActionInstalled: true,
    });
    expect(mocks.createProxy).toHaveBeenCalledWith(
      runtime,
      'server-secret',
      undefined,
      wrongSecret,
      undefined
    );
    expect(proxy.close).toHaveBeenCalledTimes(1);
    expect(mocks.spawn).not.toHaveBeenCalled();
  });

  it('refuses an existing daemon without an installed proof before asking for a secret', async () => {
    mocks.discover.mockReturnValue({ ...runtime, user_action_installed: false });
    successfulFetch();
    const requestExisting = vi.fn(async () => '!'.repeat(32));
    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestUserActionKey: requestExisting,
      })
    ).rejects.toThrow(/no installed human approval proof/);
    expect(requestExisting).not.toHaveBeenCalled();
    expect(mocks.createProxy).not.toHaveBeenCalled();
  });

  it('waits for a new child to publish readiness and leaves an already-exited child alone', async () => {
    const child = makeChild(runtime.pid);
    mocks.spawn.mockReturnValue(child);
    let discoveries = 0;
    mocks.discover.mockImplementation(() => {
      discoveries += 1;
      if (discoveries === 2) child.exitCode = 1;
      return undefined;
    });

    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => '!'.repeat(32),
      })
    ).rejects.toThrow(/failed to start/i);
    expect(discoveries).toBe(2);
    expect(child.kill).not.toHaveBeenCalled();
  });

  it('reports a spawn failure from a child with no pid without attempting termination', async () => {
    let child: FakeChild | undefined;
    mocks.spawn.mockImplementation(() => {
      child = makeChild(undefined);
      queueMicrotask(() => child?.emit('error', new Error('spawn failed')));
      return child;
    });
    mocks.discover.mockReturnValue(undefined);

    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => '!'.repeat(32),
      })
    ).rejects.toThrow(/failed to start/i);
    expect(child?.pid).toBeUndefined();
    expect(child?.signalCode).toBeNull();
    expect(child?.kill).not.toHaveBeenCalled();
  });

  it('stops an owned child after readiness attach refusal with bounded graceful cleanup', async () => {
    const child = makeChild(runtime.pid);
    mocks.spawn.mockReturnValue(child);
    mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(runtime);
    successfulProxy();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, body: { cancel: vi.fn() } }));

    await expect(
      startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => '!'.repeat(32),
      })
    ).rejects.toThrow(/did not accept human-authorized access/);
    expect(child.kill).toHaveBeenCalledWith('SIGINT');
    expect(child.kill).not.toHaveBeenCalledWith('SIGKILL');
  });

  it('escalates an owned child that ignores SIGINT and reports the cleanup deadline', async () => {
    vi.useFakeTimers();
    try {
      const child = makeChild(runtime.pid);
      child.kill.mockImplementation(() => true);
      mocks.spawn.mockReturnValue(child);
      mocks.discover.mockReturnValueOnce(undefined).mockReturnValue(runtime);
      successfulProxy();
      vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: false, body: { cancel: vi.fn() } }));
      const pending = startBiorouterd({
        app: makeApp(),
        serverSecret: 'server-secret',
        dir: process.cwd(),
        requestNewUserActionKey: async () => '!'.repeat(32),
      });
      const observed = pending.catch((error: Error) => error);
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

describe('approval secret validator boundaries', () => {
  it('accepts printable ASCII at both supported boundaries', () => {
    expect(() => validateDaemonApprovalSecret('!'.repeat(32))).not.toThrow();
    expect(() => validateDaemonApprovalSecret('~'.repeat(4096))).not.toThrow();
  });

  it.each([
    ['missing', undefined],
    ['too short', '!'.repeat(31)],
    ['too long', '!'.repeat(4097)],
    ['trailing LF', `${'!'.repeat(32)}\n`],
    ['trailing CR', `${'!'.repeat(32)}\r`],
    ['trailing CRLF', `${'!'.repeat(32)}\r\n`],
    ['line separator', `${'!'.repeat(32)}\u2028`],
    ['paragraph separator', `${'!'.repeat(32)}\u2029`],
  ])('rejects %s', (_label, value) => {
    expect(() => validateDaemonApprovalSecret(value)).toThrow();
  });
});
