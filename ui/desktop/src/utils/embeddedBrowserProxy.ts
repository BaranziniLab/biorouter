import dns from 'node:dns/promises';
import net, { isIP, type Server, type Socket } from 'node:net';
import {
  noteLocalRefusal,
  proxyMayConnect,
  reserveLocalPort,
  resolveHostClass,
} from '../browser/localNetwork';
import type { PublicNetworkTarget } from './embeddedBrowserPolicy';

const SOCKS_VERSION = 5;
const SOCKS_CONNECT = 1;
const SOCKS_NO_AUTH = 0;
const SOCKS_NO_ACCEPTABLE_AUTH = 0xff;
const SOCKS_SUCCEEDED = 0;
const SOCKS_GENERAL_FAILURE = 1;
const SOCKS_COMMAND_NOT_SUPPORTED = 7;
const SOCKS_ADDRESS_NOT_SUPPORTED = 8;
const MAX_HANDSHAKE_BYTES = 4096;
const SOCKET_TIMEOUT_MS = 15_000;

type ProxyRuntime = {
  server: Server;
  port: number;
  sockets: Set<Socket>;
  state: { stopped: boolean };
};

let runtimePromise: Promise<ProxyRuntime> | null = null;

function reply(socket: Socket, status: number): void {
  socket.write(Buffer.from([SOCKS_VERSION, status, 0, 1, 0, 0, 0, 0, 0, 0]));
}

function ipv6Address(bytes: Buffer): string {
  const groups: string[] = [];
  for (let offset = 0; offset < 16; offset += 2)
    groups.push(bytes.readUInt16BE(offset).toString(16));
  return groups.join(':');
}

function requestLength(buffer: Buffer): number | null {
  if (buffer.length < 4) return null;
  if (buffer[3] === 1) return 10;
  if (buffer[3] === 4) return 22;
  if (buffer[3] === 3) return buffer.length < 5 ? null : 7 + buffer[4];
  return -1;
}

function requestHost(buffer: Buffer): string | null {
  if (buffer[3] === 1) return `${buffer[4]}.${buffer[5]}.${buffer[6]}.${buffer[7]}`;
  if (buffer[3] === 4) return ipv6Address(buffer.subarray(4, 20));
  if (buffer[3] === 3) {
    const length = buffer[4];
    const host = buffer.subarray(5, 5 + length).toString('utf8');
    return Buffer.from(host, 'utf8').equals(buffer.subarray(5, 5 + length)) ? host : null;
  }
  return null;
}

/**
 * Resolve once and pin the socket to the answer, so a public name cannot be rebound to a local
 * address between the check and the connect (DNS rebinding).
 *
 * A local answer (private network or loopback) is connected only while the request filter has
 * granted that exact host and port (BB-06, `browser/localNetwork.ts`): the person opened it, or a
 * page on that same local host asked for it. A reserved loopback port (the daemon, the dev
 * renderer, the proxies themselves) is never connected.
 */
async function pinnedTarget(host: string, port: number): Promise<PublicNetworkTarget> {
  const resolved = await resolveHostClass(host, (candidate) =>
    dns.lookup(candidate, { all: true, verbatim: true })
  );
  if (!resolved || resolved.addresses.length === 0) {
    throw new Error('The embedded browser could not resolve this host.');
  }
  if (!proxyMayConnect(host, port, resolved.cls)) {
    if (resolved.cls !== 'public') noteLocalRefusal(host, port);
    throw new Error('Local addresses are not available to this page.');
  }
  const address = resolved.addresses[0];
  return { address, family: isIP(address) as 4 | 6 };
}

