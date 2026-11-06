/**
 * The client side of one dictation stream (DD-27 to DD-31, "The client side of
 * a stream" in `docs/dictation/surfaces.md`).
 *
 * Audio goes up while the person speaks, in small `PUT`s, so the daemon can
 * decode closed segments before they stop; on stop only the last couple of
 * seconds are left to send, and `finish` follows the last `PUT` at once. No
 * request is ever held open by the client: the daemon's `finish` waits up to
 * 3 s, then answers 202, and the client polls.
 *
 * The rules, each with its reason in the design documents:
 *
 * - The stream is created when capture starts, in parallel with `getUserMedia`;
 *   audio pushed before the daemon answers is buffered.
 * - A 429 at creation (the daemon's stream or segment limit) shows MSG-SERVER-BUSY
 *   counted down from `Retry-After`, then tries again: on every `Retry-After`
 *   while the person speaks, and for up to 30 s after the stop. The audio stays
 *   buffered meanwhile, so a busy server costs time, never speech. After that the
 *   failure is retryable like `engineFailed`.
 * - Every 2 s of audio (64,000 B at 16 kHz) is one `PUT`, never more than
 *   `max_chunk_bytes`, with exactly one in flight and a 10 s timeout. A timeout
 *   resends the same `seq` with the same bytes, which the daemon treats as a
 *   repeat; the sixth timeout in a row on one chunk (the daemon's 60 s idle
 *   expiry) is MSG-LOST, and so is the fourth network error, 5xx or 429 in a row
 *   on a chunk that cannot shrink.
 * - A 413, or a network error on a chunk over 64 KiB, halves the chunk, with a
 *   floor of 64 KiB; a 413 at the floor ends the dictation with MSG-413.
 * - A 409 `outOfOrder` resends from `expected_seq`.
 * - A 410 the person did not cause opens one new stream and replays the audio
 *   from `seq=0`; a second one ends the dictation with MSG-LOST.
 * - A failure while the person still speaks is reported at once through
 *   `onFailed`, not at the stop, and the stream takes no more audio.
 * - MSG-WAIT shows while a `PUT` or a poll has had no answer for 1.5 s.
 * - MSG-CAP-WARN fires 30 s before `max_seconds`; at `max_seconds` the stream
 *   takes no more audio and asks the caller to stop and insert.
 * - A `500 engineFailed` keeps the audio, and `retry()` replays it into a new
 *   stream; the caller drops it after 5 minutes.
 *
 * The audio is held in renderer memory only, until the transcript has been
 * inserted (`release()`) or the dictation is cancelled, and is zeroed when it is
 * dropped. It is never logged.
 */
import {
  dictationApi,
  isTranscriptPending,
  type DictationApi,
  type DictationResult,
  type StreamRequest,
  type Transcript,
  type TranscriptPending,
} from './dictationApi';

export type { DictationTargetKind, StreamRequest, Transcript } from './dictationApi';

/** Seconds of audio per `PUT` while recording. */
export const UPLOAD_EVERY_S = 2;
export const PUT_TIMEOUT_MS = 10_000;
/** The halving floor. */
export const MIN_CHUNK_BYTES = 65_536;
export const DEFAULT_MAX_CHUNK_BYTES = 524_288;
export const DEFAULT_MAX_SECONDS = 600;
export const CAP_WARNING_LEAD_S = 30;
export const WAIT_NOTICE_MS = 1_500;
export const POLL_EVERY_MS = 500;
/** `finish` holds up to 3 s on the daemon; anything far past that is a hung request. */
export const FINISH_TIMEOUT_MS = 15_000;
/**
 * Timeouts in a row on one chunk that call the stream lost: six of 10 s are the
 * daemon's 60 s idle expiry, so the sixth ends it.
 */
export const MAX_TIMEOUTS_PER_CHUNK = 6;
/**
 * Network errors on a chunk that cannot shrink, 5xx answers or 429 answers in a
 * row that are retried; the next one calls the stream lost.
 */
