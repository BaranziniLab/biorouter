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

/**
 * How the attached daemon instance was lost. `replaced`: something answers on the profile's socket
 * but it is not the instance this app verified (a restart always brings a new instance id and pid,
 * and usually a new secret), so the old one cannot come back. `gone`: nothing answers on the socket
 * (it was removed, or nothing listens on it); the same instance could still be restarting.
 */
export type DaemonLossReason = 'replaced' | 'gone';

export interface DaemonLoss {
  reason: DaemonLossReason;
  /** The instance that was lost. */
  instanceId: string;
}

export interface DaemonProxy {
  baseUrl: string;
  close: () => void;
  /** The instance every request is checked against now. */
  instanceId(): string;
  /**
   * Called each time a request finds the attached instance lost, and once when a `gone` instance
   * answers again. Returns an unsubscribe.
   */
  onConnection(listener: (event: DaemonConnectionEvent) => void): () => void;
  /** Check the attached instance now: the loss, or `undefined` when it answers as itself. */
  probe(): Promise<DaemonLoss | undefined>;
  /**
   * Point this proxy, at the same local address, at another instance of the same profile's
   * daemon. Only after a person agreed to it: the proxy never follows a different instance on its
   * own. The new instance is verified first (its identity on the private socket, and that it
   * accepts `daemonProof` as a person's approval); if either check fails nothing changes.
   */
  retarget(runtime: DaemonRuntime, daemonProof: string | undefined): Promise<void>;
}

export type DaemonConnectionEvent =
  | ({ kind: 'lost' } & DaemonLoss)
  | { kind: 'answered'; instanceId: string };

/**
 * The attached instance answered as a different one: its identity, or its refusal of the secret
 * this app holds for it. Never followed; a person decides whether to reattach.
 */
export class DaemonIdentityChangedError extends Error {
  constructor() {
    super('Daemon instance identity changed; reconnect explicitly.');
    this.name = 'DaemonIdentityChangedError';
  }
}

/** The loss a failed verification means, or `undefined` for a failure that says nothing about it. */
export function daemonLossOf(error: unknown): DaemonLossReason | undefined {
  if (error instanceof DaemonIdentityChangedError) return 'replaced';
  const failure = error as { code?: unknown; syscall?: unknown } | null;
  if (
    failure &&
    failure.syscall === 'connect' &&
    (failure.code === 'ENOENT' || failure.code === 'ECONNREFUSED')
  )
    return 'gone';
  return undefined;
}

/** The status a proxied request fails with when the attached instance is lost. */
export const DAEMON_RESTARTED_CODE = 'daemon_restarted';
/** What a person reads for it, wherever a surface shows the daemon's own words. */
export const DAEMON_RESTARTED_MESSAGE =
  "Biorouter's background service restarted. Reconnect when Biorouter asks, or quit and reopen Biorouter.";
/** The status a proxied request fails with for any other verification failure. */
export const DAEMON_UNAVAILABLE_CODE = 'daemon_unavailable';
export const DAEMON_UNAVAILABLE_MESSAGE =
  "Biorouter couldn't reach its background service. Try again in a moment, or quit and reopen Biorouter.";

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
            // A daemon that refuses this instance's secret is another instance: the secret is
            // per instance, and the socket path is the profile's, not the instance's.
            if (response.statusCode === 401 || response.statusCode === 403) {
              reject(new DaemonIdentityChangedError());
              return;
            }
            let identity: Record<string, unknown> | null = null;
            try {
              identity = JSON.parse(body);
            } catch {
              identity = null;
            }
            if (response.statusCode !== 200 || !identity || typeof identity !== 'object') {
              reject(new Error('Daemon identity could not be read.'));
              return;
            }
            if (
              identity.version !== 1 ||
              identity.profile_id !== runtime.profile_id ||
              identity.instance_id !== runtime.instance_id ||
              identity.pid !== runtime.pid ||
              identity.user_action_installed !== runtime.user_action_installed
            ) {
              reject(new DaemonIdentityChangedError());
              return;
            }
            resolve();
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

/**
 * Whether the verified instance accepts `daemonProof` as a person's approval: a person-gated
 * route (`GET /crew/connections`) asked over the same verified connection, with the instance's
 * own secret. What the desktop's first attach asks through its proxy, asked before a proxy is
 * pointed at the instance.
 */
