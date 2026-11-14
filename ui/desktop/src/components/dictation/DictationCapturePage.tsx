import { useEffect, useState } from 'react';
import { startCapture, type Capture } from './capture';
import { createDictationStream, DictationStreamError, type DictationStream } from './transport';
import {
  ensureDictationStatus,
  getDictationStatusSnapshot,
  resolveDictationPhase,
} from './dictationStore';
import {
  externalStreamRequest,
  sameCaptureOperation,
  elapsedFromPress,
  abortFinishingStream,
  keepsFailedRecording,
  HELD_FOR_RETRY_MS,
  type AnywhereSource,
} from '../../utils/dictationAnywhere';

type CaptureCommand =
  | {
      type: 'start';
      source: AnywhereSource;
      privateConfirmed: boolean;
      generation: number;
      pressedAtEpochMs: number;
    }
  | { type: 'stop' }
  | { type: 'cancel' }
  /** **Retry** on the review card: transcribe the held recording again. */
  | { type: 'retry' }
  /** The hold ended in main (time out, the card closed, a new dictation, Off): drop the audio. */
  | { type: 'drop' };

type CaptureReply =
  | { type: 'started'; firstFrameMs?: number }
  | {
      type: 'finished';
      transcript: Awaited<ReturnType<DictationStream['finish']>>;
      stopToTextMs: number;
    }
  /** `held`: the recording is kept for **Retry** (`keepsFailedRecording`). */
  | { type: 'failed'; reason: string; held?: boolean }
  | { type: 'cancelled' };

