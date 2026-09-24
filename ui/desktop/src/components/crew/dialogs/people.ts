import type { Invitation, Snapshot } from '../crewApi';
import {
  channelName,
  cleanName,
  nameKey,
  type CrewPerson,
  type PeopleDirectory,
} from '../identity';

/**
 * Who a picker may offer, and how a sentence names a person by first name.
 *
 * Pickers never list someone the action cannot apply to (L4): a team's candidates exclude its
 * members; a channel's candidates are the team's members who are not in the channel. While the
 * broker only invites, anyone already invited is left out too, and named as waiting instead. The
 * broker still refuses anything else — this only keeps impossible choices off the screen.
 */

/**
 * `{first}` in the copy deck: the first word of the display name, else of the name on the server
 * account (a joiner has no display name yet), else `@username`.
 */
export function firstName(person: CrewPerson | null | undefined): string {
  if (!person) return '';
  const name =
    person.displayName && person.displayName.toLowerCase() !== person.username.toLowerCase()
      ? person.displayName
      : (person.serverName ?? '');
  const first = cleanName(name).split(' ')[0];
  return first || `@${person.username}`;
}

/** Invitations that still stand: not expired by the broker's word or by the clock. */
export function liveInvitations(
  invitations: readonly Invitation[] | null | undefined,
  nowSeconds = Date.now() / 1000
): Invitation[] {
  return (invitations ?? []).filter(
    (invitation) =>
      invitation &&
      invitation.expired !== true &&
      !(typeof invitation.expires_at === 'number' && invitation.expires_at <= nowSeconds)
  );
}

export type PickerTarget =
  | { kind: 'team'; teamId: string }
  | { kind: 'channel'; channelId: string };

/** The broker `hello` capability for adding a workspace member straight into a team or channel. */
export const DIRECT_ADD_CAPABILITY = 'direct_add_v1';

/**
 * Whether the connected broker adds people directly (`team.add_member`, `channel.add_member`)
 * instead of inviting them to accept. Only the broker's own signed word counts — there is no
 * projection to infer it from — and it decides only which request a dialog sends and what it says
 * will happen: the broker still refuses an addition the person may not make.
 */
export function directAddSupported(capabilities: readonly string[] | null | undefined): boolean {
  return Array.isArray(capabilities) && capabilities.includes(DIRECT_ADD_CAPABILITY);
}

export interface PickerCandidates {
  candidates: CrewPerson[];
  /**
   * The people the target could ever offer, before members and invitees are taken out: everyone
   * active in the workspace but the viewer for a team, and the team's members for a channel.
   */
  others: CrewPerson[];
  /** People invited to the target who have not accepted yet (never offered while inviting). */
  pending: CrewPerson[];
  /** For a channel: people invited to its TEAM who have not accepted yet, so are not in it. */
  pendingTeam: CrewPerson[];
}

/** The people with a live invitation of `kind` to `targetId`, in directory order. */
function invitedTo(
  snapshot: Snapshot,
  dir: PeopleDirectory,
  kind: 'team' | 'channel',
  targetId: string
): CrewPerson[] {
  const ids = new Set(
    liveInvitations(snapshot.invitations)
      .filter((invitation) => invitation.kind === kind && invitation.target_id === targetId)
      .map((invitation) => invitation.principal_id)
  );
  return dir.people.filter(
    (person) => person.id && ids.has(person.id) && !person.isYou && !person.isFormer
  );
}

/**
 * The people an Add people picker offers for a team or channel.
 *
 * Inviting (an older broker), a person with a live invitation is not offered again and is listed
 * as waiting instead, because the invitation stands until they accept it. Adding directly
 * (`directAdd`), everyone who is not in yet is offered — an invitation still waiting is no reason
 * to keep them out.
 */
export function addPeopleCandidates(
  snapshot: Snapshot | null | undefined,
  dir: PeopleDirectory,
  target: PickerTarget,
  options: { directAdd?: boolean } = {}
): PickerCandidates {
  const everyone = dir.people.filter((person) => !person.isYou && !person.isFormer && person.id);
  if (!snapshot) return { candidates: [], others: everyone, pending: [], pendingTeam: [] };
  if (target.kind === 'team') {
    const team = snapshot.teams.find((item) => item.id === target.teamId);
    const members = new Set(team?.members ?? []);
    const pending = invitedTo(snapshot, dir, 'team', target.teamId).filter(
      (person) => !members.has(person.id!)
    );
    const waiting = new Set(pending.map((person) => person.id));
    return {
      candidates: everyone.filter(
        (person) => !members.has(person.id!) && (options.directAdd || !waiting.has(person.id))
      ),
      others: everyone,
      pending,
      pendingTeam: [],
    };
  }
  const channel = snapshot.channels.find((item) => item.id === target.channelId);
  const team = channel ? snapshot.teams.find((item) => item.id === channel.team_id) : undefined;
  const teamMembers = new Set(team?.members ?? []);
  const channelMembers = new Set(channel?.members ?? []);
  const inTeam = everyone.filter((person) => teamMembers.has(person.id!));
  const pending = invitedTo(snapshot, dir, 'channel', target.channelId).filter(
    (person) => !channelMembers.has(person.id!)
  );
  const pendingTeam = team
    ? invitedTo(snapshot, dir, 'team', team.id).filter((person) => !teamMembers.has(person.id!))
    : [];
  const waiting = new Set(pending.map((person) => person.id));
  return {
    candidates: inTeam.filter(
      (person) => !channelMembers.has(person.id!) && (options.directAdd || !waiting.has(person.id))
    ),
    others: inTeam,
    pending,
    pendingTeam,
  };
}

