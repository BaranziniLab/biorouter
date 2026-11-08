import { afterEach, beforeEach, describe, expect, it, vi, type Mock } from 'vitest';
import type { Anchor } from '../../utils/dictatedText';
import { BROWSER_SURFACE_MARKER } from '../../utils/surface';
import { BUNDLED_BIOROUTER_VERSION } from '../../utils/biorouterVersion';
import type { Capture, CaptureOptions } from './capture';
import type { DictationStatus, StreamRequest, Transcript } from './dictationApi';
import type { DictationStatusSnapshot, PhaseDecision } from './dictationStore';
import {
  anchorOutsideWord,
  attachDictationComposer,
  cancelDictation,
  getDictationLevel,
  getDictationSnapshot,
  isDictationRunning,
  KEEP_FAILED_MS,
  noteDictationEdit,
  resetDictationSessionForTests,
  retryDictation,
  startDictation,
  stopDictation,
  subscribeDictation,
  subscribeDictationRunning,
  type DictationComposer,
} from './dictationSession';
import { isPrivateDraft, resetPrivateDraftsForTests } from './privateDraft';
import { acknowledgeSharedOriginNotice, resetSharedOriginNoticeForTests } from './sharedOriginGate';
import { DictationStreamError, type DictationStream } from './transport';

const RATE = 16000;
const last = <T>(items: readonly T[]): T | undefined => items[items.length - 1];

