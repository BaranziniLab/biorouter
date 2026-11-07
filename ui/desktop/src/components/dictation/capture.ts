/**
 * Microphone capture in a Biorouter page (DD-64, "Capture in a Biorouter page"
 * in `docs/dictation/surfaces.md`).
 *
 * `startCapture` is called inside the click or keydown that starts dictation.
 * It checks for a secure context and calls `getUserMedia` before anything else,
 * because a 16 kHz `AudioContext` made or resumed beside it holds the device
 * back. Measured in the dev app on an Apple Silicon Mac (Electron 39.8.10, the
 * fake device, real presses, PR-19's live check, 2026-11-07), press to the first
 * audio frame, eight presses each:
 *
 * - context made and module loaded beside `getUserMedia` (the first PR-19 draft):
 *   88 to 100 ms, `getUserMedia` itself 87 to 99 ms;
 * - context made before the press, module loaded and suspended, resumed beside
 *   `getUserMedia`: 44 to 53 ms, `getUserMedia` 44 to 48 ms;
 * - that context resumed once `getUserMedia` has resolved, as shipped: 24 to
 *   35 ms, median 27, `getUserMedia` 21 to 32 ms; the device alone takes 23 to
 *   25 ms.
 *
 * So the context is made before the press ({@link prewarmCapture}), and resumed
 * after the stream where contexts may start without a gesture: the desktop app,
 * and a browser page whose prepared context started on its own. Elsewhere
 * (WebKit's `resume()` never settled without a gesture) it is resumed inside the
 * gesture, beside `getUserMedia`, before any `await`. A press with no prepared
 * context makes one after the stream where it may, and inside the gesture
 * otherwise. Nothing waits for the daemon.
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
 * A context made before the press holds no microphone: it is suspended, so it
 * renders nothing and holds no audio device.
 */
import { isBrowserSurface } from '../../utils/surface';
import workletUrl from './pcmCapture.worklet.js?url&no-inline';

/** The rate the daemon wants; anything else is resampled there. */
export const CAPTURE_SAMPLE_RATE = 16000;
/** The worklet posts a chunk every 100 ms of audio. */
export const CAPTURE_CHUNK_SECONDS = 0.1;
/** The processor name `pcmCapture.worklet.js` registers. */
export const PCM_CAPTURE_PROCESSOR = 'biorouter-pcm-capture';
/**
 * After a stop, how long the context stays open for the worklet to say it has
 * ended. Nothing on the stop path waits for it, except audio recorded before
 * the stop that is still on its way (see {@link STOP_BACKLOG_SLACK_MS}), which
 * this also bounds.
 */
const FLUSH_TIMEOUT_MS = 250;
/**
 * At a stop, audio the worklet posted before the stop that this thread has not
 * received yet is waited for only when there is more of it than this, two
 * render quanta at 16 kHz. Within it, the stop hands over what it holds at once
 * and drops the rest, at most the last 16 ms before the press. Above it, the
 * main thread was busy (a render, a long task) while the person spoke, the
 * messages are queued behind it, and dropping them would cut the end of the
 * utterance.
 *
 * Measured in Chrome 157 with the fake microphone and real presses (PR-19
 * review, 2026-11-07): at 20 presses on an idle page nothing was on its way
 * (0 ms every time), so the stop never waited; after a 150 ms long task just
 * before the press, 144 to 152 ms was on its way at each of 10 presses, which a
 * stop that discarded every message after it would have lost, and this one
 * waited 0.5 to 0.7 ms for it, because the messages are already queued when the
 * press is handled.
 */
export const STOP_BACKLOG_SLACK_MS = 16;

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
  /**
   * Worklet mode, at the stop: milliseconds of audio the worklet had posted
   * that this thread had not received yet, by the context's clock.
   */
  stopBacklogMs?: number;
  /** Worklet mode: how long the stop waited for that audio; 0 within the slack. */
  stopWaitMs?: number;
}

