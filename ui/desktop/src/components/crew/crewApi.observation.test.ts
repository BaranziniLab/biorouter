import { beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../api/client.gen';
import { observeCrew } from './crewApi';

vi.mock('../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'observer-test-proof' }),
}));

const encoder = new TextEncoder();

function responseFromChunks(chunks: Uint8Array[]): Response {
  const body = new ReadableStream<Uint8Array>({
    start(controller) {
      chunks.forEach((chunk) => controller.enqueue(chunk));
      controller.close();
    },
  });
  return {
    ok: true,
    status: 200,
    headers: new Headers({ 'content-type': 'application/x-ndjson' }),
    body,
  } as unknown as Response;
}

function messageFrame(body = ''): string {
  return JSON.stringify({
    type: 'messages',
    channel_id: 'channel-1',
    messages: [{ id: 'message-1', sequence: 'sequence-1', channel_id: 'channel-1', body }],
    cursor: 'cursor-1',
    reset: false,
  });
}

function reconnectFrame(cursor = 'cursor-2'): string {
  return JSON.stringify({ type: 'reconnect', cursor });
}

describe('observeCrew NDJSON framing', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({ baseUrl: 'http://crew-observer.test', headers: {} });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('observer-secret') },
    });
  });

  it('joins incremental chunks, preserves typed frames, and returns the reconnect cursor', async () => {
    const line = `${messageFrame()}\n${reconnectFrame('cursor-typed')}\n`;
    const bytes = encoder.encode(line);
    const receivedChunks = [bytes.slice(0, 17), bytes.slice(17, 43), bytes.slice(43)];
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(responseFromChunks(receivedChunks)));
    const received: unknown[] = [];

    await expect(
      observeCrew(
        'connection-1',
        'channel-1',
        'cursor-before',
        new AbortController().signal,
        (frame) => received.push(frame)
      )
    ).resolves.toBe('reconnect');

    expect(received).toEqual([
      expect.objectContaining({ type: 'messages', cursor: 'cursor-1' }),
      { type: 'reconnect', cursor: 'cursor-typed' },
    ]);
    expect(fetch).toHaveBeenCalledWith(
      'http://crew-observer.test/crew/connections/connection-1/observe',
      expect.objectContaining({
        method: 'POST',
        body: JSON.stringify({
          channel_id: 'channel-1',
          after: 'cursor-before',
          initial: 'latest',
        }),
      })
    );
  });

  it('accepts a frame exactly at the 1 MiB limit when its newline is present', async () => {
    const limit = 1_048_576;
    const empty = messageFrame();
    const line = messageFrame('x'.repeat(limit - 1 - empty.length));
    expect(line).toHaveLength(limit - 1);
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(responseFromChunks([encoder.encode(`${line}\n${reconnectFrame()}\n`)]))
    );

    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, vi.fn())
    ).resolves.toBe('reconnect');
  });

  it('refuses a frame larger than 1 MiB before dispatching it', async () => {
    const limit = 1_048_576;
    const empty = messageFrame();
    const line = messageFrame('x'.repeat(limit - empty.length));
    expect(line).toHaveLength(limit);
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(responseFromChunks([encoder.encode(`${line}\n`)]))
    );

    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, vi.fn())
    ).rejects.toThrow('exceeds its frame limit');
  });

  it('rejects malformed JSON rather than turning a broken stream into a reconnect', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(responseFromChunks([encoder.encode('{not-json}\n')]))
    );

    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, vi.fn())
    ).rejects.toThrow();
  });

  it('rejects a truncated final frame that never supplied the required newline', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(responseFromChunks([encoder.encode(messageFrame())]))
    );
    const receive = vi.fn();

    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, receive)
    ).rejects.toThrow('incomplete frame');
    expect(receive).not.toHaveBeenCalled();
  });

  it('cancels the reader when the caller aborts an in-flight observation', async () => {
    let releaseRead!: () => void;
    const reader = {
      read: vi.fn(
        () =>
          new Promise<{ done: boolean; value?: Uint8Array }>((resolve) => {
            releaseRead = () => resolve({ done: true });
          })
      ),
      cancel: vi.fn().mockResolvedValue(undefined),
      releaseLock: vi.fn(),
    };
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        headers: new Headers({ 'content-type': 'application/x-ndjson' }),
        body: { getReader: () => reader },
      })
    );
    const controller = new AbortController();
    const pending = observeCrew('connection-1', 'channel-1', null, controller.signal, vi.fn());
    await vi.waitFor(() => expect(reader.read).toHaveBeenCalled());
    controller.abort();
    releaseRead();

    await expect(pending).resolves.toBe('terminal');
    expect(reader.cancel).toHaveBeenCalled();
    expect(reader.releaseLock).toHaveBeenCalled();
  });

  it('treats a typed stale policy error as terminal without opening a fresh retry', async () => {
    const fetchMock = vi
      .fn()
      .mockResolvedValue(
        responseFromChunks([
          encoder.encode(
            `${JSON.stringify({ type: 'error', clear: true, code: 'policy_changed', error: 'stale policy' })}\n`
          ),
        ])
      );
    vi.stubGlobal('fetch', fetchMock);

    const received: unknown[] = [];
    await expect(
      observeCrew(
        'connection-1',
        'channel-1',
        'cursor-old',
        new AbortController().signal,
        (frame) => received.push(frame)
      )
    ).resolves.toBe('terminal');
    expect(received).toEqual([
      { type: 'error', clear: true, code: 'policy_changed', error: 'stale policy' },
    ]);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