export const MAX_TRANSIENT_FAILURES = 3;
/** The longest single wait a 429 asks for that the client honours. */
export const MAX_BUSY_WAIT_S = 30;
/** After the stop, how long a 429 at creation is still waited out. */
export const BUSY_AFTER_STOP_MS = 30_000;

export type DictationFailureCode =
  /** 409 `noModel`: open the setup popover. */
  | 'noModel'
  /** 403: the policy sentence in `serverMessage`. */
  | 'policy'
  /** 429 at creation still 30 s after the stop: MSG-SERVER-BUSY-FAILED with **Retry**. */
  | 'tooManyStreams'
  /** 409 `notCompiled`, `unsupportedCpu`, `languageUnsupported`: the sentence in `serverMessage`. */
  | 'unavailable'
  /** 503 `serverMemory`: MSG-SERVER-MEMORY. */
  | 'serverMemory'
  /** 500 `engineFailed`: MSG-ENGINE-FAILED with **Retry**. */
  | 'engineFailed'
  /** A second 410, or no answer at all: MSG-LOST. */
  | 'lost'
  /** A 413 at the 64 KiB floor: MSG-413. */
  | 'proxy413'
  /** 404 or 405 from the create route: a backend from before dictation, MSG-REMOTE-OLD. */
  | 'absent'
  /** 422 or another refusal a conforming client never meets. */
  | 'badRequest'
  | 'cancelled';

export class DictationStreamError extends Error {
  constructor(
    readonly code: DictationFailureCode,
    /** The daemon's `message`, the sentence to show for `policy` and `unavailable`. */
    readonly serverMessage = '',
    readonly detail: { status?: number; serverCode?: string; retryAfterS?: number | null } = {}
  ) {
    super(`dictation stream: ${code}${detail.serverCode ? ` (${detail.serverCode})` : ''}`);
    this.name = 'DictationStreamError';
  }
  /** The audio is kept, and `retry()` sends it into a new stream. */
  get retryable(): boolean {
    return this.code === 'engineFailed' || this.code === 'tooManyStreams';
  }
}

export interface DictationStream {
  /** The daemon's id for the current stream; a replay or a retry replaces it. */
  readonly id: string | null;
  /** Settles when the daemon accepted the stream; rejects with a {@link DictationStreamError}. */
  readonly ready: Promise<void>;
  /** Seconds of audio pushed so far. */
  readonly audioSeconds: number;
  /** Buffer audio; it goes up on the stream's own clock. */
  push(pcm: Int16Array): void;
  /** Send what is left, then the transcript. */
  finish(): Promise<Transcript>;
  /** After a retryable failure: a new stream with the same audio, then `finish`. */
  retry(): Promise<Transcript>;
  /** `DELETE`, and drop the audio. Nothing is inserted. */
  cancel(): Promise<void>;
  /** After the text went in: `DELETE` and drop the audio. */
  release(): void;
  onPartial(cb: (text: string) => void): () => void;
  /** Drives MSG-WAIT. */
  onWaiting(cb: (waiting: boolean) => void): () => void;
  /** 30 s before `max_seconds`. */
  onCapWarning(cb: () => void): () => void;
  /** At `max_seconds`: stop and insert, then MSG-CAP-STOPPED. */
  onCapReached(cb: () => void): () => void;
  /** A 429 at creation: MSG-SERVER-BUSY counting down from `seconds`, then another try. */
  onBusy(cb: (seconds: number) => void): () => void;
  /**
   * The upload failed while the person still speaks (a 413 at the floor, the
   * sixth timeout, a fourth 5xx, a second 410, a refused re-create). The stream
   * takes no more audio; `finish` rejects with the same error.
   */
  onFailed(cb: (error: DictationStreamError) => void): () => void;
}

export interface StreamOptions {
  api?: DictationApi;
  /** Aborting it is a cancel. */
  signal?: AbortSignal;
}

class Emitter<T extends unknown[]> {
  private readonly listeners = new Set<(...args: T) => void>();
  on(cb: (...args: T) => void): () => void {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }
  emit(...args: T): void {
    for (const cb of [...this.listeners]) {
      try {
        cb(...args);
      } catch {
        // A listener's failure is its own; the stream goes on.
      }
    }
  }
}

