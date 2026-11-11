import { useEffect, useState } from 'react';
import { startCapture, type Capture } from './capture';
import { createDictationStream, type DictationStream } from './transport';
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
  | { type: 'cancel' };

type CaptureReply =
  | { type: 'started'; firstFrameMs?: number }
  | {
      type: 'finished';
      transcript: Awaited<ReturnType<DictationStream['finish']>>;
      stopToTextMs: number;
    }
  | { type: 'failed'; reason: string }
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
    const reply = (value: CaptureReply, token = generation) => {
      if (token === generation)
        window.electron.dictationAnywhereCaptureReply?.({ ...value, generation: mainGeneration });
    };

    const start = async (
      source: AnywhereSource,
      privateConfirmed: boolean,
      commandGeneration: number,
      pressedAtEpochMs: number
    ) => {
      if (capture || stream || starting) return;
      const token = ++generation;
      mainGeneration = commandGeneration;
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
      } catch {
        void currentStream.cancel();
        if (sameCaptureOperation(token, generation, cancelled))
          reply({ type: 'failed', reason: 'engine-failed' });
      } finally {
        if (finishingStream === currentStream) finishingStream = null;
        if (token === generation) setPhase('idle');
      }
    };

    const cancel = () => {
      generation += 1;
      cancelled = true;
      starting = false;
      stopRequested = false;
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
    });
    const removeLock = window.electron.onDictationLockScreen?.(() => void stop());
    window.electron.dictationAnywhereCaptureReady?.();
    return () => {
      remove?.();
      removeLock?.();
      generation += 1;
      cancelled = true;
      capture?.cancel();
      void stream?.cancel();
      abortFinishingStream(finishingStream);
    };
  }, []);

  return <main aria-label="Dictation capture" data-phase={phase} />;
}
