import { Blob as NodeBlob } from 'node:buffer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  InsecureContextError,
  PCM_CAPTURE_PROCESSOR,
  PREWARM_EVERY_MS,
  prewarmCapture,
  resetCapturePrewarmForTests,
  startCapture,
  STOP_BACKLOG_SLACK_MS,
  type CaptureOptions,
} from './capture';

/**
 * jsdom has no Web Audio and no media devices, so these fakes record what
 * `capture.ts` asks for. Whether the real browser does what the fakes stand
 * for is the Electron job's question (`scripts/dictation-capture-e2e.mjs`).
 */
const log: string[] = [];

type ContextOptions = { sampleRate?: number; latencyHint?: string };
type NodeOptions = {
  channelCount?: number;
  channelCountMode?: string;
  processorOptions?: unknown;
};
type TrackConstraints = { deviceId?: unknown };
const last = <T>(items: readonly T[]): T | undefined => items[items.length - 1];

class FakeTrack extends EventTarget {
  readyState: 'live' | 'ended' = 'live';
  constructor(readonly label = 'Mic') {
    super();
  }
  stop() {
    this.readyState = 'ended';
    log.push('track.stop');
  }
}

class FakeMediaStream {
  tracks = [new FakeTrack()];
  getTracks() {
    return this.tracks;
  }
  getAudioTracks() {
    return this.tracks;
  }
}

class FakePort {
  onmessage: ((event: MessageEvent) => void) | null = null;
  posted: unknown[] = [];
  answerFlush = true;
  owner: FakeWorkletNode | null = null;
  postMessage(message: unknown) {
    this.posted.push(message);
    if ((message as { type?: string }).type === 'flush' && this.answerFlush) {
      // The worklet's answer to a flush: no samples, only that it has ended.
      queueMicrotask(() =>
        this.onmessage?.({
          data: { type: 'chunk', pcm: new Int16Array(0).buffer, final: true },
        } as MessageEvent)
      );
    }
  }
  deliver(data: unknown) {
    this.onmessage?.({ data } as MessageEvent);
  }
}

let contexts: FakeAudioContext[] = [];
let nodes: FakeWorkletNode[] = [];
let failSourceAtRate: number | null = null;
let failAddModule = false;
let nativeRate = 48000;
/** Electron lets a context start without a gesture; a browser starts it suspended. */
let startsRunning = false;

class FakeAudioContext {
  sampleRate: number;
  state: 'running' | 'suspended' | 'closed' = startsRunning ? 'running' : 'suspended';
  /** The rendering clock, which the stop reads: set by a test to stand for the audio thread. */
  currentTime = 0;
  options: ContextOptions | undefined;
  destination = { name: 'destination' };
  audioWorklet = {
    addModule: vi.fn(async (url: string) => {
      log.push(`addModule ${url.split('?')[0].split('/').pop()}`);
      if (failAddModule) throw new DOMException('blocked', 'AbortError');
    }),
  };
  constructor(options?: ContextOptions) {
    this.options = options;
    this.sampleRate = options?.sampleRate ?? nativeRate;
    contexts.push(this);
    log.push(`new AudioContext ${options?.sampleRate ?? 'native'}`);
  }
  async resume() {
    log.push('resume');
    this.state = 'running';
  }
  async suspend() {
    log.push('suspend');
    this.state = 'suspended';
  }
  async close() {
    this.state = 'closed';
    log.push('context.close');
  }
  createMediaStreamSource(_stream: FakeMediaStream) {
    if (failSourceAtRate === this.sampleRate) {
      throw new DOMException('rates differ', 'NotSupportedError');
    }
    const source = {
      connected: [] as unknown[],
      connect(target: unknown) {
        this.connected.push(target);
        log.push('source.connect');
      },
      disconnect() {
        log.push('source.disconnect');
      },
    };
    return source;
  }
}

class FakeWorkletNode {
  port = new FakePort();
  connections: unknown[] = [];
  constructor(
    readonly context: FakeAudioContext,
    readonly name: string,
    readonly options: NodeOptions
  ) {
    nodes.push(this);
    this.port.owner = this;
  }
  connect(target: unknown) {
    this.connections.push(target);
  }
  disconnect() {
    log.push('node.disconnect');
  }
}

let getUserMedia: ReturnType<typeof vi.fn>;
let enumerateDevices: ReturnType<typeof vi.fn>;

