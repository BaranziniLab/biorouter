/**
 * Fakes shared by the microphone's component tests: a capture, a stream and a
 * status API that record what the session and the store ask of them. Whether a
 * real browser does what they stand for is the Electron job's question
 * (`scripts/dictation-capture-e2e.mjs`).
 */
import { vi } from 'vitest';
import type { Capture, CaptureOptions } from './capture';
import type { DictationApi, DictationStatus, StreamRequest, Transcript } from './dictationApi';
import type { DictationStream } from './transport';

export const RATE = 16000;

export class Deferred<T> {
  promise: Promise<T>;
  resolve!: (value: T) => void;
  reject!: (error: unknown) => void;
  constructor() {
    this.promise = new Promise<T>((resolve, reject) => {
      this.resolve = resolve;
      this.reject = reject;
    });
  }
}

export function transcript(text: string, extra: Partial<Transcript> = {}): Transcript {
  return {
    text,
    raw_text: text,
    changes: [],
    segments: [],
    model: 'whisper-small.en-q8_0',
    audio_s: 1,
    elapsed_ms: 80,
    bias_scope: 'public',
    retried: null,
    no_speech: false,
    ...extra,
  };
}

export class FakeCapture implements Capture {
  readonly mode = 'worklet' as const;
  readonly sampleRate = RATE;
  readonly timings = {};
  stopped = false;
  cancelled = false;
  private readonly endedDeferred = new Deferred<'stopped' | 'track-ended'>();
  readonly ended = this.endedDeferred.promise;
  readonly closed = Promise.resolve();
  constructor(readonly options: CaptureOptions) {}
  speak(seconds: number, amplitude = 8000): void {
    const chunk = RATE / 10;
    for (let done = 0; done < seconds * RATE; done += chunk) {
      this.options.onChunk(new Int16Array(chunk).fill(amplitude), {
        rms: amplitude / 32768,
        peak: amplitude / 32768,
      });
    }
  }
  live(): void {
    this.options.onLive?.();
  }
  get lastChunkDelivered(): boolean {
    return this.stopped;
  }
  stop(): Promise<void> {
    this.stopped = true;
    this.endedDeferred.resolve('stopped');
    return Promise.resolve();
  }
  cancel(): void {
    this.cancelled = true;
    this.endedDeferred.resolve('stopped');
  }
}

export class FakeStream implements DictationStream {
  id: string | null = 'stream-1';
  ready = Promise.resolve();
  samples = 0;
  finishDeferred = new Deferred<Transcript>();
  finishCalls = 0;
  released = false;
  cancelled = false;
  constructor(readonly request: StreamRequest) {}
  get audioSeconds(): number {
    return this.samples / RATE;
  }
  push(pcm: Int16Array): void {
    this.samples += pcm.length;
  }
  finish(): Promise<Transcript> {
    this.finishCalls++;
    return this.finishDeferred.promise;
  }
  retry(): Promise<Transcript> {
    return this.finishDeferred.promise;
  }
  async cancel(): Promise<void> {
    this.cancelled = true;
  }
  release(): void {
    this.released = true;
  }
  private none = () => () => {};
  onPartial = this.none;
  onWaiting = this.none;
  onCapWarning = this.none;
  onCapReached = this.none;
  onBusy = this.none;
  onFailed = this.none;
}

type StatusModel = DictationStatus['models'][number];

export function speechStatus(
  overrides: Partial<DictationStatus> = {},
  model: Partial<StatusModel> = {}
): DictationStatus {
  const allowed = { allowed: true, reason: null };
  return {
    engine: {
      compiled: true,
      state: 'ready',
      backend: 'metal',
      device: 'Apple M4 Max',
      threads: 8,
      missing_cpu_features: [],
      rss_bytes: null,
    },
    models: [
      {
        id: 'whisper-small.en-q8_0',
        display_name: 'Whisper small.en (Light)',
        installed: true,
        bytes: 264_477_561,
        location: 'user',
        superseded: false,
        suitability: 'recommended',
        recommended: true,
        ...model,
      },
    ],
    active_model: 'whisper-small.en-q8_0',
    language: 'en',
    install: allowed,
    install_job: null,
    policy: {
      managed: false,
      enabled: allowed,
      capture: allowed,
      install: allowed,
      external_engine: allowed,
      dictionary: allowed,
      agent_writes: allowed,
      allowed_models: null,
    },
    limits: { max_seconds: 600, max_chunk_bytes: 524_288, max_streams: 4 },
    queue: { streams: 0, segments: 0 },
    account: 'wgu',
    host: 'this-mac',
    models_dir: '/Users/wgu/.local/share/biorouter/dictation/models',
    ...overrides,
  } as DictationStatus;
}

/** A status API whose answer the test decides; every read is counted. */
export function fakeStatusApi(status: () => DictationStatus | null) {
  const reads = vi.fn();
  const api = {
    status: async () => {
      reads();
      const value = status();
      return value
        ? { ok: true as const, status: 200, data: value }
        : {
            ok: false as const,
            status: 500,
            error: { code: 'http500', message: '' },
            retryAfterS: null,
          };
    },
    createStream: vi.fn(),
    putAudio: vi.fn(),
    finish: vi.fn(),
    poll: vi.fn(),
    drop: vi.fn(async () => undefined),
  } as unknown as DictationApi;
  return { api, reads };
}

/** A page the capture checks accept: a secure context with `getUserMedia`. */
export function secureMediaPage(): void {
  Object.defineProperty(window, 'isSecureContext', { value: true, configurable: true });
  Object.defineProperty(navigator, 'mediaDevices', {
    value: {
      getUserMedia: vi.fn(),
      enumerateDevices: vi.fn(async () => []),
    },
    configurable: true,
  });
}
