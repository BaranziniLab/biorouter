import { useId, useState } from 'react';
import { useNavigate } from 'react-router-dom';
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
import { identityCopy, PersonName, personLabel } from '../identity';
import type { CrewMessage } from '../crewApi';
import { timelineCopy } from './copy';
import type { TimelineGroup, TimelineMessageEntry, TimelineTraceEntry } from './groupMessages';
import { MessageBody } from './MessageBody';
import { CopyForSupport, CopyIconButton, useMenuCopy, useTimelineCopy } from './TimelineCopy';
import { useTimeline, type OwnAgentChat } from './TimelineContext';
import { fullDateTime, gutterTime, isoTime, shortTime } from './timelineTime';

/**
 * One row of a message group: the head (avatar, author, time) or a
 * continuation (the body, with its time in the gutter on hover and focus), and
 * the row's floating actions. An agent's folded tool updates are a row too.
 *
 * Every row is reached with the arrow keys (`data-crew-row`, focused by
 * script), and only the active row's controls are Tab stops: its actions, the
 * link to its agent's chat, and — through the attachments slot's `active` — its
 * file cards (Q3-05). The actions are revealed on hover and focus-within, and
 * always shown without a hover pointer (`timeline.css`).
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

/**
 * The viewer's own agent's post, when one of the viewer's chats posted it (Q3-22): "Your agent ·
 * {chat title}", the title opening that chat. It is looked up by the group's run among the
 * viewer's OWN grants, and only for a group the viewer's agent wrote, so another person's agent
 * is never named by their chat: it stays "{name}'s agent".
 */
function useOwnAgentChat(group: TimelineGroup): OwnAgentChat | null {
  const { viewerId, ownAgentChats } = useTimeline();
  if (!group.agent || !group.runId || viewerId === null || group.authorId !== viewerId) return null;
  return ownAgentChats.get(group.runId) ?? null;
}

function AgentChatHead({ chat, tabIndex }: { chat: OwnAgentChat; tabIndex: number }) {
  const { readOnly } = useTimeline();
  const navigate = useNavigate();
  return (
    <>
      <span className="crew-message-author-lead text-label">{identityCopy.yourAgent}</span>
      {identityCopy.separator}
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="crew-message-agent-chat text-label"
            tabIndex={tabIndex}
            disabled={readOnly}
            onClick={() => navigate(`/pair?resumeSessionId=${encodeURIComponent(chat.sessionId)}`)}
          >
            <bdi>{chat.title}</bdi>
          </button>
        </TooltipTrigger>
        <TooltipContent>{timelineCopy.openAgentChat(chat.title)}</TooltipContent>
      </Tooltip>
    </>
  );
}

function HeadMeta({
  group,
  time,
  restricted,
  ids,
  tabIndex,
}: {
  group: TimelineGroup;
  time: Date;
  restricted: boolean;
  ids: GroupLabelIds;
  /** The row's controls' Tab stop: 0 on the active row only. */
  tabIndex: number;
}) {
  const { dir, viewerId } = useTimeline();
  const chat = useOwnAgentChat(group);
  const date = spokenDate(time);
  return (
    <div className="crew-message-meta">
      <span id={ids.author} className="crew-message-author">
        {chat ? (
          <AgentChatHead chat={chat} tabIndex={tabIndex} />
        ) : (
          <PersonName
            person={group.authorId}
            context="header"
            dir={dir}
            agent={group.agent}
            you={group.agent && group.authorId === viewerId}
          />
        )}
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
 * Copy text, then ⋯ → Copy text and, last, "Copy for support" ▸ Copy message ID
 * (Q3-26). The ID lives only behind that submenu, and the menu never holds the
 * ID alone: a whole menu for one machine string reads as the message's only
 * other action. Both controls are named for the message they act on. A copy
 * from the menu answers in the menu: the item reads "Copied" and the menu
 * closes 600ms later (Q2-34).
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
  const menuCopy = useMenuCopy<'text' | 'id'>(copy, setMenuOpen);
  const moreName = timelineCopy.moreActionsFor(who, time);
  return (
    <div className="crew-row-actions" data-open={menuOpen ? 'true' : undefined}>
      <CopyIconButton
        text={message.body}
        label={timelineCopy.copyText}
        name={timelineCopy.copyTextOf(who, time)}
        tabIndex={tabIndex}
      />
      <DropdownMenu open={menuOpen} onOpenChange={menuCopy.onOpenChange}>
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
          <DropdownMenuItem
            data-crew-copy-state={menuCopy.state('text')}
            onSelect={menuCopy.select('text', message.body)}
          >
            {menuCopy.label('text', timelineCopy.copyText)}
          </DropdownMenuItem>
          <CopyForSupport>
            <DropdownMenuItem
              data-crew-copy-state={menuCopy.state('id')}
              onSelect={menuCopy.select('id', message.id)}
            >
              {menuCopy.label('id', timelineCopy.copyMessageId)}
            </DropdownMenuItem>
          </CopyForSupport>
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
  const active = activeRow === entry.key;
  const tabIndex = active ? 0 : -1;
  const ownTime = useId();
  const timeId = entry.head ? ids.time : ownTime;
  const who = useAuthorLabel(group);
  // Two rows of one author in one minute would read the same: each says which of them it is.
  const when = entry.sameMinute
    ? timelineCopy.timeInMinute(
        shortTime(entry.time),
        entry.sameMinute.index,
        entry.sameMinute.count
      )
    : shortTime(entry.time);
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
          <HeadMeta
            group={group}
            time={entry.time}
            restricted={entry.restrictedMarker}
            ids={ids}
            tabIndex={tabIndex}
          />
        ) : (
          entry.restrictedMarker && <RestrictedMarker />
        )}
        <MessageBody body={message.body} />
        {renderAttachments?.(message, { active })}
      </div>
      <RowActions message={message} tabIndex={tabIndex} who={who} time={when} />
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
  const { activeRow, setActiveRow, arriving } = useTimeline();
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
        {entry.head && (
          <HeadMeta
            group={group}
            time={entry.time}
            restricted={false}
            ids={ids}
            tabIndex={activeRow === entry.key ? 0 : -1}
          />
        )}
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
 *
 * `head`: it carries the viewer's own avatar and name, as the message will once
 * it lands. Without them it sat under the previous person's message and read as
 * theirs for a second (Q3-20). The timeline leaves the head off only where the
 * delivered message would itself continue the viewer's own group.
 */
export function PendingPostRow({ body, head }: { body: string; head: boolean }) {
  const { dir, viewerId } = useTimeline();
  const showHead = head && viewerId !== null;
  const person = viewerId !== null ? dir.byId(viewerId) : undefined;
  return (
    <div
      className="crew-message-row crew-pending-row"
      data-pending="true"
      data-head={showHead ? 'true' : undefined}
      aria-hidden="true"
      inert
    >
      <div className="crew-message-gutter">
        {showHead && (
          <Avatar
            size={32}
            shape="circle"
            fallback={person ? person.avatar : '?'}
            name={person?.displayName}
            username={person?.username}
          />
        )}
      </div>
      <div className="crew-message-main">
        {showHead && viewerId !== null && (
          <div className="crew-message-meta">
            <span className="crew-message-author">
              <PersonName person={viewerId} context="header" dir={dir} tooltip={false} />
            </span>
          </div>
        )}
        {body.trim() && <MessageBody body={body} />}
        <p className="crew-pending-status text-supporting text-text-muted">
          <Loader2 aria-hidden className="crew-pending-spinner animate-spin" />
          {timelineCopy.sending}
        </p>
      </div>
    </div>
  );
}