function transcript(text: string, extra: Partial<Transcript> = {}): Transcript {
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

class Deferred<T> {
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

class FakeCapture implements Capture {
  readonly mode = captureMode;
  readonly sampleRate = captureRate;
  readonly timings = {};
  stopped = false;
  cancelled = false;
  private readonly endedDeferred = new Deferred<'stopped' | 'track-ended'>();
  readonly ended = this.endedDeferred.promise;
  readonly closed = Promise.resolve();
  /** Audio the worklet still holds when stop() is called. */
  tail: Int16Array | null = null;
  /**
   * Audio recorded before the stop that is still queued behind a busy main thread: the stop
   * waits for it, and delivers it and the tail once this resolves.
   */
  backlog: { pcm: Int16Array; arrived: Deferred<void> } | null = null;
  lastChunkDelivered = false;
  constructor(private readonly options: CaptureOptions) {}
  speak(seconds: number, amplitude = 8000): void {
    const chunk = RATE / 10;
    for (let done = 0; done < seconds * RATE; done += chunk) {
      const pcm = new Int16Array(chunk).fill(amplitude);
      this.options.onChunk(pcm, { rms: amplitude / 32768, peak: amplitude / 32768 });
    }
  }
  live(): void {
    this.options.onLive?.();
  }
  private stopping: Promise<void> | null = null;
  /** Idempotent, as the real capture's stop is: the session may ask twice. */
  stop(): Promise<void> {
    this.stopping ??= this.stopOnce();
    return this.stopping;
  }
  private async stopOnce(): Promise<void> {
    this.stopped = true;
    this.endedDeferred.resolve('stopped');
    const backlog = this.backlog;
    if (backlog) {
      await backlog.arrived.promise;
      this.options.onChunk(backlog.pcm, { rms: 0.2, peak: 0.2 });
    }
    if (this.tail) this.options.onChunk(this.tail, { rms: 0.2, peak: 0.2 });
    this.lastChunkDelivered = true;
    // The MediaRecorder fallback decodes on stop, and that can fail.
    if (captureStopError) throw captureStopError;
  }
  cancel(): void {
    this.cancelled = true;
    this.endedDeferred.resolve('stopped');
  }
  unplug(): void {
    this.endedDeferred.resolve('track-ended');
  }
}

class FakeStream implements DictationStream {
  id: string | null = 'stream-1';
  readyDeferred = new Deferred<void>();
  ready = this.readyDeferred.promise;
  samples = 0;
  pushes: Int16Array[] = [];
  finishDeferred = new Deferred<Transcript>();
  retryDeferred = new Deferred<Transcript>();
  finishCalls = 0;
  released = false;
  cancelled = false;
  private readonly cbs: Record<string, Array<(...args: unknown[]) => void>> = {};
  constructor(readonly request: StreamRequest) {
    if (streamFailure) this.readyDeferred.reject(streamFailure);
    else this.readyDeferred.resolve();
  }
  get audioSeconds(): number {
    return this.samples / RATE;
  }
  push(pcm: Int16Array): void {
    this.pushes.push(pcm);
    this.samples += pcm.length;
  }
  finish(): Promise<Transcript> {
    this.finishCalls++;
    return this.finishDeferred.promise;
  }
  retry(): Promise<Transcript> {
    return this.retryDeferred.promise;
  }
  async cancel(): Promise<void> {
    this.cancelled = true;
  }
  release(): void {
    this.released = true;
  }
  private on(name: string, cb: (...args: unknown[]) => void) {
    (this.cbs[name] ??= []).push(cb);
    return () => {
      this.cbs[name] = (this.cbs[name] ?? []).filter((other) => other !== cb);
    };
  }
  emit(name: string, ...args: unknown[]) {
    for (const cb of this.cbs[name] ?? []) cb(...args);
  }
  onPartial = (cb: (text: string) => void) => this.on('partial', cb as (...a: unknown[]) => void);
  onWaiting = (cb: (w: boolean) => void) => this.on('waiting', cb as (...a: unknown[]) => void);
  onCapWarning = (cb: () => void) => this.on('capWarning', cb);
  onCapReached = (cb: () => void) => this.on('capReached', cb);
  onBusy = (cb: (s: number) => void) => this.on('busy', cb as (...a: unknown[]) => void);
  onFailed = (cb: (e: DictationStreamError) => void) =>
    this.on('failed', cb as (...a: unknown[]) => void);
}

let captures: FakeCapture[];
let streams: FakeStream[];
let captureGate: Deferred<void> | null;
let captureError: unknown;
let captureMode: 'worklet' | 'recorder';
let captureRate: number;
let captureStopError: unknown;
let streamFailure: unknown;
let refreshStatus: Mock<() => Promise<unknown>>;
let phase: PhaseDecision;
let status: DictationStatusSnapshot;
let ensureStatus: Mock<() => Promise<unknown>>;
let insertIntoDraft: Mock<(key: string, at: Anchor, text: string) => void>;
let lockScreen: (() => void) | null;
let busy: boolean[];
let prewarm: Mock<() => void>;

function install() {
  resetDictationSessionForTests({
    startCapture: (options) => {
      const capture = new FakeCapture(options);
      captures.push(capture);
      if (captureError) return Promise.reject(captureError);
      return (captureGate?.promise ?? Promise.resolve()).then(() => capture);
    },
    prewarmCapture: prewarm,
    createStream: (request) => {
      const stream = new FakeStream(request);
      streams.push(stream);
      return stream;
    },
    statusSnapshot: () => status,
    ensureStatus,
    refreshStatus,
    phase: () => phase,
    insertIntoDraft,
  });
}

function composer(
  key: string,
  value = ''
): { field: HTMLTextAreaElement; binding: DictationComposer; detach: () => void } {
  const card = document.createElement('div');
  card.className = 'biorouter-composer-card';
  const field = document.createElement('textarea');
  field.value = value;
  card.appendChild(field);
  document.body.appendChild(card);
  const binding: DictationComposer = {
    key,
    field: () => (field.isConnected ? field : null),
    setByState: vi.fn((next: string) => {
      field.value = next;
    }),
    storesDraft: true,
    onInserted: vi.fn(),
  };
  const detach = attachDictationComposer(binding);
  return { field, binding, detach };
}

const settle = async () => {
  for (let i = 0; i < 10; i++) await Promise.resolve();
};

beforeEach(() => {
  resetSharedOriginNoticeForTests();
  captures = [];
  streams = [];
  captureGate = null;
  captureError = null;
  captureMode = 'worklet';
  captureRate = RATE;
  captureStopError = null;
  streamFailure = null;
  refreshStatus = vi.fn(async () => null);
  phase = { phase: 'idle' };
  status = { load: 'loaded', status: null, micStatus: 'granted' };
  ensureStatus = vi.fn(async () => null);
  insertIntoDraft = vi.fn();
  lockScreen = null;
  busy = [];
  prewarm = vi.fn();
  Object.assign(window, {
    // The desktop's own daemon, as main.ts tells a window it started.
    appConfig: {
      get: (key: string) =>
        key === 'BIOROUTER_LOCAL_BACKEND'
          ? true
          : key === 'BIOROUTER_API_HOST'
            ? 'http://127.0.0.1:4567'
            : undefined,
    },
    electron: {
      platform: 'darwin',
      setDictationBusy: (value: boolean) => busy.push(value),
      onDictationLockScreen: (cb: () => void) => {
        lockScreen = cb;
        return () => undefined;
      },
      dictationMicStatus: vi.fn(async () => 'denied'),
      dictationMicRequest: vi.fn(async () => true),
    },
  });
  if (typeof document.execCommand !== 'function') {
    Object.assign(document, { execCommand: () => false });
  }
  vi.spyOn(document, 'execCommand').mockImplementation((_cmd, _ui, text) => {
    const field = document.activeElement as HTMLTextAreaElement;
    const start = field.selectionStart;
    field.value =
      field.value.slice(0, start) + (text ?? '') + field.value.slice(field.selectionEnd);
    field.setSelectionRange(start + (text ?? '').length, start + (text ?? '').length);
    return true;
  });
  install();
});

afterEach(() => {
  delete document.documentElement.dataset.biorouterSurface;
  resetDictationSessionForTests();
  resetPrivateDraftsForTests();
  document.body.innerHTML = '';
  vi.restoreAllMocks();
  vi.useRealTimers();
});

describe('starting', () => {
  it('refuses a direct browser start until the origin notice was acknowledged', async () => {
    document.documentElement.dataset.biorouterSurface = BROWSER_SURFACE_MARKER;
    composer('tab:a');
    const options = { key: 'tab:a', target: { target_kind: 'chat' as const } };
    await expect(startDictation(options)).resolves.toEqual({ kind: 'origin-notice' });
    expect(captures).toHaveLength(0);
    expect(streams).toHaveLength(0);
    acknowledgeSharedOriginNotice();
    const started = startDictation(options);
    expect(captures).toHaveLength(1);
    await expect(started).resolves.toEqual({ kind: 'recording' });
  });

  it('starts capture and the stream inside the press when the status is loaded', async () => {
    const { field } = composer('tab:a', 'Check');
    field.focus();
    const outcome = startDictation({
      key: 'tab:a',
      target: { target_kind: 'chat', session_id: 's1' },
    });
    // Both began before startDictation returned: nothing waits on the network first.
    expect(captures).toHaveLength(1);
    expect(streams).toHaveLength(1);
    expect(streams[0].request).toMatchObject({
      target_kind: 'chat',
      session_id: 's1',
      sample_rate: 16000,
      format: 's16le',
    });
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:a', activity: 'arming' });
    await expect(outcome).resolves.toEqual({ kind: 'recording' });
    captures[0].live();
    expect(getDictationSnapshot().activity).toBe('recording');
  });

  it('before the status loads, captures at once and opens the stream after the answer, losing no audio', async () => {
    status = { load: 'unloaded', status: null, micStatus: 'unknown' };
    const statusAnswer = new Deferred<null>();
    ensureStatus.mockImplementation(() => statusAnswer.promise);
    composer('tab:a');
    const outcome = startDictation({ key: 'tab:a', target: { target_kind: 'new_chat' } });
    expect(captures).toHaveLength(1);
    expect(streams).toHaveLength(0);
    await settle();
    captures[0].speak(1);
    statusAnswer.resolve(null);
    await expect(outcome).resolves.toEqual({ kind: 'recording' });
    expect(streams).toHaveLength(1);
    expect(streams[0].audioSeconds).toBeCloseTo(1, 5);
  });

  it('before the status loads, a microphone that is not set up stops capture and opens the setup popover', async () => {
    status = { load: 'unloaded', status: null, micStatus: 'unknown' };
    ensureStatus.mockImplementation(async () => {
      phase = { phase: 'not-installed' };
      return null;
    });
    composer('tab:a');
    await expect(
      startDictation({ key: 'tab:a', target: { target_kind: 'chat' } })
    ).resolves.toEqual({
      kind: 'setup',
    });
    expect(captures[0].cancelled).toBe(true);
    expect(streams).toHaveLength(0);
  });

  it('when the store already knows the microphone is not set up, nothing opens', async () => {
    phase = { phase: 'not-installed' };
    await expect(
      startDictation({ key: 'tab:a', target: { target_kind: 'chat' } })
    ).resolves.toEqual({
      kind: 'setup',
    });
    expect(captures).toHaveLength(0);
  });

  it('maps a refused microphone through the error map', async () => {
    captureError = new DOMException('denied', 'NotAllowedError');
    composer('tab:a');
    const outcome = await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    expect(outcome).toMatchObject({
      kind: 'failed',
      notice: { id: 'MSG-MIC-DENIED-MAC', action: 'open-settings' },
    });
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'error',
      notice: { id: 'MSG-MIC-DENIED-MAC' },
    });
    expect(streams[0].cancelled).toBe(true);
  });

  it('on macOS asks for the microphone first while the system has not decided', async () => {
    status = { load: 'loaded', status: null, micStatus: 'not-determined' };
    const request = window.electron.dictationMicRequest as unknown as Mock<() => Promise<boolean>>;
    request.mockResolvedValue(false);
    composer('tab:a');
    const outcome = await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    expect(request).toHaveBeenCalledTimes(1);
    expect(outcome).toMatchObject({ kind: 'failed', notice: { id: 'MSG-MIC-DENIED-MAC' } });
    expect(captures).toHaveLength(0);
  });

  it('a press while arming cancels: nothing has been heard yet', async () => {
    captureGate = new Deferred<void>();
    composer('tab:a');
    const first = startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    expect(getDictationSnapshot().activity).toBe('arming');
    await expect(
      startDictation({ key: 'tab:a', target: { target_kind: 'chat' } })
    ).resolves.toEqual({ kind: 'cancelled' });
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:a', activity: 'idle' });
    expect(streams[0].cancelled).toBe(true);
    captureGate.resolve();
    await first;
    expect(captures[0].cancelled).toBe(true);
    expect(busy).toEqual([true, false]);
  });

  it('a cancel while the macOS prompt is open never opens the microphone', async () => {
    status = { load: 'loaded', status: null, micStatus: 'not-determined' };
    const answer = new Deferred<boolean>();
    const request = window.electron.dictationMicRequest as unknown as Mock<() => Promise<boolean>>;
    request.mockImplementation(() => answer.promise);
    composer('tab:a');
    const outcome = startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    cancelDictation('tab:a');
    answer.resolve(true);
    await expect(outcome).resolves.toEqual({ kind: 'cancelled' });
    expect(captures).toHaveLength(0);
    expect(busy).toEqual([true, false]);
  });

  it('a 409 noModel from the daemon asks this composer to open the setup popover', async () => {
    streamFailure = new DictationStreamError('noModel');
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    await settle();
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'idle',
      notice: null,
      setupRequest: { key: 'tab:a' },
    });
    expect(captures[0].cancelled).toBe(true);
    expect(refreshStatus).toHaveBeenCalledTimes(1);
  });
});

