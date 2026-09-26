import { beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../api/client.gen';
import { observeCrew, type CrewObservation } from './crewApi';

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

function stateFrame(overrides: Record<string, unknown> = {}): string {
  return JSON.stringify({
    type: 'state',
    connection_id: 'connection-1',
    connection_mode: 'private',
    connection_policy_epoch: 2,
    connection_institution_id: null,
    snapshot: {
      actor: { id: 'actor-1', uid: 1, username: 'alice' },
      workspace: { id: 'workspace-1', host_uid: 1, mode: 'private', policy_epoch: 3 },
      principals: [],
      invitations: [],
      runs: [],
      channels: [],
      teams: [],
    },
    runs: [],
    ...overrides,
  });
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

  it('accepts a null institution but rejects missing, blank, newline, and overlong ids', async () => {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(
          responseFromChunks([encoder.encode(`${stateFrame()}\n${reconnectFrame()}\n`)])
        )
    );
    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, vi.fn())
    ).resolves.toBe('reconnect');

    for (const connection_institution_id of [undefined, '', 'ucsf\n', 'a'.repeat(65)]) {
      vi.stubGlobal(
        'fetch',
        vi
          .fn()
          .mockResolvedValue(
            responseFromChunks([encoder.encode(`${stateFrame({ connection_institution_id })}\n`)])
          )
      );
      await expect(
        observeCrew('connection-1', 'channel-1', null, new AbortController().signal, vi.fn())
      ).rejects.toThrow('invalid Crew observation');
    }
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

describe('the display projections beside a messages frame', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({ baseUrl: 'http://crew-observer.test', headers: {} });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('observer-secret') },
    });
  });

  async function observedMessages(line: string) {
    vi.stubGlobal(
      'fetch',
      vi
        .fn()
        .mockResolvedValue(responseFromChunks([encoder.encode(`${line}\n${reconnectFrame()}\n`)]))
    );
    const received: CrewObservation[] = [];
    await expect(
      observeCrew('connection-1', 'channel-1', null, new AbortController().signal, (frame) =>
        received.push(frame)
      )
    ).resolves.toBe('reconnect');
    const first = received[0];
    if (first?.type !== 'messages') throw new Error('expected a messages frame');
    return first;
  }

  const withFields = (fields: Record<string, unknown>) =>
    JSON.stringify({ ...JSON.parse(messageFrame()), ...fields });

  it('passes the backlog count, the page size and the names through', async () => {
    const frame = await observedMessages(
      withFields({
        remaining: 0,
        page_size: 100,
        people: { 'person-dan': { username: 'dan', display_name: 'Dan Wu', active: false } },
        channel_names: { 'channel-1': 'general' },
      })
    );
    expect(frame.remaining).toBe(0);
    expect(frame.page_size).toBe(100);
    expect({ ...frame.people }).toEqual({
      'person-dan': { username: 'dan', display_name: 'Dan Wu', active: false },
    });
    expect({ ...frame.channel_names }).toEqual({ 'channel-1': 'general' });
  });

  it('leaves out what an older daemon does not send', async () => {
    const frame = await observedMessages(messageFrame());
    for (const key of ['remaining', 'page_size', 'people', 'channel_names'])
      expect(frame).not.toHaveProperty(key);
  });

  it('drops a malformed count, page size or name and keeps the frame', async () => {
    for (const [remaining, pageSize] of [
      [-1, 0],
      [1.5, -3],
      ['2', '200'],
      [null, null],
    ]) {
      const frame = await observedMessages(
        withFields({ remaining, page_size: pageSize, people: ['dan'], channel_names: 'general' })
      );
      expect(frame.messages).toHaveLength(1);
      for (const key of ['remaining', 'page_size', 'people', 'channel_names'])
        expect(frame).not.toHaveProperty(key);
    }
    const frame = await observedMessages(
      withFields({
        people: {
          good: { username: 'erin', display_name: 7, active: 'no', uid: 1002 },
          'no-username': { display_name: 'Nameless' },
          blank: { username: '  ' },
          scalar: 'mallory',
        },
        channel_names: { 'channel-1': 'general', 'channel-2': 7, 'channel-3': '' },
      })
    );
    expect({ ...frame.people }).toEqual({ good: { username: 'erin' } });
    expect({ ...frame.channel_names }).toEqual({ 'channel-1': 'general' });
  });

  it('never lets a "__proto__" or "constructor" name reach a prototype', async () => {
    const line = messageFrame().replace(
      /}$/,
      ',"people":{"__proto__":{"username":"mallory"},"constructor":{"username":"eve"},' +
        '"person-1":{"username":"alice"}},"channel_names":{"__proto__":"polluted"}}'
    );
    const frame = await observedMessages(line);
    const people = frame.people!;
    expect(Object.getPrototypeOf(people)).toBeNull();
    expect(Object.keys(people)).toEqual(['person-1']);
    expect(Object.keys(frame.channel_names!)).toEqual([]);
    expect(({} as Record<string, unknown>).username).toBeUndefined();
    expect(({} as Record<string, unknown>).polluted).toBeUndefined();
  });
});
