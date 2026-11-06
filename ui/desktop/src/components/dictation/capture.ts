/**
 * Microphone capture in a Biorouter page (DD-64, "Capture in a Biorouter page"
 * in `docs/dictation/surfaces.md`).
 *
 * `startCapture` is called inside the click or keydown that starts dictation,
 * and does its first three steps synchronously, before any `await`: the secure
 * context check, `getUserMedia`, and a 16 kHz `AudioContext` with `resume()`.
 * The worklet module starts loading in the same moment, so the device, the
 * context and the module come up in parallel and the first audio frame arrives
 * as early as the device allows. Nothing waits for the daemon.
 *
 * The worklet is imported as `?url&no-inline`, and `vite.renderer.config.mts`
 * also refuses to inline any `*.worklet.js`: Vite inlines assets under 4,096 B
 * as `data:` URLs, which the renderer's content security policy blocks, and
 * dictation would then die on the first click with `AbortError: Unable to load
 * a worklet's module` (measured in Electron, Chromium and Firefox). A source
 * guard pins both, and the `dictation-capture-electron` job loads the built
 * worklet from `app.asar` under the real policy.
 *
 * Fallbacks, in order: a `NotSupportedError` from `createMediaStreamSource`
 * (Firefox before 148 cannot connect a stream to a context at another rate)
 * reopens the context at the device's own rate, which the stream then declares
 * to the daemon; any failure to load the worklet, or to create its node once
 * loaded, records with `MediaRecorder` and decodes to 16 kHz on stop, with no
 * live level; an `OverconstrainedError` for a stale device id retries by the
 * device's label, then the default device.
 *
 * Every way out (stop, cancel, error, a track that ends) stops every track and
 * closes the context, so the operating system's microphone indicator goes off.
 */
import workletUrl from './pcmCapture.worklet.js?url&no-inline';

/** The rate the daemon wants; anything else is resampled there. */
export const CAPTURE_SAMPLE_RATE = 16000;
/** The worklet posts a chunk every 100 ms of audio. */
export const CAPTURE_CHUNK_SECONDS = 0.1;
/** The processor name `pcmCapture.worklet.js` registers. */
export const PCM_CAPTURE_PROCESSOR = 'biorouter-pcm-capture';
/** How long `stop()` waits for the worklet's last partial chunk. */
const FLUSH_TIMEOUT_MS = 250;

export interface CaptureLevel {
  rms: number;
  peak: number;
}

export interface CaptureOptions {
  /** The device the person chose in Settings; a stale id falls back by label, then default. */
  deviceId?: string;
  /** That device's label when it was chosen, for the fallback. */
  deviceLabel?: string;
  /** "Reduce background noise", a per viewer choice, default false. */
  noiseSuppression: boolean;
  /** 100 ms of mono 16-bit PCM at `sampleRate`, with its level. */
  onChunk(pcm: Int16Array, level: CaptureLevel): void;
  /** The first render quantum that carries the microphone's input. */
  onLive?(): void;
}

/** Milliseconds from the `startCapture` call, for latency measurements. */
export interface CaptureTimings {
  streamMs?: number;
  moduleMs?: number;
  connectedMs?: number;
  liveMs?: number;
  firstChunkMs?: number;
}

export interface Capture {
  /** 16000, or the device's rate on the Firefox fallback. */
  readonly sampleRate: number;
  /** `'recorder'` has no live level and delivers its audio on stop. */
  readonly mode: 'worklet' | 'recorder';
  readonly timings: CaptureTimings;
  /** Delivers the last partial chunk through `onChunk`, ends the tracks and starts closing the context. */
  stop(): Promise<void>;
  /** Ends the tracks and closes the context; no further chunk is delivered. */
  cancel(): void;
  /** `'track-ended'` when the device went away (a headset unplugged) before a stop or cancel. */
  readonly ended: Promise<'stopped' | 'track-ended'>;
  /** Settles once the context is closed. */
  readonly closed: Promise<void>;
}

