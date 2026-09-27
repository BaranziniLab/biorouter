import { memo } from 'react';
import type { CrewMessage } from '../crewApi';
import { AttachmentCard } from '../files/AttachmentCard';
import { ServerPathRow } from '../files/ServerPathRow';
import { messageTime } from '../timeline/timelineTime';

/**
 * A message's server paths and attachments, under its body: the files area's rows, handed to the
 * timeline through its `renderAttachments` slot so neither area imports the other. Paths first,
 * then files, as the message carries them.
 *
 * `active`: the message is the log's active row. Only then are the rows' controls Tab stops, as
 * the row's own actions are: every card's Save and ⋯ used to be in the Tab order, so a channel
 * with two files was 8–10 stops from the log to the composer (Q3-05). A click on a control still
 * focuses it, and focus inside a row makes that row the active one.
 *
 * `sending`: the files of a post on its way (Q4-19), as cards in their sending state — the same
 * row the posted card will be, with no controls — named from the draft (`fileNames`) until their
 * own names load.
 */
export const MessageFiles = memo(function MessageFiles({
  connectionId,
  message,
  active,
  sending = false,
  fileNames,
}: {
  connectionId: string;
  message: CrewMessage;
  active: boolean;
  sending?: boolean;
  fileNames?: Readonly<Record<string, string>>;
}) {
  const references = message.references ?? [];
  const attachments = message.attachments ?? [];
  if (references.length === 0 && attachments.length === 0) return null;
  if (sending) {
    return (
      <div className="crew-frame-files">
        {attachments.map((id) => (
          <AttachmentCard
            key={`file:${id}`}
            connectionId={connectionId}
            blobId={id}
            sending
            fallbackName={fileNames?.[id] ?? ''}
          />
        ))}
      </div>
    );
  }
  const tabIndex = active ? 0 : -1;
  const postedAt = messageTime(message.created_at).getTime();
  return (
    <div className="crew-frame-files">
      {references.map((id) => (
        <ServerPathRow
          key={`path:${id}`}
          connectionId={connectionId}
          referenceId={id}
          tabIndex={tabIndex}
        />
      ))}
      {attachments.map((id) => (
        <AttachmentCard
          key={`file:${id}`}
          connectionId={connectionId}
          blobId={id}
          tabIndex={tabIndex}
          postedAt={postedAt}
        />
      ))}
    </div>
  );
});
