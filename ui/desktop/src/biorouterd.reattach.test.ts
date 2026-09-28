// @vitest-environment node
import type { App } from 'electron';
import { EventEmitter } from 'node:events';
import type { PathLike, Stats } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * R-1: reconnecting a shared attachment after the daemon restarted. The same discovery, prompts
 * and start as a first launch, but pointed at the existing proxy (`retarget`), never a second one.
 * The Electron and daemon edges are faked exactly as `biorouterd.shared.test.ts` fakes them.
 */
const mocks = vi.hoisted(() => ({
  spawn: vi.fn(),
  discover: vi.fn(),
  verify: vi.fn(),
  createProxy: vi.fn(),
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
  default: { info: vi.fn(), error: vi.fn(), warn: vi.fn(), debug: vi.fn() },
}));

vi.mock('./biorouterdSingleton', () => ({ isSharedDaemonEnabled: () => true }));
vi.mock('./daemonRuntime', () => ({
  createDaemonProxy: mocks.createProxy,
  discoverDaemonRuntime: mocks.discover,
  verifyDaemonRuntime: mocks.verify,
}));

import { startBiorouterd } from './biorouterd';

const runtimeA = {
  version: 1 as const,
  profile_id: '11111111-1111-4111-8111-111111111111',
  instance_id: '22222222-2222-4222-8222-222222222222',
  pid: 88,
  endpoint: { kind: 'unix' as const, path: '/private/tmp/biorouterd.sock' },
  api_secret: 'daemon-secret-a-123456',
  user_action_installed: true,
};
const runtimeB = {
  ...runtimeA,
  instance_id: '33333333-3333-4333-8333-333333333333',
  pid: 99,
  api_secret: 'daemon-secret-b-123456',
};

class FakeChild extends EventEmitter {
  exitCode: number | null = null;
  signalCode: string | null = null;
  stdin = { write: vi.fn(), end: vi.fn() };
  stdout = Object.assign(new EventEmitter(), { destroy: vi.fn() });
  stderr = Object.assign(new EventEmitter(), { destroy: vi.fn() });
  unref = vi.fn();
  kill = vi.fn(() => true);
  constructor(public pid: number | undefined) {
    super();
  }
}

const makeApp = () =>
  ({ isPackaged: false, on: vi.fn(), once: vi.fn(), removeListener: vi.fn() }) as unknown as App;

/** A proxy attached to A whose instance moves only through `retarget`. */
function fakeProxy() {
  let instance = runtimeA.instance_id;
  const proxy = {
    baseUrl: 'http://127.0.0.1:4555',
    close: vi.fn(),
    instanceId: vi.fn(() => instance),
    onConnection: vi.fn(() => () => undefined),
    probe: vi.fn(async () => undefined),
    retarget: vi.fn(async (runtime: { instance_id: string }) => {
      instance = runtime.instance_id;
    }),
  };
  mocks.createProxy.mockResolvedValue(proxy);
  return proxy;
}

const APPROVAL_A = 'a'.repeat(32);
const APPROVAL_B = 'b'.repeat(32);

async function attachToA(requestExisting = vi.fn(async () => APPROVAL_A)) {
  mocks.discover.mockReturnValue(runtimeA);
  const proxy = fakeProxy();
  const requestNew = vi.fn(async () => 'n'.repeat(32));
  const app = makeApp();
  const result = await startBiorouterd({
    app,
    serverSecret: 'server-secret',
    userActionKey: 'renderer-proof',
    dir: process.cwd(),
    requestNewUserActionKey: requestNew,
    requestUserActionKey: requestExisting,
  });
  return { result, proxy, requestExisting, requestNew, app };
}

describe('reconnecting a shared attachment (R-1)', () => {
  beforeEach(() => {
    mocks.spawn.mockReset();
    mocks.discover.mockReset();
    mocks.verify.mockReset().mockResolvedValue(undefined);
    mocks.createProxy.mockReset();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, body: { cancel: vi.fn() } }));
  });
  afterEach(() => vi.unstubAllGlobals());

  it('asks for the new instance’s own approval secret and points the same proxy at it', async () => {
    const requestExisting = vi
      .fn()
      .mockResolvedValueOnce(APPROVAL_A)
      .mockResolvedValueOnce(APPROVAL_B);
    const { result, proxy, requestNew } = await attachToA(requestExisting);
    expect(result.sharedDaemon).toBeDefined();

    mocks.discover.mockReturnValue(runtimeB);
    await result.sharedDaemon!.reconnect();

    expect(requestExisting).toHaveBeenLastCalledWith({
      profileId: runtimeB.profile_id,
      instanceId: runtimeB.instance_id,
      userActionInstalled: true,
    });
    expect(proxy.retarget).toHaveBeenCalledWith(runtimeB, APPROVAL_B);
    // One proxy, one address: never a second attachment, never a daemon started.
    expect(mocks.createProxy).toHaveBeenCalledTimes(1);
    expect(mocks.spawn).not.toHaveBeenCalled();
    expect(requestNew).not.toHaveBeenCalled();
  });

  it('asks for nothing when the instance it verified still answers as itself', async () => {
    const { result, proxy, requestExisting } = await attachToA();
    await result.sharedDaemon!.reconnect();
    expect(proxy.probe).toHaveBeenCalledTimes(1);
    expect(proxy.retarget).not.toHaveBeenCalled();
    expect(requestExisting).toHaveBeenCalledTimes(1);
  });

  it('changes nothing when the person cancels the approval prompt', async () => {
    const requestExisting = vi
      .fn()
      .mockResolvedValueOnce(APPROVAL_A)
      .mockResolvedValueOnce(undefined);
    const { result, proxy } = await attachToA(requestExisting);
    mocks.discover.mockReturnValue(runtimeB);
    await expect(result.sharedDaemon!.reconnect()).rejects.toThrow(/approval secret/);
    expect(proxy.retarget).not.toHaveBeenCalled();
  });

  it('refuses an instance with no installed approval key before asking for anything', async () => {
    const { result, proxy, requestExisting } = await attachToA();
    mocks.discover.mockReturnValue({ ...runtimeB, user_action_installed: false });
    await expect(result.sharedDaemon!.reconnect()).rejects.toThrow(/no installed human approval/);
    expect(requestExisting).toHaveBeenCalledTimes(1);
    expect(proxy.retarget).not.toHaveBeenCalled();
  });

  it('starts a new daemon, with a new approval secret, when nothing answers any more', async () => {
    const { result, proxy, requestNew, app } = await attachToA();
    const child = new FakeChild(runtimeB.pid);
    mocks.spawn.mockReturnValue(child);
    // A's descriptor is still there, but nothing listens on its socket.
    mocks.verify.mockImplementation(async (runtime: { instance_id: string }) => {
      if (runtime.instance_id === runtimeA.instance_id)
        throw Object.assign(new Error('connect ECONNREFUSED'), {
          code: 'ECONNREFUSED',
          syscall: 'connect',
        });
    });
    mocks.discover.mockReturnValueOnce(runtimeA).mockReturnValue(runtimeB);
    await result.sharedDaemon!.reconnect();

    const secret = 'n'.repeat(32);
    expect(requestNew).toHaveBeenCalledTimes(1);
    expect(child.stdin.write).toHaveBeenCalledWith(
      createHash('sha256').update(secret).digest('hex') + '\n'
    );
    expect(proxy.retarget).toHaveBeenCalledWith(runtimeB, secret);
    expect(mocks.createProxy).toHaveBeenCalledTimes(1);
    expect(app.once).toHaveBeenCalledWith('will-quit', expect.any(Function));
  });
});