/** Thrown before any request or prompt when the page cannot reach a microphone at all. */
export class InsecureContextError extends Error {
  constructor() {
    super('Dictation needs a secure context with getUserMedia.');
    this.name = 'InsecureContextError';
  }
}

/** Whether this page could ever capture: a secure context with `getUserMedia`. */
export function captureSupported(): boolean {
  return (
    typeof window !== 'undefined' &&
    window.isSecureContext === true &&
    typeof navigator !== 'undefined' &&
    typeof navigator.mediaDevices?.getUserMedia === 'function'
  );
}

/** A warm-up is repeated at most this often. */
export const PREWARM_EVERY_MS = 30_000;
let lastPrewarm: number | null = null;

/**
 * Warm the browser's media device list before the press, so the press does not
 * pay for it. Called when dictation becomes likely (a composer's field takes
 * focus, the pointer reaches a microphone; `dictationStore.ts`
 * `noteDictationInterest`).
 *
 * `enumerateDevices()` opens no device, shows no microphone indicator and asks
 * for no permission. Measured in the `dictation-capture-electron` probe on an
 * Apple Silicon Mac (Electron 39.8.10, fake device): the first press in a fresh
 * window took 222 to 463 ms from the press to `getUserMedia` resolving, and 30 to
 * 38 ms after this warm-up. Warming an `AudioContext` instead only reached 91 ms.
 */
export function prewarmCapture(): void {
  if (!captureSupported() || typeof navigator.mediaDevices.enumerateDevices !== 'function') return;
  const t = now();
  if (lastPrewarm !== null && t - lastPrewarm < PREWARM_EVERY_MS) return;
  lastPrewarm = t;
  void navigator.mediaDevices.enumerateDevices().catch(() => undefined);
}

/** For tests: forget the last warm-up. */
export function resetCapturePrewarmForTests(): void {
  lastPrewarm = null;
}

const errorName = (error: unknown): string =>
  error && typeof error === 'object' && 'name' in error ? String(error.name) : '';

const now = (): number =>
  typeof performance !== 'undefined' && typeof performance.now === 'function'
    ? performance.now()
    : Date.now();

function audioConstraints(options: CaptureOptions, deviceId?: string): MediaStreamConstraints {
  return {
    audio: {
      ...(deviceId ? { deviceId: { exact: deviceId } } : {}),
      echoCancellation: false,
      noiseSuppression: options.noiseSuppression,
      // Chromium's gain control on Windows rewrites the system microphone volume.
      autoGainControl: false,
    },
  };
}

/**
 * `getUserMedia`, called synchronously by the first line, with the stale device
 * fallback after it.
 */
async function openStream(options: CaptureOptions): Promise<MediaStream> {
  const devices = navigator.mediaDevices;
  try {
    return await devices.getUserMedia(audioConstraints(options, options.deviceId));
  } catch (error) {
    if (!options.deviceId || errorName(error) !== 'OverconstrainedError') throw error;
  }
  if (options.deviceLabel) {
    try {
      const match = (await devices.enumerateDevices()).find(
        (device) => device.kind === 'audioinput' && device.label === options.deviceLabel
      );
      if (match && match.deviceId !== options.deviceId) {
        return await devices.getUserMedia(audioConstraints(options, match.deviceId));
      }
    } catch (error) {
      if (errorName(error) !== 'OverconstrainedError') throw error;
    }
  }
  return devices.getUserMedia(audioConstraints(options));
}

function stopTracks(stream: MediaStream): void {
  for (const track of stream.getTracks()) {
    try {
      track.stop();
    } catch {
      // A track that is already gone has nothing left to stop.
    }
  }
}

function closeContext(context: AudioContext | null): Promise<void> {
  if (!context || context.state === 'closed') return Promise.resolve();
  return context.close().then(
    () => undefined,
    () => undefined
  );
}

