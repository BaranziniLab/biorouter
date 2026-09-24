import { useEffect, useRef, useState } from 'react';
import { MoreHorizontal } from '../../icons/app-icons';
import { Avatar } from '../../ui/avatar';
import { Badge } from '../../ui/badge';
import { Button } from '../../ui/button';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../../ui/dropdown-menu';
import { cn } from '../../../utils';
import { channelName, PersonName, personLabel, type CrewPerson } from '../identity';
import type { Channel } from '../crewApi';
import type { CrewController } from '../state/types';
import { membersCopy } from './copy';
import {
  COPY_FEEDBACK_MS,
  copyText,
  MENU_COPY_CLOSE_MS,
  usePanePresentation,
} from './presentation';

export interface MembersTabProps {
  className?: string;
}

interface MemberRow {
  id: string;
  person: CrewPerson | null;
  isOwner: boolean;
}

function byDisplayName(a: MemberRow, b: MemberRow): number {
  if (a.isOwner !== b.isOwner) return a.isOwner ? -1 : 1;
  const formerA = a.person?.isFormer ?? true;
  const formerB = b.person?.isFormer ?? true;
  if (formerA !== formerB) return formerA ? 1 : -1;
  return (a.person?.displayName ?? '').localeCompare(b.person?.displayName ?? '', undefined, {
    sensitivity: 'base',
  });
}

type CopyItem = 'username' | 'id';

/**
 * One member's `⋯`. A copy answers ON THE ITEM (Q2-34): the menu stays open, the item reads
 * "Copied" for a moment and then the menu closes, returning focus to the `⋯`. A refused copy
 * reads "Couldn't copy" and leaves the menu open, so the person can try again or leave. Either
 * result is also spoken through the tab's live region, since a menu item's new name is not.
 */