class RequestTimeout extends Error {
  constructor() {
    super('request timed out');
    this.name = 'RequestTimeout';
  }
}

const CANCELLED = (): DictationStreamError => new DictationStreamError('cancelled');

function asStreamError(error: unknown): DictationStreamError {
  return error instanceof DictationStreamError ? error : new DictationStreamError('lost');
}

function createFailure(
  result: Extract<DictationResult<unknown>, { ok: false }>
): DictationStreamError {
  const { status, error, retryAfterS } = result;
  const detail = { status, serverCode: error.code, retryAfterS };
  if (status === 403) return new DictationStreamError('policy', error.message, detail);
  if (status === 409 && error.code === 'noModel') {
    return new DictationStreamError('noModel', error.message, detail);
  }
  if (status === 409) return new DictationStreamError('unavailable', error.message, detail);
  if (status === 429) return new DictationStreamError('tooManyStreams', error.message, detail);
  if (status === 503) return new DictationStreamError('serverMemory', error.message, detail);
  if (status === 404 || status === 405) return new DictationStreamError('absent', '', detail);
  if (status >= 400 && status < 500) return new DictationStreamError('badRequest', '', detail);
  return new DictationStreamError('lost', '', detail);
}

class Stream implements DictationStream {
  id: string | null = null;
  ready: Promise<void>;

  private readonly api: DictationApi;
  private readonly rate: number;
  private readonly abort = new AbortController();
  private audio: Int16Array;
  private total = 0;
  /** Samples the daemon has acknowledged; the next chunk starts here. */
  private cursor = 0;
  private seq = 0;
  private seqStarts: number[] = [];
  private maxChunkBytes = DEFAULT_MAX_CHUNK_BYTES;
  private chunkLimitBytes = DEFAULT_MAX_CHUNK_BYTES;
  private maxSeconds = DEFAULT_MAX_SECONDS;
  private closing = false;
  /** When `finish` (or `retry`) was asked for; bounds the wait a 429 at creation gets. */
  private closedAt: number | null = null;
  private cancelled = false;
  private released = false;
  private failed: DictationStreamError | null = null;
  private pump: Promise<void> | null = null;
  private finishing: Promise<Transcript> | null = null;
  private recoveries = 0;
  private capWarned = false;
  private capReached = false;
  private waits = 0;
  private lastPartial: string | null = null;

  private readonly partial = new Emitter<[string]>();
  private readonly waiting = new Emitter<[boolean]>();
  private readonly capWarning = new Emitter<[]>();
  private readonly capStop = new Emitter<[]>();
  private readonly busy = new Emitter<[number]>();
  private readonly failure = new Emitter<[DictationStreamError]>();

  constructor(
    private readonly request: StreamRequest,
    options: StreamOptions
  ) {
    this.api = options.api ?? dictationApi;
    this.rate = request.sample_rate > 0 ? request.sample_rate : 16000;
    this.audio = new Int16Array(this.rate * 30);
    if (options.signal) {
      if (options.signal.aborted) this.cancelled = true;
      else options.signal.addEventListener('abort', () => void this.cancel(), { once: true });
    }
    this.ready = this.create();
    this.ready.catch(() => undefined);
  }

  get audioSeconds(): number {
    return this.total / this.rate;
  }

  onPartial(cb: (text: string) => void): () => void {
    return this.partial.on(cb);
  }
  onWaiting(cb: (waiting: boolean) => void): () => void {
    return this.waiting.on(cb);
  }
  onCapWarning(cb: () => void): () => void {
    return this.capWarning.on(cb);
  }
  onCapReached(cb: () => void): () => void {
    return this.capStop.on(cb);
  }
  onBusy(cb: (seconds: number) => void): () => void {
    return this.busy.on(cb);
  }
  onFailed(cb: (error: DictationStreamError) => void): () => void {
    return this.failure.on(cb);
  }

  // ── audio ─────────────────────────────────────────────────────────────────

