// @vitest-environment node
import fs from 'node:fs';
import http, { type Server } from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import WebSocket, { WebSocketServer } from 'ws';
import {
  createDaemonProxy,
  DaemonKeyRefusedError,
  daemonRuntimePath,
  daemonVersion,
  discoverDaemonRuntime,
  generateUserActionKey,
  readUserActionKey,
  removeUserActionKey,
  stopProfileDaemon,
  type DaemonRuntime,
  type StopDaemonDeps,
  type StopSignal,
  userActionKeyPath,
  verifyDaemonRuntime,
  verifyHumanAuthorizedAccess,
  writeUserActionKey,
} from './daemonRuntime';

const PROFILE_ID = '11111111-1111-4111-8111-111111111111';
const INSTANCE_ID = '22222222-2222-4222-8222-222222222222';
const DESKTOP_SECRET = 'desktop-secret-never-forwarded';
const DAEMON_SECRET = 'daemon-secret-kept-upstream-123456';
const RENDERER_ORIGIN = 'http://renderer.test';

type Fixture = {
  root: string;
  socketPath: string;
  runtime: DaemonRuntime;
  identity: Record<string, unknown>;
  server: Server;
  upgrades: string[];
  upstreamRequests: { path: string; secret: string | undefined }[];
  close: () => Promise<void>;
};

async function unixFixture(): Promise<Fixture> {
  // macOS limits Unix-domain socket paths to a small fixed-size buffer. Keep
  // the synthetic root short enough that the private daemon socket remains
  // usable while still isolating each fixture.
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'br-runtime-'));
  const config = path.join(root, 'config');
  const daemonDir = path.join(root, 'state', 'daemon');
  fs.mkdirSync(config, { recursive: true, mode: 0o700 });
  fs.mkdirSync(daemonDir, { recursive: true, mode: 0o700 });
  fs.chmodSync(config, 0o700);
  fs.chmodSync(daemonDir, 0o700);
  fs.writeFileSync(
    path.join(config, 'daemon-profile.json'),
    JSON.stringify({
      version: 1,
      profile_id: PROFILE_ID,
      config_dir: fs.realpathSync(config),
    }),
    { mode: 0o600 }
  );

  const socketPath = path.join(daemonDir, 'daemon.sock');
  const identity = {
    version: 1,
    profile_id: PROFILE_ID,
    instance_id: INSTANCE_ID,
    pid: process.pid,
    user_action_installed: true,
  };
  const upgrades: string[] = [];
  const upstreamRequests: { path: string; secret: string | undefined }[] = [];
  const websocketServer = new WebSocketServer({ noServer: true });
  const server = http.createServer((request, response) => {
    const parsed = new URL(request.url ?? '/', 'http://localhost');
    upstreamRequests.push({
      path: parsed.pathname + parsed.search,
      secret:
        typeof request.headers['x-secret-key'] === 'string'
          ? request.headers['x-secret-key']
          : undefined,
    });
    if (parsed.pathname === '/daemon/identity') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify(identity));
      return;
    }
    if (parsed.pathname === '/system_info') {
      response.writeHead(200, { 'content-type': 'application/json' });
      response.end(JSON.stringify({ app_version: '9.8.7', os: 'test' }));
      return;
    }
    if (parsed.pathname === '/stream') {
      response.writeHead(200, { 'content-type': 'text/event-stream' });
      response.write('data: first\n\n');
      response.end('data: second\n\n');
      return;
    }
    response.writeHead(200, { 'content-type': 'text/plain' });
    response.end('upstream-ok');
  });
  server.on('upgrade', (request, socket, head) => {
    upgrades.push(request.url ?? '');
    websocketServer.handleUpgrade(request, socket, head, (client) => {
      setTimeout(() => client.send('upstream-connected'), 20);
      client.on('message', (data) => client.send('echo:' + data.toString()));
    });
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(socketPath, () => resolve());
  });
  fs.chmodSync(socketPath, 0o600);

  const runtime: DaemonRuntime = {
    version: 1,
    profile_id: PROFILE_ID,
    instance_id: INSTANCE_ID,
    pid: process.pid,
    endpoint: { kind: 'unix', path: socketPath },
    api_secret: DAEMON_SECRET,
    user_action_installed: true,
  };
  process.env.BIOROUTER_PATH_ROOT = root;
  fs.writeFileSync(daemonRuntimePath(), JSON.stringify(runtime), { mode: 0o600 });

  return {
    root,
    socketPath,
    runtime,
    identity,
    server,
    upgrades,
    upstreamRequests,
    close: async () => {
      websocketServer.clients.forEach((client) => client.terminate());
      websocketServer.close();
      await new Promise<void>((resolve) => server.close(() => resolve()));
      fs.rmSync(root, { recursive: true, force: true });
    },
  };
}