function options(extra: Partial<CaptureOptions> = {}) {
  const chunks: Array<{ pcm: Int16Array; rms: number; peak: number }> = [];
  const onLive = vi.fn();
  return {
    chunks,
    onLive,
    options: {
      noiseSuppression: false,
      onChunk: (pcm: Int16Array, level: { rms: number; peak: number }) =>
        chunks.push({ pcm, ...level }),
      onLive,
      ...extra,
    } satisfies CaptureOptions,
  };
}

beforeEach(() => {
  resetCapturePrewarmForTests();
  log.length = 0;
  contexts = [];
  nodes = [];
  failSourceAtRate = null;
  failAddModule = false;
  nativeRate = 48000;
  startsRunning = false;
  Object.defineProperty(window, 'isSecureContext', { value: true, configurable: true });
  getUserMedia = vi.fn(async () => {
    log.push('getUserMedia resolved');
    return new FakeMediaStream();
  });
  enumerateDevices = vi.fn(async () => [
    { kind: 'audioinput', deviceId: 'new-id', label: 'USB Mic' },
    { kind: 'videoinput', deviceId: 'cam', label: 'Camera' },
  ]);
  Object.defineProperty(navigator, 'mediaDevices', {
    value: { getUserMedia, enumerateDevices },
    configurable: true,
  });
  vi.stubGlobal('AudioContext', FakeAudioContext);
  vi.stubGlobal('AudioWorkletNode', FakeWorkletNode);
});

afterEach(() => {
  resetCapturePrewarmForTests();
  vi.unstubAllGlobals();
  vi.useRealTimers();
  delete document.documentElement.dataset.biorouterSurface;
  Object.defineProperty(navigator, 'userActivation', { value: undefined, configurable: true });
});

const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

