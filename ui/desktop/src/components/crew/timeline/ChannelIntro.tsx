import { Button } from '../../ui/button';
import { Hash } from '../../icons/app-icons';
import type { Channel, Snapshot } from '../crewApi';
import { PersonName, channelSlug, personLabel, type PeopleDirectory } from '../identity';
import { SetupChecklist } from '../onboarding/SetupChecklist';
import { useCrew } from '../state/CrewControllerContext';
import { timelineCopy } from './copy';

/**
 * The top of a channel whose start is loaded: "Welcome to #methods" (pinned),
 * who created it, and one action for one intent. There is no "Ask my agent"
 * here: the composer's button stays the only control with that name, so a
 * query for it is never ambiguous.
 *
 * Everyone but the host learns whom to ask for the other channels: "Ask Iris
 * Wong (@crew_iris) to add you to other channels." — the host in the authority
 * form the Members tab and About use, so one person reads one way on every
 * channel surface (Q4-21). The sidebar already says that other channels appear
 * once someone adds you, so this is only the next step, and it names the
 * workspace's host, as the no-team state does (Q2-64). (Crew lists no channel
 * to a non-member, so nothing here can name the ones you are not in.)
 *
 * The action (Q4-23):
 * - while nobody else is in the workspace, only the setup checklist's one line —
 *   "No one else has joined {workspace} yet." with **Invite people to
 *   {workspace}…** (T-22): opening a channel used to take the only in-channel
 *   path to an invitation away. It renders nothing for anyone but the host, and
 *   goes once someone joins or asks to;
 * - otherwise, for the channel's owner, only **Add people**. The two used to
 *   stack for the one intent, a filled button over an outlined one, and Add
 *   people had nobody to add.
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
  const crew = useCrew();
  const { openDialog } = crew;
  const owner = viewerId !== null && channel.owner_id === viewerId;
  const host = dir.viewerIsHost ? null : dir.host;
  const hostName =
    host && !host.isFormer && host.username ? personLabel(host, 'authority', dir) : null;
  const askHost = timelineCopy.introOtherChannels(hostName);
  const alone = aloneInWorkspace(crew.snapshot ?? crew.lastVerified?.snapshot ?? null);
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
      {owner && !channel.archived && !alone && (
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
      {alone && <SetupChecklist compact />}
    </div>
  );
}

/**
 * Nobody but the viewer is in the workspace, and nobody has asked to join: the setup checklist's
 * test for its invite step (`onboarding/SetupChecklist.tsx`), so the intro's action and the
 * compact invite line never both show, and never both go. Without a snapshot, not alone.
 */
function aloneInWorkspace(snapshot: Snapshot | null): boolean {
  if (!snapshot) return false;
  const active = snapshot.principals.filter((person) => person.active !== false).length;
  return active <= 1 && (snapshot.pending_joins?.length ?? 0) === 0;
}
