import { useEffect, useRef } from 'react';
import { toastSuccess } from '../../../toasts';
import type { Channel, Snapshot, Team } from '../crewApi';
import {
  buildPeopleDirectory,
  channelName,
  channelNamesAcrossTeams,
  joinedLabel,
  teamName,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { layoutCopy } from './copy';

interface Seen {
  connectionId: string;
  workspaceId: string;
  channels: ReadonlySet<string>;
  teams: ReadonlySet<string>;
}

/** The channels the viewer is in now: listed to them, not archived, and naming them a member. */
function memberChannels(snapshot: Snapshot): Channel[] {
  const viewer = snapshot.actor.id;
  return snapshot.channels.filter(
    (channel) =>
      !channel.archived && Array.isArray(channel.members) && channel.members.includes(viewer)
  );
}

/** The teams the viewer is in now: listed to them, and naming them a member. */
function memberTeams(snapshot: Snapshot): Team[] {
  const viewer = snapshot.actor.id;
  return snapshot.teams.filter(
    (team) => Array.isArray(team.members) && team.members.includes(viewer)
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
 * "Alice Chen (@alice) added you to #methods" (Q2-63; ui-redesign-spec, "Where errors render":
 * toasts only for results that happen off-screen). Someone adds you to a channel while you are
 * elsewhere; the channel just appears in the sidebar, which nobody notices. So you hear about it
 * once, when a verified view of the same workspace lists you in a channel the previous one did
 * not, and you did not make it yourself.
 *
 * Named by the one rule for inline text (M11): the person as `personLabel(…, 'inline')` — with the
 * name on their server account while they have chosen none, as the joined toast names people — and
 * the channel as every list names it, with its team only where two teams share its name (every
 * team has a #general). Being added to a TEAM — which brings its #general, and any channels
 * ticked with it — is one toast naming the team, "… added you to Bench Crew", not a channel's.
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
    const teams = memberTeams(verified);
    const before = seen.current;
    seen.current = {
      connectionId,
      workspaceId: verified.workspace.id,
      channels: new Set(current.map((channel) => channel.id)),
      teams: new Set(teams.map((team) => team.id)),
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
    const joinedTeams = teams.filter(
      (team) => !before.teams.has(team.id) && team.created_by !== viewer
    );
    if (added.length === 0 && joinedTeams.length === 0) return;
    const dir = buildPeopleDirectory(verified, labels);
    const who = (principal: string) => joinedLabel(principal, dir, verified.workspace.id);
    const announcedTeams = new Set<string>();
    for (const team of joinedTeams) {
      // Its #general came with it: whoever owns that is who the viewer hears from, as for a
      // channel. A team the viewer made is never announced.
      const general = verified.channels.find((channel) => channel.id === team.general_channel_id);
      const adder = general ? adderOf(general, viewer) : team.created_by;
      if (!adder || adder === viewer) continue;
      announcedTeams.add(team.id);
      toastSuccess({ msg: layoutCopy.teamAdded(who(adder), teamName(team)) });
    }
    const places = channelNamesAcrossTeams(verified.channels, verified.teams);
    for (const channel of added) {
      // Said once, with its team.
      if (announcedTeams.has(channel.team_id)) continue;
      const adder = adderOf(channel, viewer);
      if (!adder) continue;
      toastSuccess({
        msg: layoutCopy.channelAdded(who(adder), places.get(channel.id) ?? channelName(channel)),
      });
    }
  }, [verified, connectionId, labels]);
}
