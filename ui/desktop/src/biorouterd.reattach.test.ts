// @vitest-environment node
import type { App } from 'electron';
import { EventEmitter } from 'node:events';
import type { PathLike, Stats } from 'node:fs';
import { createHash } from 'node:crypto';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

/**
 * R-1: reconnecting a shared attachment after the daemon restarted. The same discovery and start
 * as a first launch, pointed at the existing proxy (`retarget`), never a second one, and with
 * nobody asked for anything. The Electron and daemon edges are faked exactly as
 * `biorouterd.shared.test.ts` fakes them.
 */
const mocks = vi.hoisted(() => {
  class DaemonKeyRefusedError extends Error {}
  return {
    DaemonKeyRefusedError,
    spawn: vi.fn(),
    discover: vi.fn(),
    verify: vi.fn(),
    createProxy: vi.fn(),
    readKey: vi.fn(),
    writeKey: vi.fn(),
    stop: vi.fn(),
    prompt: vi.fn(),
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
  default: { info: vi.fn(), error: vi.fn(), warn: vi.fn(), debug: vi.fn() },
}));

// A tripwire: a reconnect never asks a person for a secret.
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
  daemonVersion: async () => '1.92.1',
  DaemonKeyRefusedError: mocks.DaemonKeyRefusedError,
  discoverDaemonRuntime: mocks.discover,
  generateUserActionKey: () => 'n'.repeat(64),
  readUserActionKey: mocks.readKey,
  removeUserActionKey: vi.fn(),
  stopProfileDaemon: mocks.stop,
  verifyDaemonRuntime: mocks.verify,
  verifyHumanAuthorizedAccess: async () => undefined,
  writeUserActionKey: mocks.writeKey,
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
const runtimeC = {
  ...runtimeA,
  instance_id: '44444444-4444-4444-8444-444444444444',
  pid: 111,
  api_secret: 'daemon-secret-c-123456',
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
  ({
    isPackaged: false,
    getVersion: () => '1.92.1',
    on: vi.fn(),
    once: vi.fn(),
    removeListener: vi.fn(),
  }) as unknown as App;

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

const KEY_A = 'a'.repeat(64);
const KEY_B = 'b'.repeat(64);
const MINTED = 'n'.repeat(64);

const keys: Record<string, string | undefined> = {
  [runtimeA.instance_id]: KEY_A,
  [runtimeB.instance_id]: KEY_B,
};

async function attachToA() {
  mocks.discover.mockReturnValue(runtimeA);
  const proxy = fakeProxy();
  const app = makeApp();
  const result = await startBiorouterd({
    app,
    serverSecret: 'server-secret',
    userActionKey: 'renderer-proof',
    dir: process.cwd(),
  });
  return { result, proxy, app };
}

describe('reconnecting a shared attachment (R-1)', () => {
  beforeEach(() => {
    mocks.spawn.mockReset();
    mocks.discover.mockReset();
    mocks.verify.mockReset().mockResolvedValue(undefined);
    mocks.createProxy.mockReset();
    mocks.readKey
      .mockReset()
      .mockImplementation(async (runtime: { instance_id: string }) => keys[runtime.instance_id]);
    mocks.writeKey.mockReset();
    mocks.stop.mockReset().mockResolvedValue(undefined);
    mocks.prompt.mockReset();
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue({ ok: true, body: { cancel: vi.fn() } }));
  });
  afterEach(() => {
    expect(mocks.prompt).not.toHaveBeenCalled();
    vi.unstubAllGlobals();
  });

  it('reads the new instance’s saved key and points the same proxy at it', async () => {
    const { result, proxy } = await attachToA();
    expect(result.sharedDaemon).toBeDefined();

    mocks.discover.mockReturnValue(runtimeB);
    await result.sharedDaemon!.reconnect();

    expect(mocks.readKey).toHaveBeenLastCalledWith(runtimeB);
    expect(proxy.retarget).toHaveBeenCalledWith(runtimeB, KEY_B);
    // One proxy, one address: never a second attachment, never a daemon started.
    expect(mocks.createProxy).toHaveBeenCalledTimes(1);
    expect(mocks.spawn).not.toHaveBeenCalled();
    expect(mocks.stop).not.toHaveBeenCalled();
  });

  it('reads nothing when the instance it verified still answers as itself', async () => {
    const { result, proxy } = await attachToA();
    await result.sharedDaemon!.reconnect();
    expect(proxy.probe).toHaveBeenCalledTimes(1);
    expect(proxy.retarget).not.toHaveBeenCalled();
    expect(mocks.readKey).toHaveBeenCalledTimes(1);
  });

  it('replaces a new instance it cannot use and points the proxy at the one it starts', async () => {
    const { result, proxy } = await attachToA();
    const child = new FakeChild(runtimeC.pid);
    mocks.spawn.mockReturnValue(child);
    // B was started by an earlier Biorouter: no key saved for it.
    const legacyB = { ...runtimeB, instance_id: '55555555-5555-4555-8555-555555555555' };
    mocks.discover.mockReturnValueOnce(legacyB).mockReturnValue(runtimeC);
    await result.sharedDaemon!.reconnect();

    expect(mocks.stop).toHaveBeenCalledWith(legacyB);
    expect(child.stdin.write).toHaveBeenCalledWith(
      createHash('sha256').update(MINTED).digest('hex') + '\n'
    );
    expect(mocks.writeKey).toHaveBeenCalledWith(runtimeC, MINTED);
    expect(proxy.retarget).toHaveBeenCalledWith(runtimeC, MINTED);
    expect(mocks.createProxy).toHaveBeenCalledTimes(1);
  });

  it('starts a new daemon when nothing answers any more', async () => {
    const { result, proxy, app } = await attachToA();
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

    expect(mocks.stop).not.toHaveBeenCalled();
    expect(child.stdin.write).toHaveBeenCalledWith(
      createHash('sha256').update(MINTED).digest('hex') + '\n'
    );
    expect(mocks.writeKey).toHaveBeenCalledWith(runtimeB, MINTED);
    expect(proxy.retarget).toHaveBeenCalledWith(runtimeB, MINTED);
    expect(mocks.createProxy).toHaveBeenCalledTimes(1);
    expect(app.once).toHaveBeenCalledWith('will-quit', expect.any(Function));
  });

  it('changes nothing when the new instance cannot be verified', async () => {
    const { result, proxy } = await attachToA();
    mocks.discover.mockReturnValue(runtimeB);
    proxy.retarget.mockRejectedValueOnce(new Error('identity changed'));
    await expect(result.sharedDaemon!.reconnect()).rejects.toThrow(/identity changed/);
    expect(proxy.instanceId()).toBe(runtimeA.instance_id);
  });
});
