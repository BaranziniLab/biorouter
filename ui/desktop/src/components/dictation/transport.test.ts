import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { dictationApi, resetDictationApiForTests, type DictationApi } from './dictationApi';
import {
  createDictationStream,
  DictationStreamError,
  HELD_READ_MS,
  MIN_CHUNK_BYTES,
  type DictationStream,
  type StreamRequest,
  type Transcript,
} from './transport';

const RATE = 16000;
const BASE = 'http://127.0.0.1:4567';
const last = <T>(items: readonly T[]): T | undefined => items[items.length - 1];

const TRANSCRIPT: Transcript = {
  text: 'EPAS1 is expressed',
  raw_text: 'a pass one is expressed',
  changes: [],
  segments: [{ t0: 0, t1: 2, text: 'EPAS1 is expressed' }],
  model: 'whisper-small.en-q8_0',
  audio_s: 2,
  elapsed_ms: 120,
  bias_scope: 'public',
  retried: null,
  no_speech: false,
};

type Reply = { status: number; body?: unknown; headers?: Record<string, string> };
type Req = {
  method: string;
  path: string;
  seq: number | null;
  waitMs: number | null;
  body: Uint8Array | null;
  json: unknown;
  headers: Record<string, string>;
  signal: AbortSignal | null | undefined;
};

/** A scripted daemon behind a fake `fetch`. Every reply is a plain object, not a real Response. */
class FakeDaemon {
  requests: Req[] = [];
  inflightPuts = 0;
  maxInflightPuts = 0;
  creates = 0;
  /** seq to bytes, per stream id. */
  streams = new Map<string, Map<number, Uint8Array>>();
  onCreate: (req: Req) => Reply | Promise<Reply> = () => ({
    status: 201,
    body: {
      stream_id: `s${this.creates}`,
      max_chunk_bytes: 524288,
      idle_timeout_s: 60,
      max_seconds: 600,
    },
  });
  onPut: (req: Req, id: string) => Reply | Promise<Reply> = (req, id) => this.accept(req, id);
  onFinish: (req: Req, id: string) => Reply | Promise<Reply> = () => ({
    status: 200,
    body: TRANSCRIPT,
  });
  onPoll: (req: Req, id: string) => Reply | Promise<Reply> = () => ({
    status: 200,
    body: TRANSCRIPT,
  });

  accept(req: Req, id: string): Reply {
    const stream = this.streams.get(id) ?? new Map<number, Uint8Array>();
    this.streams.set(id, stream);
    if (req.seq !== null && !stream.has(req.seq)) stream.set(req.seq, req.body!);
    return { status: 200, body: { received_s: this.receivedSeconds(id), queue_position: 0 } };
  }

  receivedSeconds(id: string): number {
    let bytes = 0;
    for (const chunk of this.streams.get(id)?.values() ?? []) bytes += chunk.byteLength;
    return bytes / 2 / RATE;
  }

  /** Every byte the stream holds, in seq order. */
  audio(id: string): Int16Array {
    const chunks = [...(this.streams.get(id) ?? new Map()).entries()].sort((a, b) => a[0] - b[0]);
    const total = chunks.reduce((sum, [, c]) => sum + c.byteLength, 0);
    const out = new Uint8Array(total);
    let offset = 0;
    for (const [, chunk] of chunks) {
      out.set(chunk, offset);
      offset += chunk.byteLength;
    }
    return new Int16Array(out.buffer);
  }

  puts(): Req[] {
    return this.requests.filter((r) => r.method === 'PUT');
  }

  /** Finish requests, in order. */
  finishes(): Req[] {
    return this.requests.filter((r) => r.method === 'POST' && r.path.endsWith('/finish'));
  }

