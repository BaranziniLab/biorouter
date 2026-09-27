import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import net from 'node:net';
import { timingSafeEqual } from 'node:crypto';
import { biorouterConfigDir } from './utils/biorouterPaths';

export interface DaemonRuntime {
  version: 1;
  profile_id: string;
  instance_id: string;
  pid: number;
  endpoint: { kind: 'unix'; path: string };
  api_secret: string;
  user_action_installed: boolean;
}

export interface DaemonProxy {
  baseUrl: string;
  close: () => void;
}

const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

export function daemonRuntimePath(): string {
  const root = process.env.BIOROUTER_PATH_ROOT;
  const xdg = process.env.XDG_STATE_HOME;
  const state = root?.trim()
    ? path.join(root, 'state')
    : path.join(
        xdg && path.isAbsolute(xdg) ? xdg : path.join(os.homedir(), '.local/state'),
        'biorouter'
      );
  return path.join(state, 'daemon', 'runtime.json');
}

function privateOwned(location: string, kind: 'file' | 'directory' | 'socket'): void {
  const stat = fs.lstatSync(location);
  const valid =
    kind === 'file' ? stat.isFile() : kind === 'directory' ? stat.isDirectory() : stat.isSocket();
  if (
    !valid ||
    stat.isSymbolicLink() ||
    stat.uid !== process.getuid?.() ||
    (stat.mode & 0o777) !== (kind === 'directory' ? 0o700 : 0o600)
  )
    throw new Error(
      `Daemon ${kind} must be private, owned by this user, and not a symbolic link: ${location}`
    );
}

function privateJson(location: string): unknown {
  privateOwned(location, 'file');
  const handle = fs.openSync(location, fs.constants.O_RDONLY | fs.constants.O_NOFOLLOW);
  try {
    const stat = fs.fstatSync(handle);
    if (
      !stat.isFile() ||
      stat.uid !== process.getuid?.() ||
      (stat.mode & 0o777) !== 0o600 ||
      stat.size > 16384 ||
      stat.nlink !== 1
    )
      throw new Error('Invalid private daemon metadata.');
    try {
      return JSON.parse(fs.readFileSync(handle, 'utf8'));
    } catch {
      throw new Error(
        'Daemon metadata is not valid JSON. Repair the private runtime metadata before attaching.'
      );
    }
  } finally {
    fs.closeSync(handle);
  }
}

export function discoverDaemonRuntime(): DaemonRuntime | undefined {
  if (process.platform === 'win32')
    throw new Error(
      'Shared daemon attachment requires an owner-protected Windows named pipe and is unavailable in this build.'
    );
  const location = daemonRuntimePath();
  try {
    fs.lstatSync(location);
  } catch (error) {
    if ((error as Error & { code?: string }).code === 'ENOENT') return undefined;
    throw error;
  }
  privateOwned(path.dirname(location), 'directory');
  const config = fs.realpathSync(biorouterConfigDir());
  const profile = privateJson(path.join(config, 'daemon-profile.json')) as Record<string, unknown>;
  const runtime = privateJson(location) as DaemonRuntime;
  if (
    !profile ||
    profile.version !== 1 ||
    typeof profile.profile_id !== 'string' ||
    !uuid.test(profile.profile_id) ||
    profile.config_dir !== config
  )
    throw new Error(
      'Daemon profile identity does not match this canonical configuration directory.'
    );
  if (
    !runtime ||
    runtime.version !== 1 ||
    runtime.profile_id !== profile.profile_id ||
    !uuid.test(runtime.instance_id) ||
    !Number.isSafeInteger(runtime.pid) ||
    runtime.pid < 1 ||
    runtime.endpoint?.kind !== 'unix' ||
    !path.isAbsolute(runtime.endpoint.path) ||
    typeof runtime.api_secret !== 'string' ||
    runtime.api_secret.length < 32 ||
    typeof runtime.user_action_installed !== 'boolean'
  )
    throw new Error('Invalid daemon runtime descriptor.');
  validateEndpoint(runtime);
  return runtime;
}

function validateEndpoint(runtime: DaemonRuntime): void {
  if (runtime.endpoint.path !== path.join(path.dirname(daemonRuntimePath()), 'daemon.sock'))
    throw new Error('Daemon socket is outside this profile runtime directory.');
  privateOwned(path.dirname(runtime.endpoint.path), 'directory');
  try {
    privateOwned(runtime.endpoint.path, 'socket');
  } catch (error) {
    if ((error as Error & { code?: string }).code !== 'ENOENT') throw error;
  }
}