export interface Capture {
  /** 16000, or the device's rate on the Firefox fallback. */
  readonly sampleRate: number;
  /** `'recorder'` has no live level and delivers its audio on stop. */
  readonly mode: 'worklet' | 'recorder';
  readonly timings: CaptureTimings;
  /**
   * Delivers the last partial chunk through `onChunk`, ends the tracks and starts closing the
   * context. In `'worklet'` mode the chunk is usually delivered before `stop()` returns, with the
   * promise already settled; when audio recorded before the stop is still queued behind a busy
   * main thread, more than {@link STOP_BACKLOG_SLACK_MS} of it, the promise settles once that audio
   * has been delivered (at most {@link FLUSH_TIMEOUT_MS} later). `'recorder'` decodes first.
   */
  stop(): Promise<void>;
  /**
   * Whether the last chunk has been delivered: false while recording, and in `'worklet'` mode set
   * inside `stop()` unless it waits for audio still on its way, so a caller can ask for the
   * transcript in the same task as the stop.
   */
  readonly lastChunkDelivered: boolean;
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

/** The device list warm-up is repeated at most this often. */
export const PREWARM_EVERY_MS = 30_000;
let lastPrewarm: number | null = null;

/** A capture context and its worklet module, made before the press or inside it. */
interface CaptureContext {
  readonly context: AudioContext;
  /** Settles true once the module loaded, false when it cannot load. */
  readonly module: Promise<boolean>;
}

/** The context made before the press, waiting for it; at most one per page. */
let prepared: CaptureContext | null = null;
/** Set once a prepared context's module failed to load: no other is made in this page. */
let workletRefused = false;
/** Captures started and not yet ended: no context is made beside a live microphone. */
let liveCaptures = 0;
/**
 * Set once a context made outside a gesture was seen running on its own: this
 * page's contexts may start without one, so a press can wait for the device
 * before it starts its context.
 */
let startsWithoutGesture = false;

/**
 * Whether a context may be started after the gesture, once `getUserMedia` has
 * resolved: always in the desktop app, whose Electron starts a context without
 * a gesture, and in a browser once a context was seen doing so (Chromium and
 * Firefox after the page's first gesture). WebKit's `resume()` never settled
 * without a gesture [M], so a browser that has not shown it keeps `resume()`
 * inside the gesture.
 */
function contextMayStartAfterGesture(): boolean {
  return startsWithoutGesture || !isBrowserSurface();
}

/**
 * Prepare the press before it comes, so it pays only for opening the device.
 * Called when dictation becomes likely (a composer's field takes focus, the
 * pointer reaches a microphone; `dictationStore.ts` `noteDictationInterest`),
 * and by the session once a dictation has ended in a composer that is still
 * there, since the next one may follow without either.
 *
 * - `enumerateDevices()`, at most every {@link PREWARM_EVERY_MS}: the first
 *   `getUserMedia` in a window pays for the browser's first device enumeration.
 *   Measured in the `dictation-capture-electron` probe on an Apple Silicon Mac
 *   (Electron 39.8.10, fake device): the first press in a fresh window took 222
 *   to 463 ms from the press to `getUserMedia` resolving, and 30 to 38 ms after
 *   this warm-up.
 * - A 16 kHz `AudioContext` with the worklet module loaded, then suspended,
 *   when none is waiting and no capture is live. The press then only resumes
 *   it ({@link startCapture} says when). A suspended context renders nothing
 *   and holds no audio device. In a browser (`biorouter serve`) it is made only
 *   once the page has had a user gesture, because a context made before one
 *   starts suspended with a console warning; the desktop app lets a context
 *   start without one.
 *
 * Nothing here opens a microphone, shows an indicator or asks for permission.
 */
export function prewarmCapture(): void {
  if (!captureSupported()) return;
  if (typeof navigator.mediaDevices.enumerateDevices === 'function') {
    const t = now();
    if (lastPrewarm === null || t - lastPrewarm >= PREWARM_EVERY_MS) {
      lastPrewarm = t;
      void navigator.mediaDevices.enumerateDevices().catch(() => undefined);
    }
  }
  prepareContext();
}

function prepareContext(): void {
  if (workletRefused || liveCaptures > 0) return;
  if (prepared && prepared.context.state !== 'closed') return;
  if (isBrowserSurface() && !pageHasBeenActive()) return;
  const made = makeContext();
  if (!made) return;
  prepared = made;
  void made.module.then((loaded) => {
    // A press that took it in the meantime is using it: leave it running.
    if (prepared !== made) return;
    if (!loaded) {
      prepared = null;
      workletRefused = true;
      void closeContext(made.context);
      return;
    }
    if (made.context.state === 'running') {
      startsWithoutGesture = true;
      void made.context.suspend().catch(() => undefined);
    }
  });
}

function pageHasBeenActive(): boolean {
  const activation = (navigator as Navigator & { userActivation?: { hasBeenActive?: boolean } })
    .userActivation;
  return activation?.hasBeenActive !== false;
}

/** A 16 kHz context with its module loading, or null where none can be made. */
function makeContext(): CaptureContext | null {
  if (typeof AudioContext !== 'function') return null;
  let context: AudioContext;
  try {
    context = new AudioContext({ sampleRate: CAPTURE_SAMPLE_RATE, latencyHint: 'interactive' });
  } catch {
    return null;
  }
  let module: Promise<void>;
  try {
    module = context.audioWorklet
      ? context.audioWorklet.addModule(workletUrl)
      : Promise.reject(new DOMException('No AudioWorklet', 'NotSupportedError'));
  } catch (error) {
    module = Promise.reject(error);
  }
  // Settled here so a rejection is never reported as unhandled while it waits.
  return {
    context,
    module: module.then(
      () => true,
      () => false
    ),
  };
}

/** The prepared context, unless it was closed under it (by the browser, or a test). */
function takePrepared(): CaptureContext | null {
  const taken = prepared;
  prepared = null;
  return taken && taken.context.state !== 'closed' ? taken : null;
}

/** For tests: forget the last warm-up, and close and forget a prepared context. */
export function resetCapturePrewarmForTests(): void {
  lastPrewarm = null;
  if (prepared) void closeContext(prepared.context);
  prepared = null;
  workletRefused = false;
  liveCaptures = 0;
  startsWithoutGesture = false;
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

  // Steps 1 to 3 of the capture sequence: the device first, before any await;
  // then the context prepared before the press, or a new one. Where a context
  // may start after the gesture it is made or resumed once the stream is here,
  // so it does not hold the device back; elsewhere both happen here, inside the
  // gesture, beside `getUserMedia`.
  const streamPromise = openStream(options);
  const afterStream = contextMayStartAfterGesture();
  const audio = takePrepared() ?? (afterStream ? null : makeContext());
  if (audio && !afterStream) resume(audio.context);
  liveCaptures++;
  const assembled = assemble(options, t0, streamPromise, audio, afterStream);
  // The microphone is live until the capture ends, or never was when it failed.
  assembled.then((started) => void started.ended.then(captureEnded), captureEnded);
  return assembled;
}

function captureEnded(): void {
  liveCaptures = Math.max(0, liveCaptures - 1);
}

function resume(context: AudioContext): void {
  try {
    void context.resume().catch(() => undefined);
  } catch {
    // A context that cannot resume fails at the source or the node below.
  }
}

async function assemble(
  options: CaptureOptions,
  t0: number,
  streamPromise: Promise<MediaStream>,
  first: CaptureContext | null,
  startAfterStream: boolean
): Promise<Capture> {
  const timings: CaptureTimings = {};
  let stream: MediaStream;
  try {
    stream = await streamPromise;
  } catch (error) {
    void closeContext(first?.context ?? null);
    throw error;
  }
  timings.streamMs = now() - t0;

  let audio = first;
  if (startAfterStream) {
    audio ??= makeContext();
    if (audio) resume(audio.context);
  }
  let context = audio?.context ?? null;
  let loaded = audio ? await audio.module : false;
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

/**
 * The worklet posts every render quantum (128 frames, 8 ms at 16 kHz) as it is
 * made, stamped with the context's frame, and this side assembles the 100 ms
 * chunks, so on stop the page usually holds the audio already and hands over
 * the last partial chunk at once. The stop used to ask the worklet for its
 * buffered samples and wait for the answer, which Chromium delivers only after
 * it has rendered the frame that follows the click: 0.7 to 10.2 ms on the stop
 * path, 8 ms in PR-15's live check, the largest share of stop to text the
 * client added.
 *
 * What has not arrived at the stop is bounded by how late this thread receives
 * the worklet's messages, not by one quantum: messages posted while the main
 * thread was busy wait in its queue. The stop reads the context's clock, and
 * the frame stamps say how much audio recorded before that moment is still on
 * its way. Up to {@link STOP_BACKLOG_SLACK_MS} of it is dropped, and the stop
 * does not wait; more than that is taken as it arrives (it is queued already)
 * until the stamps reach the stop's frame, the worklet's `final` answer comes,
 * or {@link FLUSH_TIMEOUT_MS} passes, and only then is the last chunk handed
 * over. `timings.stopBacklogMs` and `timings.stopWaitMs` record both.
 */
class WorkletCapture implements Capture {
  readonly mode = 'worklet' as const;
  readonly sampleRate: number;
  readonly ended: Promise<'stopped' | 'track-ended'>;
  readonly closed: Promise<void>;
  private markClosed!: () => void;
  private readonly ending: Ending;
  private released = false;
  private cancelled = false;
  /** Set at the stop: from then on only audio recorded before it is taken. */
  private stopped = false;
  private stopping: Promise<void> | null = null;
  private flushed: (() => void) | null = null;
  lastChunkDelivered = false;
  private readonly chunk: Int16Array;
  private fill = 0;
  private sum = 0;
  private peak = 0;
  /** The context frame after the last quantum received, from the worklet's stamps. */
  private nextFrame: number | null = null;
  /** The context's frame at the stop. */
  private stopFrame = 0;
  /** While the stop waits for audio recorded before it: hands the last chunk over. */
  private caughtUp: (() => void) | null = null;

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
    this.chunk = new Int16Array(
      Math.max(1, Math.round(context.sampleRate * CAPTURE_CHUNK_SECONDS))
    );
    this.ending = new Ending(stream);
    this.ended = this.ending.promise;
    this.closed = new Promise<void>((resolve) => (this.markClosed = resolve));
    node.port.onmessage = (event: MessageEvent) => this.onMessage(event.data);
  }

  private onMessage(data: unknown): void {
    if (this.cancelled || !data || typeof data !== 'object') return;
    const message = data as { type?: string; pcm?: ArrayBuffer; frame?: unknown; final?: boolean };
    const frame = typeof message.frame === 'number' ? message.frame : null;
    if (message.type === 'live') {
      this.timings.liveMs ??= now() - this.t0;
      if (frame !== null) this.nextFrame ??= frame;
      if (!this.stopped) this.options.onLive?.();
      return;
    }
    if (message.type !== 'chunk' || !(message.pcm instanceof ArrayBuffer)) return;
    if (message.final) {
      // Messages arrive in order: everything the worklet posted is here.
      this.caughtUp?.();
      this.flushed?.();
      return;
    }
    const pcm = new Int16Array(message.pcm);
    if (!this.stopped) {
      this.take(pcm);
      if (frame !== null) this.nextFrame = frame + pcm.length;
      return;
    }
    // After the stop: audio recorded before it, while the stop waits for it.
    if (!this.caughtUp) return;
    if (frame === null || frame >= this.stopFrame) {
      this.caughtUp();
      return;
    }
    this.take(pcm);
    this.nextFrame = frame + pcm.length;
    if (this.nextFrame >= this.stopFrame) this.caughtUp();
  }

  /** Append one quantum, delivering each chunk as it fills. */
  private take(pcm: Int16Array): void {
    const size = this.chunk.length;
    for (let i = 0; i < pcm.length; i++) {
      const sample = pcm[i];
      const s = sample / 32767;
      this.sum += s * s;
      const a = s < 0 ? -s : s;
      if (a > this.peak) this.peak = a;
      this.chunk[this.fill++] = sample;
      if (this.fill === size) this.deliver();
    }
  }

  private deliver(): void {
    if (this.fill === 0) return;
    const pcm = this.chunk.slice(0, this.fill);
    const level = { rms: Math.sqrt(this.sum / this.fill), peak: this.peak };
    this.fill = 0;
    this.sum = 0;
    this.peak = 0;
    this.timings.firstChunkMs ??= now() - this.t0;
    this.options.onChunk(pcm, level);
  }

  /**
   * Hands over the last partial chunk through `onChunk`, ends the tracks, and
   * tells the worklet to end; the context closes once it has (or after
   * {@link FLUSH_TIMEOUT_MS}), off the stop path. The chunk goes before this
   * returns, with the promise settled, unless more than
   * {@link STOP_BACKLOG_SLACK_MS} of audio recorded before the stop is still on
   * its way; then it goes, and the promise settles, once that audio is here.
   */
  stop(): Promise<void> {
    if (this.stopping) return this.stopping;
    if (this.released) {
      this.lastChunkDelivered = true;
      this.stopping = Promise.resolve();
      return this.stopping;
    }
    const stopAt = now();
    this.stopped = true;
    this.stopFrame = Math.floor(this.context.currentTime * this.sampleRate);
    const backlog = this.nextFrame === null ? 0 : Math.max(0, this.stopFrame - this.nextFrame);
    this.timings.stopBacklogMs = (backlog / this.sampleRate) * 1000;
    const handOver = () => {
      this.caughtUp = null;
      this.timings.stopWaitMs = now() - stopAt;
      if (!this.cancelled) this.deliver();
      this.lastChunkDelivered = true;
    };
    if (this.timings.stopBacklogMs > STOP_BACKLOG_SLACK_MS) {
      this.stopping = new Promise<void>((resolve) => {
        this.caughtUp = () => {
          handOver();
          resolve();
        };
      });
    } else {
      this.stopping = Promise.resolve();
      handOver();
    }
    this.ending.resolve('stopped');
    stopTracks(this.stream);
    const timer = setTimeout(() => this.release(), FLUSH_TIMEOUT_MS);
    this.flushed = () => {
      clearTimeout(timer);
      this.release();
    };
    try {
      this.node.port.postMessage({ type: 'flush' });
    } catch {
      clearTimeout(timer);
      this.release();
    }
    return this.stopping;
  }

  cancel(): void {
    this.cancelled = true;
    this.stopped = true;
    this.release();
  }

  private release(): void {
    if (this.released) return;
    // A stop still waiting for audio hands over what it has (nothing, after a cancel).
    this.caughtUp?.();
    this.released = true;
    this.flushed = null;
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
  lastChunkDelivered = false;

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
      .then(() => (this.cancelled ? undefined : this.decodeAndDeliver()))
      .finally(() => {
        this.lastChunkDelivered = true;
      });
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