  push(pcm: Int16Array): void {
    // After a failure nothing more can go up, so nothing more is kept.
    if (this.cancelled || this.released || this.closing || this.failed || pcm.length === 0) return;
    const cap = this.maxSeconds * this.rate;
    const take = Math.min(pcm.length, cap - this.total);
    if (take <= 0) return;
    if (this.total + take > this.audio.length) {
      const grown = new Int16Array(
        Math.min(cap, Math.max(this.audio.length * 2, this.total + take))
      );
      grown.set(this.audio.subarray(0, this.total));
      this.audio.fill(0);
      this.audio = grown;
    }
    this.audio.set(take === pcm.length ? pcm : pcm.subarray(0, take), this.total);
    this.total += take;
    if (!this.capWarned && this.total >= (this.maxSeconds - CAP_WARNING_LEAD_S) * this.rate) {
      this.capWarned = true;
      this.capWarning.emit();
    }
    if (!this.capReached && this.total >= cap) {
      this.capReached = true;
      this.capStop.emit();
    }
    this.kick();
  }

  private bytes(start: number, count: number): Uint8Array {
    // A copy, so a later growth or wipe of the buffer never touches a body in flight.
    // Int16Array is little endian on every platform Biorouter ships for, which is s16le.
    return new Uint8Array(this.audio.slice(start, start + count).buffer);
  }

  private wipe(): void {
    this.audio.fill(0);
    this.audio = new Int16Array(0);
    this.total = 0;
    this.cursor = 0;
  }

  // ── creation and recovery ─────────────────────────────────────────────────

  private body(): StreamRequest {
    const body: StreamRequest = { ...this.request, format: 's16le', sample_rate: this.rate };
    if (body.context !== undefined) body.context = truncateUtf8(body.context, 2048);
    return body;
  }

  private async create(): Promise<void> {
    for (;;) {
      if (this.cancelled) throw CANCELLED();
      let result;
      try {
        result = await this.api.createStream(this.body(), this.abort.signal);
      } catch {
        if (this.cancelled) throw CANCELLED();
        throw new DictationStreamError('lost');
      }
      if (result.ok) {
        const created = result.data;
        if (this.cancelled) {
          // The answer crossed a cancel: the daemon made a stream nobody will
          // use, so it goes now rather than at the 60 s idle expiry.
          void this.api.drop(created.stream_id);
          throw CANCELLED();
        }
        this.id = created.stream_id;
        if (created.max_chunk_bytes > 0) {
          this.maxChunkBytes = created.max_chunk_bytes;
          this.chunkLimitBytes = Math.min(this.chunkLimitBytes, created.max_chunk_bytes);
        }
        if (created.max_seconds > 0) this.maxSeconds = created.max_seconds;
        this.kick();
        return;
      }
      if (result.status === 429) {
        // While the person speaks the audio is buffered, so waiting costs
        // nothing; after the stop the wait is bounded, and the audio is kept
        // for Retry either way.
        let seconds = Math.min(MAX_BUSY_WAIT_S, Math.max(1, Math.ceil(result.retryAfterS ?? 5)));
        if (this.closedAt !== null) {
          const left = BUSY_AFTER_STOP_MS - (Date.now() - this.closedAt);
          if (left <= 0) throw createFailure(result);
          seconds = Math.min(seconds, Math.max(1, Math.ceil(left / 1000)));
        }
        this.busy.emit(seconds);
        await this.sleep(seconds * 1000);
        continue;
      }
      throw createFailure(result);
    }
  }

  /** A 410 the person did not cause: one new stream, the audio replayed from `seq=0`. */
  private async recover(): Promise<void> {
    if (this.cancelled) throw CANCELLED();
    if (this.recoveries >= 1) throw new DictationStreamError('lost');
    this.recoveries++;
    await this.restart();
  }

  private async restart(): Promise<void> {
    const old = this.id;
    this.id = null;
    if (old) void this.api.drop(old);
    this.seq = 0;
    this.cursor = 0;
    this.seqStarts = [];
    this.chunkLimitBytes = this.maxChunkBytes;
    this.ready = this.create();
    this.ready.catch(() => undefined);
    await this.ready;
  }

  // ── uploading ─────────────────────────────────────────────────────────────