async function authenticatedAgent(runtime: DaemonRuntime): Promise<http.Agent> {
  validateEndpoint(runtime);
  const agent = new http.Agent({ keepAlive: true, maxSockets: 1 });
  let connected = false;
  agent.createConnection = () => {
    if (connected) {
      throw new Error(
        'Daemon connection closed; this request will not reconnect to another instance.'
      );
    }
    connected = true;
    validateEndpoint(runtime);
    return net.createConnection({ path: runtime.endpoint.path });
  };
  try {
    await new Promise<void>((resolve, reject) => {
      const request = http.request(
        {
          socketPath: runtime.endpoint.path,
          path: '/daemon/identity',
          agent,
          headers: { 'X-Secret-Key': runtime.api_secret },
        },
        (response) => {
          let body = '';
          response.on('data', (chunk: Buffer) => {
            body += chunk.toString();
            if (body.length > 16384)
              response.destroy(new Error('Daemon identity exceeds its size limit.'));
          });
          response.on('error', reject);
          response.on('end', () => {
            try {
              const identity = JSON.parse(body);
              if (
                response.statusCode !== 200 ||
                identity.version !== 1 ||
                identity.profile_id !== runtime.profile_id ||
                identity.instance_id !== runtime.instance_id ||
                identity.pid !== runtime.pid ||
                identity.user_action_installed !== runtime.user_action_installed
              )
                throw new Error('Daemon instance identity changed; reconnect explicitly.');
              resolve();
            } catch (error) {
              reject(error);
            }
          });
        }
      );
      request.setTimeout(5000, () =>
        request.destroy(new Error('Daemon identity verification timed out.'))
      );
      request.on('error', reject);
      request.end();
    });
    return agent;
  } catch (error) {
    agent.destroy();
    throw error;
  }
}

export async function verifyDaemonRuntime(runtime: DaemonRuntime): Promise<void> {
  const agent = await authenticatedAgent(runtime);
  agent.destroy();
}

function matches(value: string | string[] | undefined, expected: string): boolean {
  if (typeof value !== 'string') return false;
  const actual = Buffer.from(value);
  const secret = Buffer.from(expected);
  return actual.length === secret.length && timingSafeEqual(actual, secret);
}

