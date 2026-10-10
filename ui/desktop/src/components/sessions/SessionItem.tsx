import React from 'react';
import { formatDate } from '../../utils/date';
import { Session } from '../../api';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { messageCount } from './copy';
import { folderName } from './historyRow';

interface SessionItemProps {
  session: Session;
  extraActions?: React.ReactNode;
}

/**
 * A session is enumerable, so it is a ROW, not a card (design.md P2, §4.14):
 * one title line and one muted line under it.
 *
 * The second line names the folder, never the path (principle 10): a path is a
 * machine string and belongs in a tooltip or a "Copy" item, not on an everyday
 * row. The kind glyph draws in the 16px slot every chat row uses.
 */
const SessionItem: React.FC<SessionItemProps> = ({ session, extraActions }) => {
  const meta = [
    formatDate(session.updated_at),
    messageCount(session.message_count),
    folderName(session.working_dir),
  ].filter(Boolean);
  return (
    // `.biorouter-list-row:hover` paints the shared list wash; a second hover
    // class here is how the list-vs-settings 42%/38% fork started.
    <div className="biorouter-list-row flex cursor-pointer items-center justify-between gap-3 px-3 py-2">
      <div className="min-w-0">
        {/* One leading glyph carrying both what this chat is and whether it is
            private. `flex-none` on the icon is load-bearing: the truncating
            title beside it must not shrink it away. */}
        <div className="flex min-w-0 items-center gap-2">
          <ChatKindIcon session={session} tier={session.privacy_tier} className="h-4 w-4" />
          <p className="truncate text-label text-text-default">{session.name}</p>
        </div>
        <p className="mt-0.5 truncate pl-6 text-supporting text-text-muted tabular-nums">
          {meta.join(' · ')}
        </p>
      </div>
      {extraActions && <div className="flex shrink-0 items-center gap-1">{extraActions}</div>}
    </div>
  );
};

export default SessionItem;
