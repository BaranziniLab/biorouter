import { Badge } from '../../ui/badge';
import { File, Link, XIcon } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import type { DraftFile, DraftReference } from '../state/types';
import { UploadChip } from '../files/UploadChip';
import { composerCopy } from './copy';
import './composer.css';

/**
 * What goes with the message, above the text: uploads still on their way, attached files and
 * shared server paths. Each is a `Badge` chip; a finished one carries a 14px `XIcon` remove
 * control whose name says exactly what it removes ("Remove counts.csv", "Remove remote
 * reference Remote results"). Renders nothing when there is nothing to show, so an empty
 * composer has no empty row.
 */
export function ComposerChips({
  attachments,
  references,
  uploads = [],
  onRemoveAttachment,
  onRemoveReference,
  onPauseUpload,
  onResumeUpload,
}: {
  attachments: readonly DraftFile[];
  references: readonly DraftReference[];
  uploads?: readonly CrewTransfer[];
  onRemoveAttachment(id: string): void;
  onRemoveReference(id: string): void;
  onPauseUpload(transfer: CrewTransfer): void;
  onResumeUpload(transfer: CrewTransfer): void;
}) {
  if (attachments.length === 0 && references.length === 0 && uploads.length === 0) return null;
  return (
    <ul className="crew-composer-chips" aria-label={composerCopy.chips}>
      {uploads.map((transfer) => (
        <li key={`upload:${transfer.id}`} className="crew-composer-chip-item">
          <UploadChip transfer={transfer} onPause={onPauseUpload} onResume={onResumeUpload} />
        </li>
      ))}
      {attachments.map((file) => (
        <li key={`file:${file.id}`} className="crew-composer-chip-item">
          <Badge variant="chip" className="crew-chip max-w-full min-w-0">
            <File className="crew-chip-icon" aria-hidden />
            <span className="min-w-0 truncate text-text-default">{file.name}</span>
            <button
              type="button"
              className="crew-chip-action"
              aria-label={composerCopy.removeFile(file.name)}
              onClick={() => onRemoveAttachment(file.id)}
            >
              <XIcon className="crew-chip-icon" aria-hidden />
            </button>
          </Badge>
        </li>
      ))}
      {references.map((item) => (
        <li key={`ref:${item.id}`} className="crew-composer-chip-item">
          <Badge variant="chip" className="crew-chip max-w-full min-w-0">
            <Link className="crew-chip-icon" aria-hidden />
            <span className="min-w-0 truncate text-text-default">{item.label}</span>
            <button
              type="button"
              className="crew-chip-action"
              aria-label={composerCopy.removeRef(item.label)}
              onClick={() => onRemoveReference(item.id)}
            >
              <XIcon className="crew-chip-icon" aria-hidden />
            </button>
          </Badge>
        </li>
      ))}
    </ul>
  );
}
