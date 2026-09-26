import { Badge } from '../../ui/badge';
import { File, Link, XIcon } from '../../icons/app-icons';
import type { CrewTransfer } from '../crewTransfers';
import type { DraftFile, DraftReference } from '../state/types';
import { ChipAction } from '../files/GlyphButton';
import { UploadChip } from '../files/UploadChip';
import { composerCopy } from './copy';
import './composer.css';

/**
 * What goes with the message, above the text: uploads still on their way, attached files and
 * shared server paths. Each is a `Badge` chip; a finished one carries a 14px `XIcon` remove
 * control whose name says exactly what it removes ("Remove counts.csv from this message", "Remove
 * remote reference Remote results"). A file's × says in its tooltip that the uploaded copy stays
 * on the server: taking it out of the message does not delete it, and the broker cannot yet
 * (Q3-14).
 *
 * Under the chips, while a finished file waits: one line, "Press Send to share it." — a drop or a
 * paste only attaches (Q3-14) — and, for a file that is already in the channel, a note saying so
 * (`duplicates`, Q3-13). Neither blocks anything.
 *
 * Renders nothing when there is nothing to show, so an empty composer has no empty row.
 */
export function ComposerChips({
  attachments,
  references,
  uploads = [],
  duplicates = {},
  server = '',
  onRemoveAttachment,
  onRemoveReference,
  onPauseUpload,
  onResumeUpload,
}: {
  attachments: readonly DraftFile[];
  references: readonly DraftReference[];
  uploads?: readonly CrewTransfer[];
  /** A note per draft file that is already in the channel, by the file's ID. */
  duplicates?: Readonly<Record<string, string>>;
  /** What to call the server the uploads are on, for the remove control's tooltip. */
  server?: string;
  onRemoveAttachment(id: string): void;
  onRemoveReference(id: string): void;
  onPauseUpload(transfer: CrewTransfer): void;
  onResumeUpload(transfer: CrewTransfer): void;
}) {
  if (attachments.length === 0 && references.length === 0 && uploads.length === 0) return null;
  const notes = attachments.flatMap((file) =>
    duplicates[file.id] ? [{ id: file.id, text: duplicates[file.id] }] : []
  );
  return (
    <div className="crew-compose-files">
      <ul className="crew-compose-chips" aria-label={composerCopy.chips}>
        {uploads.map((transfer) => (
          <li key={`upload:${transfer.id}`} className="crew-compose-chip-item">
            <UploadChip transfer={transfer} onPause={onPauseUpload} onResume={onResumeUpload} />
          </li>
        ))}
        {attachments.map((file) => (
          <li key={`file:${file.id}`} className="crew-compose-chip-item">
            <Badge variant="chip" className="crew-chip max-w-full min-w-0">
              <File className="crew-chip-icon" aria-hidden />
              <span className="min-w-0 truncate text-text-default">{file.name}</span>
              <ChipAction
                label={composerCopy.removeFile(file.name)}
                tooltip={composerCopy.removeFileHelp(server)}
                onClick={() => onRemoveAttachment(file.id)}
              >
                <XIcon className="crew-chip-icon" aria-hidden />
              </ChipAction>
            </Badge>
          </li>
        ))}
        {references.map((item) => (
          <li key={`ref:${item.id}`} className="crew-compose-chip-item">
            <Badge variant="chip" className="crew-chip max-w-full min-w-0">
              <Link className="crew-chip-icon" aria-hidden />
              <span className="min-w-0 truncate text-text-default">{item.label}</span>
              <ChipAction
                label={composerCopy.removeRef(item.label)}
                onClick={() => onRemoveReference(item.id)}
              >
                <XIcon className="crew-chip-icon" aria-hidden />
              </ChipAction>
            </Badge>
          </li>
        ))}
      </ul>
      {notes.map((note) => (
        <p key={`duplicate:${note.id}`} className="crew-compose-chip-note" data-crew-duplicate="">
          {note.text}
        </p>
      ))}
      {attachments.length > 0 ? (
        <p className="crew-compose-chip-note" data-crew-send-hint="">
          {composerCopy.pressSend(attachments.length)}
        </p>
      ) : null}
    </div>
  );
}