async function responseText(url: string, init?: RequestInit) {
  const target = new URL(url);
  const response = await new Promise<{
    statusCode: number;
    headers: http.IncomingHttpHeaders;
    body: string;
  }>((resolve, reject) => {
    const request = http.request(
      target,
      {
        method: init?.method,
        headers: init?.headers as Record<string, string> | undefined,
      },
      (incoming) => {
        let body = '';
        incoming.setEncoding('utf8');
        incoming.on('data', (chunk) => (body += chunk));
        incoming.on('end', () =>
          resolve({ statusCode: incoming.statusCode ?? 0, headers: incoming.headers, body })
        );
      }
    );
    request.once('error', reject);
    request.end(init?.body as string | undefined);
  });
  return {
    response: {
      status: response.statusCode,
      headers: { get: (name: string) => response.headers[name.toLowerCase()] ?? null },
    },
    body: response.body,
  };
}

function waitForMessage(socket: WebSocket): Promise<string> {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('WebSocket message timeout')), 2000);
    socket.once('message', (data) => {
      clearTimeout(timer);
      resolve(data.toString());
    });
    socket.once('error', reject);
  });
}

async function openSocket(url: string, options: WebSocket.ClientOptions = {}): Promise<WebSocket> {
  const socket = new WebSocket(url, options);
  await new Promise<void>((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error('WebSocket open timeout')), 2000);
    socket.once('open', () => {
      clearTimeout(timer);
      resolve();
    });
    socket.once('error', reject);
  });
  return socket;
}

async function expectClosed(url: string, options: WebSocket.ClientOptions = {}) {
  const socket = new WebSocket(url, options);
  await new Promise<void>((resolve) => {
    const timer = setTimeout(resolve, 2000);
    const done = () => {
      clearTimeout(timer);
      resolve();
    };
    socket.once('close', done);
    socket.once('error', done);
  });
  socket.terminate();
}