describe('stopping and inserting', () => {
  it('stops, finishes, inserts at the caret as one undoable step, and releases the stream', async () => {
    const { field, binding } = composer('tab:a', 'Check the gene');
    field.focus();
    field.setSelectionRange(9, 9);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat', session_id: 's1' } });
    captures[0].speak(2);
    captures[0].tail = new Int16Array(800).fill(4000);
    stopDictation('tab:a');
    expect(getDictationSnapshot().activity).toBe('transcribing');
    await settle();
    expect(captures[0].stopped).toBe(true);
    // The last partial chunk was pushed before finish was asked for, and the
    // Transcribing card shows no level for it.
    expect(streams[0].samples).toBe(2 * RATE + 800);
    expect(getDictationSnapshot()).toMatchObject({ activity: 'transcribing' });
    expect(getDictationLevel()).toBe(0);
    expect(streams[0].finishCalls).toBe(1);
    streams[0].finishDeferred.resolve(transcript('EPAS1'));
    await settle();
    expect(field.value).toBe('Check the EPAS1 gene');
    expect(binding.setByState).not.toHaveBeenCalled();
    expect(binding.onInserted).toHaveBeenCalledWith(
      expect.objectContaining({ key: 'tab:a', path: 'undoable', anchor: { start: 9, end: 9 } })
    );
    expect(streams[0].released).toBe(true);
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'idle',
      announcement: 'Inserted',
      notice: null,
    });
  });

  it.each([
    // "Call th|e OMOP": the end of "the" is nearer.
    [7, 'Call the now OMOP tables'],
    // "Call t|he OMOP": its start is nearer.
    [6, 'Call now the OMOP tables'],
    // At a word's edge the caret stays where it is.
    [8, 'Call the now OMOP tables'],
    [4, 'Call now the OMOP tables'],
  ])(
    'a caret inside a word (at %i) takes the text beside the word, never splitting it',
    async (at, expected) => {
      const { field } = composer('tab:a', 'Call the OMOP tables');
      field.focus();
      field.setSelectionRange(at, at);
      await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
      captures[0].speak(1);
      stopDictation('tab:a');
      await settle();
      streams[0].finishDeferred.resolve(transcript('now'));
      await settle();
      expect(field.value).toBe(expected);
    }
  );

  it('keeps a word joined by an apostrophe or a hyphen whole', () => {
    // "I do|n't": the word runs to the end of "don't", so its start is nearer.
    expect(anchorOutsideWord("I don't", { start: 4, end: 4 })).toEqual({ start: 2, end: 2 });
    expect(anchorOutsideWord('HIF-2α', { start: 5, end: 5 })).toEqual({ start: 6, end: 6 });
    expect(anchorOutsideWord('HIF-2α', { start: 2, end: 2 })).toEqual({ start: 0, end: 0 });
    // A selection is replaced as it is.
    expect(anchorOutsideWord('hello', { start: 1, end: 3 })).toEqual({ start: 1, end: 3 });
    // A hyphen with no letter beyond it ends the word.
    expect(anchorOutsideWord('ab- x', { start: 1, end: 1 })).toEqual({ start: 2, end: 2 });
  });

  it('prepares the next press once a dictation has ended in a composer that is still there', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    composer('tab:a', '');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation('tab:a');
    await settle();
    expect(prewarm).not.toHaveBeenCalled();
    streams[0].finishDeferred.resolve(transcript('the whole sentence'));
    await settle();
    // After the text went in, and after the card's render.
    expect(getDictationSnapshot()).toMatchObject({ activity: 'idle', announcement: 'Inserted' });
    expect(prewarm).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(0);
    expect(prewarm).toHaveBeenCalledTimes(1);
  });

  it('prepares nothing for a composer that has gone', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { detach } = composer('tab:a', '');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    detach();
    cancelDictation('tab:a');
    await settle();
    await vi.advanceTimersByTimeAsync(0);
    expect(prewarm).not.toHaveBeenCalled();
  });

  it('asks for finish before it tells the card Transcribing, so the request leaves first', async () => {
    composer('tab:a', '');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat', session_id: 's1' } });
    captures[0].speak(1);
    captures[0].tail = new Int16Array(400).fill(4000);
    let atPublish: { finishCalls: number; samples: number } | null = null;
    const off = subscribeDictation(() => {
      if (getDictationSnapshot().activity === 'transcribing' && atPublish === null) {
        atPublish = { finishCalls: streams[0].finishCalls, samples: streams[0].samples };
      }
    });
    stopDictation('tab:a');
    off();
    // Synchronously inside the stop: the tail went up, finish was asked, then the card.
    expect(atPublish).toEqual({ finishCalls: 1, samples: RATE + 400 });
    await settle();
    expect(streams[0].finishCalls).toBe(1);
  });

  it('a stop that waits for audio queued behind a busy main thread asks for finish only once it is up', async () => {
    composer('tab:a', '');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat', session_id: 's1' } });
    captures[0].speak(1);
    const arrived = new Deferred<void>();
    captures[0].backlog = { pcm: new Int16Array(4800).fill(3000), arrived };
    captures[0].tail = new Int16Array(400).fill(4000);
    stopDictation('tab:a');
    expect(getDictationSnapshot().activity).toBe('transcribing');
    await settle();
    // Nothing is asked for while the end of the utterance is still on its way.
    expect(streams[0].finishCalls).toBe(0);
    expect(streams[0].samples).toBe(RATE);
    arrived.resolve();
    await settle();
    expect(streams[0].samples).toBe(RATE + 4800 + 400);
    expect(streams[0].finishCalls).toBe(1);
    streams[0].finishDeferred.resolve(transcript('the whole sentence'));
    await settle();
    expect(getDictationSnapshot()).toMatchObject({ activity: 'idle', announcement: 'Inserted' });
  });

  it('maps the anchor through edits the person makes while recording', async () => {
    const { field } = composer('tab:a', 'end');
    field.focus();
    field.setSelectionRange(3, 3);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    // The person types "Start " before the anchor while still recording.
    noteDictationEdit('tab:a', 'end', 'Start end', 6);
    field.value = 'Start end';
    stopDictation('tab:a');
    await settle();
    streams[0].finishDeferred.resolve(transcript('words'));
    await settle();
    expect(field.value).toBe('Start end words');
  });

  it('under half a second: MSG-TOO-SHORT, the stream dropped, nothing transcribed', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(0.3);
    stopDictation();
    await settle();
    expect(streams[0].finishCalls).toBe(0);
    expect(streams[0].cancelled).toBe(true);
    expect(getDictationSnapshot().notice).toMatchObject({ id: 'MSG-TOO-SHORT', failure: false });
  });

  it('only exact zeros: MSG-SILENT', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(2, 0);
    stopDictation();
    await settle();
    expect(streams[0].finishCalls).toBe(0);
    expect(getDictationSnapshot().notice?.id).toBe('MSG-SILENT');
  });

  it('three seconds under the noise floor says MSG-NO-AUDIO-YET and keeps recording', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(2.9, 10);
    expect(getDictationSnapshot().notice).toBeNull();
    captures[0].speak(0.2, 10);
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'recording',
      notice: { id: 'MSG-NO-AUDIO-YET' },
    });
    captures[0].speak(0.1, 9000);
    expect(getDictationSnapshot().notice).toBeNull();
  });

  it('no speech: MSG-NOTHING and nothing inserted', async () => {
    const { field } = composer('tab:a', 'keep');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.resolve(transcript('', { no_speech: true }));
    await settle();
    expect(field.value).toBe('keep');
    expect(getDictationSnapshot().notice?.id).toBe('MSG-NOTHING');
    expect(streams[0].released).toBe(true);
  });

  it('Escape cancels: nothing inserted, the stream dropped, capture stopped', async () => {
    const { field } = composer('tab:a', 'keep');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    cancelDictation('tab:a');
    await settle();
    expect(captures[0].cancelled).toBe(true);
    expect(streams[0].cancelled).toBe(true);
    expect(field.value).toBe('keep');
    expect(getDictationSnapshot().activity).toBe('idle');
  });
});

