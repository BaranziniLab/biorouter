import { useState } from 'react';
import { Avatar } from '../../ui/avatar';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import { Disclosure } from '../../ui/disclosure';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../../ui/Tooltip';
import { Bot, MoreHorizontal } from '../../icons/app-icons';
import { PersonName } from '../identity';
import type { CrewMessage } from '../crewApi';
import { timelineCopy } from './copy';
import type { TimelineGroup, TimelineMessageEntry, TimelineTraceEntry } from './groupMessages';
import { MessageBody } from './MessageBody';
import { CopyIconButton, useTimelineCopy } from './TimelineCopy';
import { useTimeline } from './TimelineContext';
import { fullDateTime, gutterTime, isoTime, shortTime } from './timelineTime';

/**
 * One row of a message group: the head (avatar, author, time) or a
 * continuation (the body, with its time in the gutter on hover and focus), and
 * the row's floating actions. An agent's folded tool updates are a row too.
 *
 * Every row is reached with the arrow keys (`data-crew-row`, focused by
 * script), and only the active row's actions are Tab stops. The actions are
 * revealed on hover and focus-within, and always shown without a hover pointer
 * (`timeline.css`).
 */

/** The IDs the group's `<article>` is labelled by. */
export interface GroupLabelIds {
  author: string;
  time: string;
}

/** Circle initials for a person, a square Bot tile for an agent. Decorative beside the name. */
export function AuthorAvatar({ group }: { group: TimelineGroup }) {
  const { dir } = useTimeline();
  if (group.agent) return <Avatar size={32} shape="square" icon={<Bot aria-hidden />} />;
  const person = dir.byId(group.authorId);
  return (
    <Avatar
      size={32}
      shape="circle"
      fallback={person ? person.avatar : '?'}
      name={person?.displayName}
      username={person?.username}
    />
  );
}

function HeadMeta({
  group,
  time,
  restricted,
  ids,
}: {
  group: TimelineGroup;
  time: Date;
  restricted: boolean;
  ids: GroupLabelIds;
}) {
  const { dir, viewerId } = useTimeline();
  return (
    <div className="crew-message-meta">
      <span id={ids.author} className="crew-message-author">
        <PersonName
          person={group.authorId}
          context="header"
          dir={dir}
          agent={group.agent}
          you={group.agent && group.authorId === viewerId}
        />
      </span>
      {group.agent && <Badge tone="neutral">{timelineCopy.agentBadge}</Badge>}
      <Tooltip>
        <TooltipTrigger asChild>
          <time
            id={ids.time}
            dateTime={isoTime(time)}
            className="text-supporting text-text-muted tabular-nums"
          >
            {shortTime(time)}
          </time>
        </TooltipTrigger>
        <TooltipContent>{fullDateTime(time)}</TooltipContent>
      </Tooltip>
      {restricted && <RestrictedMarker />}
    </div>
  );
}

/** Shown only where a message's restriction differs from its channel's. */
function RestrictedMarker() {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span className="crew-message-restricted text-supporting text-text-muted">
          {timelineCopy.restricted}
          <span className="sr-only">: {timelineCopy.restrictedTooltip}</span>
        </span>
      </TooltipTrigger>
      <TooltipContent>{timelineCopy.restrictedTooltip}</TooltipContent>
    </Tooltip>
  );
}

function GutterTime({ time }: { time: Date }) {
  return (
    <time
      dateTime={isoTime(time)}
      className="crew-message-gutter-time text-supporting text-text-muted tabular-nums"
    >
      {gutterTime(time)}
    </time>
  );
}

/** Copy text, then ⋯ → Copy message ID. The ID lives only behind this menu. */
function RowActions({ message, tabIndex }: { message: CrewMessage; tabIndex: number }) {
  const copy = useTimelineCopy();
  const [menuOpen, setMenuOpen] = useState(false);
  return (
    <div className="crew-row-actions" data-open={menuOpen ? 'true' : undefined}>
      <CopyIconButton text={message.body} label={timelineCopy.copyText} tabIndex={tabIndex} />
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={timelineCopy.moreActions}
                tabIndex={tabIndex}
                className="text-text-muted"
              >
                <MoreHorizontal aria-hidden />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>{timelineCopy.moreActions}</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="end">
          <DropdownMenuItem onSelect={() => void copy(message.id)}>
            {timelineCopy.copyMessageId}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}

export function MessageRow({
  group,
  entry,
  ids,
}: {
  group: TimelineGroup;
  entry: TimelineMessageEntry;
  ids: GroupLabelIds;
}) {
  const { activeRow, setActiveRow, renderAttachments, arriving } = useTimeline();
  const { message } = entry;
  return (
    <div
      className="crew-message-row"
      data-crew-row=""
      data-head={entry.head ? 'true' : undefined}
      data-arriving={arriving.has(message.id) ? 'true' : undefined}
      tabIndex={-1}
      onFocus={() => setActiveRow(entry.key)}
    >
      <div className="crew-message-gutter">
        {entry.head ? <AuthorAvatar group={group} /> : <GutterTime time={entry.time} />}
      </div>
      <div className="crew-message-main">
        {entry.head ? (
          <HeadMeta group={group} time={entry.time} restricted={entry.restrictedMarker} ids={ids} />
        ) : (
          entry.restrictedMarker && <RestrictedMarker />
        )}
        <MessageBody body={message.body} />
        {renderAttachments?.(message)}
      </div>
      <RowActions message={message} tabIndex={activeRow === entry.key ? 0 : -1} />
    </div>
  );
}

/** An agent's consecutive tool updates, folded behind "Show details". */
export function TraceRow({
  group,
  entry,
  ids,
}: {
  group: TimelineGroup;
  entry: TimelineTraceEntry;
  ids: GroupLabelIds;
}) {
  const { setActiveRow, arriving } = useTimeline();
  const first = entry.messages[0];
  return (
    <div
      className="crew-message-row"
      data-crew-row=""
      data-head={entry.head ? 'true' : undefined}
      data-arriving={first && arriving.has(first.id) ? 'true' : undefined}
      tabIndex={-1}
      onFocus={() => setActiveRow(entry.key)}
    >
      <div className="crew-message-gutter">
        {entry.head ? <AuthorAvatar group={group} /> : <GutterTime time={entry.time} />}
      </div>
      <div className="crew-message-main">
        {entry.head && <HeadMeta group={group} time={entry.time} restricted={false} ids={ids} />}
        <Disclosure
          label={timelineCopy.showDetails}
          summary={timelineCopy.detailsSummary(entry.messages.length)}
        >
          <ul className="crew-trace-list">
            {entry.messages.map((message) => (
              <li key={message.id} className="crew-trace-line text-supporting text-text-muted">
                {message.body}
              </li>
            ))}
          </ul>
        </Disclosure>
      </div>
    </div>
  );
}
