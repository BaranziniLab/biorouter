import { useEffect, useState } from 'react';
import { Badge } from '../../ui/badge';
import { ConfirmationModal } from '../../ui/ConfirmationModal';
import { Loader2, Pause, Play, XIcon } from '../../icons/app-icons';
import { failureSentence } from '../../../utils/ipcError';
import { cancelUpload, type CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
import { visibleFileText } from './fileName';
import { ChipAction } from './GlyphButton';
import { refreshCrewTransfers } from './useCrewTransfers';
import './files.css';

const RING_RADIUS = 6.5;
const RING_CIRCUMFERENCE = 2 * Math.PI * RING_RADIUS;

/**
 * A 16px progress ring. The number beside it carries the value for everyone, so the ring
 * itself is decoration and hidden from assistive technology. Its fill eases with the progress
 * bar's own `--dur-med-min`, which the global reduced-motion reset makes instant.
 */
export function ProgressRing({ percent }: { percent: number }) {
  const clamped = Math.max(0, Math.min(100, percent));
  return (
    <svg className="crew-progress-ring" viewBox="0 0 16 16" aria-hidden="true" focusable="false">
      <circle className="crew-progress-ring-track" cx="8" cy="8" r={RING_RADIUS} />
      <circle
        className="crew-progress-ring-fill"
        cx="8"
        cy="8"
        r={RING_RADIUS}
        strokeDasharray={RING_CIRCUMFERENCE}
        strokeDashoffset={RING_CIRCUMFERENCE * (1 - clamped / 100)}
      />
    </svg>
  );
}

/** How long an upload shows "Uploading…" before a number and Pause (Q3-16). */
export const UPLOAD_SETTLE_MS = 1000;

/**
 * An upload still on its way into the composer: `[counts.csv 42% ⏸]`.
 *
 * For its first second, AND until it has moved 1%, it shows the one spinner and "Uploading…": a
 * 100-byte file used to sit at an empty ring, "0%" and a pause glyph for a second or two, and read
 * as paused (Q3-16). Round 3's fix joined the two with "and" where "or" was meant, so a file still
 * at 0% after its first second showed "0%" and Pause anyway (Q4-16). After both it shows the ring,
 * the percent, and Pause (tooltip "Pause upload"); Pause never shows sooner, so a file that
 * finishes in under a second, or that never moves, never offers it.
 * While it starts or finishes, the spinner. One this composer started that paused offers Resume,
 * which reopens the secure picker for the same file, with why it stopped on hover. When it
 * completes the chip goes, and the file becomes an ordinary attachment chip.
 *
 * Every upload chip also offers Cancel upload (FILES-F7), after a confirmation saying what it
 * cannot undo: the part already sent stays on the server for up to a day and counts toward the
 * workspace's file space until then. Cancelling pauses the upload and forgets its record here,
 * which takes the chip away; a refusal is shown in the confirmation that asked.
 */
export function UploadChip({
  transfer,
  onPause,
  onResume,
}: {
  transfer: CrewTransfer;
  onPause(transfer: CrewTransfer): void;
  onResume(transfer: CrewTransfer): void;
}) {
  const presentation = transferStatePresentation(transfer);
  const settled = useSettled(transfer.id);
  const moving = presentation.key === 'uploading';
  const percent = presentation.percent ?? 0;
  // The first second, or while nothing has moved: a spinner and a word, never "0%".
  const starting = moving && (!settled || percent < 1);
  // Pause only once there is something to pause: a second up, and at least 1% moved.
  const canPause =
    settled &&
    presentation.active &&
    presentation.key !== 'pausing' &&
    (presentation.percent ?? 0) >= 1;
  const canResume = presentation.key === 'paused';
  const state = starting ? filesCopy.uploading : moving ? `${percent}%` : presentation.word;
  const name = visibleFileText(transfer.name);
  const [confirming, setConfirming] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [cancelError, setCancelError] = useState('');
  const closeConfirmation = () => {
    setConfirming(false);
    setCancelError('');
  };
  const confirmCancel = async () => {
    setCancelling(true);
    setCancelError('');
    try {
      await cancelUpload(transfer.id);
      setConfirming(false);
    } catch (failure) {
      setCancelError(failureSentence(failure, filesCopy.cancelFailed));
    } finally {
      setCancelling(false);
      // Cancelled or not, the record may have moved: the one poller says what it is now.
      void refreshCrewTransfers(transfer.connection_id);
    }
  };
  return (
    <Badge
      variant="chip"
      className="crew-chip max-w-full min-w-0"
      data-transfer-state={presentation.key}
      title={presentation.reason ?? transfer.error ?? undefined}
    >
      {moving && !starting ? (
        <ProgressRing percent={percent} />
      ) : presentation.active ? (
        <Loader2 className="crew-chip-icon animate-spin" aria-hidden />
      ) : null}
      <span className="min-w-0 truncate text-text-default">{name}</span>
      <span className="shrink-0 tabular-nums">{state}</span>
      {canPause ? (
        <ChipAction
          label={filesCopy.pauseNamed(name)}
          tooltip={filesCopy.pauseUpload}
          onClick={() => onPause(transfer)}
        >
          <Pause className="crew-chip-icon" aria-hidden />
        </ChipAction>
      ) : null}
      {canResume ? (
        <ChipAction label={filesCopy.resumeNamed(name)} onClick={() => onResume(transfer)}>
          <Play className="crew-chip-icon" aria-hidden />
        </ChipAction>
      ) : null}
      <ChipAction
        label={filesCopy.cancelUploadNamed(name)}
        tooltip={filesCopy.cancelUpload}
        onClick={() => setConfirming(true)}
      >
        <XIcon className="crew-chip-icon" aria-hidden />
      </ChipAction>
      {confirming ? (
        <ConfirmationModal
          isOpen
          title={filesCopy.cancelUploadTitle(name)}
          message={filesCopy.unfinishedPartStays}
          confirmLabel={filesCopy.cancelUpload}
          cancelLabel={
            moving || presentation.active ? filesCopy.keepUploading : filesCopy.keepUpload
          }
          confirmVariant="destructive"
          isSubmitting={cancelling}
          onCancel={closeConfirmation}
          onConfirm={() => void confirmCancel()}
        >
          {cancelError ? (
            <p role="alert" className="crew-file-row-error">
              {cancelError}
            </p>
          ) : null}
        </ConfirmationModal>
      ) : null}
    </Badge>
  );
}

/** Whether this upload's chip has been up for {@link UPLOAD_SETTLE_MS}. */
function useSettled(id: string): boolean {
  const [settled, setSettled] = useState<string | null>(null);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(id), UPLOAD_SETTLE_MS);
    return () => window.clearTimeout(timer);
  }, [id]);
  return settled === id;
}