describe('failures', () => {
  it('engineFailed: MSG-ENGINE-FAILED with Retry, which replays the kept audio', async () => {
    const { field } = composer('tab:a');
    field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'error',
      notice: { id: 'MSG-ENGINE-FAILED', action: 'retry', failure: true },
    });
    expect(getDictationSnapshot().notice?.text).toContain('on this computer');
    expect(streams[0].cancelled).toBe(false);
    const retried = retryDictation('tab:a');
    expect(getDictationSnapshot().activity).toBe('transcribing');
    streams[0].retryDeferred.resolve(transcript('second try'));
    await retried;
    expect(field.value).toBe('second try');
  });

  it('Retry shows MSG-WAIT again while the new stream waits', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    void retryDictation('tab:a');
    streams[0].emit('waiting', true);
    expect(getDictationSnapshot()).toMatchObject({ activity: 'transcribing', waiting: true });
    expect(last(busy)).toBe(true);
  });

  it('a composer that unmounts drops its kept recording, and Retry goes away', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const a = composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    expect(getDictationSnapshot().notice?.action).toBe('retry');
    a.detach();
    vi.advanceTimersByTime(0);
    expect(streams[0].cancelled).toBe(true);
    expect(getDictationSnapshot().notice).toMatchObject({
      id: 'MSG-ENGINE-FAILED',
      action: null,
    });
    await retryDictation('tab:a');
    expect(getDictationSnapshot().activity).toBe('error');
  });

  it('a kept recording is dropped after 5 minutes, and Retry goes away', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    vi.advanceTimersByTime(KEEP_FAILED_MS);
    expect(streams[0].cancelled).toBe(true);
    expect(getDictationSnapshot().notice?.action).toBeNull();
  });

  it('lost, proxy 413 and a policy sentence show their notices', async () => {
    for (const [error, id] of [
      [new DictationStreamError('lost'), 'MSG-LOST'],
      [new DictationStreamError('proxy413'), 'MSG-413'],
    ] as const) {
      composer('tab:x');
      await startDictation({ key: 'tab:x', target: { target_kind: 'chat' } });
      last(captures)!.speak(1);
      stopDictation();
      await settle();
      last(streams)!.finishDeferred.reject(error);
      await settle();
      expect(getDictationSnapshot().notice?.id).toBe(id);
      document.body.innerHTML = '';
    }
    composer('tab:y');
    await startDictation({ key: 'tab:y', target: { target_kind: 'chat' } });
    last(captures)!.speak(1);
    stopDictation();
    await settle();
    last(streams)!.finishDeferred.reject(
      new DictationStreamError('policy', 'Your organization says no.')
    );
    await settle();
    expect(getDictationSnapshot().notice).toMatchObject({
      id: 'server',
      text: 'Your organization says no.',
    });
  });
});

