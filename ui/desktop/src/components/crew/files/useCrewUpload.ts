import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { beginTransfer, pauseTransfer, resumeTransfer, type CrewTransfer } from '../crewTransfers';
import type { CrewShareDroppedFileResult } from '../../../utils/crewSharePathBridge';
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

/** How the native share confirmation names where a dropped file goes. Display text only. */
export interface CrewShareNames {
  channelName: string;
  workspaceName: string;
}

export interface CrewUpload {
  /** Open the secure native picker and start an upload to this channel. */
  upload(): Promise<void>;
  /**
   * Share a file the person dropped or pasted (D-DROP): the preload resolves the `File` itself
   * and the main process asks in a native Share / Cancel dialog naming the file, its size, its
   * full path and this channel. Only Share yields the file capability, and the upload then
   * starts exactly as the picker's does. Cancel shows nothing; a refusal (a folder, a shortcut
   * out of its folder, a file over the limit…) is the one error. Only on a desktop surface that
   * has the confirmation ({@link canShareDroppedFiles}).
   */
  shareFile(file: File, names: CrewShareNames): Promise<void>;
  /** True while the picker or the share confirmation is open, or the upload is being registered. */
  choosing: boolean;
  /** True while the native share confirmation (not the picker) is the thing open. */
  confirming: boolean;
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
  /**
   * Every upload record of this channel on this computer, finished or not: what a draft file's
   * checksum is read from once its upload completes (Q3-13).
   */
  uploads: CrewTransfer[];
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

type ShareDroppedFile = (
  file: File,
  destination: {
    expectedMode?: 'private' | 'public';
    connectionId: string;
    channelId: string;
    channelName: string;
    workspaceName: string;
  }
) => Promise<CrewShareDroppedFileResult>;

/** The preload's D-DROP confirmation, when this surface has one (a desktop build with it). */
function shareDroppedFile(): ShareDroppedFile | null {
  const share = (window as { electron?: { crewShareDroppedFile?: unknown } }).electron
    ?.crewShareDroppedFile;
  return typeof share === 'function' ? (share as ShareDroppedFile) : null;
}

/** True when a dropped or pasted file can be shared through the native confirmation here. */
export function canShareDroppedFiles(): boolean {
  return shareDroppedFile() !== null;
}

/** The main process's answer, read defensively: anything unexpected is treated as a failure. */
function readShareResult(
  result: unknown
):
  | { outcome: 'shared'; capability_id: string; name: string; size: number | null }
  | { outcome: 'cancelled' }
  | { outcome: 'refused'; message: string }
  | null {
  if (!result || typeof result !== 'object') return null;
  const answer = result as Record<string, unknown>;
  if (answer.outcome === 'cancelled') return { outcome: 'cancelled' };
  if (answer.outcome === 'refused')
    return typeof answer.message === 'string' && answer.message
      ? { outcome: 'refused', message: answer.message }
      : null;
  if (
    answer.outcome === 'shared' &&
    typeof answer.capability_id === 'string' &&
    answer.capability_id &&
    typeof answer.name === 'string'
  )
    return {
      outcome: 'shared',
      capability_id: answer.capability_id,
      name: answer.name,
      size: typeof answer.size === 'number' ? answer.size : null,
    };
  return null;
}

/**
 * The composer's upload: the same main-process picker IPC and the same `beginTransfer`
 * payload as the legacy `CrewUpload`, byte for byte. The file capability comes only from the
 * main process: from the picker it shows, or, for a dropped or pasted file, from the native
 * Share / Cancel confirmation it shows (`shareFile`, D-DROP). The renderer never hands a path to
 * either, and never reads a byte of the file.
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
  const [confirming, setConfirming] = useState(false);
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

  const shareFile = useCallback(
    async (file: File, names: CrewShareNames) => {
      const share = shareDroppedFile();
      if (!share || choosingNow.current) return;
      const current = generation.current;
      choosingNow.current = true;
      setChoosing(true);
      setConfirming(true);
      setError('');
      try {
        if (!expectedMode) throw new Error(filesCopy.privacyPending);
        // The preload turns the `File` into a path itself; nothing here names one.
        const answer = readShareResult(
          await share(file, {
            expectedMode,
            connectionId,
            channelId,
            channelName: names.channelName,
            workspaceName: names.workspaceName,
          })
        );
        setConfirming(false);
        if (!answer) throw new Error(filesCopy.uploadFailed);
        if (answer.outcome === 'cancelled') return;
        if (answer.outcome === 'refused') {
          if (current === generation.current) setError(answer.message);
          return;
        }
        // The person chose Share for this channel, so the upload goes there even if the view
        // moved on meanwhile; it then waits under "Uploaded, not sent", as a picked file does.
        const transfer = await beginTransfer(
          {
            expected_mode: expectedMode,
            connection_id: connectionId,
            channel_id: channelId,
            direction: 'upload',
          },
          { capability_id: answer.capability_id, name: answer.name, size: answer.size }
        );
        if (current !== generation.current) return;
        if (transfer) setWatched((items) => new Set(items).add(transfer.id));
        await refresh();
      } catch (failure) {
        if (current === generation.current) setError(message(failure, filesCopy.uploadFailed));
      } finally {
        choosingNow.current = false;
        setChoosing(false);
        setConfirming(false);
      }
    },
    [expectedMode, connectionId, channelId, refresh]
  );

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
    shareFile,
    choosing,
    confirming,
    error,
    reportError: setError,
    dismissError,
    chips,
    uploads: channelUploads,
    pause,
    resume,
    forget,
  };
}
