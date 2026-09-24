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
import { membersCopy } from './copy';
import { copyText, usePanePresentation } from './presentation';

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

/**
 * The details pane's Members tab (ui-redesign-spec, "The details pane"): the CHANNEL's members —
 * the same set, and the same count, as the header's member stack — never the team's people.
 *
 * Every row names the person at an authority point, "Display name (@username)", with Owner, you
 * and former-member markers. The row's `⋯` holds Copy person ID (the only place a person's ID
 * appears) and, for the owner acting on someone else, Make owner… and Remove from #name…, which
 * open the transfer dialog and the removal confirmation. Channel invitations the viewer sent and
 * nobody has accepted yet appear as muted "invited" rows.
 *
 * The owner adds people from here; anyone else is told who can, instead of being shown nothing.
 * The broker decides every one of these actions.
 */
export function MembersTab({ className }: MembersTabProps) {
  const { crew, snapshot, channel, dir, isOwner } = usePanePresentation();
  if (!snapshot || !channel) return null;

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
              <DropdownMenu>
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
                  {canManage && (
                    <>
                      <DropdownMenuItem
                        onSelect={() =>
                          crew.openDialog({
                            kind: 'transfer-ownership',
                            channelId: channel.id,
                            successorId: id,
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
                              principalId: id,
                            },
                          })
                        }
                      >
                        {membersCopy.remove(name)}
                      </DropdownMenuItem>
                      <DropdownMenuSeparator />
                    </>
                  )}
                  <DropdownMenuItem onSelect={() => void copyText(id)}>
                    {membersCopy.copyPersonId}
                  </DropdownMenuItem>
                </DropdownMenuContent>
              </DropdownMenu>
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
    </div>
  );
}