describe.sequential('shared daemon runtime and pinned Unix proxy', () => {
  let previousRoot: string | undefined;
  let fixture: Fixture | undefined;

  beforeEach(() => {
    previousRoot = process.env.BIOROUTER_PATH_ROOT;
  });
  afterEach(async () => {
    if (fixture) await fixture.close();
    fixture = undefined;
    if (previousRoot === undefined) delete process.env.BIOROUTER_PATH_ROOT;
    else process.env.BIOROUTER_PATH_ROOT = previousRoot;
  });

  it('discovers a private manifest and rejects a runtime symlink', async () => {
    const current = (fixture = await unixFixture());
    const discovered = discoverDaemonRuntime();
    expect(discovered).toMatchObject({
      profile_id: PROFILE_ID,
      instance_id: INSTANCE_ID,
      endpoint: { path: current.socketPath },
    });

    const runtimePath = daemonRuntimePath();
    const replacement = runtimePath + '.real';
    fs.renameSync(runtimePath, replacement);
    fs.symlinkSync(replacement, runtimePath);
    expect(() => discoverDaemonRuntime()).toThrow(/symbolic link|private/i);
  });

  it('rejects a hard-linked runtime descriptor instead of trusting a second directory entry', async () => {
    fixture = await unixFixture();
    const runtimePath = daemonRuntimePath();
    fs.linkSync(runtimePath, runtimePath + '.alias');
    expect(() => discoverDaemonRuntime()).toThrow(/Invalid private daemon metadata/);
  });

  it('rejects a valid-looking descriptor that points outside the profile daemon socket', async () => {
    const current = (fixture = await unixFixture());
    const runtimePath = daemonRuntimePath();
    fs.writeFileSync(
      runtimePath,
      JSON.stringify({
        ...current.runtime,
        endpoint: { kind: 'unix', path: path.join(current.root, 'foreign.sock') },
      }),
      { mode: 0o600 }
    );
    expect(() => discoverDaemonRuntime()).toThrow(/outside this profile runtime directory/);
  });

  it('rejects a recycled instance on the same private socket before any proof callback', async () => {
    const current = (fixture = await unixFixture());
    const runtime = discoverDaemonRuntime()!;
    current.identity.instance_id = '33333333-3333-4333-8333-333333333333';
    await expect(verifyDaemonRuntime(runtime)).rejects.toThrow(/identity changed|instance/i);
  });

  it('verifies identity then forwards authenticated HTTP and SSE without exposing the desktop secret', async () => {
    const current = (fixture = await unixFixture());
    const runtime = discoverDaemonRuntime()!;
    const proxy = await createDaemonProxy(
      runtime,
      DESKTOP_SECRET,
      'desktop-proof',
      'daemon-proof',
      RENDERER_ORIGIN
    );
    try {
      const ordinary = await responseText(proxy.baseUrl + '/crew/echo', {
        headers: { 'X-Secret-Key': DESKTOP_SECRET, Origin: RENDERER_ORIGIN },
      });
      expect(ordinary.response.status).toBe(200);
      expect(ordinary.body).toBe('upstream-ok');
      expect(current.upstreamRequests[current.upstreamRequests.length - 1]).toMatchObject({
        path: '/crew/echo',
        secret: DAEMON_SECRET,
      });
      expect(proxy.baseUrl).not.toContain(DESKTOP_SECRET);

      const stream = await responseText(proxy.baseUrl + '/stream', {
        headers: { 'X-Secret-Key': DESKTOP_SECRET, Origin: RENDERER_ORIGIN },
      });
      expect(stream.response.status).toBe(200);
      expect(stream.response.headers.get('content-type')).toContain('text/event-stream');
      expect(stream.body).toBe('data: first\n\ndata: second\n\n');
      expect(stream.response.headers.get('access-control-allow-origin')).toBe(RENDERER_ORIGIN);
    } finally {
      proxy.close();
    }
  });

  it('refuses wrong origin and query credentials on ordinary HTTP, while exact CORS preflight succeeds', async () => {
    const current = (fixture = await unixFixture());
    const proxy = await createDaemonProxy(
      current.runtime,
      DESKTOP_SECRET,
      undefined,
      undefined,
      RENDERER_ORIGIN
    );
    try {
      const wrongOrigin = await responseText(proxy.baseUrl + '/crew/echo', {
        headers: { 'X-Secret-Key': DESKTOP_SECRET, Origin: 'http://evil.test' },
      });
      expect(wrongOrigin.response.status).toBe(403);
      const querySecret = await responseText(
        proxy.baseUrl + '/crew/echo?secret=' + encodeURIComponent(DESKTOP_SECRET)
      );
      expect(querySecret.response.status).toBe(403);
      const preflight = await responseText(proxy.baseUrl + '/crew/echo', {
        method: 'OPTIONS',
        headers: {
          Origin: RENDERER_ORIGIN,
          'Access-Control-Request-Method': 'GET',
          'Access-Control-Request-Headers': 'X-Secret-Key',
        },
      });
      expect(preflight.response.status).toBe(204);
      expect(preflight.response.headers.get('access-control-allow-origin')).toBe(RENDERER_ORIGIN);
      expect(current.upstreamRequests.filter(({ path }) => path === '/crew/echo')).toHaveLength(0);
    } finally {
      proxy.close();
    }
  });

  it('allows only the workspace query-secret WebSocket and the header-authenticated Crew WebSocket', async () => {
    const current = (fixture = await unixFixture());
    const proxy = await createDaemonProxy(
      current.runtime,
      DESKTOP_SECRET,
      undefined,
      undefined,
      RENDERER_ORIGIN
    );
    const wsBase = proxy.baseUrl.replace(/^http/, 'ws');
    try {
      const workspace = await openSocket(
        wsBase + '/ui/workspace?secret=' + encodeURIComponent(DESKTOP_SECRET),
        { origin: RENDERER_ORIGIN }
      );
      expect(await waitForMessage(workspace)).toBe('upstream-connected');
      workspace.send('workspace-frame');
      expect(await waitForMessage(workspace)).toBe('echo:workspace-frame');
      workspace.close();
      expect(current.upgrades).toContain(
        '/ui/workspace?secret=' + encodeURIComponent(DAEMON_SECRET)
      );

      const crew = await openSocket(wsBase + '/crew/terminal', {
        origin: RENDERER_ORIGIN,
        headers: { 'X-Secret-Key': DESKTOP_SECRET },
      });
      expect(await waitForMessage(crew)).toBe('upstream-connected');
      crew.close();

      await expectClosed(wsBase + '/crew/terminal?secret=' + encodeURIComponent(DESKTOP_SECRET), {
        origin: RENDERER_ORIGIN,
      });
      expect(current.upgrades.filter((url) => url.startsWith('/crew/terminal?'))).toHaveLength(0);
    } finally {
      proxy.close();
    }
  });
});