describe('startCapture', () => {
  it('asks for the device inside the call, and on the desktop starts the 16 kHz context once the stream is here', async () => {
    const { options: opts } = options({ noiseSuppression: true, deviceId: 'dev-1' });
    const pending = startCapture(opts);
    // Synchronously, in the gesture: the device, and nothing that would hold it back.
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        deviceId: { exact: 'dev-1' },
        echoCancellation: false,
        noiseSuppression: true,
        autoGainControl: false,
      },
    });
    expect(contexts).toHaveLength(0);
    await pending;
    expect(contexts).toHaveLength(1);
    expect(contexts[0].options).toEqual({ sampleRate: 16000, latencyHint: 'interactive' });
    expect(log.slice(0, 4)).toEqual([
      'getUserMedia resolved',
      'new AudioContext 16000',
      'addModule pcmCapture.worklet.js',
      'resume',
    ]);
    expect(String(contexts[0].audioWorklet.addModule.mock.calls[0][0])).toMatch(
      /pcmCapture\.worklet/
    );
  });

  it('in a browser, opens the device, the context and the worklet module inside the call, before any await', () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    void startCapture(options().options);
    // All of it happened synchronously, in the gesture (WebKit's resume() needs one).
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(contexts).toHaveLength(1);
    expect(log).toContain('resume');
    expect(contexts[0].audioWorklet.addModule).toHaveBeenCalledTimes(1);
  });

  it('connects the stream to the worklet, never to the speakers, and reports 100 ms chunks', async () => {
    const { options: opts, chunks, onLive } = options();
    const capture = await startCapture(opts);
    expect(capture.mode).toBe('worklet');
    expect(capture.sampleRate).toBe(16000);
    const [node] = nodes;
    expect(node.name).toBe(PCM_CAPTURE_PROCESSOR);
    expect(node.options).toMatchObject({
      channelCount: 1,
      channelCountMode: 'explicit',
      processorOptions: { chunkFrames: 1600 },
    });
    expect(node.connections).toEqual([]);
    expect(log).toContain('source.connect');

    node.port.deliver({ type: 'live' });
    expect(onLive).toHaveBeenCalledTimes(1);
    // The worklet posts each 128 frame render quantum; this side assembles 100 ms chunks.
    const quantum = new Int16Array(128).fill(1000);
    for (let i = 0; i < 12; i++) node.port.deliver({ type: 'chunk', pcm: quantum.slice().buffer });
    expect(chunks).toHaveLength(0);
    node.port.deliver({ type: 'chunk', pcm: quantum.slice().buffer });
    expect(chunks).toHaveLength(1);
    expect(chunks[0].pcm.length).toBe(1600);
    expect(chunks[0].rms).toBeCloseTo(1000 / 32767, 5);
    expect(chunks[0].peak).toBeCloseTo(1000 / 32767, 5);
    expect(capture.timings.liveMs).toBeGreaterThanOrEqual(0);
    expect(capture.timings.firstChunkMs).toBeGreaterThanOrEqual(0);
  });

  it('stop() hands over the samples already here before it returns, without waiting for the worklet', async () => {
    const { options: opts, chunks } = options();
    const capture = await startCapture(opts);
    nodes[0].port.answerFlush = false;
    nodes[0].port.deliver({ type: 'chunk', pcm: new Int16Array([7, 8, 9]).buffer });
    expect(chunks).toHaveLength(0);
    void capture.stop();
    // Synchronously: the last partial chunk is out, the tracks are ended, the worklet is told.
    expect(Array.from(last(chunks)!.pcm)).toEqual([7, 8, 9]);
    expect(log).toContain('track.stop');
    expect(nodes[0].port.posted).toEqual([{ type: 'flush' }]);
    await expect(capture.ended).resolves.toBe('stopped');
    // A quantum still in flight when the person stopped is not delivered.
    nodes[0].port.deliver({ type: 'chunk', pcm: new Int16Array([1, 2]).buffer });
    expect(chunks).toHaveLength(1);
  });

  it('stop() is already settled, and closes the context once the worklet has ended', async () => {
    const { options: opts } = options();
    const capture = await startCapture(opts);
    let settled = false;
    void capture.stop().then(() => (settled = true));
    await Promise.resolve();
    expect(settled).toBe(true);
    await capture.closed;
    expect(contexts[0].state).toBe('closed');
  });

  it('closes the context after 250 ms for a worklet that never answers, and the stop never waits for it', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { options: opts } = options();
    const capture = await startCapture(opts);
    nodes[0].port.answerFlush = false;
    let stopped = false;
    void capture.stop().then(() => (stopped = true));
    await vi.advanceTimersByTimeAsync(0);
    expect(stopped).toBe(true);
    expect(log).toContain('track.stop');
    expect(contexts[0].state).not.toBe('closed');
    await vi.advanceTimersByTimeAsync(251);
    expect(contexts[0].state).toBe('closed');
  });

  describe('audio recorded before the stop that has not arrived yet', () => {
    const QUANTUM = 128;
    /** Posts `count` stamped quanta of `value`, starting at context frame `from`. */
    const post = (from: number, count: number, value: number) => {
      for (let i = 0; i < count; i++) {
        nodes[0].port.deliver({
          type: 'chunk',
          pcm: new Int16Array(QUANTUM).fill(value).buffer,
          frame: from + i * QUANTUM,
          final: false,
        });
      }
    };
    const samples = (chunks: Array<{ pcm: Int16Array }>) =>
      chunks.reduce((n, c) => n + c.pcm.length, 0);

    it('within the slack: the stop hands over what is here at once and drops the rest', async () => {
      const { options: opts, chunks } = options();
      const capture = await startCapture(opts);
      nodes[0].port.answerFlush = false;
      nodes[0].port.deliver({ type: 'live', frame: 0 });
      post(0, 10, 500);
      // The clock is two quanta (16 ms, the slack) ahead of what arrived.
      contexts[0].currentTime = (12 * QUANTUM) / 16000;
      expect(STOP_BACKLOG_SLACK_MS).toBe(16);
      let settled = false;
      void capture.stop().then(() => (settled = true));
      expect(capture.lastChunkDelivered).toBe(true);
      expect(samples(chunks)).toBe(10 * QUANTUM);
      expect(capture.timings.stopBacklogMs).toBeCloseTo(16, 5);
      expect(capture.timings.stopWaitMs).toBeLessThan(5);
      post(10 * QUANTUM, 2, 500);
      expect(samples(chunks)).toBe(10 * QUANTUM);
      await Promise.resolve();
      expect(settled).toBe(true);
    });

    it('above the slack: the stop takes the queued audio as it arrives, then hands over the last chunk', async () => {
      const { options: opts, chunks } = options();
      const capture = await startCapture(opts);
      nodes[0].port.answerFlush = false;
      nodes[0].port.deliver({ type: 'live', frame: 0 });
      post(0, 10, 500);
      // A 200 ms render held the main thread: 25 quanta wait in its queue.
      contexts[0].currentTime = (35 * QUANTUM) / 16000;
      let settled = false;
      void capture.stop().then(() => (settled = true));
      expect(capture.lastChunkDelivered).toBe(false);
      expect(capture.timings.stopBacklogMs).toBeCloseTo(200, 5);
      // The microphone goes off at once all the same.
      expect(log).toContain('track.stop');
      await Promise.resolve();
      expect(settled).toBe(false);
      post(10 * QUANTUM, 24, 700);
      expect(capture.lastChunkDelivered).toBe(false);
      post(34 * QUANTUM, 1, 700);
      // The stamps reached the stop's frame: everything recorded before it is handed over.
      expect(capture.lastChunkDelivered).toBe(true);
      expect(samples(chunks)).toBe(35 * QUANTUM);
      expect(Array.from(last(chunks)!.pcm.slice(-3))).toEqual([700, 700, 700]);
      // A quantum recorded after the stop is not.
      post(35 * QUANTUM, 1, 900);
      expect(samples(chunks)).toBe(35 * QUANTUM);
      await Promise.resolve();
      expect(settled).toBe(true);
      expect(capture.timings.stopWaitMs).toBeGreaterThanOrEqual(0);
    });

    it('the worklet’s final answer ends the wait: nothing it posted is still to come', async () => {
      const { options: opts, chunks } = options();
      const capture = await startCapture(opts);
      nodes[0].port.answerFlush = false;
      nodes[0].port.deliver({ type: 'live', frame: 0 });
      post(0, 4, 500);
      contexts[0].currentTime = (20 * QUANTUM) / 16000;
      const stopped = capture.stop();
      // The track went quiet before the stop's frame (an empty input posts nothing).
      post(4 * QUANTUM, 6, 600);
      expect(capture.lastChunkDelivered).toBe(false);
      nodes[0].port.deliver({ type: 'chunk', pcm: new Int16Array(0).buffer, final: true });
      await stopped;
      expect(capture.lastChunkDelivered).toBe(true);
      expect(samples(chunks)).toBe(10 * QUANTUM);
      await capture.closed;
      expect(contexts[0].state).toBe('closed');
    });

    it('a worklet that never answers ends the wait after 250 ms, with what arrived', async () => {
      vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
      const { options: opts, chunks } = options();
      const capture = await startCapture(opts);
      nodes[0].port.answerFlush = false;
      nodes[0].port.deliver({ type: 'live', frame: 0 });
      post(0, 4, 500);
      contexts[0].currentTime = (20 * QUANTUM) / 16000;
      let settled = false;
      void capture.stop().then(() => (settled = true));
      post(4 * QUANTUM, 2, 600);
      await vi.advanceTimersByTimeAsync(249);
      expect(settled).toBe(false);
      await vi.advanceTimersByTimeAsync(2);
      expect(settled).toBe(true);
      expect(samples(chunks)).toBe(6 * QUANTUM);
      expect(contexts[0].state).toBe('closed');
    });

    it('a cancel during the wait settles the stop and delivers nothing more', async () => {
      const { options: opts, chunks } = options();
      const capture = await startCapture(opts);
      nodes[0].port.answerFlush = false;
      nodes[0].port.deliver({ type: 'live', frame: 0 });
      post(0, 4, 500);
      contexts[0].currentTime = (20 * QUANTUM) / 16000;
      const stopped = capture.stop();
      capture.cancel();
      await stopped;
      post(4 * QUANTUM, 16, 600);
      expect(chunks).toHaveLength(0);
    });
  });

  it('cancel() ends everything and delivers nothing more', async () => {
    const { options: opts, chunks } = options();
    const capture = await startCapture(opts);
    capture.cancel();
    expect(log).toContain('track.stop');
    nodes[0].port.deliver({ type: 'chunk', pcm: new Int16Array(10).buffer, rms: 0, peak: 0 });
    expect(chunks).toHaveLength(0);
    await capture.closed;
    expect(contexts[0].state).toBe('closed');
  });

  it('a track that ends (a headset unplugged) is reported', async () => {
    const { options: opts } = options();
    const stream = new FakeMediaStream();
    getUserMedia.mockResolvedValueOnce(stream);
    const capture = await startCapture(opts);
    stream.tracks[0].dispatchEvent(new Event('ended'));
    await expect(capture.ended).resolves.toBe('track-ended');
  });

  it('falls back to the device rate when the context cannot take a stream at 16 kHz', async () => {
    failSourceAtRate = 16000;
    const { options: opts } = options();
    const capture = await startCapture(opts);
    expect(contexts).toHaveLength(2);
    expect(contexts[0].state).toBe('closed');
    expect(contexts[1].options).toEqual({ latencyHint: 'interactive' });
    expect(capture.sampleRate).toBe(48000);
    expect(nodes[0].options.processorOptions).toEqual({ chunkFrames: 4800 });
  });

  it('retries a stale device by its label, then the default device', async () => {
    getUserMedia.mockImplementation(async (constraints: MediaStreamConstraints) => {
      const id = (
        (constraints.audio as TrackConstraints).deviceId as { exact?: string } | undefined
      )?.exact;
      if (id === 'stale') throw new DOMException('gone', 'OverconstrainedError');
      return new FakeMediaStream();
    });
    const { options: opts } = options({ deviceId: 'stale', deviceLabel: 'USB Mic' });
    await startCapture(opts);
    const ids = getUserMedia.mock.calls.map(
      ([c]) => ((c.audio as TrackConstraints).deviceId as { exact?: string } | undefined)?.exact
    );
    expect(ids).toEqual(['stale', 'new-id']);

    getUserMedia.mockClear();
    const { options: unknownLabel } = options({ deviceId: 'stale', deviceLabel: 'Not here' });
    await startCapture(unknownLabel);
    const second = getUserMedia.mock.calls.map(
      ([c]) => ((c.audio as TrackConstraints).deviceId as { exact?: string } | undefined)?.exact
    );
    expect(second).toEqual(['stale', undefined]);
  });

  it('a worklet node that cannot be created leaves no track live and no context open', async () => {
    // The module loaded, but the processor is not registered.
    vi.stubGlobal(
      'AudioWorkletNode',
      class {
        constructor() {
          throw new DOMException('no such processor', 'InvalidStateError');
        }
      }
    );
    // No MediaRecorder to fall back to either: the capture fails, and the
    // microphone indicator goes off.
    vi.stubGlobal('MediaRecorder', undefined);
    const stream = new FakeMediaStream();
    getUserMedia.mockResolvedValueOnce(stream);
    const { options: opts } = options();
    await expect(startCapture(opts)).rejects.toMatchObject({ name: 'NotSupportedError' });
    expect(stream.tracks.every((track) => track.readyState === 'ended')).toBe(true);
    await Promise.resolve();
    expect(contexts.every((context) => context.state === 'closed')).toBe(true);
  });

  it('a worklet node that cannot be created falls back to MediaRecorder where there is one', async () => {
    vi.stubGlobal(
      'AudioWorkletNode',
      class {
        constructor() {
          throw new DOMException('no such processor', 'InvalidStateError');
        }
      }
    );
    class StartingRecorder extends EventTarget {
      state: 'inactive' | 'recording' = 'inactive';
      mimeType = 'audio/webm';
      start() {
        this.state = 'recording';
        queueMicrotask(() => this.dispatchEvent(new Event('start')));
      }
    }
    vi.stubGlobal('MediaRecorder', StartingRecorder);
    const stream = new FakeMediaStream();
    getUserMedia.mockResolvedValueOnce(stream);
    const { options: opts } = options();
    const capture = await startCapture(opts);
    expect(capture.mode).toBe('recorder');
    expect(stream.tracks[0].readyState).toBe('live');
    await Promise.resolve();
    expect(contexts[0].state).toBe('closed');
  });

  it('a refused microphone passes the error on and leaves no context open', async () => {
    getUserMedia.mockRejectedValueOnce(new DOMException('no', 'NotAllowedError'));
    await expect(startCapture(options().options)).rejects.toMatchObject({
      name: 'NotAllowedError',
    });
    // On the desktop none was made yet.
    expect(contexts).toHaveLength(0);

    // A prepared context, and one made inside a browser's gesture, are closed.
    prewarmCapture();
    await flush();
    getUserMedia.mockRejectedValueOnce(new DOMException('no', 'NotAllowedError'));
    await expect(startCapture(options().options)).rejects.toMatchObject({
      name: 'NotAllowedError',
    });
    document.documentElement.dataset.biorouterSurface = 'headless';
    getUserMedia.mockRejectedValueOnce(new DOMException('no', 'NotAllowedError'));
    await expect(startCapture(options().options)).rejects.toMatchObject({
      name: 'NotAllowedError',
    });
    await flush();
    expect(contexts).toHaveLength(2);
    expect(contexts.every((context) => context.state === 'closed')).toBe(true);
  });

  it('refuses before any prompt on an insecure page', async () => {
    Object.defineProperty(window, 'isSecureContext', { value: false, configurable: true });
    const { options: opts } = options();
    await expect(startCapture(opts)).rejects.toBeInstanceOf(InsecureContextError);
    expect(getUserMedia).not.toHaveBeenCalled();
    expect(contexts).toHaveLength(0);
  });
});