  private get threshold(): number {
    return UPLOAD_EVERY_S * this.rate;
  }

  private wantsUpload(): boolean {
    const pending = this.total - this.cursor;
    return pending > 0 && (this.closing || pending >= this.threshold);
  }

  private kick(): void {
    if (this.pump || this.cancelled || this.failed || !this.id || !this.wantsUpload()) return;
    this.pump = this.pumpLoop().finally(() => {
      this.pump = null;
      this.kick();
    });
  }

  private async pumpLoop(): Promise<void> {
    try {
      while (!this.cancelled && !this.failed && this.id && this.wantsUpload()) {
        await this.sendNext();
      }
    } catch (error) {
      if (this.failed) return;
      this.failed = asStreamError(error);
      // Said at once, not at the stop: the person may still be speaking into a
      // stream that can take nothing more.
      if (!this.cancelled && this.failed.code !== 'cancelled') this.failure.emit(this.failed);
    }
  }

  /** Wait until everything pushed has been acknowledged, or the stream failed. */
  private async drain(): Promise<void> {
    for (;;) {
      if (this.cancelled) throw CANCELLED();
      if (!this.id) {
        try {
          await this.ready;
        } catch (error) {
          this.failed ??= asStreamError(error);
        }
      }
      if (this.failed) throw this.failed;
      this.kick();
      if (this.pump) {
        await this.pump;
        continue;
      }
      if (this.total - this.cursor <= 0) return;
    }
  }

  private async sendNext(): Promise<void> {
    const seq = this.seq;
    const start = this.cursor;
    const streamId = this.id as string;
    let count = Math.min(this.total - start, Math.floor(this.chunkLimitBytes / 2));
    let timeouts = 0;
    let transient = 0;
    let fullCount: number | null = null;

    for (;;) {
      if (this.cancelled) throw CANCELLED();
      const body = this.bytes(start, count);
      let result: DictationResult<{ received_s: number; partial?: string | null }>;
      try {
        result = await this.timed(
          (signal) => this.api.putAudio(streamId, seq, body, signal),
          PUT_TIMEOUT_MS
        );
      } catch (error) {
        if (this.cancelled) throw CANCELLED();
        if (error instanceof RequestTimeout) {
          // The same seq with the same bytes: the daemon treats a repeat as a repeat.
          if (++timeouts >= MAX_TIMEOUTS_PER_CHUNK) throw new DictationStreamError('lost');
          continue;
        }
        if (body.byteLength > MIN_CHUNK_BYTES) {
          fullCount ??= count;
          count = this.halve(body.byteLength);
          continue;
        }
        if (++transient > MAX_TRANSIENT_FAILURES) throw new DictationStreamError('lost');
        await this.sleep(POLL_EVERY_MS * transient);
        continue;
      }

      if (result.ok) {
        let end = start + count;
        if (fullCount !== null && fullCount > count) {
          // After a network error the first, larger body may have arrived and
          // only its answer been lost. The repeat of this seq is then ignored
          // and answered as before, with the full chunk's total; do not send
          // the second half again.
          const held = Math.round((result.data.received_s ?? 0) * this.rate);
          const fullEnd = start + fullCount;
          if (held >= (end + fullEnd) / 2) end = fullEnd;
        }
        this.seqStarts[seq] = start;
        this.cursor = end;
        this.seq = seq + 1;
        const partial = result.data.partial;
        if (typeof partial === 'string' && partial !== this.lastPartial) {
          this.lastPartial = partial;
          this.partial.emit(partial);
        }
        return;
      }

      switch (result.status) {
        case 413:
          if (body.byteLength > MIN_CHUNK_BYTES) {
            count = this.halve(body.byteLength);
            continue;
          }
          throw new DictationStreamError('proxy413', '', { status: 413 });
        case 409: {
          const expected = result.error.expected_seq;
          if (
            result.error.code === 'outOfOrder' &&
            typeof expected === 'number' &&
            expected >= 0 &&
            expected < seq &&
            this.seqStarts[expected] !== undefined
          ) {
            this.cursor = this.seqStarts[expected];
            this.seq = expected;
            this.seqStarts.length = expected;
            return;
          }
          throw new DictationStreamError('lost', '', {
            status: 409,
            serverCode: result.error.code,
          });
        }
        case 410:
          await this.recover();
          return;
        case 429:
          // The daemon never answers a PUT with 429; a proxy in front that rate
          // limits does, and is retried no longer than any other transient failure.
          if (++transient > MAX_TRANSIENT_FAILURES) {
            throw new DictationStreamError('lost', '', {
              status: 429,
              serverCode: result.error.code,
            });
          }
          await this.sleep(
            Math.min(PUT_TIMEOUT_MS, Math.max(1000, (result.retryAfterS ?? 1) * 1000))
          );
          continue;
        default:
          if (result.status >= 500 && ++transient <= MAX_TRANSIENT_FAILURES) {
            await this.sleep(POLL_EVERY_MS * transient);
            continue;
          }
          throw new DictationStreamError('lost', '', {
            status: result.status,
            serverCode: result.error.code,
          });
      }
    }
  }

