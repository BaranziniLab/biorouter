import net from 'node:net';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { startEmbeddedBrowserProxy, stopEmbeddedBrowserProxy } from './embeddedBrowserProxy';
import {
  clearLocalGrantsForTests,
  grantLocalDestination,
  isReservedLocalPort,
  reserveLocalPort,
  wasRecentlyRefused,
} from '../browser/localNetwork';

afterEach(async () => {
  vi.restoreAllMocks();
  clearLocalGrantsForTests();
  await stopEmbeddedBrowserProxy();
});

/** A SOCKS5 CONNECT to 127.0.0.1:`port`; resolves with the reply's status byte. */
async function connectThroughProxy(proxyPort: number, port: number): Promise<number> {
  const socket = await connect(proxyPort);
  socket.write(Buffer.from([5, 1, 0]));
  await nextChunk(socket);
  socket.write(Buffer.from([5, 1, 0, 1, 127, 0, 0, 1, port >> 8, port & 0xff]));
  const response = await nextChunk(socket);
  socket.destroy();
  return response[1];
}

async function localServer(): Promise<{ port: number; close: () => Promise<void> }> {
  const server = net.createServer((socket) => socket.end());
  await new Promise<void>((resolve) => server.listen(0, '127.0.0.1', resolve));
  const address = server.address() as net.AddressInfo;
  return {
    port: address.port,
    close: () => new Promise<void>((resolve) => server.close(() => resolve())),
  };
}

function connect(port: number): Promise<net.Socket> {
  return new Promise((resolve, reject) => {
    const socket = net.createConnection({ host: '127.0.0.1', port });
    socket.once('connect', () => resolve(socket));
    socket.once('error', reject);
  });
}

function nextChunk(socket: net.Socket): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    socket.once('data', resolve);
    socket.once('error', reject);
  });
}

describe('embedded browser pinned-IP proxy', () => {
  it('rejects a direct loopback target', async () => {
    const port = await startEmbeddedBrowserProxy();
    const socket = await connect(port);
    socket.write(Buffer.from([5, 1, 0]));
    await expect(nextChunk(socket)).resolves.toEqual(Buffer.from([5, 0]));
    socket.write(Buffer.from([5, 1, 0, 1, 127, 0, 0, 1, 0, 80]));
    const response = await nextChunk(socket);
    expect(response[0]).toBe(5);
    expect(response[1]).not.toBe(0);
    socket.destroy();
  });

  it('rejects an expanded IPv6 loopback target', async () => {
    const port = await startEmbeddedBrowserProxy();
    const socket = await connect(port);
    socket.write(Buffer.from([5, 1, 0]));
    await expect(nextChunk(socket)).resolves.toEqual(Buffer.from([5, 0]));
    socket.write(Buffer.from([5, 1, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 80]));
    const response = await nextChunk(socket);
    expect(response[0]).toBe(5);
    expect(response[1]).not.toBe(0);
    socket.destroy();
  });

  it('bounds and destroys a pending upstream when its client closes', async () => {
    const port = await startEmbeddedBrowserProxy();
    const socket = await connect(port);
    socket.write(Buffer.from([5, 1, 0]));
    await expect(nextChunk(socket)).resolves.toEqual(Buffer.from([5, 0]));

    const upstream = new net.Socket();
    const destroyUpstream = vi.spyOn(upstream, 'destroy');
    const createConnection = vi.spyOn(net, 'createConnection').mockReturnValueOnce(upstream);
    socket.write(Buffer.from([5, 1, 0, 1, 8, 8, 8, 8, 0, 80]));
    await vi.waitFor(() => expect(createConnection).toHaveBeenCalledOnce());
    expect(upstream.timeout).toBe(15_000);

    const upstreamClosed = new Promise<void>((resolve) => upstream.once('close', () => resolve()));
    socket.destroy();
    await upstreamClosed;
    expect(upstream.destroyed).toBe(true);
    await stopEmbeddedBrowserProxy();
    expect(destroyUpstream).toHaveBeenCalledOnce();
  });

  it('connects a loopback target only while the request filter has granted it (BB-06)', async () => {
    const proxyPort = await startEmbeddedBrowserProxy();
    const target = await localServer();
    try {
      expect(await connectThroughProxy(proxyPort, target.port)).not.toBe(0);
      expect(wasRecentlyRefused('127.0.0.1', target.port)).toBe(true);
      grantLocalDestination('127.0.0.1', target.port);
      expect(await connectThroughProxy(proxyPort, target.port)).toBe(0);
    } finally {
      await target.close();
    }
  });

  it('never connects a reserved port, its own included, even with a grant', async () => {
    const proxyPort = await startEmbeddedBrowserProxy();
    expect(isReservedLocalPort(proxyPort)).toBe(true);
    const target = await localServer();
    try {
      reserveLocalPort(target.port);
      grantLocalDestination('127.0.0.1', target.port);
      expect(await connectThroughProxy(proxyPort, target.port)).not.toBe(0);
      grantLocalDestination('127.0.0.1', proxyPort);
      expect(await connectThroughProxy(proxyPort, proxyPort)).not.toBe(0);
    } finally {
      await target.close();
    }
  });

  it('closes the listener and active clients on teardown', async () => {
    const port = await startEmbeddedBrowserProxy();
    const socket = await connect(port);
    const closed = new Promise<void>((resolve) => socket.once('close', () => resolve()));
    await stopEmbeddedBrowserProxy();
    await closed;
    expect(socket.destroyed).toBe(true);
    await expect(connect(port)).rejects.toBeDefined();
  });
});