describe('prewarmCapture', () => {
  it('lists the devices at most every 30 s, prepares one context, and opens no microphone', () => {
    vi.useFakeTimers({ toFake: ['performance', 'Date'] });
    prewarmCapture();
    prewarmCapture();
    expect(enumerateDevices).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(PREWARM_EVERY_MS + 1);
    prewarmCapture();
    expect(enumerateDevices).toHaveBeenCalledTimes(2);
    expect(getUserMedia).not.toHaveBeenCalled();
    expect(contexts).toHaveLength(1);
    expect(contexts[0].options).toEqual({ sampleRate: 16000, latencyHint: 'interactive' });
    expect(contexts[0].audioWorklet.addModule).toHaveBeenCalledTimes(1);
  });

  it('suspends the prepared context once its module has loaded, so it renders nothing', async () => {
    startsRunning = true;
    prewarmCapture();
    expect(contexts[0].state).toBe('running');
    await flush();
    expect(contexts[0].state).toBe('suspended');
    expect(log).toContain('suspend');
  });

  it('on the desktop a press takes the prepared context and resumes it once the stream is here', async () => {
    startsRunning = true;
    prewarmCapture();
    await flush();
    log.length = 0;
    const pending = startCapture(options().options);
    // Synchronously, in the gesture: only the device (the fake logs when it is called).
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(log).toEqual(['getUserMedia resolved']);
    const capture = await pending;
    expect(log.slice(0, 2)).toEqual(['getUserMedia resolved', 'resume']);
    expect(capture.mode).toBe('worklet');
    expect(contexts).toHaveLength(1);
    expect(nodes[0].context).toBe(contexts[0]);
    expect(contexts[0].audioWorklet.addModule).toHaveBeenCalledTimes(1);
    // The next press, with nothing prepared, makes its own context after the stream.
    await capture.stop();
    await capture.closed;
    await startCapture(options().options);
    expect(contexts).toHaveLength(2);
    expect(contexts[1].audioWorklet.addModule).toHaveBeenCalledTimes(1);
  });

  it('in a browser a prepared context that waited for a gesture is resumed inside the call', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    // It starts suspended, as a context made outside a gesture does in WebKit.
    prewarmCapture();
    await flush();
    log.length = 0;
    const pending = startCapture(options().options);
    expect(log).toEqual(['getUserMedia resolved', 'resume']);
    expect((await pending).mode).toBe('worklet');
    expect(contexts).toHaveLength(1);
  });

  it('in a browser whose prepared context started on its own, later presses wait for the device first', async () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    startsRunning = true;
    prewarmCapture();
    await flush();
    expect(contexts[0].state).toBe('suspended');
    log.length = 0;
    const pending = startCapture(options().options);
    expect(log).toEqual(['getUserMedia resolved']);
    await pending;
    expect(log.slice(0, 2)).toEqual(['getUserMedia resolved', 'resume']);
  });

  it('a press while the prepared module still loads waits for it, and the context is not suspended under it', async () => {
    startsRunning = true;
    let loadModule!: () => void;
    contextsAddModule(() => new Promise<void>((resolve) => (loadModule = resolve)));
    prewarmCapture();
    const pending = startCapture(options().options);
    loadModule();
    const capture = await pending;
    await flush();
    expect(capture.mode).toBe('worklet');
    expect(contexts).toHaveLength(1);
    expect(contexts[0].state).toBe('running');
    expect(log).not.toContain('suspend');
  });

  it('prepares nothing while a capture is live, and again once it has ended', async () => {
    const capture = await startCapture(options().options);
    prewarmCapture();
    expect(contexts).toHaveLength(1);
    await capture.stop();
    await capture.closed;
    prewarmCapture();
    expect(contexts).toHaveLength(2);
    expect(contexts[1].state).not.toBe('closed');
  });

  it('a prepared context closed under it is not used', async () => {
    prewarmCapture();
    await flush();
    await contexts[0].close();
    await startCapture(options().options);
    expect(contexts).toHaveLength(2);
    expect(nodes[0].context).toBe(contexts[1]);
  });

  it('closes a prepared context whose module cannot load, and prepares no other in this page', async () => {
    failAddModule = true;
    prewarmCapture();
    await flush();
    expect(contexts[0].state).toBe('closed');
    prewarmCapture();
    expect(contexts).toHaveLength(1);
  });

  it('in a browser, prepares a context only once the page has had a gesture', () => {
    document.documentElement.dataset.biorouterSurface = 'headless';
    const userActivation = { hasBeenActive: false };
    Object.defineProperty(navigator, 'userActivation', {
      value: userActivation,
      configurable: true,
    });
    prewarmCapture();
    expect(enumerateDevices).toHaveBeenCalledTimes(1);
    expect(contexts).toHaveLength(0);
    userActivation.hasBeenActive = true;
    prewarmCapture();
    expect(contexts).toHaveLength(1);
  });

  it('does nothing on a page that could never capture', () => {
    Object.defineProperty(window, 'isSecureContext', { value: false, configurable: true });
    prewarmCapture();
    expect(enumerateDevices).not.toHaveBeenCalled();
    expect(contexts).toHaveLength(0);
  });
});

