import { EventEmitter } from 'node:events';
import type { BrowserWindow } from 'electron';
import type { BiorouterdResult } from '../biorouterd';
import { describe, expect, it, vi } from 'vitest';
import {
  bindManagedAppPreviewBackend,
  fetchDaemonAppOrigin,
  loopbackAppOrigin,
} from './managedAppPreviewBackend';

function fixture(managed = true, signalCode: string | null = null) {
  const process = Object.assign(new EventEmitter(), {
    exitCode: null as number | null,
    signalCode,
    killed: false,
  });
  const owner = Object.assign(new EventEmitter(), { isDestroyed: () => false });
  const result = {
    baseUrl: 'http://127.0.0.1:64005',
    managed,
    process,
    workingDir: '/tmp/qa',
    errorLog: [],
  } as unknown as BiorouterdResult;
  return {
    process,
    owner,
    result,
    context: bindManagedAppPreviewBackend(result, owner as unknown as BrowserWindow),
  };
}

describe('managed app provenance and lifetime', () => {
  it('never grants an externally configured daemon, even on loopback', () => {
    expect(fixture(false).context).toBeUndefined();
  });
  it.each(['exit', 'error'])('revokes on process %s without waiting for stdio close', (event) => {
    const f = fixture();
    expect(f.context?.signal.aborted).toBe(false);
    f.process.emit(event, event === 'error' ? new Error('synthetic') : 0);
    expect(f.context?.signal.aborted).toBe(true);
    expect(f.process.listenerCount('close')).toBe(0);
  });
  it('revokes only the closing owner, not another owner of the same daemon', () => {
    const f = fixture();
    const other = Object.assign(new EventEmitter(), { isDestroyed: () => false });
    const second = bindManagedAppPreviewBackend(f.result, other as unknown as BrowserWindow)!;
    f.owner.emit('closed');
    expect(f.context?.signal.aborted).toBe(true);
    expect(second.signal.aborted).toBe(false);
    f.process.emit('exit', 0);
    expect(second.signal.aborted).toBe(true);
  });
  it('does not revive a prior generation at the same address', () => {
    const first = fixture();
    first.process.emit('exit', 0);
    const second = fixture();
    expect(first.context?.signal.aborted).toBe(true);
    expect(second.context?.signal.aborted).toBe(false);
    expect(first.context).not.toBe(second.context);
  });
  it('starts revoked when the managed child has already exited', () => {
    const f = fixture();
    f.process.exitCode = 1;
    expect(
      bindManagedAppPreviewBackend(f.result, f.owner as unknown as BrowserWindow)?.signal.aborted
    ).toBe(true);
  });
  it('keeps an observed backend error revoked for later owners', () => {
    const f = fixture();
    f.process.emit('error', new Error('synthetic backend error'));
    const laterOwner = Object.assign(new EventEmitter(), { isDestroyed: () => false });
    expect(
      bindManagedAppPreviewBackend(f.result, laterOwner as unknown as BrowserWindow)?.signal.aborted
    ).toBe(true);
  });
  it('revokes a new owner binding after a signal-only exit, even when close is pending', () => {
    const f = fixture();
    f.process.signalCode = 'SIGKILL';
    f.process.emit('exit', null, 'SIGKILL');
    expect(f.context?.signal.aborted).toBe(true);
    const laterOwner = Object.assign(new EventEmitter(), { isDestroyed: () => false });
    const later = bindManagedAppPreviewBackend(f.result, laterOwner as unknown as BrowserWindow);
    expect(f.process.exitCode).toBeNull();
    expect(f.process.killed).toBe(false);
    expect(later?.signal.aborted).toBe(true);
  });
  it('rejects a process already signal-terminated before its first binding', () => {
    const f = fixture(true, 'SIGTERM');
    expect(f.process.exitCode).toBeNull();
    expect(f.process.killed).toBe(false);
    expect(f.context?.signal.aborted).toBe(true);
  });
});

