// @vitest-environment node
import fs from 'node:fs';
import http, { type Server } from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { afterEach, beforeEach, describe, expect, it } from 'vitest';
import WebSocket, { WebSocketServer } from 'ws';
import {
  createDaemonProxy,
  daemonRuntimePath,
  discoverDaemonRuntime,
  type DaemonRuntime,
  verifyDaemonRuntime,
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
