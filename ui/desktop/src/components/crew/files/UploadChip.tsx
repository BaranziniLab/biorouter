import { Badge } from '../../ui/badge';
import { Loader2, Pause, Play } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
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

/**
 * An upload still on its way into the composer: `[counts.csv 42% ⏸]`.
 *
 * While it moves it shows the ring and a Pause glyph; while it starts or finishes, the one
 * spinner. One this composer started that stopped (paused, or failed with the reason on
 * hover) offers Resume, which reopens the secure picker for the same file. When it completes
 * the chip goes, and the file becomes an ordinary attachment chip.
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
  const moving = presentation.key === 'uploading';
  const canPause = presentation.active && presentation.key !== 'pausing';
  const canResume = presentation.key === 'paused' || presentation.key === 'failed';
  const state = moving ? `${presentation.percent ?? 0}%` : presentation.word;
  return (
    <Badge
      variant="chip"
      className="crew-chip max-w-full min-w-0"
      data-transfer-state={presentation.key}
      title={transfer.error ?? undefined}
    >
      {moving ? (
        <ProgressRing percent={presentation.percent ?? 0} />
      ) : presentation.active ? (
        <Loader2 className="crew-chip-icon animate-spin" aria-hidden />
      ) : null}
      <span className="min-w-0 truncate text-text-default">{transfer.name}</span>
      <span className="shrink-0 tabular-nums">{state}</span>
      {canPause ? (
        <button
          type="button"
          className="crew-chip-action"
          aria-label={filesCopy.pauseNamed(transfer.name)}
          onClick={() => onPause(transfer)}
        >
          <Pause className="crew-chip-icon" aria-hidden />
        </button>
      ) : null}
      {canResume ? (
        <button
          type="button"
          className="crew-chip-action"
          aria-label={filesCopy.resumeNamed(transfer.name)}
          onClick={() => onResume(transfer)}
        >
          <Play className="crew-chip-icon" aria-hidden />
        </button>
      ) : null}
    </Badge>
  );
}
