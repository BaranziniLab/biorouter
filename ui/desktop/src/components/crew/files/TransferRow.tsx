import { useId } from 'react';
import { Button } from '../../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem } from '../../ui/dropdown-menu';
import { Progress } from '../../ui/progress';
import { Download, Pause, Play, Trash2, Upload } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
import { visibleFileText } from './fileName';
import { MoreActionsTrigger } from './GlyphButton';
import { formatBytes } from './formatBytes';
import './files.css';

export interface TransferActions {
  onPause(transfer: CrewTransfer): void;
  onResume(transfer: CrewTransfer): void;
  onRemove(transfer: CrewTransfer): void;
}

/**
 * A paused transfer can be picked up again. A finished or unconfirmed one cannot, and neither can
 * a failed one: the workspace refused it, and reselecting the file cannot change that (F-1).
 */
export function canResumeTransfer(transfer: CrewTransfer): boolean {
  return transferStatePresentation(transfer).key === 'paused';
}

/**
 * The `⋯` items every transfer record shares: Resume… (reopens the secure picker for the same
 * file or destination) and Remove from list, with what removing does and does not touch.
 * Nothing is offered while the transfer moves: Pause is its one action then.
 */
export function TransferMenuItems({
  transfer,
  onResume,
  onRemove,
}: {
  transfer: CrewTransfer;
  onResume(transfer: CrewTransfer): void;
  onRemove(transfer: CrewTransfer): void;
}) {
  const helpId = useId();
  if (transferStatePresentation(transfer).active) return null;
  return (
    <>
      {canResumeTransfer(transfer) ? (
        <DropdownMenuItem onSelect={() => onResume(transfer)}>
          <Play aria-hidden />
          {filesCopy.resume}
        </DropdownMenuItem>
      ) : null}
      <DropdownMenuItem
        onSelect={() => onRemove(transfer)}
        aria-describedby={helpId}
        className="items-start"
      >
        <Trash2 aria-hidden className="mt-0.5" />
        <span className="flex min-w-0 flex-col">
          <span>{filesCopy.removeFromList}</span>
          <span id={helpId} className="crew-menu-help">
            {transfer.direction === 'upload' && transfer.state !== 'completed'
              ? filesCopy.removeUnfinishedUploadHelp
              : filesCopy.removeFromListHelp}
          </span>
        </span>
      </DropdownMenuItem>
    </>
  );
}

/**
 * Whether a moving transfer has yet to move 1%: it then reads "Uploading…" (or "Downloading…"),
 * with no percent, no bar and no Pause — "Uploading 0% · Pause" read as stuck, for a 100-byte file
 * as much as a large one (Q4-16), exactly as the composer's chip does.
 */
function notYetMoving(transfer: CrewTransfer): boolean {
  const presentation = transferStatePresentation(transfer);
  return (
    (presentation.key === 'uploading' || presentation.key === 'downloading') &&
    (presentation.percent ?? 0) < 1
  );
}

/**
 * One transfer in the Files tab: direction glyph, name, the state in words ("Uploading 42%",
 * "Paused", "Not confirmed"), a thin bar while it has a position, Pause while it moves and a
 * `⋯` for the rest. Until it has moved 1% it says only "Uploading…" (Q4-16). A paused transfer
 * says why in a few words ("You paused it", "The connection dropped") with the daemon's whole
 * sentence on hover; the daemon's reason for a failure is shown as written.
 */
export function TransferRow({
  transfer,
  onPause,
  onResume,
  onRemove,
  serverStorageNote,
}: TransferActions & {
  transfer: CrewTransfer;
  /**
   * What the viewer does about a transfer the workspace server could not save: the host's own
   * next step, or whom a member waits for (`filesCopy.serverStorage*`, RES2-N3).
   */
  serverStorageNote?: string;
}) {
  const presentation = transferStatePresentation(transfer);
  const Glyph = transfer.direction === 'upload' ? Upload : Download;
  const starting = notYetMoving(transfer);
  const word = starting
    ? transfer.direction === 'upload'
      ? filesCopy.uploading
      : filesCopy.downloading
    : presentation.word;
  const showBar = presentation.percent !== undefined && presentation.key !== 'failed' && !starting;
  const canPause = presentation.active && presentation.key !== 'pausing' && !starting;
  // A download's name began as another member's (RENDERER-1): hidden characters made visible.
  const name = visibleFileText(transfer.name);
  return (
    <li
      className="crew-file-row"
      data-transfer-state={presentation.key}
      data-starting={starting ? 'true' : undefined}
    >
      <div className="crew-file-row-main">
        <Glyph className="crew-file-row-icon" aria-hidden />
        <span className="crew-file-row-name">{name}</span>
        <span className="crew-file-row-meta">
          {word}
          {presentation.key === 'failed' || presentation.key === 'not-confirmed'
            ? ''
            : ` · ${formatBytes(transfer.size)}`}
        </span>
        {canPause ? (
          <Button
            type="button"
            variant="ghost"
            size="sm"
            onClick={() => onPause(transfer)}
            aria-label={filesCopy.pauseNamed(name)}
          >
            <Pause aria-hidden />
            {filesCopy.pause}
          </Button>
        ) : null}
        {presentation.active ? null : (
          <DropdownMenu>
            <MoreActionsTrigger name={name} />
            <DropdownMenuContent align="end" className="crew-menu">
              <TransferMenuItems transfer={transfer} onResume={onResume} onRemove={onRemove} />
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {showBar ? (
        <Progress
          value={presentation.percent}
          label={`${name}: ${presentation.word}`}
          className="crew-file-row-progress"
        />
      ) : null}
      {presentation.key === 'paused' ? (
        presentation.reason ? (
          <p className="crew-file-row-reason" title={transfer.error ?? undefined}>
            {presentation.reason}
            {presentation.serverStorage && serverStorageNote ? `. ${serverStorageNote}` : null}
          </p>
        ) : null
      ) : transfer.error ? (
        <p className="crew-file-row-error">{transfer.error}</p>
      ) : null}
    </li>
  );
}