function handleClient(client: Socket, sockets: Set<Socket>, state: { stopped: boolean }): void {
  if (state.stopped) {
    client.destroy();
    return;
  }
  sockets.add(client);
  let upstreamForClient: Socket | null = null;
  client.on('close', () => {
    sockets.delete(client);
    upstreamForClient?.destroy();
    upstreamForClient = null;
  });
  client.setTimeout(SOCKET_TIMEOUT_MS, () => client.destroy());

  let buffer = Buffer.alloc(0);
  let phase: 'greeting' | 'request' | 'connecting' = 'greeting';

  const fail = (status = SOCKS_GENERAL_FAILURE) => {
    if (!client.destroyed) reply(client, status);
    client.end();
  };

  const process = () => {
    if (buffer.length > MAX_HANDSHAKE_BYTES) {
      fail();
      return;
    }

    if (phase === 'greeting') {
      if (buffer.length < 2) return;
      const methodsLength = buffer[1];
      const total = 2 + methodsLength;
      if (buffer.length < total) return;
      if (buffer[0] !== SOCKS_VERSION || !buffer.subarray(2, total).includes(SOCKS_NO_AUTH)) {
        client.write(Buffer.from([SOCKS_VERSION, SOCKS_NO_ACCEPTABLE_AUTH]));
        client.end();
        return;
      }
      buffer = buffer.subarray(total);
      phase = 'request';
      client.write(Buffer.from([SOCKS_VERSION, SOCKS_NO_AUTH]));
    }

    if (phase !== 'request') return;
    const total = requestLength(buffer);
    if (total === null) return;
    if (total < 0) {
      fail(SOCKS_ADDRESS_NOT_SUPPORTED);
      return;
    }
    if (buffer.length < total) return;
    if (buffer[0] !== SOCKS_VERSION || buffer[1] !== SOCKS_CONNECT || buffer[2] !== 0) {
      fail(SOCKS_COMMAND_NOT_SUPPORTED);
      return;
    }

    const host = requestHost(buffer);
    if (!host) {
      fail(SOCKS_ADDRESS_NOT_SUPPORTED);
      return;
    }
    const port = buffer.readUInt16BE(total - 2);
    if (port === 0) {
      fail();
      return;
    }
    buffer = buffer.subarray(total);
    phase = 'connecting';

    void pinnedTarget(host, port)
      .then((target) => {
        if (state.stopped || client.destroyed) throw new Error('Proxy stopped');
        return new Promise<Socket>((resolve, reject) => {
          const upstream = net.createConnection({
            host: target.address,
            family: target.family,
            port,
          });
          upstreamForClient = upstream;
          sockets.add(upstream);
          let connected = false;
          upstream.setTimeout(SOCKET_TIMEOUT_MS, () =>
            upstream.destroy(new Error('Embedded browser upstream connect timed out.'))
          );
          upstream.once('connect', () => {
            connected = true;
            upstream.setTimeout(0);
            resolve(upstream);
          });
          upstream.once('error', reject);
          upstream.once('close', () => {
            sockets.delete(upstream);
            if (upstreamForClient === upstream) upstreamForClient = null;
            if (!connected) reject(new Error('Embedded browser upstream closed before connect.'));
          });
        });
      })
      .then((upstream) => {
        if (client.destroyed) {
          upstream.destroy();
          return;
        }
        client.removeListener('data', onData);
        client.setTimeout(0);
        reply(client, SOCKS_SUCCEEDED);
        if (buffer.length > 0) upstream.write(buffer);
        client.pipe(upstream);
        upstream.pipe(client);
      })
      .catch(() => fail());
  };

  const onData = (chunk: Buffer) => {
    buffer = Buffer.concat([buffer, chunk]);
    process();
  };
  client.on('data', onData);
  client.on('error', () => client.destroy());
}

async function createRuntime(): Promise<ProxyRuntime> {
  const sockets = new Set<Socket>();
  const state = { stopped: false };
  const server = net.createServer((socket) => handleClient(socket, sockets, state));
  await new Promise<void>((resolve, reject) => {
    server.once('error', reject);
    server.listen({ host: '127.0.0.1', port: 0, exclusive: true }, () => {
      server.removeListener('error', reject);
      resolve();
    });
  });
  server.on('error', () => {
    state.stopped = true;
    for (const socket of sockets) socket.destroy();
    sockets.clear();
  });
  server.unref();
  const address = server.address();
  if (!address || typeof address === 'string') {
    server.close();
    throw new Error('Could not bind the embedded browser proxy.');
  }
  // A page must never reach the proxy itself as a destination.
  reserveLocalPort(address.port);
  return { server, port: address.port, sockets, state };
}

export async function startEmbeddedBrowserProxy(): Promise<number> {
  if (!runtimePromise) {
    const operation = createRuntime();
    runtimePromise = operation;
    void operation.catch(() => {
      if (runtimePromise === operation) runtimePromise = null;
    });
  }
  return (await runtimePromise).port;
}

export async function stopEmbeddedBrowserProxy(): Promise<void> {
  const operation = runtimePromise;
  runtimePromise = null;
  if (!operation) return;
  const runtime = await operation.catch(() => null);
  if (!runtime) return;
  runtime.state.stopped = true;
  for (const socket of runtime.sockets) socket.destroy();
  runtime.sockets.clear();
  await new Promise<void>((resolve) => runtime.server.close(() => resolve()));
}