  /**
   * Both shapes a request reaches `fetch` in: the hand written audio routes pass a URL
   * and an init, the generated client a `Request`.
   */
  fetch = async (input: string | Request, init: RequestInit = {}) => {
    const request = typeof input === 'string' ? null : input;
    const url = new URL(request ? request.url : (input as string));
    const rawHeaders: Record<string, string> = request
      ? Object.fromEntries(request.headers.entries())
      : ((init.headers as Record<string, string>) ?? {});
    const headers = Object.fromEntries(
      Object.entries(rawHeaders).map(([k, v]) => [k.toLowerCase(), v])
    );
    const text = request ? await request.text() : null;
    const signal = request ? request.signal : init.signal;
    const req: Req = {
      method: request ? request.method : (init.method ?? 'GET'),
      path: url.pathname,
      seq: url.searchParams.has('seq') ? Number(url.searchParams.get('seq')) : null,
      waitMs: url.searchParams.has('wait_ms') ? Number(url.searchParams.get('wait_ms')) : null,
      body: init.body instanceof Uint8Array ? new Uint8Array(init.body) : null,
      json: text ? JSON.parse(text) : typeof init.body === 'string' ? JSON.parse(init.body) : null,
      headers,
      signal,
    };
    this.requests.push(req);
    const id = url.pathname.split('/')[3] ?? '';
    let reply: Reply | Promise<Reply>;
    if (req.method === 'POST' && url.pathname === '/dictation/streams') {
      this.creates++;
      reply = this.onCreate(req);
    } else if (req.method === 'PUT') {
      this.inflightPuts++;
      this.maxInflightPuts = Math.max(this.maxInflightPuts, this.inflightPuts);
      try {
        reply = await raceAbort(this.onPut(req, id), signal);
      } finally {
        this.inflightPuts--;
      }
    } else if (req.method === 'POST' && url.pathname.endsWith('/finish')) {
      reply = this.onFinish(req, id);
    } else if (req.method === 'GET') {
      reply = this.onPoll(req, id);
    } else if (req.method === 'DELETE') {
      reply = { status: 204 };
    } else {
      reply = { status: 404 };
    }
    const settled = await raceAbort(reply, signal);
    // As the daemon does: the tail chunk `finish` carries is taken before it answers,
    // unless the answer refuses the chunk itself.
    if (
      req.method === 'POST' &&
      url.pathname.endsWith('/finish') &&
      req.body &&
      req.seq !== null &&
      ![409, 413, 422].includes(settled.status)
    ) {
      this.accept(req, id);
    }
    const body = settled.body === undefined ? null : JSON.stringify(settled.body);
    return new Response(body, {
      status: settled.status,
      headers: {
        ...(body === null ? {} : { 'Content-Type': 'application/json' }),
        ...(settled.headers ?? {}),
      },
    });
  };
}

function raceAbort<T>(value: T | Promise<T>, signal: AbortSignal | null | undefined): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    if (signal?.aborted) return reject(new DOMException('aborted', 'AbortError'));
    signal?.addEventListener('abort', () => reject(new DOMException('aborted', 'AbortError')), {
      once: true,
    });
    Promise.resolve(value).then(resolve, reject);
  });
}

const never = <T>(): Promise<T> => new Promise<T>(() => undefined);

/** `seconds` of a ramp, so every sample says where it came from. */
function audio(seconds: number, offset = 0): Int16Array {
  const out = new Int16Array(Math.round(seconds * RATE));
  for (let i = 0; i < out.length; i++) out[i] = ((offset + i) % 30000) - 15000;
  return out;
}

const REQUEST: StreamRequest = {
  sample_rate: RATE,
  format: 's16le',
  target_kind: 'chat',
  session_id: 'chat-1',
};

let daemon: FakeDaemon;

beforeEach(() => {
  vi.useFakeTimers({
    toFake: ['setTimeout', 'clearTimeout', 'setInterval', 'clearInterval', 'Date'],
  });
  daemon = new FakeDaemon();
  resetDictationApiForTests();
  Object.assign(window, {
    appConfig: { get: (key: string) => (key === 'BIOROUTER_API_HOST' ? BASE : undefined) },
    electron: {
      getSecretKey: vi.fn(async () => 'secret-1'),
      getUserActionKey: vi.fn(async () => 'proof-1'),
    },
  });
  vi.stubGlobal('fetch', daemon.fetch);
});

afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

const flush = () => vi.advanceTimersByTimeAsync(0);

/** Push `seconds` of audio in 100 ms chunks, as the worklet does, letting the clock run. */
async function speak(stream: DictationStream, seconds: number, offset = 0): Promise<number> {
  const chunk = RATE / 10;
  const total = Math.round(seconds * RATE);
  for (let done = 0; done < total; done += chunk) {
    stream.push(audio(Math.min(chunk, total - done) / RATE, offset + done));
    await vi.advanceTimersByTimeAsync(100);
  }
  return offset + total;
}

