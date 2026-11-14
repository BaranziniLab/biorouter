import { act, cleanup, render } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { HELD_FOR_RETRY_MS, type AnywhereSource } from '../../utils/dictationAnywhere';
import type { CaptureOptions } from './capture';
import type { StreamRequest, Transcript } from './dictationApi';
import DictationCapturePage from './DictationCapturePage';
import { KEEP_FAILED_MS } from './dictationSession';
import { resetDictationStoreForTests } from './dictationStore';
import {
  Deferred,
  FakeCapture,
  FakeStream,
  fakeStatusApi,
  secureMediaPage,
  speechStatus,
  transcript,
} from './dictationTestKit';
import { DictationStreamError } from './transport';

/**
 * The hidden page behind **Dictate to clipboard** keeps a recording whose transcription failed for
 * **Retry**, as the composer does (owner decision, 2026-11-13). Main's half, the card and the hold's
 * end, is pinned at its source in `utils/dictationAnywhereMain.test.ts`.
 */

const fakes = vi.hoisted(() => ({
  captures: [] as unknown[],
  streams: [] as unknown[],
}));

vi.mock('./capture', async (importActual) => ({
  ...(await importActual<typeof import('./capture')>()),
  startCapture: vi.fn(async (options: CaptureOptions) => {
    const capture = new FakeCapture(options);
    fakes.captures.push(capture);
    return capture;
  }),
}));

vi.mock('./transport', async (importActual) => ({
  ...(await importActual<typeof import('./transport')>()),
  createDictationStream: vi.fn((request: StreamRequest) => {
    const stream = new RetryableStream(request);
    fakes.streams.push(stream);
    return stream;
  }),
}));

/** A stream whose `retry` the test answers apart from its first `finish`. */
class RetryableStream extends FakeStream {
  retries: Array<Deferred<Transcript>> = [];
  retry(): Promise<Transcript> {
    const answer = new Deferred<Transcript>();
    this.retries.push(answer);
    return answer.promise;
  }
}

type Command =
  | {
      type: 'start';
      source: AnywhereSource;
      privateConfirmed: boolean;
      generation: number;
      pressedAtEpochMs: number;
    }
  | { type: 'stop' | 'cancel' | 'retry' | 'drop' };

let send: (command: Command) => void;
let replies: ReturnType<typeof vi.fn>;

const settle = async () => {
  for (let i = 0; i < 8; i++) await act(async () => {});
};
const lastReply = () => replies.mock.calls[replies.mock.calls.length - 1]?.[0];
const stream = (index = 0) => fakes.streams[index] as RetryableStream;
const capture = (index = 0) => fakes.captures[index] as FakeCapture;

/** Start a dictation from `source`, speak, stop, and fail its finish with `error`. */
async function failedDictation(source: AnywhereSource, error: unknown, generation = 7) {
  const index = fakes.streams.length;
  act(() =>
    send({ type: 'start', source, privateConfirmed: false, generation, pressedAtEpochMs: 0 })
  );
  await settle();
  capture(index).live();
  capture(index).speak(1);
  act(() => send({ type: 'stop' }));
  await settle();
  stream(index).finishDeferred.reject(error);
  await settle();
  return stream(index);
}

beforeEach(() => {
  fakes.captures.length = 0;
  fakes.streams.length = 0;
  secureMediaPage();
  resetDictationStoreForTests(fakeStatusApi(() => speechStatus()).api);
  replies = vi.fn();
  Object.assign(window, {
    electron: {
      dictationAnywhereCaptureReply: replies,
      dictationAnywhereCaptureReady: vi.fn(),
      onDictationAnywhereCapture: (callback: (command: Command) => void) => {
        send = callback;
        return () => undefined;
      },
      onDictationLockScreen: () => () => undefined,
    },
  });
  render(<DictationCapturePage />);
});

afterEach(() => {
  cleanup();
  resetDictationStoreForTests();
  vi.useRealTimers();
});

describe('a clipboard dictation whose transcription failed', () => {
  it('holds the recording as long as the composer does', () => {
    expect(HELD_FOR_RETRY_MS).toBe(KEEP_FAILED_MS);
  });

  it('keeps its stream for Retry, and a Retry sends the same audio again and copies the text', async () => {
    const failed = await failedDictation('tray', new DictationStreamError('engineFailed'));
    expect(lastReply()).toEqual({
      type: 'failed',
      reason: 'engine-failed',
      held: true,
      generation: 7,
    });
    expect(failed.cancelled).toBe(false);

    act(() => send({ type: 'retry' }));
    await settle();
    expect(failed.retries).toHaveLength(1);
    const text = transcript('the held recording');
    failed.retries[0].resolve(text);
    await settle();
    expect(lastReply()).toEqual({
      type: 'finished',
      transcript: text,
      stopToTextMs: expect.any(Number),
      generation: 7,
    });
    expect(failed.released).toBe(true);
    expect(failed.cancelled).toBe(false);
  });

  it('keeps it again when the Retry fails the same way', async () => {
    const failed = await failedDictation('shortcut', new DictationStreamError('tooManyStreams'));
    act(() => send({ type: 'retry' }));
    await settle();
    failed.retries[0].reject(new DictationStreamError('engineFailed'));
    await settle();
    expect(lastReply()).toMatchObject({ type: 'failed', held: true });
    expect(failed.cancelled).toBe(false);
  });

  it('drops the audio when the hold times out, and a later Retry has nothing to send', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] });
    const failed = await failedDictation('tray', new DictationStreamError('engineFailed'));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(HELD_FOR_RETRY_MS - 1);
    });
    expect(failed.cancelled).toBe(false);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(1);
    });
    expect(failed.cancelled).toBe(true);
    act(() => send({ type: 'retry' }));
    await settle();
    expect(failed.retries).toHaveLength(0);
    expect(lastReply()).toEqual({ type: 'failed', reason: 'engine-failed', generation: 7 });
  });

  it('drops the audio when main ends the hold, or a new dictation starts', async () => {
    const first = await failedDictation('tray', new DictationStreamError('engineFailed'));
    act(() => send({ type: 'drop' }));
    expect(first.cancelled).toBe(true);

    const second = await failedDictation('tray', new DictationStreamError('engineFailed'), 8);
    expect(second.cancelled).toBe(false);
    act(() =>
      send({
        type: 'start',
        source: 'shortcut',
        privateConfirmed: false,
        generation: 9,
        pressedAtEpochMs: 0,
      })
    );
    await settle();
    expect(second.cancelled).toBe(true);
  });

  it('drops the audio at once after a failure Retry cannot help, or for a preview dictation', async () => {
    const lost = await failedDictation('tray', new DictationStreamError('lost'));
    expect(lost.cancelled).toBe(true);
    expect(lastReply()).toEqual({ type: 'failed', reason: 'engine-failed', generation: 7 });

    const preview = await failedDictation('preview', new DictationStreamError('engineFailed'), 8);
    expect(preview.cancelled).toBe(true);
    expect(lastReply()).toEqual({ type: 'failed', reason: 'engine-failed', generation: 8 });
  });
});