  /** Halve the chunk limit, floor 64 KiB, and answer the new sample count. */
  private halve(bytes: number): number {
    const halved = Math.max(MIN_CHUNK_BYTES, Math.floor(bytes / 2));
    this.chunkLimitBytes = halved - (halved % 2);
    return Math.min(this.total - this.cursor, this.chunkLimitBytes / 2);
  }

  // ── finishing ─────────────────────────────────────────────────────────────

  finish(): Promise<Transcript> {
    if (this.finishing) return this.finishing;
    this.closing = true;
    this.closedAt ??= Date.now();
    this.finishing = this.runFinish();
    this.finishing.catch(() => undefined);
    return this.finishing;
  }

  private async runFinish(): Promise<Transcript> {
    try {
      for (;;) {
        await this.drain();
        const outcome = await this.finishOnce();
        if (outcome !== 'restarted') return outcome;
      }
    } catch (error) {
      const failure = asStreamError(error);
      this.failed = failure;
      throw failure;
    }
  }

  private async finishOnce(): Promise<Transcript | 'restarted'> {
    const streamId = this.id as string;
    let transient = 0;
    let result: DictationResult<Transcript | TranscriptPending>;
    for (;;) {
      if (this.cancelled) throw CANCELLED();
      try {
        result = await this.timed(
          (signal) => this.api.finish(streamId, signal),
          FINISH_TIMEOUT_MS,
          false
        );
        break;
      } catch (error) {
        if (this.cancelled) throw CANCELLED();
        if (++transient > MAX_TRANSIENT_FAILURES) throw new DictationStreamError('lost');
        if (!(error instanceof RequestTimeout)) await this.sleep(POLL_EVERY_MS * transient);
      }
    }

    transient = 0;
    for (;;) {
      if (result.ok && !isTranscriptPending(result.data)) return result.data;
      if (!result.ok) {
        if (result.status === 410) {
          await this.recover();
          return 'restarted';
        }
        if (result.status === 503) {
          throw new DictationStreamError('serverMemory', result.error.message, {
            status: 503,
            serverCode: result.error.code,
          });
        }
        if (result.status >= 500) {
          throw new DictationStreamError('engineFailed', result.error.message, {
            status: result.status,
            serverCode: result.error.code,
          });
        }
        throw new DictationStreamError('lost', '', {
          status: result.status,
          serverCode: result.error.code,
        });
      }
      // 202: poll every 500 ms; each poll answers at once.
      await this.sleep(POLL_EVERY_MS);
      try {
        result = await this.timed((signal) => this.api.poll(streamId, signal), PUT_TIMEOUT_MS);
        transient = 0;
      } catch {
        if (this.cancelled) throw CANCELLED();
        if (++transient > MAX_TRANSIENT_FAILURES) throw new DictationStreamError('lost');
        result = { ok: true, status: 202, data: { pending: true } };
      }
    }
  }

