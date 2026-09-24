import { Button } from '../../ui/button';
import { Hash } from '../../icons/app-icons';
import type { Channel } from '../crewApi';
import { PersonName, channelSlug, isolate, type PeopleDirectory } from '../identity';
import { SetupChecklist } from '../onboarding/SetupChecklist';
import { useCrew } from '../state/CrewControllerContext';
import { timelineCopy } from './copy';

/**
 * The top of a channel whose start is loaded: "Welcome to #methods" (pinned),
 * who created it, and — for its owner — one secondary Add people. There is no
 * "Ask my agent" here: the composer's button stays the only control with that
 * name, so a query for it is never ambiguous.
 *
 * Everyone but the host learns whom to ask for the other channels: "Ask
 * @alice to add you to other channels." The sidebar already says that other
 * channels appear once someone adds you, so this is only the next step, and it
 * names the workspace's host, as the no-team state does (Q2-64). (Crew lists no
 * channel to a non-member, so nothing here can name the ones you are not in.)
 *
 * While the host is still alone in the workspace, the setup checklist's one line — "No one else
 * has joined {workspace} yet." with **Invite people to {workspace}…** — follows (T-22): opening a
 * channel used to take the only in-channel path to an invitation away. It renders nothing for
 * anyone else, and goes once someone joins or asks to.
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
  const host = dir.viewerIsHost ? null : dir.host;
  const hostHandle = host && !host.isFormer && host.username ? isolate(`@${host.username}`) : null;
  const askHost = timelineCopy.introOtherChannels(hostHandle);
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
      {askHost && (
        <p className="crew-channel-intro-hint text-supporting text-text-muted">{askHost}</p>
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
      {/* The host alone in the workspace keeps "Invite people to {workspace}…" here (T-22). */}
      <SetupChecklist compact />
    </div>
  );
}