export async function verifyHumanAuthorizedAccess(
  runtime: DaemonRuntime,
  daemonProof: string | undefined
): Promise<void> {
  const agent = await authenticatedAgent(runtime);
  try {
    const status = await new Promise<number>((resolve, reject) => {
      const request = http.request(
        {
          socketPath: runtime.endpoint.path,
          path: '/crew/connections',
          agent,
          headers: {
            'X-Secret-Key': runtime.api_secret,
            ...(daemonProof ? { 'X-User-Action': daemonProof } : {}),
          },
        },
        (response) => {
          response.resume();
          response.on('error', reject);
          response.on('end', () => resolve(response.statusCode ?? 0));
        }
      );
      request.setTimeout(10000, () =>
        request.destroy(new Error('Daemon approval check timed out.'))
      );
      request.on('error', reject);
      request.end();
    });
    if (status < 200 || status >= 300)
      throw new Error(
        'The background service did not accept that approval secret. Check it, then reconnect again.'
      );
  } finally {
    agent.destroy();
  }
}

function matches(value: string | string[] | undefined, expected: string): boolean {
  if (typeof value !== 'string') return false;
  const actual = Buffer.from(value);
  const secret = Buffer.from(expected);
  return actual.length === secret.length && timingSafeEqual(actual, secret);
}

/**
 * The desktop's local proxy to the profile's shared daemon, on an ephemeral loopback port.
 *
 * Every request is checked against the ONE instance this proxy is attached to: its identity on the
 * private socket is verified on the same connection the request then travels, so a request never
 * reaches a different instance. When that instance is lost (restarted, stopped, replaced), requests
 * fail with `daemon_restarted` and listeners hear `lost`; the proxy never follows the new instance
 * by itself. {@link DaemonProxy.retarget} is how a person's decision to reattach is carried out,
 * at the same local address, so every window's `BIOROUTER_API_HOST` stays valid (R-1).
 */