  async retry(): Promise<Transcript> {
    if (this.cancelled || this.released) throw CANCELLED();
    if (!this.failed?.retryable) throw this.failed ?? new DictationStreamError('lost');
    this.failed = null;
    this.finishing = null;
    this.closing = true;
    // The person asked again: a busy server gets its full wait again.
    this.closedAt = Date.now();
    this.recoveries = 0;
    this.finishing = (async () => {
      try {
        await this.restart();
      } catch (error) {
        this.failed = asStreamError(error);
        throw this.failed;
      }
      return this.runFinish();
    })();
    this.finishing.catch(() => undefined);
    return this.finishing;
  }

  // ── ending ────────────────────────────────────────────────────────────────

  async cancel(): Promise<void> {
    if (this.cancelled) return;
    this.cancelled = true;
    this.abort.abort();
    this.wipe();
    this.setWaiting(false, true);
    const streamId = this.id;
    if (streamId) await this.api.drop(streamId);
  }

  release(): void {
    if (this.released || this.cancelled) return;
    this.released = true;
    this.wipe();
    if (this.id) void this.api.drop(this.id);
  }

  // ── timing ────────────────────────────────────────────────────────────────

  private setWaiting(on: boolean, reset = false): void {
    const before = this.waits > 0;
    if (reset) this.waits = 0;
    else this.waits = Math.max(0, this.waits + (on ? 1 : -1));
    const after = this.waits > 0;
    if (before !== after) this.waiting.emit(after);
  }

  /**
   * Run one request with its own deadline, aborted by a cancel too. With
   * `notice`, a request still unanswered after 1.5 s turns MSG-WAIT on until it
   * settles.
   */
  private timed<T>(
    send: (signal: AbortSignal) => Promise<T>,
    timeoutMs: number,
    notice = true
  ): Promise<T> {
    const controller = new AbortController();
    const onCancel = () => controller.abort();
    this.abort.signal.addEventListener('abort', onCancel, { once: true });
    let timedOut = false;
    let noticed = false;
    const deadline = setTimeout(() => {
      timedOut = true;
      controller.abort();
    }, timeoutMs);
    const waitTimer = notice
      ? setTimeout(() => {
          noticed = true;
          this.setWaiting(true);
        }, WAIT_NOTICE_MS)
      : null;
    const settle = () => {
      clearTimeout(deadline);
      if (waitTimer) clearTimeout(waitTimer);
      if (noticed) this.setWaiting(false);
      this.abort.signal.removeEventListener('abort', onCancel);
    };
    return send(controller.signal).then(
      (value) => {
        settle();
        if (timedOut) throw new RequestTimeout();
        return value;
      },
      (error: unknown) => {
        settle();
        throw timedOut ? new RequestTimeout() : error;
      }
    );
  }

  private sleep(ms: number): Promise<void> {
    return new Promise((resolve, reject) => {
      if (this.cancelled) return reject(CANCELLED());
      const onCancel = () => {
        clearTimeout(timer);
        reject(CANCELLED());
      };
      const timer = setTimeout(() => {
        this.abort.signal.removeEventListener('abort', onCancel);
        resolve();
      }, ms);
      this.abort.signal.addEventListener('abort', onCancel, { once: true });
    });
  }
}

/** Cut a string to at most `maxBytes` of UTF-8, on a character boundary. */
export function truncateUtf8(text: string, maxBytes: number): string {
  const encoder = new TextEncoder();
  if (encoder.encode(text).length <= maxBytes) return text;
  let bytes = 0;
  let out = '';
  for (const ch of text) {
    const size = encoder.encode(ch).length;
    if (bytes + size > maxBytes) break;
    bytes += size;
    out += ch;
  }
  return out;
}

/**
 * Start a stream now and return at once, so audio can be pushed while the
 * daemon is still answering the create. `ready` settles with that answer.
 */
export function createDictationStream(
  request: StreamRequest,
  options: StreamOptions = {}
): DictationStream {
  return new Stream(request, options);
}

/** As {@link createDictationStream}, settling once the daemon accepted the stream. */
export async function openDictationStream(
  request: StreamRequest,
  signal: AbortSignal
): Promise<DictationStream> {
  const stream = createDictationStream(request, { signal });
  await stream.ready;
  return stream;
}
