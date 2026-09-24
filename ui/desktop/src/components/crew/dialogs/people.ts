import type { Invitation, Snapshot } from '../crewApi';
import { cleanName, nameKey, type CrewPerson, type PeopleDirectory } from '../identity';

/**
 * Who a picker may offer, and how a sentence names a person by first name.
 *
 * Pickers never list someone the action cannot apply to (L4): a team's candidates exclude its
 * members and anyone with a live invitation to it; a channel's candidates are the team's members
 * who are not in the channel and not already invited. The broker still refuses anything else —
 * this only keeps impossible choices off the screen.
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

export interface PickerCandidates {
  candidates: CrewPerson[];
  /** Active people other than the viewer, before excluding members and invitees. */
  others: CrewPerson[];
}

/** The people an Add people picker offers for a team or channel. */
export function addPeopleCandidates(
  snapshot: Snapshot | null | undefined,
  dir: PeopleDirectory,
  target: PickerTarget
): PickerCandidates {
  const others = dir.people.filter((person) => !person.isYou && !person.isFormer && person.id);
  if (!snapshot) return { candidates: [], others };
  const pending = new Set(
    liveInvitations(snapshot.invitations)
      .filter((invitation) =>
        target.kind === 'team'
          ? invitation.kind === 'team' && invitation.target_id === target.teamId
          : invitation.kind === 'channel' && invitation.target_id === target.channelId
      )
      .map((invitation) => invitation.principal_id)
  );
  if (target.kind === 'team') {
    const team = snapshot.teams.find((item) => item.id === target.teamId);
    const members = new Set(team?.members ?? []);
    return {
      candidates: others.filter((person) => !members.has(person.id!) && !pending.has(person.id!)),
      others,
    };
  }
  const channel = snapshot.channels.find((item) => item.id === target.channelId);
  const team = channel ? snapshot.teams.find((item) => item.id === channel.team_id) : undefined;
  const teamMembers = new Set(team?.members ?? []);
  const channelMembers = new Set(channel?.members ?? []);
  return {
    candidates: others.filter(
      (person) =>
        teamMembers.has(person.id!) && !channelMembers.has(person.id!) && !pending.has(person.id!)
    ),
    others: others.filter((person) => teamMembers.has(person.id!)),
  };
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
