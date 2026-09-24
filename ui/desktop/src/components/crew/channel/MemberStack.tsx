import { Avatar } from '../../ui/avatar';
import { Button } from '../../ui/button';
import { cn } from '../../../utils';
import type { PeopleDirectory } from '../identity';
import { channelCopy } from './copy';
import './channel.css';

export interface MemberStackProps {
  /** The CHANNEL's member IDs — the count the Members tab lists, never the team's people. */
  memberIds: readonly string[];
  dir: PeopleDirectory;
  /** Open the details pane on the Members tab. */
  onOpen(): void;
  className?: string;
}

const STACK_SIZE = 3;

/**
 * Up to three member avatars, then the channel's member count (ui-redesign-spec, "The channel
 * header and channel menu"). The avatars are decorative — the button's name is "{n} members" —
 * and they prefer current members over former ones. A member the viewer has no projection for
 * gets a blank circle, never initials made from an ID.
 */
export function MemberStack({ memberIds, dir, onOpen, className }: MemberStackProps) {
  const count = memberIds.length;
  const people = memberIds
    .map((id) => dir.byId(id))
    .sort((a, b) => Number(a?.isFormer ?? true) - Number(b?.isFormer ?? true))
    .slice(0, STACK_SIZE);
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
          {people.map((person, index) => (
            <Avatar
              key={person?.id ?? `unknown-${index}`}
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