describe('who owns the microphone', () => {
  it('starting in pane B stops pane A, and A’s text goes into A', async () => {
    const a = composer('tab:a', '');
    const b = composer('tab:b', '');
    a.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:b' });
    await settle();
    expect(captures[0].stopped).toBe(true);
    streams[0].finishDeferred.resolve(transcript('for A'));
    await settle();
    expect(a.field.value).toBe('for A');
    expect(b.field.value).toBe('');
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:b' });
  });

  it('a composer that unmounts stops the recording and its text goes into the stored draft', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const a = composer('tab:a', 'Draft so far');
    a.field.setSelectionRange(12, 12);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    a.detach();
    a.field.parentElement!.remove();
    vi.advanceTimersByTime(0);
    await settle();
    expect(captures[0].stopped).toBe(true);
    streams[0].finishDeferred.resolve(transcript('and more'));
    await settle();
    expect(insertIntoDraft).toHaveBeenCalledWith(
      'tab:a',
      { start: 12, end: 12 },
      'and more',
      expect.objectContaining({
        recordId: 'stream-1',
        raw: 'and more',
        changes: [],
        model: 'whisper-small.en-q8_0',
        biasScope: 'public',
      })
    );
  });

  it('text dictated with private terms into the stored draft of a composer that unmounted flags that draft', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const a = composer('tab:a', 'Draft so far');
    a.field.setSelectionRange(12, 12);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat', session_id: 's1' } });
    captures[0].speak(1);
    // The person switches tab while the recording is still open.
    a.detach();
    a.field.parentElement!.remove();
    vi.advanceTimersByTime(0);
    await settle();
    streams[0].finishDeferred.resolve(transcript('EPAS1 again', { bias_scope: 'private' }));
    await settle();
    expect(insertIntoDraft).toHaveBeenCalledWith(
      'tab:a',
      { start: 12, end: 12 },
      'EPAS1 again',
      expect.objectContaining({ recordId: 'stream-1', raw: 'EPAS1 again', biasScope: 'private' })
    );
    // No composer was there to call `onInserted`, and the draft is flagged all the same.
    expect(a.binding.onInserted).not.toHaveBeenCalled();
    expect(isPrivateDraft('tab:a')).toBe(true);
  });

  it('public terms into the stored draft leave it unflagged', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const a = composer('tab:a', '');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    a.detach();
    a.field.parentElement!.remove();
    vi.advanceTimersByTime(0);
    await settle();
    streams[0].finishDeferred.resolve(transcript('plain words'));
    await settle();
    expect(insertIntoDraft).toHaveBeenCalledTimes(1);
    expect(isPrivateDraft('tab:a')).toBe(false);
  });

  it('a composer without a stored draft that unmounts drops the text rather than writing a draft', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const editor = composer('editor:message-1', 'edited text');
    editor.binding.storesDraft = false;
    await startDictation({ key: 'editor:message-1', target: { target_kind: 'editor' } });
    captures[0].speak(1);
    editor.detach();
    editor.field.parentElement!.remove();
    vi.advanceTimersByTime(0);
    await settle();
    streams[0].finishDeferred.resolve(transcript('lost words'));
    await settle();
    expect(insertIntoDraft).not.toHaveBeenCalled();
    expect(streams[0].released).toBe(true);
  });

  it('a composer that mounts again under the same key in the same commit keeps recording', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const a = composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    a.detach();
    composer('tab:a');
    vi.advanceTimersByTime(0);
    await settle();
    expect(captures[0].stopped).toBe(false);
    expect(getDictationSnapshot().key).toBe('tab:a');
  });

  it('a screen lock and an unplugged headset stop and insert', async () => {
    const a = composer('tab:a');
    a.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    lockScreen!();
    await settle();
    expect(captures[0].stopped).toBe(true);
    streams[0].finishDeferred.resolve(transcript('locked'));
    await settle();
    expect(a.field.value).toBe('locked');

    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[1].speak(1);
    captures[1].unplug();
    await settle();
    expect(getDictationSnapshot().activity).toBe('transcribing');
  });

  it('warns 30 s before the cap and, at the cap, stops, inserts and says so', async () => {
    const a = composer('tab:a');
    a.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    streams[0].emit('capWarning');
    expect(getDictationSnapshot().notice).toMatchObject({ id: 'MSG-CAP-WARN', failure: false });
    streams[0].emit('capReached');
    await settle();
    streams[0].finishDeferred.resolve(transcript('ten minutes of speech'));
    await settle();
    expect(a.field.value).toBe('ten minutes of speech');
    expect(getDictationSnapshot().notice).toMatchObject({ id: 'MSG-CAP-STOPPED' });
  });

  it('tells the main process while dictation holds the window, and when it lets go', async () => {
    const a = composer('tab:a');
    a.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    expect(busy).toEqual([true]);
    streams[0].finishDeferred.resolve(transcript('done'));
    await settle();
    expect(busy).toEqual([true, false]);
  });

  it('stays busy while a pane that another press displaced still waits for its text', async () => {
    const a = composer('tab:a');
    composer('tab:b');
    a.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    await settle();
    // B ends at once (too short); A is still transcribing in the background.
    stopDictation('tab:b');
    await settle();
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:b', activity: 'idle' });
    expect(last(busy)).toBe(true);
    streams[0].finishDeferred.resolve(transcript('for A'));
    await settle();
    expect(a.field.value).toBe('for A');
    expect(busy).toEqual([true, false]);
  });

  it('isDictationRunning says what main is told, and calls back each time it changes', async () => {
    // Settings' Test speed reads it, so a displaced pane still transcribing counts, and the end
    // of one that ends in the background (no snapshot change) still calls back.
    const changes: boolean[] = [];
    subscribeDictationRunning(() => changes.push(isDictationRunning()));
    const a = composer('tab:a');
    composer('tab:b');
    a.field.focus();
    expect(isDictationRunning()).toBe(false);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    expect(isDictationRunning()).toBe(true);
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    await settle();
    stopDictation('tab:b');
    await settle();
    expect(getDictationSnapshot().activity).toBe('idle');
    expect(isDictationRunning()).toBe(true);
    streams[0].finishDeferred.resolve(transcript('for A'));
    await settle();
    expect(isDictationRunning()).toBe(false);
    expect(changes).toEqual(busy);
  });

  it('shows partial text and MSG-WAIT from the stream', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    streams[0].emit('partial', 'so far');
    streams[0].emit('waiting', true);
    expect(getDictationSnapshot()).toMatchObject({ partial: 'so far', waiting: true });
  });
});

