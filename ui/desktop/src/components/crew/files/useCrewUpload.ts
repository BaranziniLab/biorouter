import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { beginTransfer, pauseTransfer, resumeTransfer, type CrewTransfer } from '../crewTransfers';
import { filesCopy } from './copy';
import { isTransferActive, useCrewTransfers } from './useCrewTransfers';

export interface CrewUploadOptions {
  connectionId: string;
  channelId: string;
  /**
   * The privacy mode the observer verified for this connection, or `undefined` while it is
   * unverified. It is sent as `expected_mode`, so the daemon refuses the file selection if the
   * connection's privacy changed after the view was verified. Without it no picker opens.
   */
  expectedMode: 'private' | 'public' | undefined;
  /** A watched upload finished: add it to the draft. */
  onReady(file: { id: string; name: string }): void;
}

export interface CrewUpload {
  /** Open the secure native picker and start an upload to this channel. */
  upload(): Promise<void>;
  /** True while the picker is open or the upload is being registered. */
  choosing: boolean;
  /**
   * The last upload failure in words, for exactly one alert. Empty when there is none. It
   * lasts only while it is true: the next upload, pause or resume, another connection or
   * channel, another verified privacy mode, `forget()` and `dismissError()` each clear it.
   */
  error: string;
  /** Show a failure found before the picker opened (a folder, a file too large…). */
  reportError(message: string): void;
  /** Clear the failure: the person dismissed it, or moved on (edited the draft, sent). */
  dismissError(): void;
  /**
   * The uploads to show as composer chips: every active upload to this channel, plus those
   * this composer started that stopped before finishing (paused or failed).
   */
  chips: CrewTransfer[];
  pause(transfer: CrewTransfer): Promise<void>;
  resume(transfer: CrewTransfer): Promise<void>;
  /**
   * Stop watching: an upload still on its way is no longer added to the draft when it
   * finishes (it waits under "Uploaded, not sent" instead), an answer still in flight is
   * ignored, and the failure on screen is cleared, since it describes the view that was
   * reset. For a reset that cleared the draft's protected state, or a new privacy scope.
   */
  forget(): void;
}

const message = (failure: unknown, fallback: string) =>
  failure instanceof Error && failure.message ? failure.message : fallback;

/**
 * The composer's upload: the same main-process picker IPC and the same `beginTransfer`
 * payload as the legacy `CrewUpload`, byte for byte. The file capability comes only from the
 * picker the main process shows; the renderer never sees a path or a byte of the file.
 *
 * An upload this hook starts is watched; when its record reaches `completed` it is handed to
 * `onReady` once. A different connection or channel forgets the watch and ignores any answer
 * still in flight, so an upload never lands in another channel's draft. Records come from the
 * one shared transfers poller.
 */
export function useCrewUpload({
  connectionId,
  channelId,
  expectedMode,
  onReady,
}: CrewUploadOptions): CrewUpload {
  const { transfers, refresh } = useCrewTransfers(connectionId);
  const [error, setError] = useState('');
  const [choosing, setChoosing] = useState(false);
  const [watched, setWatched] = useState<ReadonlySet<string>>(() => new Set());
  const choosingNow = useRef(false);
  /** Uploads already handed to `onReady`, so a re-render can never add one twice. */
  const delivered = useRef<Set<string>>(new Set());
  const generation = useRef(0);
  const ready = useRef(onReady);
  useEffect(() => {
    ready.current = onReady;
  }, [onReady]);

  useEffect(() => {
    setWatched(new Set());
    setError('');
    delivered.current = new Set();
    return () => {
      generation.current += 1;
    };
  }, [connectionId, channelId]);

  // A failure met under one privacy mode no longer describes another. Above all, the pinned
  // "Refresh the workspace to verify connection privacy…" must not outlive the verification it
  // asks for: once the observer verifies the mode, the advice is wrong.
  useEffect(() => {
    setError('');
  }, [expectedMode]);

  const channelUploads = useMemo(
    () =>
      transfers.filter(
        (item) =>
          item.direction === 'upload' &&
          item.connection_id === connectionId &&
          item.channel_id === channelId
      ),
    [transfers, connectionId, channelId]
  );

  useEffect(() => {
    const finished = channelUploads.filter(
      (item) =>
        watched.has(item.id) &&
        item.state === 'completed' &&
        item.blob_id &&
        !delivered.current.has(item.id)
    );
    if (finished.length === 0) return;
    for (const item of finished) delivered.current.add(item.id);
    setWatched((current) => {
      const next = new Set(current);
      for (const item of finished) next.delete(item.id);
      return next;
    });
    for (const item of finished) ready.current({ id: item.blob_id as string, name: item.name });
  }, [channelUploads, watched]);

  const upload = useCallback(async () => {
    if (choosingNow.current) return;
    const current = generation.current;
    choosingNow.current = true;
    setChoosing(true);
    setError('');
    try {
      if (!expectedMode) throw new Error(filesCopy.privacyPending);
      const transfer = await beginTransfer({
        expected_mode: expectedMode,
        connection_id: connectionId,
        channel_id: channelId,
        direction: 'upload',
      });
      if (current !== generation.current) return;
      if (transfer) setWatched((items) => new Set(items).add(transfer.id));
      await refresh();
    } catch (failure) {
      if (current === generation.current) setError(message(failure, filesCopy.uploadFailed));
    } finally {
      choosingNow.current = false;
      setChoosing(false);
    }
  }, [expectedMode, connectionId, channelId, refresh]);

  const run = useCallback(
    async (operation: () => Promise<unknown>) => {
      const current = generation.current;
      setError('');
      try {
        await operation();
        await refresh();
      } catch (failure) {
        if (current === generation.current) setError(message(failure, filesCopy.transferFailed));
      }
    },
    [refresh]
  );

  const chips = useMemo(
    () =>
      channelUploads.filter(
        (item) =>
          isTransferActive(item) ||
          (watched.has(item.id) &&
            item.state !== 'completed' &&
            item.state !== 'publication_unconfirmed')
      ),
    [channelUploads, watched]
  );

  const dismissError = useCallback(() => setError(''), []);
  const forget = useCallback(() => {
    generation.current += 1;
    setWatched(new Set());
    setError('');
  }, []);
  const pause = useCallback(
    (transfer: CrewTransfer) => run(() => pauseTransfer(transfer.id)),
    [run]
  );
  const resume = useCallback(
    (transfer: CrewTransfer) => run(() => resumeTransfer(transfer)),
    [run]
  );

  return {
    upload,
    choosing,
    error,
    reportError: setError,
    dismissError,
    chips,
    pause,
    resume,
    forget,
  };
}