/**
 * A stand-in for a daemon instance on the profile's socket: its identity, its own secret, and
 * the user-action key its person-gated routes take (as the raw `X-User-Action`, which the daemon
 * hashes and compares with its installed digest).
 */
async function instanceOn(
  socketPath: string,
  identity: { instance_id: string; pid: number },
  secret: string,
  approval: string
) {
  const seen: { path: string; secret?: string; userAction?: string }[] = [];
  const server = http.createServer((request, response) => {
    const headers = request.headers;
    seen.push({
      path: request.url ?? '',
      secret: typeof headers['x-secret-key'] === 'string' ? headers['x-secret-key'] : undefined,
      userAction:
        typeof headers['x-user-action'] === 'string' ? headers['x-user-action'] : undefined,
    });
    if (headers['x-secret-key'] !== secret) {
      response.writeHead(401).end();
      return;
    }
    if (request.url === '/daemon/identity') {
      response.writeHead(200, { 'content-type': 'application/json' }).end(
        JSON.stringify({
          version: 1,
          profile_id: PROFILE_ID,
          ...identity,
          user_action_installed: true,
        })
      );
      return;
    }
    if (request.url === '/crew/connections' && headers['x-user-action'] !== approval) {
      response.writeHead(403).end();
      return;
    }
    response
      .writeHead(200, { 'content-type': 'text/plain' })
      .end(`ok from ${identity.instance_id}`);
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(socketPath, () => resolve());
  });
  fs.chmodSync(socketPath, 0o600);
  return {
    seen,
    stop: async () => {
      await new Promise<void>((resolve) => server.close(() => resolve()));
      fs.rmSync(socketPath, { force: true });
    },
  };
}

