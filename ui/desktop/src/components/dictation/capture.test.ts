import { Blob as NodeBlob } from 'node:buffer';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import {
  InsecureContextError,
  PCM_CAPTURE_PROCESSOR,
  PREWARM_EVERY_MS,
  prewarmCapture,
  resetCapturePrewarmForTests,
  startCapture,
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
      const pcm = new Int16Array([7, 8, 9]);
      queueMicrotask(() =>
        this.onmessage?.({
          data: { type: 'chunk', pcm: pcm.buffer, rms: 0.1, peak: 0.2, final: true },
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

class FakeAudioContext {
  sampleRate: number;
  state: 'running' | 'suspended' | 'closed' = 'suspended';
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
  log.length = 0;
  contexts = [];
  nodes = [];
  failSourceAtRate = null;
  failAddModule = false;
  nativeRate = 48000;
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
  vi.unstubAllGlobals();
  vi.useRealTimers();
});

describe('startCapture', () => {
  it('opens the device, the 16 kHz context and the worklet module inside the call, before any await', () => {
    const { options: opts } = options({ noiseSuppression: true, deviceId: 'dev-1' });
    void startCapture(opts);
    // All of it happened synchronously, in the gesture.
    expect(getUserMedia).toHaveBeenCalledTimes(1);
    expect(getUserMedia).toHaveBeenCalledWith({
      audio: {
        deviceId: { exact: 'dev-1' },
        echoCancellation: false,
        noiseSuppression: true,
        autoGainControl: false,
      },
    });
    expect(contexts).toHaveLength(1);
    expect(contexts[0].options).toEqual({ sampleRate: 16000, latencyHint: 'interactive' });
    expect(log).toContain('resume');
    expect(contexts[0].audioWorklet.addModule).toHaveBeenCalledTimes(1);
    expect(String(contexts[0].audioWorklet.addModule.mock.calls[0][0])).toMatch(
      /pcmCapture\.worklet/
    );
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
    const pcm = new Int16Array(1600).fill(1000);
    node.port.deliver({ type: 'chunk', pcm: pcm.buffer, rms: 0.03, peak: 0.03, final: false });
    expect(chunks).toHaveLength(1);
    expect(chunks[0].pcm.length).toBe(1600);
    expect(chunks[0].rms).toBe(0.03);
    expect(capture.timings.liveMs).toBeGreaterThanOrEqual(0);
    expect(capture.timings.firstChunkMs).toBeGreaterThanOrEqual(0);
  });

  it('stop() delivers the worklet’s last partial chunk, ends the tracks and closes the context', async () => {
    const { options: opts, chunks } = options();
    const capture = await startCapture(opts);
    await capture.stop();
    expect(nodes[0].port.posted).toEqual([{ type: 'flush' }]);
    expect(Array.from(last(chunks)!.pcm)).toEqual([7, 8, 9]);
    expect(log).toContain('track.stop');
    await capture.closed;
    expect(contexts[0].state).toBe('closed');
    await expect(capture.ended).resolves.toBe('stopped');
  });

  it('stop() does not wait more than 250 ms for a worklet that never answers', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const { options: opts } = options();
    const capture = await startCapture(opts);
    nodes[0].port.answerFlush = false;
    let stopped = false;
    void capture.stop().then(() => (stopped = true));
    await vi.advanceTimersByTimeAsync(249);
    expect(stopped).toBe(false);
    await vi.advanceTimersByTimeAsync(2);
    expect(stopped).toBe(true);
    expect(log).toContain('track.stop');
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

  it('a refused microphone closes the context and passes the error on', async () => {
    getUserMedia.mockRejectedValueOnce(new DOMException('no', 'NotAllowedError'));
    const { options: opts } = options();
    await expect(startCapture(opts)).rejects.toMatchObject({ name: 'NotAllowedError' });
    await Promise.resolve();
    expect(contexts[0].state).toBe('closed');
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
  it('lists the devices at most every 30 s, and opens nothing', () => {
    vi.useFakeTimers({ toFake: ['performance', 'Date'] });
    resetCapturePrewarmForTests();
    prewarmCapture();
    prewarmCapture();
    expect(enumerateDevices).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(PREWARM_EVERY_MS + 1);
    prewarmCapture();
    expect(enumerateDevices).toHaveBeenCalledTimes(2);
    expect(getUserMedia).not.toHaveBeenCalled();
    expect(contexts).toHaveLength(0);
  });

  it('does nothing on a page that could never capture', () => {
    resetCapturePrewarmForTests();
    Object.defineProperty(window, 'isSecureContext', { value: false, configurable: true });
    prewarmCapture();
    expect(enumerateDevices).not.toHaveBeenCalled();
  });
});

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
