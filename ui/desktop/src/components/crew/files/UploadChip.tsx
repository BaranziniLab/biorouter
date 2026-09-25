import { useEffect, useState } from 'react';
import { Badge } from '../../ui/badge';
import { Loader2, Pause, Play } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
import { ChipAction } from './GlyphButton';
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
 * For its first second, and until it has moved 1%, it shows the one spinner and "Uploading…":
 * a 100-byte file used to sit at an empty ring, "0%" and a pause glyph for a second or two, and
 * read as paused (Q3-16). After that it shows the ring, the percent, and Pause (tooltip "Pause
 * upload"); Pause never shows sooner, so a file that finishes in under a second never offers it.
 * While it starts or finishes, the spinner. One this composer started that stopped (paused, or
 * failed with the reason on hover) offers Resume, which reopens the secure picker for the same
 * file. When it completes the chip goes, and the file becomes an ordinary attachment chip.
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
  // The first second, and while nothing has moved: a spinner and a word, never "0%".
  const starting = moving && !settled && percent < 1;
  const canPause = settled && presentation.active && presentation.key !== 'pausing';
  const canResume = presentation.key === 'paused' || presentation.key === 'failed';
  const state = starting ? filesCopy.uploading : moving ? `${percent}%` : presentation.word;
  return (
    <Badge
      variant="chip"
      className="crew-chip max-w-full min-w-0"
      data-transfer-state={presentation.key}
      title={transfer.error ?? undefined}
    >
      {moving && !starting ? (
        <ProgressRing percent={percent} />
      ) : presentation.active ? (
        <Loader2 className="crew-chip-icon animate-spin" aria-hidden />
      ) : null}
      <span className="min-w-0 truncate text-text-default">{transfer.name}</span>
      <span className="shrink-0 tabular-nums">{state}</span>
      {canPause ? (
        <ChipAction
          label={filesCopy.pauseNamed(transfer.name)}
          tooltip={filesCopy.pauseUpload}
          onClick={() => onPause(transfer)}
        >
          <Pause className="crew-chip-icon" aria-hidden />
        </ChipAction>
      ) : null}
      {canResume ? (
        <ChipAction label={filesCopy.resumeNamed(transfer.name)} onClick={() => onResume(transfer)}>
          <Play className="crew-chip-icon" aria-hidden />
        </ChipAction>
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
