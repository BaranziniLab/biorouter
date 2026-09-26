// @vitest-environment node
import http from 'node:http';
import { afterEach, describe, expect, it } from 'vitest';
import WebSocket, { WebSocketServer } from 'ws';
import { createCrewDaemonTerminal } from './crewDaemonTerminal';

type RequestRecord = {
  method: string;
  path: string;
  headers: http.IncomingHttpHeaders;
  body: string;
};

const readRequest = async (request: http.IncomingMessage): Promise<string> => {
  let body = '';
  for await (const chunk of request) body += chunk.toString();
  return body;
};

type FetchInput = Parameters<typeof fetch>[0];

const localFetch = async (input: FetchInput, init?: RequestInit): Promise<Response> => {
  const target =
    typeof input === 'string' ? new URL(input) : input instanceof URL ? input : new URL(input.url);
  const body = typeof init?.body === 'string' ? init.body : undefined;
  const result = await new Promise<{ status: number; body: string }>((resolve, reject) => {
    const request = http.request(
      target,
      {
        method: init?.method,
        headers: init?.headers as Record<string, string> | undefined,
      },
      (response) => {
        let responseBody = '';
        response.setEncoding('utf8');
        response.on('data', (chunk) => (responseBody += chunk));
        response.on('end', () => resolve({ status: response.statusCode ?? 0, body: responseBody }));
      }
    );
    request.once('error', reject);
    request.end(body);
  });
  return {
    ok: result.status >= 200 && result.status < 300,
    status: result.status,
    json: async () => JSON.parse(result.body),
  } as Response;
};

describe('Crew daemon terminal adapter', () => {
  const originalFetch = globalThis.fetch;
  let server: http.Server | undefined;
  let websocketServer: WebSocketServer | undefined;

  afterEach(async () => {
    globalThis.fetch = originalFetch;
    websocketServer?.clients.forEach((client) => client.terminate());
    websocketServer?.close();
    if (server?.listening) await new Promise<void>((resolve) => server?.close(() => resolve()));
    websocketServer = undefined;
    server = undefined;
  });

  it('authenticates, streams split UTF-8 output, clamps resize, and disposes through DELETE', async () => {
    const requests: RequestRecord[] = [];
    const frames: Record<string, unknown>[] = [];
    let client: WebSocket | undefined;
    server = http.createServer(async (request, response) => {
      const body = await readRequest(request);
      requests.push({
        method: request.method ?? '',
        path: request.url ?? '',
        headers: request.headers,
        body,
      });
      if (request.method === 'POST') {
        response.writeHead(201, { 'content-type': 'application/json' });
        response.end(JSON.stringify({ authentication_id: 'auth-1', connection_id: 'conn-1' }));
        return;
      }
      response.writeHead(204).end();
    });
    websocketServer = new WebSocketServer({ noServer: true });
    server.on('upgrade', (request, socket, head) => {
      websocketServer?.handleUpgrade(request, socket, head, (upstream) => {
        client = upstream;
        upstream.on('message', (data) => {
          const frame = JSON.parse(data.toString()) as Record<string, unknown>;
          frames.push(frame);
          if (frame.type === 'resize') {
            upstream.send(Buffer.from('hello '), { binary: true });
            upstream.send(Buffer.from('🌎'), { binary: true });
            upstream.send(JSON.stringify({ type: 'exit', authenticated: true, exit_code: 7 }));
            setTimeout(() => upstream.close(), 10);
          }
        });
      });
    });
    await new Promise<void>((resolve) => server?.listen(0, '127.0.0.1', () => resolve()));
    const address = server.address() as { port: number };
    globalThis.fetch = localFetch;
    const output: string[] = [];
    const exit = new Promise<number | null>((resolve) => {
      void createCrewDaemonTerminal(
        {
          baseUrl: `http://127.0.0.1:${address.port}`,
          secret: 'desktop-secret',
          userAction: 'approval-proof',
        },
        'conn-1',
        (data) => output.push(data),
        resolve
      ).then(async (terminal) => {
        terminal.write('typed input');
        terminal.resize(3, 999);
        await exit;
        terminal.dispose();
      });
    });

    expect(await exit).toBe(0);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(output.join('')).toBe('hello 🌎');
    expect(frames).toEqual([
      { type: 'input', data: 'typed input' },
      { type: 'resize', cols: 20, rows: 200 },
    ]);
    expect(client?.readyState).toBe(WebSocket.CLOSED);
    expect(requests).toHaveLength(2);
    expect(requests[0]).toMatchObject({
      method: 'POST',
      path: '/crew/connections/conn-1/authentication',
      headers: {
        'x-secret-key': 'desktop-secret',
        'x-user-action': 'approval-proof',
      },
    });
    expect(JSON.parse(requests[0].body)).toMatchObject({
      controller_id: requests[0].headers['x-crew-controller'],
      cols: 80,
      rows: 12,
    });
    expect(requests[1]).toMatchObject({
      method: 'DELETE',
      path: '/crew/authentication/auth-1',
      headers: {
        'x-secret-key': 'desktop-secret',
        'x-user-action': 'approval-proof',
      },
    });
  });

  it('refuses a daemon session whose response belongs to another connection before opening a WebSocket', async () => {
    let upgrades = 0;
    server = http.createServer((request, response) => {
      if (request.method === 'POST') {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end(
          JSON.stringify({ authentication_id: 'auth-2', connection_id: 'other-connection' })
        );
      }
    });
    websocketServer = new WebSocketServer({ noServer: true });
    server.on('upgrade', () => {
      upgrades += 1;
    });
    await new Promise<void>((resolve) => server?.listen(0, '127.0.0.1', () => resolve()));
    const address = server.address() as { port: number };
    globalThis.fetch = localFetch;
    await expect(
      createCrewDaemonTerminal(
        { baseUrl: `http://127.0.0.1:${address.port}`, secret: 'secret', userAction: 'proof' },
        'conn-1',
        () => {},
        () => {}
      )
    ).rejects.toThrow('Invalid daemon authentication session.');
    expect(upgrades).toBe(0);
  });

  it('does not promote an unauthenticated zero exit into a successful connection', async () => {
    server = http.createServer((request, response) => {
      if (request.method === 'POST') {
        response.writeHead(200, { 'content-type': 'application/json' });
        response.end(JSON.stringify({ authentication_id: 'auth-3', connection_id: 'conn-1' }));
      } else response.writeHead(204).end();
    });
    websocketServer = new WebSocketServer({ noServer: true });
    server.on('upgrade', (request, socket, head) => {
      websocketServer?.handleUpgrade(request, socket, head, (upstream) => {
        setTimeout(() => {
          upstream.send(JSON.stringify({ type: 'exit', authenticated: false, exit_code: 0 }));
          upstream.close();
        }, 20);
      });
    });
    await new Promise<void>((resolve) => server?.listen(0, '127.0.0.1', () => resolve()));
    const address = server.address() as { port: number };
    globalThis.fetch = localFetch;
    let resolveExit!: (code: number | null) => void;
    const exit = new Promise<number | null>((resolve) => {
      resolveExit = resolve;
    });
    const terminal = await createCrewDaemonTerminal(
      { baseUrl: `http://127.0.0.1:${address.port}`, secret: 'secret', userAction: 'proof' },
      'conn-1',
      () => {},
      resolveExit
    );
    expect(await exit).toBeNull();
    terminal.dispose();
  });
});
