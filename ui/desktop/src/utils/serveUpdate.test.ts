import { describe, expect, it, vi } from 'vitest';
import {
  isNewerVersion,
  offersUpdate,
  RELOADED_AT_KEY,
  ServeUpdateStore,
  UPDATE_REQUESTED_KEY,
  updateUrl,
  type ServeUpdateStatus,
  type ServeUpdateView,
} from './serveUpdate';

/**
 * SD-19: the browser half of updating a `biorouter serve` installation. The
 * store polls the daemon, asks it to install, and follows the restart through
 * to a reload. Every case drives it with a scripted fetch, so what it sends and
 * what it does with each answer are both visible.
 */

const READY: ServeUpdateStatus = {
  available: true,
  current: '1.94.1',
  latest: '1.95.0',
  phase: 'ready',
  percent: 100,
  installKind: 'package',
  runningTurns: 0,
};

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'Content-Type': 'application/json' },
  });
}

function memorySession() {
  const data = new Map<string, string>();
  return {
    getItem: (k: string) => data.get(k) ?? null,
    setItem: (k: string, v: string) => void data.set(k, v),
    removeItem: (k: string) => void data.delete(k),
    data,
  };
}

/** A store whose fetch answers from `script`, one answer per call. */
function storeWith(script: Array<Response | Error>, now = () => 1_000_000) {
  const calls: Array<{ url: string; init?: RequestInit }> = [];
  const reload = vi.fn();
  const session = memorySession();
  const fetchMock = vi.fn(async (url: string | URL | Request, init?: RequestInit) => {
    calls.push({ url: String(url), init });
    const next = script.shift();
    if (!next) throw new Error('no scripted answer');
    if (next instanceof Error) throw next;
    return next;
  });
  const store = new ServeUpdateStore({
    fetch: fetchMock as unknown as typeof fetch,
    secret: async () => 'old-secret',
    reload,
    now,
    session,
  });
  return { store, calls, reload, session };
}