export default function DictationCapturePage() {
  const [phase, setPhase] = useState<'idle' | 'recording' | 'transcribing'>('idle');

  useEffect(() => {
    let capture: Capture | null = null;
    let stream: DictationStream | null = null;
    let finishingStream: DictationStream | null = null;
    let pending: Int16Array[] = [];
    let cancelled = false;
    let generation = 0;
    let starting = false;
    let stopRequested = false;
    let mainGeneration = 0;
    let startedFrom: AnywhereSource | null = null;
    // A failed recording kept for **Retry**, as the composer keeps one (`keepForRetry` in
    // `dictationSession.ts`): its stream, which holds the audio already sent, and the timer that
    // drops it. Main owns the hold and says `drop` when it ends; this page drops the audio at the
    // same deadline on its own as well, so the audio never outlives it, whatever main does.
    let held: { stream: DictationStream; timer: ReturnType<typeof setTimeout> } | null = null;
    const reply = (value: CaptureReply, token = generation) => {
      if (token === generation)
        window.electron.dictationAnywhereCaptureReply?.({ ...value, generation: mainGeneration });
    };

    /** Drop a held recording: `DELETE` the stream and zero its audio. */
    const drop = () => {
      if (!held) return;
      clearTimeout(held.timer);
      void held.stream.cancel();
      held = null;
    };

    /**
     * A finish or a retry failed. A tray or shortcut dictation whose failure the stream can send
     * again keeps its recording for **Retry**; anything else drops it at once.
     */
    const fail = (token: number, failed: DictationStream, error: unknown) => {
      const current = sameCaptureOperation(token, generation, cancelled);
      const retryable = error instanceof DictationStreamError && error.retryable;
      if (current && keepsFailedRecording(startedFrom, retryable)) {
        drop();
        const timer = setTimeout(() => {
          if (held?.stream === failed) drop();
        }, HELD_FOR_RETRY_MS);
        held = { stream: failed, timer };
        reply({ type: 'failed', reason: 'engine-failed', held: true });
        return;
      }
      void failed.cancel();
      if (current) reply({ type: 'failed', reason: 'engine-failed' });
    };

    const start = async (
      source: AnywhereSource,
      privateConfirmed: boolean,
      commandGeneration: number,
      pressedAtEpochMs: number
    ) => {
      if (capture || stream || starting) return;
      // A new dictation ends any hold; main sends `drop` first as well.
      drop();
      const token = ++generation;
      mainGeneration = commandGeneration;
      startedFrom = source;
      starting = true;
      cancelled = false;
      stopRequested = false;
      const status = await ensureDictationStatus().catch(() => null);
      if (!sameCaptureOperation(token, generation, cancelled)) return;
      if (!status || resolveDictationPhase(getDictationStatusSnapshot()).phase !== 'idle') {
        starting = false;
        reply({ type: 'failed', reason: 'policy-or-no-model' });
        return;
      }
      try {
        const nextCapture = await startCapture({
          noiseSuppression: false,
          onChunk: (pcm) => {
            if (!sameCaptureOperation(token, generation, cancelled)) return;
            if (stream) stream.push(pcm);
            else pending.push(pcm);
          },
          onLive: () => {
            if (sameCaptureOperation(token, generation, cancelled))
              reply({
                type: 'started',
                firstFrameMs: elapsedFromPress(
                  pressedAtEpochMs,
                  performance.timeOrigin + performance.now()
                ),
              });
          },
        });
        if (!sameCaptureOperation(token, generation, cancelled)) {
          nextCapture.cancel();
          return;
        }
        starting = false;
        capture = nextCapture;
        void nextCapture.ended.then((reason) => {
          if (token === generation && reason === 'track-ended') void stop();
        });
        stream = createDictationStream(
          externalStreamRequest(nextCapture.sampleRate, source, privateConfirmed)
        );
        for (const pcm of pending) stream.push(pcm);
        pending = [];
        setPhase('recording');
        if (stopRequested) void stop();
      } catch {
        if (!sameCaptureOperation(token, generation, cancelled)) return;
        starting = false;
        pending = [];
        setPhase('idle');
        reply({ type: 'failed', reason: 'capture-failed' });
      }
    };

    const stop = async () => {
      if (!capture || !stream) {
        stopRequested = true;
        return;
      }
      const token = generation;
      const currentCapture = capture;
      const currentStream = stream;
      capture = null;
      stream = null;
      finishingStream = currentStream;
      setPhase('transcribing');
      const stoppedAt = performance.now();
      try {
        await currentCapture.stop();
        if (!sameCaptureOperation(token, generation, cancelled)) return;
        const transcript = await currentStream.finish();
        if (!sameCaptureOperation(token, generation, cancelled)) return;
        currentStream.release();
        reply({ type: 'finished', transcript, stopToTextMs: performance.now() - stoppedAt });
      } catch (error) {
        fail(token, currentStream, error);
      } finally {
        if (finishingStream === currentStream) finishingStream = null;
        if (token === generation) setPhase('idle');
      }
    };

    /** **Retry**: the held audio into a new stream, then the transcript, as a first finish. */
    const retry = async () => {
      if (capture || stream || starting || finishingStream) return;
      const kept = held;
      if (!kept) {
        // The hold ended here first: there is nothing to send again.
        reply({ type: 'failed', reason: 'engine-failed' });
        return;
      }
      clearTimeout(kept.timer);
      held = null;
      const token = generation;
      finishingStream = kept.stream;
      setPhase('transcribing');
      const retriedAt = performance.now();
      try {
        const transcript = await kept.stream.retry();
        if (!sameCaptureOperation(token, generation, cancelled)) return;
        kept.stream.release();
        reply({ type: 'finished', transcript, stopToTextMs: performance.now() - retriedAt });
      } catch (error) {
        fail(token, kept.stream, error);
      } finally {
        if (finishingStream === kept.stream) finishingStream = null;
        if (token === generation) setPhase('idle');
      }
    };

    const cancel = () => {
      generation += 1;
      cancelled = true;
      starting = false;
      stopRequested = false;
      drop();
      capture?.cancel();
      void stream?.cancel();
      abortFinishingStream(finishingStream);
      finishingStream = null;
      capture = null;
      stream = null;
      pending = [];
      setPhase('idle');
      reply({ type: 'cancelled' });
    };

    const remove = window.electron.onDictationAnywhereCapture?.((command: CaptureCommand) => {
      if (command.type === 'start')
        void start(
          command.source,
          command.privateConfirmed,
          command.generation,
          command.pressedAtEpochMs
        );
      if (command.type === 'stop') void stop();
      if (command.type === 'cancel') cancel();
      if (command.type === 'retry') void retry();
      if (command.type === 'drop') drop();
    });
    const removeLock = window.electron.onDictationLockScreen?.(() => void stop());
    window.electron.dictationAnywhereCaptureReady?.();
    return () => {
      remove?.();
      removeLock?.();
      generation += 1;
      cancelled = true;
      drop();
      capture?.cancel();
      void stream?.cancel();
      abortFinishingStream(finishingStream);
    };
  }, []);

  return <main aria-label="Dictation capture" data-phase={phase} />;
}