describe('what the card shows, and how often', () => {
  it('keeps the level out of the snapshot: a second of chunks renders at most twice', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].live();
    const renders = vi.fn();
    subscribeDictation(renders);
    captures[0].speak(1);
    expect(renders.mock.calls.length).toBeLessThanOrEqual(2);
    expect(getDictationLevel()).toBeCloseTo(8000 / 32768, 6);
    expect(getDictationSnapshot()).toMatchObject({ activity: 'recording', elapsedS: 1 });
    expect(getDictationSnapshot()).not.toHaveProperty('level');
    stopDictation();
    await settle();
    expect(getDictationLevel()).toBe(0);
  });

  it('counts MSG-SERVER-BUSY down while a 429 is waited out, and clears it for the next try', async () => {
    vi.useFakeTimers({ toFake: ['setInterval', 'clearInterval'] });
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    streams[0].emit('busy', 3);
    expect(getDictationSnapshot().notice).toMatchObject({ id: 'MSG-SERVER-BUSY', failure: false });
    expect(getDictationSnapshot().notice?.text).toContain('Trying again in 3 seconds.');
    vi.advanceTimersByTime(1000);
    expect(getDictationSnapshot().notice?.text).toContain('Trying again in 2 seconds.');
    // A second 429 starts the count again, with one timer.
    streams[0].emit('busy', 5);
    vi.advanceTimersByTime(1000);
    expect(getDictationSnapshot().notice?.text).toContain('Trying again in 4 seconds.');
    vi.advanceTimersByTime(4000);
    expect(getDictationSnapshot().notice).toBeNull();
    expect(vi.getTimerCount()).toBe(0);
  });

  it('a server still busy 30 s after the stop: MSG-SERVER-BUSY-FAILED with Retry, the recording kept', async () => {
    const { field } = composer('tab:a');
    field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('tooManyStreams'));
    await settle();
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'error',
      notice: { id: 'MSG-SERVER-BUSY-FAILED', action: 'retry', failure: true },
    });
    expect(getDictationSnapshot().notice?.text).not.toContain('Trying again');
    expect(streams[0].cancelled).toBe(false);
    const retried = retryDictation('tab:a');
    streams[0].retryDeferred.resolve(transcript('kept speech'));
    await retried;
    expect(field.value).toBe('kept speech');
  });
});