describe.sequential('a shared daemon that restarts under the app (R-1)', () => {
  let previousRoot: string | undefined;
  let fixture: Fixture | undefined;
  const cleanups: (() => Promise<void> | void)[] = [];

  beforeEach(() => {
    previousRoot = process.env.BIOROUTER_PATH_ROOT;
  });
  afterEach(async () => {
    for (const cleanup of cleanups.splice(0).reverse()) await cleanup();
    if (fixture) await fixture.close();
    fixture = undefined;
    if (previousRoot === undefined) delete process.env.BIOROUTER_PATH_ROOT;
    else process.env.BIOROUTER_PATH_ROOT = previousRoot;
  });

  const INSTANCE_B = '44444444-4444-4444-8444-444444444444';
  const SECRET_B = 'daemon-secret-of-the-new-instance-b';
  const APPROVAL_A = 'user-action-key-of-instance-a-0123456789';
  const APPROVAL_B = 'user-action-key-of-instance-b-9876543210';
  const get = (proxy: { baseUrl: string }, headers: Record<string, string> = {}) =>
    responseText(proxy.baseUrl + '/crew/echo', {
      headers: { 'X-Secret-Key': DESKTOP_SECRET, Origin: RENDERER_ORIGIN, ...headers },
    });

  it('refuses every request once the instance is gone or replaced, until an explicit reattach, then serves the new one at the same address', async () => {
    const current = (fixture = await unixFixture());
    const proxy = await createDaemonProxy(
      current.runtime,
      DESKTOP_SECRET,
      'desktop-proof',
      APPROVAL_A,
      RENDERER_ORIGIN
    );
    cleanups.push(() => proxy.close());
    const events: unknown[] = [];
    proxy.onConnection((event) => events.push(event));
    expect((await get(proxy)).response.status).toBe(200);

    // A stops (`biorouter crew daemon stop`): nothing answers on the socket.
    await new Promise<void>((resolve) => current.server.close(() => resolve()));
    fs.rmSync(current.socketPath, { force: true });
    const gone = await get(proxy);
    expect(gone.response.status).toBe(502);
    expect(JSON.parse(gone.body)).toMatchObject({
      code: 'daemon_restarted',
      message: expect.stringContaining('background service restarted'),
    });
    expect(gone.body).not.toMatch(/explicitly/);
    expect(events).toEqual([{ kind: 'lost', reason: 'gone', instanceId: INSTANCE_ID }]);
    await expect(proxy.probe()).resolves.toEqual({ reason: 'gone', instanceId: INSTANCE_ID });

    // B starts on the same socket, with its own identity and secret.
    const b = await instanceOn(
      current.socketPath,
      { instance_id: INSTANCE_B, pid: process.pid + 1 },
      SECRET_B,
      APPROVAL_B
    );
    cleanups.push(() => b.stop());
    // Concurrent requests that all fail the identity check report the replacement once.
    const refusals = await Promise.all(
      [0, 1, 2, 3].map(() => get(proxy, { 'X-User-Action': 'desktop-proof' }))
    );
    for (const refused of refusals) {
      expect(refused.response.status).toBe(502);
      expect(JSON.parse(refused.body).code).toBe('daemon_restarted');
    }
    // Replaced, once: the proxy never followed B, and never sent B anything but the identity
    // check with A's secret, which B refused.
    expect(events.slice(1)).toEqual([
      { kind: 'lost', reason: 'replaced', instanceId: INSTANCE_ID },
    ]);
    expect(b.seen.every(({ path }) => path === '/daemon/identity')).toBe(true);
    expect(b.seen.every(({ secret }) => secret === DAEMON_SECRET)).toBe(true);

    const runtimeB: DaemonRuntime = {
      ...current.runtime,
      instance_id: INSTANCE_B,
      pid: process.pid + 1,
      api_secret: SECRET_B,
    };
    // A key B refuses changes nothing.
    await expect(proxy.retarget(runtimeB, 'not-the-key-of-instance-b-0000000000')).rejects.toThrow(
      DaemonKeyRefusedError
    );
    expect(proxy.instanceId()).toBe(INSTANCE_ID);
    expect((await get(proxy)).response.status).toBe(502);

    // The app reattached with B's saved key: the same address now reaches B.
    const baseUrl = proxy.baseUrl;
    await proxy.retarget(runtimeB, APPROVAL_B);
    expect(proxy.baseUrl).toBe(baseUrl);
    expect(proxy.instanceId()).toBe(INSTANCE_B);
    const served = await get(proxy, { 'X-User-Action': 'desktop-proof' });
    expect(served.response.status).toBe(200);
    expect(served.body).toBe(`ok from ${INSTANCE_B}`);
    expect(b.seen[b.seen.length - 1]).toEqual({
      path: '/crew/echo',
      secret: SECRET_B,
      userAction: APPROVAL_B,
    });
    expect(events[events.length - 1]).toEqual({ kind: 'answered', instanceId: INSTANCE_B });
    await expect(proxy.probe()).resolves.toBeUndefined();
  });

  it('reports an instance that only stopped answering for a moment when it answers again', async () => {
    const current = (fixture = await unixFixture());
    const proxy = await createDaemonProxy(
      current.runtime,
      DESKTOP_SECRET,
      undefined,
      undefined,
      RENDERER_ORIGIN
    );
    cleanups.push(() => proxy.close());
    const events: unknown[] = [];
    proxy.onConnection((event) => events.push(event));
    await new Promise<void>((resolve) => current.server.close(() => resolve()));
    fs.rmSync(current.socketPath, { force: true });
    expect((await get(proxy)).response.status).toBe(502);
    expect((await get(proxy)).response.status).toBe(502);
    // The same instance back on the socket (its identity unchanged).
    const again = await instanceOn(
      current.socketPath,
      { instance_id: INSTANCE_ID, pid: process.pid },
      DAEMON_SECRET,
      APPROVAL_A
    );
    cleanups.push(() => again.stop());
    expect((await get(proxy)).response.status).toBe(200);
    expect(events).toEqual([
      { kind: 'lost', reason: 'gone', instanceId: INSTANCE_ID },
      { kind: 'answered', instanceId: INSTANCE_ID },
    ]);
  });

  it('never retargets to another profile, and not after it closed', async () => {
    const current = (fixture = await unixFixture());
    const proxy = await createDaemonProxy(
      current.runtime,
      DESKTOP_SECRET,
      undefined,
      undefined,
      RENDERER_ORIGIN
    );
    await expect(
      proxy.retarget(
        { ...current.runtime, profile_id: '99999999-9999-4999-8999-999999999999' },
        APPROVAL_A
      )
    ).rejects.toThrow(/another Biorouter profile/);
    proxy.close();
    await expect(proxy.retarget(current.runtime, APPROVAL_A)).rejects.toThrow(/closed/);
  });
});