/** `@a`, `@a and @b`, `@a, @b and @c`: people named by username, as the copy deck lists them. */
export function usernameList(people: readonly CrewPerson[]): string {
  return listOf(people.map((person) => `@${person.username}`));
}

/** `a`, `a and b`, `a, b and c`. */
export function listOf(items: readonly string[]): string {
  if (items.length <= 1) return items[0] ?? '';
  return `${items.slice(0, -1).join(', ')} and ${items[items.length - 1]}`;
}

/** A channel a direct team addition can also add the person to. */
export interface ChannelChoice {
  id: string;
  /** `#slug`. */
  label: string;
  /** The team's #general, which comes with the team and cannot be left out. */
  always: boolean;
  /** Checked until the person unchecks it: #general, and every channel the viewer owns. */
  checked: boolean;
}

/**
 * The channels offered beside "Add @x to {team}": #general first (always, since joining a team
 * joins its #general), then the team's other open channels the viewer may add people to — those
 * they own, and as the host any they can see — owned ones checked. Display only: the broker adds
 * the person to a listed channel only where the caller owns it or hosts the workspace.
 */
export function directAddChannels(
  snapshot: Snapshot | null | undefined,
  teamId: string,
  dir: PeopleDirectory
): ChannelChoice[] {
  const team = snapshot?.teams.find((item) => item.id === teamId);
  if (!snapshot || !team) return [];
  const me = dir.me?.id ?? null;
  const general = snapshot.channels.find((item) => item.id === team.general_channel_id);
  const others = snapshot.channels
    .filter(
      (channel) =>
        channel.team_id === teamId &&
        channel.id !== team.general_channel_id &&
        !channel.archived &&
        ((me !== null && channel.owner_id === me) || dir.viewerIsHost)
    )
    .map((channel) => ({
      id: channel.id,
      label: channelName(channel),
      always: false,
      checked: me !== null && channel.owner_id === me,
    }))
    .sort((a, b) => a.label.localeCompare(b.label));
  return [
    {
      id: team.general_channel_id,
      label: general ? channelName(general) : '#general',
      always: true,
      checked: true,
    },
    ...others,
  ];
}

/** What `team.add_member` / `channel.add_member` answered, as far as the dialogs need it. */
export interface DirectAddResult {
  addedChannels: string[];
  alreadyMember: boolean;
}

/** The broker's answer to a direct addition, read leniently: an absent field means none. */
export function directAddResultFrom(value: unknown): DirectAddResult {
  const record =
    typeof value === 'object' && value !== null ? (value as Record<string, unknown>) : {};
  const added = Array.isArray(record.added_channels)
    ? record.added_channels.filter((id): id is string => typeof id === 'string' && id.length > 0)
    : [];
  return { addedChannels: added, alreadyMember: record.already_member === true };
}

/**
 * The channels a person can see after a direct TEAM addition, as `#a and #b`: the team's #general
 * (which comes with the team) and every channel the broker says it added, in that order, once each.
 */
export function channelsSeenAfterTeamAdd(
  snapshot: Snapshot | null | undefined,
  teamId: string,
  added: readonly string[]
): string {
  const team = snapshot?.teams.find((item) => item.id === teamId);
  const ids = [...(team ? [team.general_channel_id] : []), ...added];
  const labels: string[] = [];
  for (const id of new Set(ids)) {
    const channel = snapshot?.channels.find((item) => item.id === id);
    labels.push(channel ? channelName(channel) : id === team?.general_channel_id ? '#general' : '');
  }
  return listOf(labels.filter(Boolean));
}

/** The channel's other active members, who could accept its ownership. */
export function ownershipCandidates(
  snapshot: Snapshot | null | undefined,
  dir: PeopleDirectory,
  channelId: string
): CrewPerson[] {
  const channel = snapshot?.channels.find((item) => item.id === channelId);
  if (!channel) return [];
  const members = new Set(channel.members);
  return dir.people.filter(
    (person) => person.id && !person.isYou && !person.isFormer && members.has(person.id)
  );
}

/**
 * Whether a person matches a picker query: by display name or username, ignoring case, spacing
 * and a leading `@`. Display-name matching is safe here because every row shows `@username`
 * before it can be chosen (naming design, "Selectors and the resolver", rule 3).
 */
export function personMatches(person: CrewPerson, query: string): boolean {
  const wanted = nameKey(query.trim().replace(/^@/, ''));
  if (!wanted) return true;
  return nameKey(person.displayName).includes(wanted) || nameKey(person.username).includes(wanted);
}
