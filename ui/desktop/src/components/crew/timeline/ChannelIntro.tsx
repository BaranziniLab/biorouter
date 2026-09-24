import { Button } from '../../ui/button';
import { Hash } from '../../icons/app-icons';
import type { Channel } from '../crewApi';
import { PersonName, channelSlug, isolate, type PeopleDirectory } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { timelineCopy } from './copy';

/**
 * The top of a channel whose start is loaded: "Welcome to #methods" (pinned),
 * who created it, and — for its owner — one secondary Add people. There is no
 * "Ask my agent" here: the composer's button stays the only control with that
 * name, so a query for it is never ambiguous.
 *
 * Everyone else learns how the other channels appear: only channels someone
 * added you to are listed, so a `#methods` named in a post and missing from the
 * sidebar is not broken — and the owner is who to ask. (Crew lists no channel
 * to a non-member, so nothing here can name the ones you are not in.)
 *
 * `pending` keeps its place while the live tail is still streaming in and the
 * start is not yet known to be loaded: laid out but invisible, hidden from
 * assistive technology and inert, so it claims nothing and cannot be reached.
 */
export function ChannelIntro({
  channel,
  viewerId,
  dir,
  readOnly,
  pending = false,
}: {
  channel: Channel;
  viewerId: string | null;
  dir: PeopleDirectory;
  readOnly: boolean;
  pending?: boolean;
}) {
  const { openDialog } = useCrew();
  const owner = viewerId !== null && channel.owner_id === viewerId;
  const ownerPerson = owner ? null : dir.byId(channel.owner_id);
  const ownerHandle =
    ownerPerson && !ownerPerson.isFormer && ownerPerson.username
      ? isolate(`@${ownerPerson.username}`)
      : null;
  return (
    <div
      className="crew-channel-intro"
      data-pending={pending ? 'true' : undefined}
      aria-hidden={pending ? true : undefined}
      inert={pending}
    >
      <Hash aria-hidden className="crew-channel-intro-icon" />
      <h2 className="text-subheading text-text-default">
        {timelineCopy.introTitle(channelSlug(channel))}
      </h2>
      <p className="text-body text-text-muted">
        <PersonName person={channel.created_by} context="inline" dir={dir} />
        {timelineCopy.introCreatedBy}
      </p>
      {!owner && (
        <p className="crew-channel-intro-hint text-supporting text-text-muted">
          {timelineCopy.introOtherChannels(ownerHandle)}
        </p>
      )}
      {owner && !channel.archived && (
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="crew-channel-intro-action"
          disabled={readOnly}
          onClick={() =>
            openDialog({ kind: 'add-people', target: 'channel', targetId: channel.id })
          }
        >
          {timelineCopy.introAddPeople}
        </Button>
      )}
    </div>
  );
}