describe('a stream that fails while the person speaks', () => {
  it('ends the dictation at once: the microphone goes off and the notice shows without a stop', async () => {
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(2);
    streams[0].emit('failed', new DictationStreamError('proxy413'));
    expect(captures[0].cancelled).toBe(true);
    expect(streams[0].cancelled).toBe(true);
    expect(streams[0].finishCalls).toBe(0);
    expect(getDictationSnapshot()).toMatchObject({
      key: 'tab:a',
      activity: 'error',
      notice: { id: 'MSG-413', failure: true },
    });
    expect(busy).toEqual([true, false]);
  });
});

describe('the anchor follows edits until the text goes in', () => {
  it('while the recording is kept for Retry', async () => {
    const { field } = composer('tab:a', 'end');
    field.focus();
    field.setSelectionRange(3, 3);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    noteDictationEdit('tab:a', 'end', 'Start end', 6);
    field.value = 'Start end';
    const retried = retryDictation('tab:a');
    streams[0].retryDeferred.resolve(transcript('words'));
    await retried;
    expect(field.value).toBe('Start end words');
  });

  it('while a pane another press displaced still transcribes', async () => {
    const a = composer('tab:a', 'end');
    composer('tab:b');
    a.field.focus();
    a.field.setSelectionRange(3, 3);
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    await settle();
    noteDictationEdit('tab:a', 'end', 'Start end', 6);
    a.field.value = 'Start end';
    streams[0].finishDeferred.resolve(transcript('words'));
    await settle();
    expect(a.field.value).toBe('Start end words');
  });
});

describe('a press in another pane', () => {
  it('leaves the recording pane alone when the press cannot start', async () => {
    composer('tab:a');
    composer('tab:b');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    // A window focus refresh learned meanwhile that the model is gone.
    phase = { phase: 'not-installed' };
    await expect(
      startDictation({ key: 'tab:b', target: { target_kind: 'chat' } })
    ).resolves.toEqual({ kind: 'setup' });
    expect(captures[0].stopped).toBe(false);
    expect(captures).toHaveLength(1);
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:a', activity: 'recording' });
  });

  it('a displaced pane’s failure shows in its own card once the other dictation ends, with Retry', async () => {
    const a = composer('tab:a');
    const b = composer('tab:b');
    b.field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    await settle();
    captures[1].speak(1);
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    // B holds the card while it records.
    expect(getDictationSnapshot()).toMatchObject({ key: 'tab:b', activity: 'recording' });
    stopDictation('tab:b');
    await settle();
    streams[1].finishDeferred.resolve(transcript('for B'));
    await settle();
    expect(b.field.value).toBe('for B');
    expect(getDictationSnapshot()).toMatchObject({
      key: 'tab:a',
      activity: 'error',
      notice: { id: 'MSG-ENGINE-FAILED', action: 'retry' },
    });
    const retried = retryDictation('tab:a');
    streams[0].retryDeferred.resolve(transcript('for A'));
    await retried;
    expect(a.field.value).toBe('for A');
  });

  it('a displaced pane’s failure shows at once when nothing else holds the card', async () => {
    composer('tab:a');
    composer('tab:b');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    await startDictation({ key: 'tab:b', target: { target_kind: 'chat' } });
    await settle();
    cancelDictation('tab:b');
    streams[0].finishDeferred.reject(new DictationStreamError('lost'));
    await settle();
    expect(getDictationSnapshot()).toMatchObject({
      key: 'tab:a',
      activity: 'error',
      notice: { id: 'MSG-LOST' },
    });
  });
});

