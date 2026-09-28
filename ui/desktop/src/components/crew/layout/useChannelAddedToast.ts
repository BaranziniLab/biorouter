import { useEffect } from 'react';
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
import { lastSeenMemberships, rememberSeenMemberships } from '../state/viewMemory';
import { layoutCopy } from './copy';

/**
 * The teams and channels being renamed on this computer, by ID, with when: the person who renames
 * one is not told of it again (M12). Marked before the request, since the new name can arrive in a
 * state frame before the request answers, and forgotten when it fails, when the notice it stands
 * for is skipped, or after {@link OWN_RENAME_MS}. Module state, as the rename dialog and this hook
 * are mounted separately; display only.
 */
const ownRenames = new Map<string, number>();
const OWN_RENAME_MS = 60_000;

/** The rename dialog is about to rename `id`. */
export function noteOwnRename(id: string): void {
  ownRenames.set(id, Date.now());
}

/** The rename of `id` failed; a later rename by someone else is announced again. */
export function forgetOwnRename(id: string): void {
  ownRenames.delete(id);
}

/** Whether the viewer renamed `id` here just now, which then no longer counts. */
function takeOwnRename(id: string): boolean {
  const at = ownRenames.get(id);
  ownRenames.delete(id);
  return at !== undefined && Date.now() - at <= OWN_RENAME_MS;
}

/** Tests only. */
export function forgetOwnRenames(): void {
  ownRenames.clear();
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
 * Also, once per rename, "#history-qa is now #plate-history" or "Bench QA is now Bench Crew" for a
 * channel or team the viewer is in, renamed elsewhere (M12): the broker only overwrites the name,
 * and the old one then stops resolving in the CLI, with nothing to say why.
 *
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
 * workspace, so re-verifying after a failure never announces the channels already there, and the
 * first view of a connection this app session announces nothing. The two views need not be one
 * Crew screen's: coming back to Crew, or to a workspace, compares with the last view of it this app
 * session showed, so an add while the person was away is said on their return (MSG2-N8). Display
 * only: membership is the broker's, and this reads the snapshot it projected.
 */
export function useChannelAddedToast(): void {
  const crew = useCrew();
  const verified =
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
      ? crew.snapshot
      : null;
  const { connectionId, labels } = crew;

  useEffect(() => {
    if (!verified || !connectionId) return;
    const current = memberChannels(verified);
    const teams = memberTeams(verified);
    // What the last view of this connection showed, on this Crew screen or an earlier one: kept
    // by the layout, it went with the screen, and an add made while the person was on Home was
    // never said (MSG2-N8).
    const before = lastSeenMemberships(connectionId);
    const places = channelNamesAcrossTeams(verified.channels, verified.teams);
    const channelNames = new Map(
      current.map((channel) => {
        const own = channelName(channel);
        return [channel.id, { own, label: places.get(channel.id) ?? own }];
      })
    );
    const teamNames = new Map(teams.map((team) => [team.id, teamName(team)]));
    rememberSeenMemberships(connectionId, {
      workspaceId: verified.workspace.id,
      channels: new Set(current.map((channel) => channel.id)),
      teams: new Set(teams.map((team) => team.id)),
      channelNames,
      teamNames,
    });
    if (!before || before.workspaceId !== verified.workspace.id) return;
    // Renamed while the viewer was a member, and not by this computer: said once (M12). A team's
    // old name also stops working in the CLI, and the sidebar just shows the new one.
    for (const [id, name] of teamNames) {
      const was = before.teamNames.get(id);
      if (was !== undefined && was !== name && !takeOwnRename(id))
        toastSuccess({ msg: layoutCopy.renamed(was, name) });
    }
    for (const [id, name] of channelNames) {
      const was = before.channelNames.get(id);
      // Only its own name: a team renamed around it, or a namesake elsewhere, changes its label.
      if (was !== undefined && was.own !== name.own && !takeOwnRename(id))
        toastSuccess({ msg: layoutCopy.renamed(was.label, name.label) });
    }
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