/**
 * Start capturing. Call inside the gesture; the promise settles once audio is
 * flowing into the worklet (or the recorder), and rejects with the
 * `getUserMedia` error or an {@link InsecureContextError}.
 */
export function startCapture(options: CaptureOptions): Promise<Capture> {
  const t0 = now();
  if (!captureSupported()) return Promise.reject(new InsecureContextError());

  // Steps 1 to 3 of the capture sequence, before any await.
  const streamPromise = openStream(options);
  let context: AudioContext | null = null;
  let modulePromise: Promise<void>;
  try {
    context = new AudioContext({ sampleRate: CAPTURE_SAMPLE_RATE, latencyHint: 'interactive' });
    void context.resume().catch(() => undefined);
    modulePromise = context.audioWorklet
      ? context.audioWorklet.addModule(workletUrl)
      : Promise.reject(new DOMException('No AudioWorklet', 'NotSupportedError'));
  } catch (error) {
    modulePromise = Promise.reject(error);
  }
  // Settled here so a rejection is never reported as unhandled while the stream is awaited.
  const moduleLoaded = modulePromise.then(
    () => true,
    () => false
  );
  return assemble(options, t0, streamPromise, context, moduleLoaded);
}

async function assemble(
  options: CaptureOptions,
  t0: number,
  streamPromise: Promise<MediaStream>,
  firstContext: AudioContext | null,
  firstModule: Promise<boolean>
): Promise<Capture> {
  const timings: CaptureTimings = {};
  let stream: MediaStream;
  try {
    stream = await streamPromise;
  } catch (error) {
    void closeContext(firstContext);
    throw error;
  }
  timings.streamMs = now() - t0;

  let context = firstContext;
  let loaded = await firstModule;
  timings.moduleMs = now() - t0;
  if (!context || !loaded) {
    void closeContext(context);
    return startRecorder(stream, options, timings, t0);
  }

  let source: MediaStreamAudioSourceNode;
  try {
    source = context.createMediaStreamSource(stream);
  } catch (error) {
    void closeContext(context);
    if (errorName(error) !== 'NotSupportedError') {
      stopTracks(stream);
      throw error;
    }
    // Firefox before 148: connect at the device's own rate instead.
    try {
      context = new AudioContext({ latencyHint: 'interactive' });
      // Not awaited: this runs after the gesture, and a resume that never
      // settles must not hold the capture; audio flows once it runs.
      void context.resume().catch(() => undefined);
      loaded = await context.audioWorklet.addModule(workletUrl).then(
        () => true,
        () => false
      );
      if (!loaded) {
        void closeContext(context);
        return startRecorder(stream, options, timings, t0);
      }
      source = context.createMediaStreamSource(stream);
    } catch (retryError) {
      void closeContext(context);
      stopTracks(stream);
      throw retryError;
    }
  }

  let node: AudioWorkletNode | null = null;
  try {
    node = new AudioWorkletNode(context, PCM_CAPTURE_PROCESSOR, {
      channelCount: 1,
      channelCountMode: 'explicit',
      processorOptions: { chunkFrames: Math.round(context.sampleRate * CAPTURE_CHUNK_SECONDS) },
    });
    const capture = new WorkletCapture(context, stream, source, node, options, timings, t0);
    // Never connected to the destination: an unconnected worklet node is still
    // processed (measured), and nothing is played back.
    source.connect(node);
    timings.connectedMs = now() - t0;
    return capture;
  } catch {
    // The module loaded but the node cannot run (an `InvalidStateError` when
    // the processor is not registered, for one): the worklet is as unusable as
    // one that never loaded, so this records the same way. `startRecorder` ends
    // the tracks itself if it cannot record either, so no exit leaves the
    // microphone on.
    try {
      source.disconnect();
      node?.disconnect();
    } catch {
      // Nothing was connected.
    }
    if (node) node.port.onmessage = null;
    void closeContext(context);
    return startRecorder(stream, options, timings, t0);
  }
}

