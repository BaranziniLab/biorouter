import { Avatar } from '../../ui/avatar';
import { Button } from '../../ui/button';
import { cn } from '../../../utils';
import type { CrewPerson, PeopleDirectory } from '../identity';
import { channelCopy } from './copy';
import './channel.css';

export interface MemberStackProps {
  /** The CHANNEL's member IDs — the set the Members tab lists, never the team's people. */
  memberIds: readonly string[];
  /** The channel's owner, who leads the stack as they lead the Members tab. */
  ownerId?: string | null;
  dir: PeopleDirectory;
  /** Open the details pane on the Members tab. */
  onOpen(): void;
  className?: string;
}

const STACK_SIZE = 3;

/**
 * The channel's current members in the header's order: the owner, then you, then everyone else
 * by name, as the Members tab lists them. A former member is still listed on the Members tab
 * (marked so), but is not in the channel now, so neither the stack nor its count includes them:
 * "2 members" used to mean you and someone who had left (Q2-54). A member the viewer has no
 * projection for is still a member, and keeps a place at the end.
 */
export function currentMembers(
  memberIds: readonly string[],
  dir: PeopleDirectory,
  ownerId?: string | null
): { id: string; person: CrewPerson | null }[] {
  const you = dir.me?.id ?? null;
  const rank = (id: string, person: CrewPerson | null) =>
    id === ownerId ? 0 : id === you ? 1 : person ? 2 : 3;
  return [...new Set(memberIds)]
    .map((id) => ({ id, person: dir.byId(id) }))
    .filter(({ person }) => !person?.isFormer)
    .sort(
      (a, b) =>
        rank(a.id, a.person) - rank(b.id, b.person) ||
        (a.person?.displayName ?? '').localeCompare(b.person?.displayName ?? '', undefined, {
          sensitivity: 'base',
        })
    );
}

/**
 * Up to three member avatars, then the channel's member count (ui-redesign-spec, "The channel
 * header and channel menu"). The avatars are decorative — the button's name is "{n} members" —
 * and both count only the people in the channel now ({@link currentMembers}). A member the viewer
 * has no projection for gets a blank circle, never initials made from an ID.
 */
export function MemberStack({ memberIds, ownerId, dir, onOpen, className }: MemberStackProps) {
  const members = currentMembers(memberIds, dir, ownerId);
  const count = members.length;
  const people = members.slice(0, STACK_SIZE);
  return (
    <Button
      type="button"
      variant="ghost"
      size="sm"
      aria-label={channelCopy.members(count)}
      className={cn('crew-member-stack no-drag gap-2', className)}
      onClick={onOpen}
    >
      {people.length > 0 && (
        <span className="flex items-center" aria-hidden="true">
          {people.map(({ id, person }, index) => (
            <Avatar
              key={id}
              size={20}
              ring
              fallback={person?.avatar}
              name={person?.displayName}
              username={person?.username}
              className={index > 0 ? '-ml-1' : undefined}
            />
          ))}
        </span>
      )}
      <span className="tabular-nums">{count}</span>
    </Button>
  );
}
