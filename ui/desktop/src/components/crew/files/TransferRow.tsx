import { useId } from 'react';
import { Button } from '../../ui/button';
import { DropdownMenu, DropdownMenuContent, DropdownMenuItem } from '../../ui/dropdown-menu';
import { Progress } from '../../ui/progress';
import { Download, Pause, Play, Trash2, Upload } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';
import { filesCopy } from './copy';
import { MoreActionsTrigger } from './GlyphButton';
import { formatBytes } from './formatBytes';
import './files.css';

export interface TransferActions {
  onPause(transfer: CrewTransfer): void;
  onResume(transfer: CrewTransfer): void;
  onRemove(transfer: CrewTransfer): void;
}

/** A stopped transfer can be picked up again; a finished or unconfirmed one cannot. */
export function canResumeTransfer(transfer: CrewTransfer): boolean {
  const { key } = transferStatePresentation(transfer);
  return key === 'paused' || key === 'failed';
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
            {filesCopy.removeFromListHelp}
          </span>
        </span>
      </DropdownMenuItem>
    </>
  );
}

/**
 * One transfer in the Files tab: direction glyph, name, the state in words ("Uploading 42%",
 * "Paused", "Not confirmed"), a thin bar while it has a position, Pause while it moves and a
 * `⋯` for the rest. The daemon's own reason for a failure is shown as written.
 */
export function TransferRow({
  transfer,
  onPause,
  onResume,
  onRemove,
}: TransferActions & {
  transfer: CrewTransfer;
}) {
  const presentation = transferStatePresentation(transfer);
  const Glyph = transfer.direction === 'upload' ? Upload : Download;
  const showBar = presentation.percent !== undefined && presentation.key !== 'failed';
  const canPause = presentation.active && presentation.key !== 'pausing';
  return (
    <li className="crew-file-row" data-transfer-state={presentation.key}>
      <div className="crew-file-row-main">
        <Glyph className="crew-file-row-icon" aria-hidden />
        <span className="crew-file-row-name">{transfer.name}</span>
        <span className="crew-file-row-meta">
          {presentation.word}
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
            aria-label={filesCopy.pauseNamed(transfer.name)}
          >
            <Pause aria-hidden />
            {filesCopy.pause}
          </Button>
        ) : null}
        {presentation.active ? null : (
          <DropdownMenu>
            <MoreActionsTrigger name={transfer.name} />
            <DropdownMenuContent align="end" className="crew-menu">
              <TransferMenuItems transfer={transfer} onResume={onResume} onRemove={onRemove} />
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {showBar ? (
        <Progress
          value={presentation.percent}
          label={`${transfer.name}: ${presentation.word}`}
          className="crew-file-row-progress"
        />
      ) : null}
      {transfer.error ? <p className="crew-file-row-error">{transfer.error}</p> : null}
    </li>
  );
}