class Ending {
  readonly promise: Promise<'stopped' | 'track-ended'>;
  private resolveFn!: (why: 'stopped' | 'track-ended') => void;
  private settled = false;
  constructor(stream: MediaStream) {
    this.promise = new Promise((resolve) => (this.resolveFn = resolve));
    for (const track of stream.getAudioTracks()) {
      track.addEventListener('ended', () => this.resolve('track-ended'), { once: true });
    }
  }
  resolve(why: 'stopped' | 'track-ended'): void {
    if (this.settled) return;
    this.settled = true;
    this.resolveFn(why);
  }
}

class WorkletCapture implements Capture {
  readonly mode = 'worklet' as const;
  readonly sampleRate: number;
  readonly ended: Promise<'stopped' | 'track-ended'>;
  readonly closed: Promise<void>;
  private markClosed!: () => void;
  private readonly ending: Ending;
  private released = false;
  private cancelled = false;
  private stopping: Promise<void> | null = null;
  private flushed: (() => void) | null = null;

  constructor(
    private readonly context: AudioContext,
    private readonly stream: MediaStream,
    private readonly source: MediaStreamAudioSourceNode,
    private readonly node: AudioWorkletNode,
    private readonly options: CaptureOptions,
    readonly timings: CaptureTimings,
    private readonly t0: number
  ) {
    this.sampleRate = context.sampleRate;
    this.ending = new Ending(stream);
    this.ended = this.ending.promise;
    this.closed = new Promise<void>((resolve) => (this.markClosed = resolve));
    node.port.onmessage = (event: MessageEvent) => this.onMessage(event.data);
  }

  private onMessage(data: unknown): void {
    if (this.cancelled || !data || typeof data !== 'object') return;
    const message = data as {
      type?: string;
      pcm?: ArrayBuffer;
      rms?: number;
      peak?: number;
      final?: boolean;
    };
    if (message.type === 'live') {
      this.timings.liveMs ??= now() - this.t0;
      this.options.onLive?.();
      return;
    }
    if (message.type !== 'chunk' || !(message.pcm instanceof ArrayBuffer)) return;
    const pcm = new Int16Array(message.pcm);
    if (pcm.length > 0) {
      this.timings.firstChunkMs ??= now() - this.t0;
      this.options.onChunk(pcm, { rms: message.rms ?? 0, peak: message.peak ?? 0 });
    }
    if (message.final) this.flushed?.();
  }

  stop(): Promise<void> {
    if (this.stopping) return this.stopping;
    this.stopping = new Promise<void>((resolve) => {
      if (this.released) return resolve();
      const timer = setTimeout(() => resolve(), FLUSH_TIMEOUT_MS);
      this.flushed = () => {
        clearTimeout(timer);
        resolve();
      };
      this.node.port.postMessage({ type: 'flush' });
    }).then(() => this.release());
    return this.stopping;
  }

  cancel(): void {
    this.cancelled = true;
    this.release();
  }

  private release(): void {
    if (this.released) return;
    this.released = true;
    this.ending.resolve('stopped');
    stopTracks(this.stream);
    try {
      this.source.disconnect();
      this.node.disconnect();
    } catch {
      // Already disconnected.
    }
    this.node.port.onmessage = null;
    void closeContext(this.context).then(this.markClosed);
  }
}

/**
 * The fallback when the worklet cannot load: record compressed audio, and on
 * stop decode it, downmix and resample it to 16 kHz mono with an
 * `OfflineAudioContext`, then deliver it in 100 ms chunks. No live level and no
 * upload while recording.
 */