export async function createDaemonProxy(
  runtime: DaemonRuntime,
  desktopSecret: string,
  desktopProof: string | undefined,
  daemonProof: string | undefined,
  rendererOrigin: string | undefined
): Promise<DaemonProxy> {
  const agents = new Set<http.Agent>();
  const sockets = new Set<net.Socket>();
  const workspaceUrl = (request: http.IncomingMessage) => {
    try {
      const url = new URL(request.url || '/', 'http://localhost');
      return url.pathname === '/ui/workspace' ? url : undefined;
    } catch {
      return undefined;
    }
  };
  const authorized = (request: http.IncomingMessage, upgrade = false) => {
    if (
      request.socket.remoteAddress !== '127.0.0.1' ||
      (request.headers.origin !== undefined && request.headers.origin !== rendererOrigin)
    )
      return false;
    if (matches(request.headers['x-secret-key'], desktopSecret)) return true;
    const url = upgrade ? workspaceUrl(request) : undefined;
    return (
      url?.searchParams.getAll('secret').length === 1 &&
      matches(url.searchParams.get('secret') ?? undefined, desktopSecret)
    );
  };
  const upstreamPath = (request: http.IncomingMessage) => {
    const url = workspaceUrl(request);
    if (!url?.searchParams.has('secret')) return request.url;
    url.searchParams.set('secret', runtime.api_secret);
    return `${url.pathname}${url.search}`;
  };
  const headersFor = (request: http.IncomingMessage): http.OutgoingHttpHeaders => {
    const headers: http.OutgoingHttpHeaders = {
      ...request.headers,
      host: 'localhost',
      'x-secret-key': runtime.api_secret,
    };
    delete headers.origin;
    delete headers['x-user-action'];
    if (desktopProof && daemonProof && matches(request.headers['x-user-action'], desktopProof))
      headers['x-user-action'] = daemonProof;
    return headers;
  };
  const connect = async () => {
    const agent = await authenticatedAgent(runtime);
    agents.add(agent);
    return agent;
  };
  const release = (agent: http.Agent) => {
    agents.delete(agent);
    agent.destroy();
  };
  const server = http.createServer(async (request, response) => {
    const cors: http.OutgoingHttpHeaders =
      request.headers.origin && request.headers.origin === rendererOrigin
        ? {
            'Access-Control-Allow-Origin': rendererOrigin,
            'Access-Control-Allow-Credentials': 'true',
            Vary: 'Origin',
          }
        : {};
    for (const [name, value] of Object.entries(cors))
      if (value !== undefined) response.setHeader(name, value);
    if (
      request.method === 'OPTIONS' &&
      request.headers.origin === rendererOrigin &&
      rendererOrigin &&
      request.socket.remoteAddress === '127.0.0.1'
    ) {
      response
        .writeHead(204, {
          ...cors,
          'Access-Control-Allow-Methods': 'GET, HEAD, POST, PUT, PATCH, DELETE, OPTIONS',
          'Access-Control-Allow-Headers':
            request.headers['access-control-request-headers'] ||
            'Content-Type, X-Secret-Key, X-User-Action',
        })
        .end();
      return;
    }
    if (!authorized(request)) {
      response.writeHead(403).end();
      return;
    }
    let agent: http.Agent | undefined;
    try {
      agent = await connect();
      if (request.destroyed) {
        release(agent);
        return;
      }
      const owned = agent;
      const upstream = http.request(
        {
          socketPath: runtime.endpoint.path,
          method: request.method,
          path: upstreamPath(request),
          headers: headersFor(request),
          agent,
        },
        (remote) => {
          response.writeHead(remote.statusCode ?? 502, { ...remote.headers, ...cors });
          remote.pipe(response);
          remote.on('error', () => response.destroy());
          remote.on('end', () => release(owned));
        }
      );
      upstream.on('error', () => {
        release(owned);
        if (!response.headersSent)
          response.writeHead(502).end('Daemon connection unavailable; reconnect explicitly.');
        else response.destroy();
      });
      response.on('close', () => {
        upstream.destroy();
        release(owned);
      });
      request.pipe(upstream);
    } catch {
      if (agent) release(agent);
      response.writeHead(502).end('Daemon identity could not be verified; reconnect explicitly.');
    }
  });
  server.on('connection', (socket) => {
    sockets.add(socket);
    socket.on('error', () => socket.destroy());
    socket.on('close', () => sockets.delete(socket));
  });
  server.on('upgrade', async (request, socket, head) => {
    if (!authorized(request, true)) {
      socket.destroy();
      return;
    }
    let agent: http.Agent | undefined;
    try {
      agent = await connect();
      if (socket.destroyed) {
        release(agent);
        return;
      }
      const owned = agent;
      const upstream = http.request({
        socketPath: runtime.endpoint.path,
        method: request.method,
        path: upstreamPath(request),
        headers: headersFor(request),
        agent,
      });
      upstream.on('upgrade', (response, remote, remainder) => {
        const lines = response.rawHeaders.reduce<string[]>(
          (all, value, index, values) =>
            index % 2 === 0 ? [...all, `${value}: ${values[index + 1]}`] : all,
          []
        );
        socket.write(`HTTP/1.1 101 Switching Protocols\r\n${lines.join('\r\n')}\r\n\r\n`);
        if (head.length) remote.write(head);
        if (remainder.length) socket.write(remainder);
        remote.pipe(socket);
        socket.pipe(remote);
        remote.on('error', () => socket.destroy());
        socket.on('close', () => {
          remote.destroy();
          release(owned);
        });
        remote.on('close', () => {
          socket.destroy();
          release(owned);
        });
      });
      upstream.on('response', (response) => {
        response.resume();
        socket.destroy();
        release(owned);
      });
      upstream.on('error', () => {
        socket.destroy();
        release(owned);
      });
      socket.on('close', () => upstream.destroy());
      upstream.end();
    } catch {
      if (agent) release(agent);
      socket.destroy();
    }
  });
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  const address = server.address() as net.AddressInfo;
  return {
    baseUrl: `http://127.0.0.1:${address.port}`,
    close: () => {
      daemonProof = undefined;
      desktopProof = undefined;
      for (const agent of agents) agent.destroy();
      agents.clear();
      for (const socket of sockets) socket.destroy();
      sockets.clear();
      server.close();
    },
  };
}
