import { useId, useState } from 'react';
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
import { Bot, Loader2, MoreHorizontal } from '../../icons/app-icons';
import { PersonName, personLabel } from '../identity';
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
 *
 * A row is a `group` named by its author and its own time, so a focused row
 * reads as whose message it is and when — a continuation included, whose
 * author's name sits in the head row above it. Its actions carry the same two
 * in their names. Every time carries its full date for assistive technology;
 * the date is otherwise only in the day divider and a hover tooltip.
 */

const SPOKEN_DATE = new Intl.DateTimeFormat('en-US', {
  weekday: 'long',
  month: 'long',
  day: 'numeric',
  year: 'numeric',
});

/** "Tuesday, September 22, 2026" (the locale `timelineTime.ts` formats in), or '' when unusable. */
function spokenDate(date: Date): string {
  return Number.isNaN(date.getTime()) ? '' : SPOKEN_DATE.format(date);
}

/**
 * How a person or an agent is named in an action's accessible name: the chip's
 * form ("Bob Lee", "@bob" when that is all there is, "Your agent"), because a
 * parenthesized handle before "’s message" reads badly and the row's own name
 * already carries the head's full form.
 */
function useAuthorLabel(group: TimelineGroup): string {
  const { dir, viewerId } = useTimeline();
  return personLabel(group.authorId, 'chip', dir, {
    agent: group.agent,
    you: group.agent && group.authorId === viewerId,
  });
}

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
  const date = spokenDate(time);
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
            {date && <span className="sr-only">, {date}</span>}
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

/**
 * A continuation's time: "10:03" in the 44px gutter, where "10:03 AM" does not
 * fit, and the whole time and date for assistive technology.
 */
function GutterTime({ time, id }: { time: Date; id: string }) {
  const date = spokenDate(time);
  return (
    <time
      id={id}
      dateTime={isoTime(time)}
      className="crew-message-gutter-time text-supporting text-text-muted tabular-nums"
    >
      <span aria-hidden="true">{gutterTime(time)}</span>
      <span className="sr-only">{date ? `${shortTime(time)}, ${date}` : shortTime(time)}</span>
    </time>
  );
}

/**
 * Copy text, then ⋯ → Copy text and Copy message ID. The ID lives only behind
 * this menu, and the menu never holds the ID alone: a whole menu for one
 * machine string reads as the message's only other action. Both controls are
 * named for the message they act on.
 */
function RowActions({
  message,
  tabIndex,
  who,
  time,
}: {
  message: CrewMessage;
  tabIndex: number;
  who: string;
  time: string;
}) {
  const copy = useTimelineCopy();
  const [menuOpen, setMenuOpen] = useState(false);
  const moreName = timelineCopy.moreActionsFor(who, time);
  return (
    <div className="crew-row-actions" data-open={menuOpen ? 'true' : undefined}>
      <CopyIconButton
        text={message.body}
        label={timelineCopy.copyText}
        name={timelineCopy.copyTextOf(who, time)}
        tabIndex={tabIndex}
      />
      <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                size="sm"
                shape="round"
                aria-label={moreName}
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
          <DropdownMenuItem onSelect={() => void copy(message.body)}>
            {timelineCopy.copyText}
          </DropdownMenuItem>
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
  const ownTime = useId();
  const timeId = entry.head ? ids.time : ownTime;
  const who = useAuthorLabel(group);
  return (
    <div
      role="group"
      aria-labelledby={`${ids.author} ${timeId}`}
      className="crew-message-row"
      data-crew-row=""
      data-head={entry.head ? 'true' : undefined}
      data-arriving={arriving.has(message.id) ? 'true' : undefined}
      tabIndex={-1}
      onFocus={() => setActiveRow(entry.key)}
    >
      <div className="crew-message-gutter">
        {entry.head ? (
          <AuthorAvatar group={group} />
        ) : (
          <GutterTime time={entry.time} id={ownTime} />
        )}
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
      <RowActions
        message={message}
        tabIndex={activeRow === entry.key ? 0 : -1}
        who={who}
        time={shortTime(entry.time)}
      />
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
  const ownTime = useId();
  const timeId = entry.head ? ids.time : ownTime;
  return (
    <div
      role="group"
      aria-labelledby={`${ids.author} ${timeId}`}
      className="crew-message-row"
      data-crew-row=""
      data-head={entry.head ? 'true' : undefined}
      data-arriving={first && arriving.has(first.id) ? 'true' : undefined}
      tabIndex={-1}
      onFocus={() => setActiveRow(entry.key)}
    >
      <div className="crew-message-gutter">
        {entry.head ? (
          <AuthorAvatar group={group} />
        ) : (
          <GutterTime time={entry.time} id={ownTime} />
        )}
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

/**
 * A post the broker accepted that the observer has not delivered yet: the words
 * that just left the composer, dimmed, with "Sending…" under them, at the end of
 * the log. It is inert and hidden from assistive technology — the real message
 * is announced by the log when it arrives, and this row then goes in the same
 * render, so the words are never shown twice.
 */
export function PendingPostRow({ body }: { body: string }) {
  return (
    <div className="crew-message-row crew-pending-row" data-pending="true" aria-hidden="true" inert>
      <div className="crew-message-gutter" />
      <div className="crew-message-main">
        {body.trim() && <MessageBody body={body} />}
        <p className="crew-pending-status text-supporting text-text-muted">
          <Loader2 aria-hidden className="crew-pending-spinner animate-spin" />
          {timelineCopy.sending}
        </p>
      </div>
    </div>
  );
}