/**
 * A shared daemon is reached through the desktop's proxy, which refuses a page, so the preview
 * is bound to the daemon's own listener, and revoked when the main process learns that daemon
 * is gone: its handle never exits, because a shared daemon outlives the app.
 */
describe('a shared daemon’s preview backend', () => {
  it('carries the daemon’s app origin beside the address launch links are minted at', () => {
    const f = fixture();
    const backend = bindManagedAppPreviewBackend(
      f.result,
      f.owner as unknown as BrowserWindow,
      'secret',
      'http://127.0.0.1:51851'
    )!;
    expect(backend).toMatchObject({
      baseUrl: 'http://127.0.0.1:64005',
      appOrigin: 'http://127.0.0.1:51851',
      secretKey: 'secret',
    });
    expect('appOrigin' in f.context!).toBe(false);
  });

  it('is revoked when its lifetime ends, and born revoked into an ended one', () => {
    const f = fixture();
    const lifetime = new AbortController();
    const backend = bindManagedAppPreviewBackend(
      f.result,
      f.owner as unknown as BrowserWindow,
      'secret',
      'http://127.0.0.1:51851',
      lifetime.signal
    )!;
    expect(backend.signal.aborted).toBe(false);
    lifetime.abort();
    expect(backend.signal.aborted).toBe(true);
    // The daemon's handle did not exit: a later binding, on a new lifetime, is live.
    const next = bindManagedAppPreviewBackend(
      f.result,
      f.owner as unknown as BrowserWindow,
      'secret',
      'http://127.0.0.1:51900',
      new AbortController().signal
    )!;
    expect(next.signal.aborted).toBe(false);
    expect(
      bindManagedAppPreviewBackend(
        f.result,
        f.owner as unknown as BrowserWindow,
        'secret',
        'http://127.0.0.1:51851',
        lifetime.signal
      )!.signal.aborted
    ).toBe(true);
  });

  it.each([
    ['http://127.0.0.1:51851', 'http://127.0.0.1:51851'],
    ['http://127.0.0.1:51851/', 'http://127.0.0.1:51851'],
    ['http://0.0.0.0:51851', 'http://127.0.0.1:51851'],
    ['http://localhost:51851', 'http://127.0.0.1:51851'],
    ['http://[::1]:51851', undefined],
    ['https://127.0.0.1:51851', undefined],
    ['http://10.0.0.5:51851', undefined],
    ['http://127.0.0.1', undefined],
    ['http://127.0.0.1:51851/below', undefined],
    ['http://u:p@127.0.0.1:51851', undefined],
    [42, undefined],
  ])('takes %j as the app origin %j', (candidate, origin) => {
    expect(loopbackAppOrigin(candidate)).toBe(origin);
  });

  it('asks the daemon, with the secret, where it serves apps', async () => {
    const fetchImpl = vi.fn(
      async () =>
        ({
          ok: true,
          status: 200,
          json: async () => ({ status: 'ok', api_base_url: 'http://127.0.0.1:51851' }),
        }) as Response
    );
    await expect(
      fetchDaemonAppOrigin('http://127.0.0.1:51852/', 'desktop-secret', fetchImpl)
    ).resolves.toBe('http://127.0.0.1:51851');
    const [url, init] = fetchImpl.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe('http://127.0.0.1:51852/headless/health');
    expect(init.headers).toEqual({ 'X-Secret-Key': 'desktop-secret' });
  });

  it.each([
    async () => ({ ok: false, status: 502, json: async () => ({}) }) as Response,
    async () =>
      ({
        ok: true,
        status: 200,
        json: async () => ({ api_base_url: 'http://evil.test:1' }),
      }) as Response,
    async () =>
      ({
        ok: true,
        status: 200,
        json: async () => {
          throw new Error('not json');
        },
      }) as unknown as Response,
    async () => {
      throw new TypeError('fetch failed');
    },
  ])('names no app origin when the daemon does not say a loopback one (%#)', async (impl) => {
    await expect(
      fetchDaemonAppOrigin('http://127.0.0.1:51852', 'desktop-secret', vi.fn(impl))
    ).resolves.toBeUndefined();
  });
});