describe('serve update store', () => {
  it('sends the secret and the access cookie with every request', async () => {
    const { store, calls } = storeWith([json(200, READY)]);
    await store.poll();
    store.stop();
    expect(calls[0].url).toBe(updateUrl(''));
    expect(calls[0].url.endsWith('/headless/update')).toBe(true);
    expect((calls[0].init?.headers as Record<string, string>)['X-Secret-Key']).toBe('old-secret');
    // The cookie is what says a person on this page asked (SD-19).
    expect(calls[0].init?.credentials).toBe('same-origin');
    expect(store.getSnapshot().status?.phase).toBe('ready');
  });

  it('follows an install through the restart to a reload', async () => {
    const { store, calls, reload, session } = storeWith([
      json(200, READY),
      new Response(null, { status: 202 }),
      json(200, { ...READY, phase: 'installing' }),
      // The old daemon is gone...
      new TypeError('Failed to fetch'),
      new TypeError('Failed to fetch'),
      // ...and a new one, with a new secret, does not know this page's.
      new Response(null, { status: 401 }),
    ]);
    await store.poll();
    expect(await store.install(false)).toBeNull();
    const install = calls[1];
    expect(install.url.endsWith('/headless/update/install')).toBe(true);
    expect(install.init?.method).toBe('POST');
    expect(JSON.parse(String(install.init?.body))).toEqual({ stopRunning: false });
    expect(session.data.get(UPDATE_REQUESTED_KEY)).toBe('1');
    expect(store.getSnapshot().dialogOpen).toBe(false);

    await store.poll();
    expect(store.getSnapshot().reconnecting).toBe(true);
    expect(store.getSnapshot().dialogOpen).toBe(true);
    await store.poll();
    expect(reload).not.toHaveBeenCalled();
    await store.poll();
    store.stop();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('reloads every open page whose status saw the restart, not only the one that asked', async () => {
    const { store, reload } = storeWith([
      json(200, READY),
      json(200, { ...READY, phase: 'restarting' }),
      new TypeError('Failed to fetch'),
      new Response(null, { status: 401 }),
    ]);
    await store.poll();
    await store.poll();
    expect(store.getSnapshot().dialogOpen).toBe(true);
    await store.poll();
    await store.poll();
    store.stop();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('reloads a page that missed the restart window, once', async () => {
    // A slow-polling tab never saw `installing`: the first it hears of the
    // restart is the new daemon refusing its old secret.
    const { store, reload, session } = storeWith([
      json(200, READY),
      new Response(null, { status: 401 }),
    ]);
    await store.poll();
    await store.poll();
    store.stop();
    expect(reload).toHaveBeenCalledTimes(1);
    expect(session.data.get(RELOADED_AT_KEY)).toBe('1000000');
  });

  it('never reloads in a loop when the address itself changed', async () => {
    const { store, reload, session } = storeWith([
      json(200, READY),
      new Response(null, { status: 401 }),
    ]);
    session.data.set(RELOADED_AT_KEY, String(1_000_000 - 5_000));
    await store.poll();
    await store.poll();
    expect(reload).not.toHaveBeenCalled();
    expect(store.getSnapshot().actionError).toContain('new address');
  });

  it('treats the failed-secret throttle like a refusal of the old secret', async () => {
    const { store, reload } = storeWith([json(200, READY), new Response(null, { status: 429 })]);
    await store.poll();
    await store.poll();
    store.stop();
    expect(reload).toHaveBeenCalledTimes(1);
  });

  it('never reloads a page the daemon has not answered yet', async () => {
    const { store, reload } = storeWith([new Response(null, { status: 401 })]);
    await store.poll();
    store.stop();
    expect(reload).not.toHaveBeenCalled();
  });

  it('says nothing when the daemon drops away outside a restart', async () => {
    const { store } = storeWith([json(200, READY), new TypeError('Failed to fetch')]);
    await store.poll();
    await store.poll();
    store.stop();
    expect(store.getSnapshot().reconnecting).toBe(false);
    expect(store.getSnapshot().dialogOpen).toBe(false);
  });

  it('turns a refusal for running chats into the line the dialog shows', async () => {
    const { store } = storeWith([
      json(200, READY),
      json(409, {
        reason: 'turns_running',
        message: '2 chats are still working.',
        runningTurns: 2,
      }),
    ]);
    await store.poll();
    expect(await store.install(false)).toBe('turns_running');
    store.stop();
    expect(store.getSnapshot().status?.runningTurns).toBe(2);
    expect(store.getSnapshot().actionError).toBe('2 chats are still working.');
  });

  it('shows the refusal a caller without the page cookie gets', async () => {
    const { store } = storeWith([
      json(200, READY),
      json(403, {
        reason: 'not_from_the_page',
        message: 'Only the person using the page can update.',
      }),
    ]);
    await store.poll();
    expect(await store.install(true)).toBe('not_from_the_page');
    store.stop();
    expect(store.getSnapshot().actionError).toContain('Only the person');
  });

  it('opens the dialog once for a recent update that was rolled back', async () => {
    const at = 1_000;
    const rolledBack = {
      ...READY,
      phase: 'up_to_date' as const,
      lastResult: {
        outcome: 'rolled_back' as const,
        from: '1.94.1',
        to: '1.95.0',
        reason: 'x',
        at,
      },
    };
    const { store } = storeWith(
      [json(200, rolledBack), json(200, rolledBack)],
      () => (at + 30) * 1000
    );
    await store.poll();
    expect(store.getSnapshot().dialogOpen).toBe(true);
    expect(store.getSnapshot().result?.to).toBe('1.95.0');
    store.closeDialog();
    await store.poll();
    store.stop();
    expect(store.getSnapshot().dialogOpen).toBe(false);
  });

  it('asks for a check with a POST', async () => {
    const { store, calls } = storeWith([
      new Response(null, { status: 202 }),
      json(200, { ...READY, phase: 'checking' }),
    ]);
    await store.check();
    store.stop();
    expect(calls[0].url.endsWith('/headless/update/check')).toBe(true);
    expect(calls[0].init?.method).toBe('POST');
    expect(store.getSnapshot().checkRequested).toBe(true);
  });
});

describe('offersUpdate', () => {
  const view = (status: Partial<ServeUpdateStatus> | null, extra: Partial<ServeUpdateView> = {}) =>
    ({
      status: status ? { ...READY, ...status } : null,
      reconnecting: false,
      waitedSeconds: 0,
      actionError: null,
      checkRequested: false,
      dialogOpen: false,
      result: null,
      ...extra,
    }) as ServeUpdateView;

  it('offers what is arriving, ready, or needs a hand', () => {
    for (const phase of ['downloading', 'ready', 'needs_admin', 'installed', 'manual'] as const) {
      expect(offersUpdate(view({ phase }))).toBe(true);
    }
  });

  it('offers nothing for a release with no file for this processor', () => {
    expect(offersUpdate(view({ phase: 'manual', manualReason: 'no_package' }))).toBe(false);
  });

  it('offers nothing when up to date, unavailable, or before the first answer', () => {
    expect(offersUpdate(view({ phase: 'up_to_date' }))).toBe(false);
    expect(offersUpdate(view({ available: false, phase: undefined }))).toBe(false);
    expect(offersUpdate(view(null))).toBe(false);
  });

  it('keeps a failed update offered only when a newer release is known', () => {
    expect(offersUpdate(view({ phase: 'error', latest: '1.95.0' }))).toBe(true);
    expect(offersUpdate(view({ phase: 'error', latest: '1.94.1' }))).toBe(false);
  });

  it('stays offered while the page waits for the restart', () => {
    expect(offersUpdate(view(null, { reconnecting: true }))).toBe(true);
  });
});

describe('isNewerVersion', () => {
  it('compares numerically and ignores a suffix', () => {
    expect(isNewerVersion('1.10.0', '1.9.9')).toBe(true);
    expect(isNewerVersion('v1.95.0', '1.94.1')).toBe(true);
    expect(isNewerVersion('1.94.1-rc.1', '1.94.1')).toBe(false);
    expect(isNewerVersion(null, '1.94.1')).toBe(false);
  });
});