describe.sequential('the user-action key file', () => {
  let previousRoot: string | undefined;
  let fixture: Fixture | undefined;

  beforeEach(() => {
    previousRoot = process.env.BIOROUTER_PATH_ROOT;
  });
  afterEach(async () => {
    if (fixture) await fixture.close();
    fixture = undefined;
    if (previousRoot === undefined) delete process.env.BIOROUTER_PATH_ROOT;
    else process.env.BIOROUTER_PATH_ROOT = previousRoot;
  });

  const noGrace = { graceMs: 0 };

  it('sits beside runtime.json and holds a fresh 64-digit hex key', async () => {
    fixture = await unixFixture();
    expect(userActionKeyPath()).toBe(
      path.join(path.dirname(daemonRuntimePath()), 'user-action-key.json')
    );
    const first = generateUserActionKey();
    expect(first).toMatch(/^[0-9a-f]{64}$/);
    expect(generateUserActionKey()).not.toBe(first);
  });

  it('round-trips atomically as a private 0600 record bound to the instance', async () => {
    const current = (fixture = await unixFixture());
    const key = generateUserActionKey();
    writeUserActionKey(current.runtime, key);
    const stat = fs.lstatSync(userActionKeyPath());
    expect(stat.isFile()).toBe(true);
    expect(stat.mode & 0o777).toBe(0o600);
    expect(JSON.parse(fs.readFileSync(userActionKeyPath(), 'utf8'))).toEqual({
      version: 1,
      profile_id: PROFILE_ID,
      instance_id: INSTANCE_ID,
      pid: process.pid,
      key,
    });
    // No temporary file left behind.
    expect(
      fs.readdirSync(path.dirname(userActionKeyPath())).filter((name) => name.endsWith('.tmp'))
    ).toEqual([]);
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBe(key);
    // A second write replaces it.
    const next = generateUserActionKey();
    writeUserActionKey(current.runtime, next);
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBe(next);
  });

  it('refuses to write anything but a 64-digit hex key', async () => {
    const current = (fixture = await unixFixture());
    expect(() => writeUserActionKey(current.runtime, 'x'.repeat(64))).toThrow();
    expect(fs.existsSync(userActionKeyPath())).toBe(false);
  });

  it('ignores a record for another instance, pid or profile, or with extra fields', async () => {
    const current = (fixture = await unixFixture());
    const key = generateUserActionKey();
    writeUserActionKey(current.runtime, key);
    const other = { ...current.runtime, instance_id: '55555555-5555-4555-8555-555555555555' };
    await expect(readUserActionKey(other, noGrace)).resolves.toBeUndefined();
    await expect(
      readUserActionKey({ ...current.runtime, pid: current.runtime.pid + 1 }, noGrace)
    ).resolves.toBeUndefined();
    await expect(
      readUserActionKey(
        { ...current.runtime, profile_id: '99999999-9999-4999-8999-999999999999' },
        noGrace
      )
    ).resolves.toBeUndefined();
    const record = JSON.parse(fs.readFileSync(userActionKeyPath(), 'utf8'));
    fs.writeFileSync(userActionKeyPath(), JSON.stringify({ ...record, extra: true }), {
      mode: 0o600,
    });
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBeUndefined();
    fs.writeFileSync(userActionKeyPath(), JSON.stringify({ ...record, key: 'A'.repeat(64) }), {
      mode: 0o600,
    });
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBeUndefined();
  });

  it('refuses a key file that is not private: readable by others, a symbolic link or a hard link', async () => {
    const current = (fixture = await unixFixture());
    const key = generateUserActionKey();
    writeUserActionKey(current.runtime, key);
    const target = userActionKeyPath();
    for (const mode of [0o400, 0o644, 0o700, 0o4600, 0o2600, 0o1600]) {
      fs.chmodSync(target, mode);
      await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBeUndefined();
    }
    fs.chmodSync(target, 0o600);

    fs.linkSync(target, target + '.alias');
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBeUndefined();
    fs.rmSync(target + '.alias');
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBe(key);

    const real = path.join(current.root, 'elsewhere.json');
    fs.renameSync(target, real);
    fs.symlinkSync(real, target);
    await expect(readUserActionKey(current.runtime, noGrace)).resolves.toBeUndefined();
  });

  it('refuses to write a key into a daemon directory with a non-0700 mode', async () => {
    const current = (fixture = await unixFixture());
    const directory = path.dirname(userActionKeyPath());
    for (const mode of [0o500, 0o600, 0o755, 0o4700, 0o2700, 0o1700]) {
      fs.chmodSync(directory, mode);
      expect(() => writeUserActionKey(current.runtime, generateUserActionKey())).toThrow();
    }
    fs.chmodSync(directory, 0o700);
  });

  it('waits for a starter that writes the key just after the daemon publishes', async () => {
    const current = (fixture = await unixFixture());
    const key = generateUserActionKey();
    setTimeout(() => writeUserActionKey(current.runtime, key), 150);
    await expect(
      readUserActionKey(current.runtime, { graceMs: 2000, intervalMs: 25 })
    ).resolves.toBe(key);
  });

  it('removes the saved key only while it belongs to the instance named', async () => {
    const current = (fixture = await unixFixture());
    writeUserActionKey(current.runtime, generateUserActionKey());
    removeUserActionKey({
      ...current.runtime,
      instance_id: '55555555-5555-4555-8555-555555555555',
    });
    expect(fs.existsSync(userActionKeyPath())).toBe(true);
    removeUserActionKey(current.runtime);
    expect(fs.existsSync(userActionKeyPath())).toBe(false);
  });

  it('reads the daemon version over the verified socket, and tells a refused key from other failures', async () => {
    const current = (fixture = await unixFixture());
    await expect(daemonVersion(current.runtime)).resolves.toBe('9.8.7');
    // The fixture answers /crew/connections 200 whatever the key.
    await expect(verifyHumanAuthorizedAccess(current.runtime, 'k'.repeat(64))).resolves.toBe(
      undefined
    );
    await new Promise<void>((resolve) => current.server.close(() => resolve()));
    fs.rmSync(current.socketPath, { force: true });
    const b = await instanceOn(
      current.socketPath,
      { instance_id: INSTANCE_ID, pid: process.pid },
      DAEMON_SECRET,
      'the-right-key'
    );
    try {
      await expect(verifyHumanAuthorizedAccess(current.runtime, 'the-wrong-key')).rejects.toThrow(
        DaemonKeyRefusedError
      );
      await expect(verifyHumanAuthorizedAccess(current.runtime, 'the-right-key')).resolves.toBe(
        undefined
      );
    } finally {
      await b.stop();
    }
  });
});