function startRecorder(
  stream: MediaStream,
  options: CaptureOptions,
  timings: CaptureTimings,
  t0: number
): Promise<Capture> {
  if (typeof MediaRecorder === 'undefined') {
    stopTracks(stream);
    return Promise.reject(new DOMException('MediaRecorder is not available', 'NotSupportedError'));
  }
  return new Promise<Capture>((resolve, reject) => {
    let recorder: MediaRecorder;
    try {
      recorder = new MediaRecorder(stream);
    } catch (error) {
      stopTracks(stream);
      reject(error);
      return;
    }
    const capture = new RecorderCapture(recorder, stream, options, timings, t0);
    recorder.addEventListener(
      'start',
      () => {
        timings.connectedMs = now() - t0;
        timings.liveMs = timings.connectedMs;
        options.onLive?.();
        resolve(capture);
      },
      { once: true }
    );
    recorder.addEventListener(
      'error',
      (event) => {
        stopTracks(stream);
        reject((event as Event & { error?: unknown }).error ?? new Error('MediaRecorder failed'));
      },
      { once: true }
    );
    recorder.start();
  });
}

class RecorderCapture implements Capture {
  readonly mode = 'recorder' as const;
  readonly sampleRate = CAPTURE_SAMPLE_RATE;
  readonly ended: Promise<'stopped' | 'track-ended'>;
  readonly closed: Promise<void> = Promise.resolve();
  private readonly ending: Ending;
  private readonly parts: Blob[] = [];
  private stopping: Promise<void> | null = null;
  private cancelled = false;

  constructor(
    private readonly recorder: MediaRecorder,
    private readonly stream: MediaStream,
    private readonly options: CaptureOptions,
    readonly timings: CaptureTimings,
    private readonly t0: number
  ) {
    this.ending = new Ending(stream);
    this.ended = this.ending.promise;
    recorder.addEventListener('dataavailable', (event) => {
      if (event.data && event.data.size > 0) this.parts.push(event.data);
    });
  }

  stop(): Promise<void> {
    if (this.stopping) return this.stopping;
    this.stopping = new Promise<void>((resolve) => {
      if (this.recorder.state === 'inactive') return resolve();
      this.recorder.addEventListener('stop', () => resolve(), { once: true });
      this.recorder.stop();
    })
      .then(() => {
        this.ending.resolve('stopped');
        stopTracks(this.stream);
      })
      .then(() => (this.cancelled ? undefined : this.decodeAndDeliver()));
    return this.stopping;
  }

  cancel(): void {
    this.cancelled = true;
    this.parts.length = 0;
    this.ending.resolve('stopped');
    try {
      if (this.recorder.state !== 'inactive') this.recorder.stop();
    } catch {
      // Already stopped.
    }
    stopTracks(this.stream);
  }

  private async decodeAndDeliver(): Promise<void> {
    if (this.parts.length === 0) return;
    const encoded = await new Blob(this.parts, { type: this.recorder.mimeType }).arrayBuffer();
    this.parts.length = 0;
    const decoder = new OfflineAudioContext(1, 1, CAPTURE_SAMPLE_RATE);
    const decoded = await decoder.decodeAudioData(encoded);
    const frames = Math.max(1, Math.ceil(decoded.duration * CAPTURE_SAMPLE_RATE));
    const offline = new OfflineAudioContext(1, frames, CAPTURE_SAMPLE_RATE);
    const source = offline.createBufferSource();
    source.buffer = decoded;
    source.connect(offline.destination);
    source.start();
    const mono = (await offline.startRendering()).getChannelData(0);
    const size = Math.round(CAPTURE_SAMPLE_RATE * CAPTURE_CHUNK_SECONDS);
    for (let offset = 0; offset < mono.length && !this.cancelled; offset += size) {
      const slice = mono.subarray(offset, Math.min(mono.length, offset + size));
      const pcm = new Int16Array(slice.length);
      let sum = 0;
      let peak = 0;
      for (let i = 0; i < slice.length; i++) {
        const s = Math.max(-1, Math.min(1, slice[i]));
        sum += s * s;
        peak = Math.max(peak, Math.abs(s));
        pcm[i] = Math.round(s * 32767);
      }
      this.timings.firstChunkMs ??= now() - this.t0;
      this.options.onChunk(pcm, { rms: Math.sqrt(sum / Math.max(1, slice.length)), peak });
    }
  }
}