/** Every context made from now on loads its module through `load`. */
function contextsAddModule(load: () => Promise<void>): void {
  const Base = FakeAudioContext;
  vi.stubGlobal(
    'AudioContext',
    class extends Base {
      constructor(options?: ContextOptions) {
        super(options);
        this.audioWorklet.addModule = vi.fn(load);
      }
    }
  );
}

describe('the MediaRecorder fallback', () => {
  it('records when the worklet cannot load, then delivers 16 kHz chunks on stop', async () => {
    failAddModule = true;
    // jsdom's Blob has no arrayBuffer(); the browser's does.
    vi.stubGlobal('Blob', NodeBlob);
    const recorders: FakeRecorder[] = [];
    class FakeRecorder extends EventTarget {
      state: 'inactive' | 'recording' = 'inactive';
      mimeType = 'audio/webm';
      constructor(readonly stream: FakeMediaStream) {
        super();
        recorders.push(this);
      }
      start() {
        this.state = 'recording';
        queueMicrotask(() => this.dispatchEvent(new Event('start')));
      }
      stop() {
        this.state = 'inactive';
        const data = new Event('dataavailable') as Event & { data: Blob };
        data.data = new Blob([new Uint8Array([1, 2, 3])]);
        this.dispatchEvent(data);
        this.dispatchEvent(new Event('stop'));
      }
    }
    const rendered = new Float32Array(16000 * 0.25).fill(0.5);
    class FakeOffline {
      destination = {};
      constructor(
        readonly channels: number,
        readonly length: number,
        readonly rate: number
      ) {}
      async decodeAudioData() {
        return { duration: 0.25 };
      }
      createBufferSource() {
        return { buffer: null, connect: () => undefined, start: () => undefined };
      }
      async startRendering() {
        return { getChannelData: () => rendered };
      }
    }
    vi.stubGlobal('MediaRecorder', FakeRecorder);
    vi.stubGlobal('OfflineAudioContext', FakeOffline);

    const { options: opts, chunks, onLive } = options();
    const capture = await startCapture(opts);
    expect(capture.mode).toBe('recorder');
    expect(onLive).toHaveBeenCalled();
    expect(contexts[0].state).toBe('closed');
    await capture.stop();
    expect(chunks.map((c) => c.pcm.length)).toEqual([1600, 1600, 800]);
    expect(chunks[0].pcm[0]).toBe(Math.round(0.5 * 32767));
    expect(log).toContain('track.stop');
  });
});
