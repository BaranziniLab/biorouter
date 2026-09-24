import { memo } from 'react';
import type { CrewMessage } from '../crewApi';
import { AttachmentCard } from '../files/AttachmentCard';
import { ServerPathRow } from '../files/ServerPathRow';

/**
 * A message's server paths and attachments, under its body: the files area's rows, handed to the
 * timeline through its `renderAttachments` slot so neither area imports the other. Paths first,
 * then files, as the message carries them.
 */
export const MessageFiles = memo(function MessageFiles({
  connectionId,
  message,
}: {
  connectionId: string;
  message: CrewMessage;
}) {
  const references = message.references ?? [];
  const attachments = message.attachments ?? [];
  if (references.length === 0 && attachments.length === 0) return null;
  return (
    <div className="crew-frame-files">
      {references.map((id) => (
        <ServerPathRow key={`path:${id}`} connectionId={connectionId} referenceId={id} />
      ))}
      {attachments.map((id) => (
        <AttachmentCard key={`file:${id}`} connectionId={connectionId} blobId={id} />
      ))}
    </div>
  );
});