describe('stopping a daemon this app cannot use', () => {
  const runtime: DaemonRuntime = {
    version: 1,
    profile_id: PROFILE_ID,
    instance_id: INSTANCE_ID,
    pid: 4242,
    endpoint: { kind: 'unix', path: '/nonexistent/daemon.sock' },
    api_secret: DAEMON_SECRET,
    user_action_installed: true,
  };
  const esrch = () => Object.assign(new Error('no such process'), { code: 'ESRCH' });

  /** A process that exits `exitsAfter` signals (of those given) after they are sent. */
  const fakeProcess = (options: {
    name?: string;
    exitsOn?: StopSignal[];
  }): StopDaemonDeps & { signals: (StopSignal | 0)[] } => {
    let alive = true;
    let clock = 0;
    const signals: (StopSignal | 0)[] = [];
    return {
      signals,
      kill: (_pid, signal) => {
        if (!alive) throw esrch();
        if (signal !== 0) {
          signals.push(signal);
          if (options.exitsOn?.includes(signal)) alive = false;
        }
      },
      processName: async () => (alive ? (options.name ?? 'biorouterd') : undefined),
      wait: async (ms) => {
        clock += ms;
      },
      now: () => clock,
    };
  };

  it('verifies the instance, checks the name, then sends SIGTERM', async () => {
    const deps = fakeProcess({ exitsOn: ['SIGTERM'] });
    const verify = vi.fn(async () => undefined);
    await stopProfileDaemon(runtime, deps, verify);
    expect(verify).toHaveBeenCalledWith(runtime);
    expect(deps.signals).toEqual(['SIGTERM']);
  });

  it('sends SIGKILL only after 15 seconds of SIGTERM being ignored', async () => {
    const deps = fakeProcess({ exitsOn: ['SIGKILL'] });
    await stopProfileDaemon(runtime, deps, async () => undefined);
    expect(deps.signals).toEqual(['SIGTERM', 'SIGKILL']);
    expect(deps.now()).toBeGreaterThanOrEqual(15000);
  });

  it('says so when even SIGKILL leaves the process running', async () => {
    const deps = fakeProcess({});
    await expect(stopProfileDaemon(runtime, deps, async () => undefined)).rejects.toThrow(
      'Biorouter could not stop the old background service (process 4242). Quit it, then open Biorouter again.'
    );
  });

  it('never signals a process that is not biorouterd', async () => {
    const deps = fakeProcess({ name: 'bash', exitsOn: ['SIGTERM'] });
    await expect(stopProfileDaemon(runtime, deps, async () => undefined)).rejects.toThrow(
      /not a Biorouter background service/
    );
    expect(deps.signals).toEqual([]);
  });

  it('never signals when the socket answers as another instance', async () => {
    const deps = fakeProcess({ exitsOn: ['SIGTERM'] });
    const { DaemonIdentityChangedError } = await import('./daemonRuntime');
    await expect(
      stopProfileDaemon(runtime, deps, async () => {
        throw new DaemonIdentityChangedError();
      })
    ).rejects.toThrow(DaemonIdentityChangedError);
    expect(deps.signals).toEqual([]);
  });

  it('does nothing when the daemon is already gone', async () => {
    const deps = fakeProcess({ exitsOn: ['SIGTERM'] });
    deps.kill(runtime.pid, 'SIGTERM');
    deps.signals.length = 0;
    await stopProfileDaemon(runtime, deps, async () => {
      throw Object.assign(new Error('connect ENOENT'), { code: 'ENOENT', syscall: 'connect' });
    });
    expect(deps.signals).toEqual([]);
  });
});
