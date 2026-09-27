import { useEffect, useRef } from 'react';
import { toastSuccess } from '../../../toasts';
import type { Channel, Snapshot } from '../crewApi';
import { buildPeopleDirectory, channelName, isolate, personLabel } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { layoutCopy } from './copy';

interface Seen {
  connectionId: string;
  workspaceId: string;
  channels: ReadonlySet<string>;
}

/** The channels the viewer is in now: listed to them, not archived, and naming them a member. */
function memberChannels(snapshot: Snapshot): Channel[] {
  const viewer = snapshot.actor.id;
  return snapshot.channels.filter(
    (channel) =>
      !channel.archived && Array.isArray(channel.members) && channel.members.includes(viewer)
  );
}

/**
 * Who added the viewer: the channel's owner, or its creator when the viewer owns it. Null when
 * the viewer both made and owns it (they added themselves: nothing to announce).
 */
function adderOf(channel: Channel, viewer: string): string | null {
  if (channel.owner_id && channel.owner_id !== viewer) return channel.owner_id;
  if (channel.created_by && channel.created_by !== viewer) return channel.created_by;
  return null;
}

/**
 * "@alice added you to #methods" (Q2-63; ui-redesign-spec, "Where errors render": toasts only
 * for results that happen off-screen). Someone adds you to a channel while you are elsewhere; the
 * channel just appears in the sidebar, which nobody notices. So you hear about it once, when a
 * verified view of the same workspace lists you in a channel the previous one did not, and you
 * did not make it yourself.
 *
 * Modelled on `useJoinedToast`: only between two verified views of the same connection and
 * workspace, so opening Crew, switching workspaces or re-verifying after a failure never
 * announces the channels already there. Display only: membership is the broker's, and this reads
 * the snapshot it projected.
 */
export function useChannelAddedToast(): void {
  const crew = useCrew();
  const verified =
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
      ? crew.snapshot
      : null;
  const seen = useRef<Seen | null>(null);
  const { connectionId, labels } = crew;

  useEffect(() => {
    if (!verified) return;
    const current = memberChannels(verified);
    const before = seen.current;
    seen.current = {
      connectionId,
      workspaceId: verified.workspace.id,
      channels: new Set(current.map((channel) => channel.id)),
    };
    if (
      !before ||
      before.connectionId !== connectionId ||
      before.workspaceId !== verified.workspace.id
    )
      return;
    const viewer = verified.actor.id;
    const added = current.filter(
      (channel) => !before.channels.has(channel.id) && channel.created_by !== viewer
    );
    if (added.length === 0) return;
    const dir = buildPeopleDirectory(verified, labels);
    for (const channel of added) {
      const adder = adderOf(channel, viewer);
      if (!adder) continue;
      const person = dir.byId(adder);
      const who =
        person && !person.isFormer && person.username
          ? isolate(`@${person.username}`)
          : personLabel(adder, 'inline', dir);
      toastSuccess({ msg: layoutCopy.channelAdded(who, channelName(channel)) });
    }
  }, [verified, connectionId, labels]);
}