export async function createDaemonProxy(
  runtime: DaemonRuntime,
  desktopSecret: string,
  desktopProof: string | undefined,
  daemonProof: string | undefined,
  rendererOrigin: string | undefined
): Promise<DaemonProxy> {
  interface Target {
    runtime: DaemonRuntime;
    daemonProof: string | undefined;
    /** Answered as another instance: it cannot come back, so nothing is asked of it again. */
    replaced: boolean;
    /** Nothing answered on the socket last time; the next answer is reported. */
    gone: boolean;
    agents: Set<http.Agent>;
  }
  const targetFor = (next: DaemonRuntime, proof: string | undefined): Target => ({
    runtime: next,
    daemonProof: proof,
    replaced: false,
    gone: false,
    agents: new Set(),
  });
  let target = targetFor(runtime, daemonProof);
  let closed = false;
  const listeners = new Set<(event: DaemonConnectionEvent) => void>();
  const emit = (event: DaemonConnectionEvent) => {
    for (const listener of [...listeners]) {
      try {
        listener(event);
      } catch {
        // A listener's failure is its own; the proxy keeps answering.
      }
    }
  };
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
  const upstreamPath = (request: http.IncomingMessage, current: Target) => {
    const url = workspaceUrl(request);
    if (!url?.searchParams.has('secret')) return request.url;
    url.searchParams.set('secret', current.runtime.api_secret);
    return `${url.pathname}${url.search}`;
  };
  const headersFor = (request: http.IncomingMessage, current: Target): http.OutgoingHttpHeaders => {
    const headers: http.OutgoingHttpHeaders = {
      ...request.headers,
      host: 'localhost',
      'x-secret-key': current.runtime.api_secret,
    };
    delete headers.origin;
    delete headers['x-user-action'];
    if (
      desktopProof &&
      current.daemonProof &&
      matches(request.headers['x-user-action'], desktopProof)
    )
      headers['x-user-action'] = current.daemonProof;
    return headers;
  };
  /**
   * A connection to `current`, verified as that instance. A request whose target was replaced
   * while it verified is refused rather than sent on: it was checked against the old instance.
   */
  const connect = async (current: Target) => {
    if (closed || current.replaced) throw new DaemonIdentityChangedError();
    let agent: http.Agent;
    try {
      agent = await authenticatedAgent(current.runtime);
    } catch (error) {
      const reason = daemonLossOf(error);
      // Reported once per change: every later request fails the same way without a new report.
      if (
        reason &&
        current === target &&
        !closed &&
        !current.replaced &&
        !(reason === 'gone' && current.gone)
      ) {
        if (reason === 'replaced') current.replaced = true;
        else current.gone = true;
        emit({ kind: 'lost', reason, instanceId: current.runtime.instance_id });
      }
      throw error;
    }
    if (closed || current !== target) {
      agent.destroy();
      throw new DaemonIdentityChangedError();
    }
    current.agents.add(agent);
    if (current.gone) {
      current.gone = false;
      emit({ kind: 'answered', instanceId: current.runtime.instance_id });
    }
    return agent;
  };
  const release = (agent: http.Agent, current: Target) => {
    current.agents.delete(agent);
    agent.destroy();
  };
  /** The JSON refusal for a request that did not reach the daemon. */
  const refuse = (response: http.ServerResponse, error: unknown) => {
    const lost = error === undefined ? false : daemonLossOf(error) !== undefined;
    const message = lost ? DAEMON_RESTARTED_MESSAGE : DAEMON_UNAVAILABLE_MESSAGE;
    response.writeHead(502, { 'content-type': 'application/json' }).end(
      JSON.stringify({
        code: lost ? DAEMON_RESTARTED_CODE : DAEMON_UNAVAILABLE_CODE,
        error: message,
        message,
      })
    );
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
    // One instance for the whole request: verified, addressed and authenticated as the same one.
    const current = target;
    let agent: http.Agent | undefined;
    try {
      agent = await connect(current);
      if (request.destroyed) {
        release(agent, current);
        return;
      }
      const owned = agent;
      const upstream = http.request(
        {
          socketPath: current.runtime.endpoint.path,
          method: request.method,
          path: upstreamPath(request, current),
          headers: headersFor(request, current),
          agent,
        },
        (remote) => {
          response.writeHead(remote.statusCode ?? 502, { ...remote.headers, ...cors });
          remote.pipe(response);
          remote.on('error', () => response.destroy());
          remote.on('end', () => release(owned, current));
        }
      );
      upstream.on('error', () => {
        release(owned, current);
        if (!response.headersSent) refuse(response, undefined);
        else response.destroy();
      });
      response.on('close', () => {
        upstream.destroy();
        release(owned, current);
      });
      request.pipe(upstream);
    } catch (error) {
      if (agent) release(agent, current);
      refuse(response, error);
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
    const current = target;
    let agent: http.Agent | undefined;
    try {
      agent = await connect(current);
      if (socket.destroyed) {
        release(agent, current);
        return;
      }
      const owned = agent;
      const upstream = http.request({
        socketPath: current.runtime.endpoint.path,
        method: request.method,
        path: upstreamPath(request, current),
        headers: headersFor(request, current),
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
          release(owned, current);
        });
        remote.on('close', () => {
          socket.destroy();
          release(owned, current);
        });
      });
      upstream.on('response', (response) => {
        response.resume();
        socket.destroy();
        release(owned, current);
      });
      upstream.on('error', () => {
        socket.destroy();
        release(owned, current);
      });
      socket.on('close', () => upstream.destroy());
      upstream.end();
    } catch {
      if (agent) release(agent, current);
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
    instanceId: () => target.runtime.instance_id,
    onConnection: (listener) => {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    probe: async () => {
      const current = target;
      try {
        release(await connect(current), current);
        return undefined;
      } catch (error) {
        const reason = daemonLossOf(error);
        return reason ? { reason, instanceId: current.runtime.instance_id } : undefined;
      }
    },
    retarget: async (next, proof) => {
      if (closed) throw new Error('The daemon proxy is closed.');
      if (next.profile_id !== target.runtime.profile_id)
        throw new Error('That background service belongs to another Biorouter profile.');
      // Verified before anything changes: the instance on the private socket, and that it takes
      // the secret the person gave as a person's approval.
      await verifyHumanAuthorizedAccess(next, proof);
      if (closed) throw new Error('The daemon proxy is closed.');
      const previous = target;
      target = targetFor(next, proof);
      previous.daemonProof = undefined;
      for (const agent of previous.agents) agent.destroy();
      previous.agents.clear();
      emit({ kind: 'answered', instanceId: next.instance_id });
    },
    close: () => {
      if (closed) return;
      closed = true;
      target.daemonProof = undefined;
      desktopProof = undefined;
      listeners.clear();
      for (const agent of target.agents) agent.destroy();
      target.agents.clear();
      for (const socket of sockets) socket.destroy();
      sockets.clear();
      server.close();
    },
  };
}