function MemberActions({
  crew,
  channel,
  memberId,
  username,
  label,
  canManage,
  announce,
}: {
  crew: CrewController;
  channel: Channel;
  memberId: string;
  username: string;
  label: string;
  canManage: boolean;
  announce(text: string): void;
}) {
  const [open, setOpen] = useState(false);
  const [outcome, setOutcome] = useState<{ item: CopyItem; copied: boolean } | null>(null);
  const timer = useRef<number | null>(null);
  const clearTimer = () => {
    if (timer.current !== null) window.clearTimeout(timer.current);
    timer.current = null;
  };
  useEffect(
    () => () => {
      if (timer.current !== null) window.clearTimeout(timer.current);
    },
    []
  );

  const copy = async (item: CopyItem, value: string) => {
    const copied = await copyText(value);
    setOutcome({ item, copied });
    announce(copied ? membersCopy.copied : membersCopy.copyFailed);
    clearTimer();
    timer.current = window.setTimeout(
      () => {
        timer.current = null;
        // The label stays "Copied" while the menu fades out; it is reset when the menu next opens.
        if (copied) setOpen(false);
        else setOutcome(null);
      },
      copied ? MENU_COPY_CLOSE_MS : COPY_FEEDBACK_MS
    );
  };
  const stateOf = (item: CopyItem) =>
    outcome?.item === item ? (outcome.copied ? 'copied' : 'failed') : undefined;
  const itemLabel = (item: CopyItem, idle: string) =>
    outcome?.item === item ? (outcome.copied ? membersCopy.copied : membersCopy.copyFailed) : idle;
  const name = channelName(channel);

  return (
    <DropdownMenu
      open={open}
      onOpenChange={(next) => {
        clearTimer();
        if (next) setOutcome(null);
        setOpen(next);
      }}
    >
      <DropdownMenuTrigger asChild>
        <Button
          type="button"
          variant="ghost"
          shape="round"
          size="sm"
          aria-label={membersCopy.more(label)}
        >
          <MoreHorizontal aria-hidden="true" />
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-56">
        {username !== '' && (
          <DropdownMenuItem
            data-crew-copy-state={stateOf('username')}
            onSelect={(event) => {
              // Stay open: the item itself shows whether the copy landed.
              event.preventDefault();
              void copy('username', username);
            }}
          >
            {itemLabel('username', membersCopy.copyUsername)}
          </DropdownMenuItem>
        )}
        <DropdownMenuItem
          data-crew-copy-state={stateOf('id')}
          onSelect={(event) => {
            event.preventDefault();
            void copy('id', memberId);
          }}
        >
          {itemLabel('id', membersCopy.copyPersonId)}
        </DropdownMenuItem>
        {canManage && (
          <>
            <DropdownMenuSeparator />
            <DropdownMenuItem
              onSelect={() =>
                crew.openDialog({
                  kind: 'transfer-ownership',
                  channelId: channel.id,
                  successorId: memberId,
                })
              }
            >
              {membersCopy.makeOwner}
            </DropdownMenuItem>
            <DropdownMenuItem
              variant="destructive"
              onSelect={() =>
                crew.openDialog({
                  kind: 'confirm',
                  confirm: {
                    action: 'remove-channel-member',
                    channelId: channel.id,
                    principalId: memberId,
                  },
                })
              }
            >
              {membersCopy.remove(name)}
            </DropdownMenuItem>
          </>
        )}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * The details pane's Members tab (ui-redesign-spec, "The details pane"): the CHANNEL's members —
 * the same set, and the same count, as the header's member stack — never the team's people.
 *
 * Every row names the person at an authority point, "Display name (@username)", with Channel
 * owner (never a bare "Owner", which read as the workspace's Host, Q2-69), you and former-member
 * markers. The row's `⋯` holds Copy username and Copy person ID (the only place
 * a person's ID appears) and, for the owner acting on someone else, Make owner… and Remove from
 * #name…, which open the transfer dialog and the removal confirmation. A `⋯` is never drawn for
 * Copy person ID alone (T-33): a machine string is not worth a menu of its own, so a row with
 * nothing else to offer (an unknown member) has no `⋯`. Channel invitations the viewer sent and
 * nobody has accepted yet appear as muted "invited" rows.
 *
 * The owner adds people from here; anyone else is told who can, instead of being shown nothing.
 * The broker decides every one of these actions.
 */
export function MembersTab({ className }: MembersTabProps) {
  const { crew, snapshot, channel, dir, isOwner } = usePanePresentation();
  const [announcement, setAnnouncement] = useState('');
  const announceTimer = useRef<number | null>(null);
  useEffect(
    () => () => {
      if (announceTimer.current !== null) window.clearTimeout(announceTimer.current);
    },
    []
  );
  if (!snapshot || !channel) return null;

  const announce = (text: string) => {
    if (announceTimer.current !== null) window.clearTimeout(announceTimer.current);
    setAnnouncement(text);
    announceTimer.current = window.setTimeout(() => {
      announceTimer.current = null;
      setAnnouncement('');
    }, COPY_FEEDBACK_MS);
  };

  const name = channelName(channel);
  const actorId = snapshot.actor.id;
  const members: MemberRow[] = channel.members
    .map((id) => ({ id, person: dir.byId(id), isOwner: id === channel.owner_id }))
    .sort(byDisplayName);
  const now = Date.now() / 1000;
  const invited = snapshot.invitations.filter(
    (invitation) =>
      invitation.kind === 'channel' &&
      invitation.target_id === channel.id &&
      invitation.inviter_id === actorId &&
      !invitation.expired &&
      !(typeof invitation.expires_at === 'number' && invitation.expires_at < now) &&
      !channel.members.includes(invitation.principal_id)
  );
  const owner = dir.byId(channel.owner_id);
  const ownerTools = isOwner && !channel.archived;

  return (
    <div className={cn('flex flex-col gap-3', className)}>
      {ownerTools ? (
        <div>
          <Button
            type="button"
            variant="secondary"
            size="sm"
            onClick={() =>
              crew.openDialog({ kind: 'add-people', target: 'channel', targetId: channel.id })
            }
          >
            {membersCopy.addPeople}
          </Button>
        </div>
      ) : !channel.archived ? (
        <p className="text-supporting text-text-muted">
          {membersCopy.askBefore}
          <PersonName person={owner ?? channel.owner_id} dir={dir} context="inline" />
          {membersCopy.askAfter}
        </p>
      ) : null}

      <p className="text-caps text-text-muted tabular-nums">{membersCopy.count(members.length)}</p>

      <ul role="list" aria-label={membersCopy.listLabel(name)} className="flex flex-col">
        {members.map(({ id, person, isOwner: rowIsOwner }) => {
          const label = personLabel(person ?? id, 'authority', dir);
          const canManage = ownerTools && id !== actorId && !(person?.isFormer ?? false);
          const username = person?.username ?? '';
          // Copy person ID never stands alone in a menu (T-33).
          const hasMenu = canManage || username !== '';
          return (
            <li
              key={id}
              className="flex min-h-row items-center gap-3 border-b border-border-subtle py-2 last:border-b-0"
            >
              <Avatar
                size={24}
                fallback={person?.avatar}
                name={person?.displayName}
                username={person?.username}
              />
              <span className="min-w-0 flex-1 truncate text-label">
                <PersonName
                  person={person ?? id}
                  dir={dir}
                  context="authority"
                  you={id === actorId}
                />
              </span>
              {rowIsOwner && <Badge tone="neutral">{membersCopy.owner}</Badge>}
              {hasMenu && (
                <MemberActions
                  crew={crew}
                  channel={channel}
                  memberId={id}
                  username={username}
                  label={label}
                  canManage={canManage}
                  announce={announce}
                />
              )}
            </li>
          );
        })}
        {invited.map((invitation) => {
          const invitee = dir.byId(invitation.principal_id);
          return (
            <li
              key={invitation.id}
              className="flex min-h-row items-center gap-3 border-b border-border-subtle py-2 text-text-muted last:border-b-0"
            >
              <Avatar size={24} name={invitee?.displayName} username={invitee?.username} />
              <span className="min-w-0 flex-1 truncate text-label">
                <PersonName person={invitation.principal_id} dir={dir} context="authority" />
              </span>
              <span className="text-supporting">{membersCopy.invited}</span>
            </li>
          );
        })}
      </ul>
      <span className="sr-only" aria-live="polite" aria-atomic="true">
        {announcement}
      </span>
    </div>
  );
}