describe('creating a stream', () => {
  it('sends the body, the secret and the proof, and buffers audio pushed before the answer', async () => {
    let answer!: (reply: Reply) => void;
    daemon.onCreate = () => new Promise<Reply>((resolve) => (answer = resolve));
    const stream = createDictationStream({ ...REQUEST, context: 'é'.repeat(2000) });
    stream.push(audio(3));
    await flush();
    expect(daemon.puts()).toHaveLength(0);
    answer({
      status: 201,
      body: { stream_id: 'abc', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    await stream.ready;
    await flush();

    const create = daemon.requests[0];
    expect(create.path).toBe('/dictation/streams');
    expect(create.headers['x-secret-key']).toBe('secret-1');
    expect(create.headers['x-user-action']).toBe('proof-1');
    expect(create.json).toMatchObject({
      sample_rate: 16000,
      format: 's16le',
      target_kind: 'chat',
      session_id: 'chat-1',
    });
    // context is cut to 2 KiB of UTF-8 on a character boundary.
    expect(new TextEncoder().encode((create.json as { context: string }).context).length).toBe(
      2048
    );

    const [put] = daemon.puts();
    expect(put.path).toBe('/dictation/streams/abc/audio');
    expect(put.seq).toBe(0);
    expect(put.headers['content-type']).toBe('application/octet-stream');
    expect(put.headers['x-secret-key']).toBe('secret-1');
    expect(put.headers['x-user-action']).toBeUndefined();
    expect(put.body!.byteLength).toBe(3 * RATE * 2);
  });

  it('maps refusals: noModel, a policy sentence, an old backend', async () => {
    daemon.onCreate = () => ({ status: 409, body: { code: 'noModel', message: '' } });
    await expect(createDictationStream(REQUEST).ready).rejects.toMatchObject({ code: 'noModel' });

    daemon.onCreate = () => ({
      status: 403,
      body: {
        code: 'policy',
        message: "Dictation is turned off on this computer by your organization's managed policy.",
      },
    });
    await expect(createDictationStream(REQUEST).ready).rejects.toMatchObject({
      code: 'policy',
      serverMessage:
        "Dictation is turned off on this computer by your organization's managed policy.",
    });

    daemon.onCreate = () => ({ status: 404 });
    await expect(createDictationStream(REQUEST).ready).rejects.toMatchObject({ code: 'absent' });
  });

  it('on a 429 counts down Retry-After, then tries again', async () => {
    daemon.onCreate = () =>
      daemon.creates === 1
        ? {
            status: 429,
            body: { code: 'tooManyStreams', message: '' },
            headers: { 'Retry-After': '3' },
          }
        : {
            status: 201,
            body: {
              stream_id: 'late',
              max_chunk_bytes: 524288,
              idle_timeout_s: 60,
              max_seconds: 600,
            },
          };
    const stream = createDictationStream(REQUEST);
    const busy = vi.fn();
    stream.onBusy(busy);
    await flush();
    expect(busy).toHaveBeenCalledWith(3);
    await vi.advanceTimersByTimeAsync(2900);
    expect(daemon.creates).toBe(1);
    await vi.advanceTimersByTimeAsync(200);
    await stream.ready;
    expect(stream.id).toBe('late');
  });

  it('keeps trying on every 429 while the person speaks, and loses no audio', async () => {
    daemon.onCreate = () =>
      daemon.creates <= 3
        ? {
            status: 429,
            body: { code: 'tooManyStreams', message: '' },
            headers: { 'Retry-After': '2' },
          }
        : {
            status: 201,
            body: {
              stream_id: 'late',
              max_chunk_bytes: 524288,
              idle_timeout_s: 60,
              max_seconds: 600,
            },
          };
    const stream = createDictationStream(REQUEST);
    const busy = vi.fn();
    stream.onBusy(busy);
    const end = await speak(stream, 7);
    expect(busy.mock.calls).toEqual([[2], [2], [2]]);
    expect(stream.id).toBe('late');
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(Array.from(daemon.audio('late'))).toEqual(Array.from(audio(end / RATE)));
  });

  it('after the stop waits out a 429 for 30 s, then fails retryably, and Retry sends the same audio', async () => {
    daemon.onCreate = () => ({
      status: 429,
      body: { code: 'tooManyStreams', message: '' },
      headers: { 'Retry-After': '10' },
    });
    const stream = createDictationStream(REQUEST);
    const failed = vi.fn();
    stream.onFailed(failed);
    stream.push(audio(2));
    await flush();
    const settled = stream.finish().catch((error: unknown) => error);
    await vi.advanceTimersByTimeAsync(29_000);
    expect(daemon.creates).toBe(3);
    await vi.advanceTimersByTimeAsync(1_100);
    const error = (await settled) as DictationStreamError;
    expect(error).toBeInstanceOf(DictationStreamError);
    expect(error.code).toBe('tooManyStreams');
    expect(error.retryable).toBe(true);
    expect(daemon.creates).toBe(4);
    // A failure found at the stop is the stop's answer, not a second report.
    expect(failed).not.toHaveBeenCalled();

    daemon.onCreate = () => ({
      status: 201,
      body: { stream_id: 'free', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    const second = stream.retry();
    await vi.advanceTimersByTimeAsync(10);
    await expect(second).resolves.toEqual(TRANSCRIPT);
    expect(Array.from(daemon.audio('free'))).toEqual(Array.from(audio(2)));
  });
});

describe('uploading while the person speaks', () => {
  it('sends a PUT every 2 s of audio, exactly one in flight, in seq order', async () => {
    daemon.onPut = async (req, id) => {
      await new Promise((resolve) => setTimeout(resolve, 700));
      return daemon.accept(req, id);
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    const end = await speak(stream, 9);
    // While recording: one PUT per 2 s of audio, none for the tail yet.
    expect(daemon.puts().length).toBeGreaterThanOrEqual(3);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(5000);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);

    expect(daemon.maxInflightPuts).toBe(1);
    const seqs = daemon.puts().map((p) => p.seq);
    expect(seqs).toEqual(seqs.map((_, i) => i));
    expect(Array.from(daemon.audio('s1'))).toEqual(Array.from(audio(end / RATE)));
    // finish follows the last PUT.
    const finishRequest = last(daemon.requests)!;
    expect(finishRequest.path).toBe('/dictation/streams/s1/finish');
  });

  it('resends the same seq with the same bytes after a 10 s timeout', async () => {
    let first = true;
    daemon.onPut = (req, id) => {
      if (first) {
        first = false;
        return never<Reply>();
      }
      return daemon.accept(req, id);
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(2));
    await flush();
    expect(daemon.puts()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(9_999);
    expect(daemon.puts()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(2);
    expect(daemon.puts()).toHaveLength(2);
    const [a, b] = daemon.puts();
    expect(b.seq).toBe(a.seq);
    expect(Array.from(b.body!)).toEqual(Array.from(a.body!));
  });

  it('shows MSG-WAIT while a PUT has had no answer for 1.5 s', async () => {
    daemon.onPut = async (req, id) => {
      await new Promise((resolve) => setTimeout(resolve, 2_000));
      return daemon.accept(req, id);
    };
    const stream = createDictationStream(REQUEST);
    const waiting = vi.fn();
    stream.onWaiting(waiting);
    await stream.ready;
    stream.push(audio(2));
    await vi.advanceTimersByTimeAsync(1_499);
    expect(waiting).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(waiting).toHaveBeenLastCalledWith(true);
    await vi.advanceTimersByTimeAsync(600);
    expect(waiting).toHaveBeenLastCalledWith(false);
  });

  it('halves the chunk on a 413 down to 64 KiB, and stops with MSG-413 at the floor', async () => {
    daemon.onPut = (req, id) =>
      req.body!.byteLength > 131072 ? { status: 413 } : daemon.accept(req, id);
    let answer!: (reply: Reply) => void;
    daemon.onCreate = () => new Promise<Reply>((resolve) => (answer = resolve));
    const stream = createDictationStream(REQUEST);
    stream.push(audio(30));
    await flush();
    answer({
      status: 201,
      body: { stream_id: 'p', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    await stream.ready;
    await flush();
    const sizes = daemon.puts().map((p) => p.body!.byteLength);
    expect(sizes.slice(0, 4)).toEqual([524288, 262144, 131072, 131072]);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(1000);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(Array.from(daemon.audio('p'))).toEqual(Array.from(audio(30)));

    daemon.onPut = () => ({ status: 413 });
    daemon.onCreate = () => ({
      status: 201,
      body: { stream_id: 'tiny', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    const tiny = createDictationStream(REQUEST);
    await tiny.ready;
    tiny.push(audio(2));
    const finished = tiny.finish();
    await vi.advanceTimersByTimeAsync(10);
    await expect(finished).rejects.toMatchObject({ code: 'proxy413' });
    expect(last(daemon.puts())!.body!.byteLength).toBeLessThanOrEqual(MIN_CHUNK_BYTES);
  });

  it('halves on a network error for a chunk over 64 KiB, and never sends a delivered half twice', async () => {
    let failed = false;
    daemon.onPut = (req, id) => {
      if (!failed && req.body!.byteLength > 200_000) {
        failed = true;
        // The daemon got the whole chunk; only the answer is lost.
        daemon.accept(req, id);
        return Promise.reject(new TypeError('network'));
      }
      return daemon.accept(req, id);
    };
    let answer!: (reply: Reply) => void;
    daemon.onCreate = () => new Promise<Reply>((resolve) => (answer = resolve));
    const stream = createDictationStream(REQUEST);
    stream.push(audio(20));
    await flush();
    answer({
      status: 201,
      body: { stream_id: 'n', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    await stream.ready;
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    const puts = daemon.puts();
    expect(puts[0].body!.byteLength).toBe(524288);
    expect(puts[1].seq).toBe(0);
    expect(puts[1].body!.byteLength).toBe(262144);
    // The repeat was answered with the full chunk's total, so seq 1 starts after
    // the full chunk and the second half is never sent twice: what is left fits in
    // one chunk and rides on finish.
    expect(puts).toHaveLength(2);
    const [finish] = daemon.finishes();
    expect(finish.seq).toBe(1);
    expect(finish.body!.byteLength).toBe(640000 - 524288);
    expect(Array.from(daemon.audio('n'))).toEqual(Array.from(audio(20)));
  });

  it('resends from expected_seq on 409 outOfOrder', async () => {
    let refused = false;
    daemon.onPut = (req, id) => {
      if (req.seq === 1 && !refused) {
        refused = true;
        daemon.streams.get(id)!.delete(0);
        return { status: 409, body: { code: 'outOfOrder', message: '', expected_seq: 0 } };
      }
      return daemon.accept(req, id);
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    await speak(stream, 6);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    const seqs = daemon.puts().map((p) => p.seq);
    expect(seqs.slice(0, 3)).toEqual([0, 1, 0]);
    expect(Array.from(daemon.audio('s1'))).toEqual(Array.from(audio(6)));
  });

  it('after a 410 opens one new stream and replays from seq 0; a second 410 is MSG-LOST', async () => {
    daemon.onPut = (req, id) =>
      id === 's1' && req.seq === 1 ? { status: 410 } : daemon.accept(req, id);
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    await speak(stream, 5);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(daemon.creates).toBe(2);
    expect(stream.id).toBe('s2');
    expect(Array.from(daemon.audio('s2'))).toEqual(Array.from(audio(5)));

    daemon.onPut = () => ({ status: 410 });
    daemon.onFinish = () => ({ status: 410 });
    const lost = createDictationStream(REQUEST);
    await lost.ready;
    lost.push(audio(1));
    const result = lost.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(result).rejects.toMatchObject({ code: 'lost' });
  });
});

describe('when the upload fails while the person speaks', () => {
  it('a 413 at the 64 KiB floor is reported at once, not at the stop, and no more audio is kept', async () => {
    daemon.onPut = () => ({ status: 413 });
    const stream = createDictationStream(REQUEST);
    const failed = vi.fn();
    stream.onFailed(failed);
    await stream.ready;
    await speak(stream, 3);
    expect(failed).toHaveBeenCalledTimes(1);
    expect(failed.mock.calls[0][0]).toMatchObject({ code: 'proxy413' });
    const held = stream.audioSeconds;
    stream.push(audio(1));
    expect(stream.audioSeconds).toBe(held);
    await expect(stream.finish()).rejects.toMatchObject({ code: 'proxy413' });
    expect(failed).toHaveBeenCalledTimes(1);
  });

  it('the sixth timeout in a row on one chunk is lost, at the daemon’s 60 s idle expiry', async () => {
    daemon.onPut = () => never<Reply>();
    const stream = createDictationStream(REQUEST);
    const failed = vi.fn();
    stream.onFailed(failed);
    await stream.ready;
    stream.push(audio(2));
    await flush();
    await vi.advanceTimersByTimeAsync(59_999);
    expect(daemon.puts()).toHaveLength(6);
    expect(failed).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(2);
    expect(failed).toHaveBeenCalledWith(expect.objectContaining({ code: 'lost' }));
    expect(daemon.puts()).toHaveLength(6);
  });

  it('the fourth 5xx, network error or 429 in a row on a chunk that cannot shrink is lost', async () => {
    const cases: Array<[string, () => Reply | Promise<Reply>, number]> = [
      ['5xx', () => ({ status: 503, body: { code: 'busy', message: '' } }), 3_000],
      ['network', () => Promise.reject(new TypeError('network')), 3_000],
      [
        '429',
        () => ({
          status: 429,
          body: { code: 'rate', message: '' },
          headers: { 'Retry-After': '1' },
        }),
        3_000,
      ],
    ];
    for (const [name, reply, lostAtMs] of cases) {
      daemon = new FakeDaemon();
      vi.stubGlobal('fetch', daemon.fetch);
      daemon.onPut = reply;
      const stream = createDictationStream(REQUEST);
      const failed = vi.fn();
      stream.onFailed(failed);
      await stream.ready;
      stream.push(audio(2));
      await flush();
      await vi.advanceTimersByTimeAsync(lostAtMs - 1);
      expect(daemon.puts(), name).toHaveLength(3);
      expect(failed, name).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(2);
      expect(daemon.puts(), name).toHaveLength(4);
      expect(failed, name).toHaveBeenCalledWith(expect.objectContaining({ code: 'lost' }));
    }
  });

  it('a PUT answering 500 engineFailed is reported at once, never resent, and Retry replays the audio', async () => {
    daemon.onPut = (req, id) =>
      id === 's1' && req.seq === 1
        ? { status: 500, body: { code: 'engineFailed', message: 'The worker stopped.' } }
        : daemon.accept(req, id);
    const stream = createDictationStream(REQUEST);
    const failed = vi.fn();
    stream.onFailed(failed);
    await stream.ready;
    await speak(stream, 4.5);
    expect(failed).toHaveBeenCalledTimes(1);
    const error = failed.mock.calls[0][0] as DictationStreamError;
    expect(error.code).toBe('engineFailed');
    expect(error.serverMessage).toBe('The worker stopped.');
    expect(error.retryable).toBe(true);
    // One answer was enough: the 500 was not taken for a passing 5xx and resent.
    expect(daemon.puts().filter((p) => p.seq === 1)).toHaveLength(1);
    await expect(stream.finish()).rejects.toMatchObject({ code: 'engineFailed' });

    const second = stream.retry();
    await vi.advanceTimersByTimeAsync(10);
    await expect(second).resolves.toEqual(TRANSCRIPT);
    expect(stream.id).toBe('s2');
    // What was kept up to the failure went into the new stream.
    expect(daemon.audio('s2').length).toBe(Math.round(stream.audioSeconds * RATE));
    expect(daemon.audio('s2').length).toBeGreaterThanOrEqual(4 * RATE);
  });

  it('a cancel is not a failure', async () => {
    daemon.onPut = () => never<Reply>();
    const stream = createDictationStream(REQUEST);
    const failed = vi.fn();
    stream.onFailed(failed);
    await stream.ready;
    stream.push(audio(2));
    await flush();
    await stream.cancel();
    await vi.advanceTimersByTimeAsync(100);
    expect(failed).not.toHaveBeenCalled();
  });
});

describe('finishing', () => {
  it('sends what is left with finish, so the stop is one round trip', async () => {
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    const end = await speak(stream, 5.3);
    const before = daemon.puts().length;
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(10);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    // No PUT after the stop: the last 1.3 s went up with finish.
    expect(daemon.puts()).toHaveLength(before);
    const [finish] = daemon.finishes();
    expect(finish.seq).toBe(before);
    expect(finish.headers['content-type']).toBe('application/octet-stream');
    expect(finish.headers['x-secret-key']).toBe('secret-1');
    expect(finish.body!.byteLength).toBe((end - 4 * RATE) * 2);
    expect(Array.from(daemon.audio('s1'))).toEqual(Array.from(audio(end / RATE)));

    // Nothing left after the last PUT: finish carries no body and no seq.
    const quiet = createDictationStream(REQUEST);
    await quiet.ready;
    await speak(quiet, 2);
    await vi.advanceTimersByTimeAsync(10);
    const done = quiet.finish();
    await vi.advanceTimersByTimeAsync(10);
    await expect(done).resolves.toEqual(TRANSCRIPT);
    expect(last(daemon.finishes())).toMatchObject({ seq: null, body: null });
  });

  it('a finish that times out is resent with the same seq and the same tail', async () => {
    let first = true;
    daemon.onFinish = () => {
      if (first) {
        first = false;
        return never<Reply>();
      }
      return { status: 200, body: TRANSCRIPT };
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(1));
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(15_001);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    const [a, b] = daemon.finishes();
    expect(b.seq).toBe(a.seq);
    expect(Array.from(b.body!)).toEqual(Array.from(a.body!));
  });

  it('a 413 on the tail halves it and sends the rest as PUTs first', async () => {
    daemon.onFinish = (req) =>
      req.body && req.body.byteLength > 65_536
        ? { status: 413 }
        : { status: 200, body: TRANSCRIPT };
    // The create answers after the stop, so all 3 s are left for finish.
    let answer!: (reply: Reply) => void;
    daemon.onCreate = () => new Promise<Reply>((resolve) => (answer = resolve));
    const stream = createDictationStream(REQUEST);
    stream.push(audio(3));
    const transcript = stream.finish();
    await flush();
    answer({
      status: 201,
      body: { stream_id: 'late', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    await vi.advanceTimersByTimeAsync(10);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(daemon.finishes()[0].body!.byteLength).toBe(3 * RATE * 2);
    expect(daemon.puts().map((p) => p.body!.byteLength)).toEqual([65_536]);
    expect(last(daemon.finishes())!.seq).toBe(1);
    expect(Array.from(daemon.audio('late'))).toEqual(Array.from(audio(3)));

    // At the 64 KiB floor a 413 on the tail is MSG-413.
    daemon.onFinish = (req) => (req.body ? { status: 413 } : { status: 200, body: TRANSCRIPT });
    daemon.onCreate = () => ({
      status: 201,
      body: { stream_id: 'small', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    const small = createDictationStream(REQUEST);
    await small.ready;
    small.push(audio(1));
    const refused = small.finish();
    await vi.advanceTimersByTimeAsync(10);
    await expect(refused).rejects.toMatchObject({ code: 'proxy413' });
  });

  it('a 409 outOfOrder on the tail resends from expected_seq', async () => {
    let refused = false;
    daemon.onFinish = (_req, id) => {
      if (!refused) {
        refused = true;
        daemon.streams.get(id)!.delete(1);
        return { status: 409, body: { code: 'outOfOrder', message: '', expected_seq: 1 } };
      }
      return { status: 200, body: TRANSCRIPT };
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    await speak(stream, 5);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(10);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(daemon.puts().map((p) => p.seq)).toEqual([0, 1]);
    expect(daemon.finishes().map((f) => f.seq)).toEqual([2, 1]);
    expect(Array.from(daemon.audio('s1'))).toEqual(Array.from(audio(5)));
  });

  it('after a 202 reads with a held GET, never a sleep between two polls', async () => {
    daemon.onFinish = () => ({ status: 202, body: { pending: true } });
    let polls = 0;
    daemon.onPoll = () =>
      ++polls < 3 ? { status: 202, body: { pending: true } } : { status: 200, body: TRANSCRIPT };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(1));
    const transcript = stream.finish();
    // Each read is answered at once here, so all three run well inside one old
    // 500 ms poll interval.
    await vi.advanceTimersByTimeAsync(400);
    expect(polls).toBe(3);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    const gets = daemon.requests.filter((r) => r.method === 'GET');
    expect(gets.every((r) => r.path === `/dictation/streams/${stream.id}`)).toBe(true);
    expect(gets.every((r) => r.waitMs === HELD_READ_MS)).toBe(true);
  });

  it('a held read answers the moment the daemon does', async () => {
    daemon.onFinish = () => ({ status: 202, body: { pending: true } });
    let polls = 0;
    daemon.onPoll = () => {
      polls++;
      return new Promise<Reply>((resolve) =>
        setTimeout(() => resolve({ status: 200, body: TRANSCRIPT }), 2_400)
      );
    };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(1));
    let settled = false;
    const transcript = stream.finish().then((t) => {
      settled = true;
      return t;
    });
    await vi.advanceTimersByTimeAsync(2_399);
    expect(settled).toBe(false);
    expect(polls).toBe(1);
    await vi.advanceTimersByTimeAsync(2);
    expect(settled).toBe(true);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(polls).toBe(1);
  });

  it('maps 500 engineFailed to a retryable failure, and Retry replays the audio into a new stream', async () => {
    daemon.onFinish = (_req, id) =>
      id === 's1'
        ? { status: 500, body: { code: 'engineFailed', message: 'x' } }
        : { status: 200, body: TRANSCRIPT };
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    await speak(stream, 3);
    const first = stream.finish();
    await vi.advanceTimersByTimeAsync(10);
    const error = await first.catch((e: unknown) => e);
    expect(error).toBeInstanceOf(DictationStreamError);
    expect((error as DictationStreamError).code).toBe('engineFailed');
    expect((error as DictationStreamError).retryable).toBe(true);

    const second = stream.retry();
    await vi.advanceTimersByTimeAsync(10);
    await expect(second).resolves.toEqual(TRANSCRIPT);
    expect(stream.id).toBe('s2');
    expect(Array.from(daemon.audio('s2'))).toEqual(Array.from(audio(3)));
    // The failed stream was dropped.
    expect(
      daemon.requests.some((r) => r.method === 'DELETE' && r.path === '/dictation/streams/s1')
    ).toBe(true);
  });
});

describe('the 600 s cap', () => {
  it('warns at 570 s and stops at 600 s, taking no audio past it', async () => {
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    const warn = vi.fn();
    const stop = vi.fn();
    stream.onCapWarning(warn);
    stream.onCapReached(stop);
    for (let s = 0; s < 560; s += 10) stream.push(audio(10));
    await flush();
    expect(warn).not.toHaveBeenCalled();
    stream.push(audio(10));
    expect(warn).toHaveBeenCalledTimes(1);
    expect(stop).not.toHaveBeenCalled();
    stream.push(audio(20));
    expect(stop).not.toHaveBeenCalled();
    stream.push(audio(15));
    expect(stop).toHaveBeenCalledTimes(1);
    expect(stream.audioSeconds).toBe(600);
    stream.push(audio(5));
    expect(stream.audioSeconds).toBe(600);
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(100);
    await expect(transcript).resolves.toEqual(TRANSCRIPT);
    expect(daemon.audio('s1').length).toBe(600 * RATE);
  });
});

describe('ending', () => {
  it('cancel sends DELETE, and nothing more goes up', async () => {
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(1));
    await stream.cancel();
    expect(last(daemon.requests)).toMatchObject({
      method: 'DELETE',
      path: '/dictation/streams/s1',
    });
    stream.push(audio(3));
    await vi.advanceTimersByTimeAsync(100);
    expect(daemon.puts()).toHaveLength(0);
    await expect(stream.finish()).rejects.toMatchObject({ code: 'cancelled' });
  });

  it('a create answered after a cancel drops the stream the daemon made', async () => {
    let answer!: (value: Awaited<ReturnType<DictationApi['createStream']>>) => void;
    const drop = vi.fn(async () => undefined);
    const api = {
      ...dictationApi,
      // A daemon that answers although the request was aborted.
      createStream: () => new Promise((resolve) => (answer = resolve)),
      drop,
    } as DictationApi;
    const stream = createDictationStream(REQUEST, { api });
    await flush();
    await stream.cancel();
    expect(drop).not.toHaveBeenCalled();
    answer({
      ok: true,
      status: 201,
      data: { stream_id: 'late', max_chunk_bytes: 524288, idle_timeout_s: 60, max_seconds: 600 },
    });
    await expect(stream.ready).rejects.toMatchObject({ code: 'cancelled' });
    expect(drop).toHaveBeenCalledWith('late');
    expect(stream.id).toBeNull();
  });

  it('release after the insert sends DELETE', async () => {
    const stream = createDictationStream(REQUEST);
    await stream.ready;
    stream.push(audio(1));
    const transcript = stream.finish();
    await vi.advanceTimersByTimeAsync(10);
    await transcript;
    stream.release();
    await flush();
    expect(last(daemon.requests)).toMatchObject({
      method: 'DELETE',
      path: '/dictation/streams/s1',
    });
  });
});