describe('stops that arrive before the microphone opened', () => {
  it('a composer that unmounts while the macOS prompt is open never opens the microphone', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    status = { load: 'loaded', status: null, micStatus: 'not-determined' };
    const answer = new Deferred<boolean>();
    const request = window.electron.dictationMicRequest as unknown as Mock<() => Promise<boolean>>;
    request.mockImplementation(() => answer.promise);
    const a = composer('tab:a');
    const outcome = startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    a.detach();
    vi.advanceTimersByTime(0);
    answer.resolve(true);
    await expect(outcome).resolves.toEqual({ kind: 'cancelled' });
    expect(captures).toHaveLength(0);
    expect(streams.every((stream) => stream.cancelled)).toBe(true);
    expect(busy).toEqual([true, false]);
  });
});

describe('the capture paths', () => {
  it('the native rate fallback replaces the 16 kHz stream with one that declares the device rate', async () => {
    captureRate = 48000;
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    expect(streams).toHaveLength(2);
    expect(streams[0].request.sample_rate).toBe(16000);
    expect(streams[0].cancelled).toBe(true);
    expect(streams[1].request.sample_rate).toBe(48000);
    captures[0].speak(1);
    expect(streams[0].pushes).toHaveLength(0);
    expect(streams[1].samples).toBe(RATE);
  });

  it('a MediaRecorder recording that cannot be decoded says so, not that it was too short', async () => {
    captureMode = 'recorder';
    captureStopError = new DOMException('cannot decode', 'EncodingError');
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    stopDictation();
    await settle();
    expect(getDictationSnapshot()).toMatchObject({
      activity: 'error',
      notice: { id: 'MSG-RECORDING-FAILED', failure: true },
    });
    expect(streams[0].finishCalls).toBe(0);
    expect(streams[0].cancelled).toBe(true);
  });

  it('Escape while the insert waits for an IME composition to end inserts nothing', async () => {
    const { field, binding } = composer('tab:a', 'keep');
    binding.isComposing = () => true;
    field.focus();
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.resolve(transcript('late words'));
    await settle();
    cancelDictation('tab:a');
    field.dispatchEvent(new Event('compositionend'));
    await settle();
    expect(field.value).toBe('keep');
    expect(binding.setByState).not.toHaveBeenCalled();
    expect(insertIntoDraft).not.toHaveBeenCalled();
    expect(getDictationSnapshot()).toMatchObject({ activity: 'idle', notice: null });
  });
});

describe('where the card says transcription happened', () => {
  it('names the server, not this computer, when the desktop app uses an external backend', async () => {
    Object.assign(window, {
      appConfig: {
        get: (key: string) =>
          key === 'BIOROUTER_LOCAL_BACKEND'
            ? false
            : key === 'BIOROUTER_API_HOST'
              ? 'https://lab.example:8765'
              : undefined,
      },
    });
    status = {
      load: 'loaded',
      status: { host: 'labhost' } as DictationStatus,
      micStatus: 'granted',
    };
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    captures[0].speak(1);
    stopDictation();
    await settle();
    streams[0].finishDeferred.reject(new DictationStreamError('engineFailed'));
    await settle();
    const text = getDictationSnapshot().notice?.text ?? '';
    expect(text).toContain('Transcription failed on the Biorouter server at labhost, so');
    expect(text).not.toContain('this computer');
  });

  it('fills every placeholder when the phase comes from the real store', async () => {
    const secure = Object.getOwnPropertyDescriptor(window, 'isSecureContext');
    Object.defineProperty(window, 'isSecureContext', { value: false, configurable: true });
    try {
      resetDictationSessionForTests({
        startCapture: () => Promise.reject(new Error('not reached')),
      });
      const outcome = await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
      expect(outcome).toMatchObject({ kind: 'unavailable' });
      const reason = (outcome as { reason: string }).reason;
      expect(reason).toContain(`http://${location.host}.`);
      expect(reason).not.toMatch(/<[^<>]+>/);
    } finally {
      if (secure) Object.defineProperty(window, 'isSecureContext', secure);
      else delete (window as { isSecureContext?: boolean }).isSecureContext;
    }
  });

  it('fills MSG-REMOTE-OLD when the create says the backend predates dictation', async () => {
    streamFailure = new DictationStreamError('absent');
    composer('tab:a');
    await startDictation({ key: 'tab:a', target: { target_kind: 'chat' } });
    await settle();
    const text = getDictationSnapshot().notice?.text ?? '';
    expect(getDictationSnapshot().notice?.id).toBe('MSG-REMOTE-OLD');
    expect(text).toContain('The Biorouter service on 127.0.0.1:4567 is');
    expect(text).toContain(`Update it to ${BUNDLED_BIOROUTER_VERSION} or later`);
    expect(text).not.toMatch(/<[^<>]+>/);
  });
});
