import type { Snapshot } from '../crewApi';
import { sanitizeUsername } from './displayText';
import { joinerPerson, personLabel, personLayout, resolvePerson } from './personLabel';
import type { PeopleDirectory } from './usePeopleDirectory';
import type { CrewPerson, PersonRef } from './types';

/**
 * The name on each joiner's server account, by workspace and username, as the host's verified
 * snapshots listed it while they waited. The `pending_joins` row is the only place that name comes
 * from, and it is gone in the very snapshot that shows them joined — so without this, the Let in
 * dialog said "Jack joined wong-lab" while the toast and the "Joined, not in your teams" row said
 * "@crew_jack" about the same event (Q4-42), and Add people listed the same person as a bare
 * "@crew_henry" that searching "Ito" did not find (F8).
 *
 * Display only, and for this app session only: nothing is saved, nothing is sent, and it never
 * becomes the person's display name (naming design D2) — it stands in only where the person has
 * not chosen one. Keyed by workspace as well as username, because the same username on another
 * server is another person.
 */
const joinerServerNames = new Map<string, string>();
const joinerKey = (workspaceId: string, username: string) => `${workspaceId}\u0000${username}`;

/** Remembers the server-account name of everyone a verified host snapshot lists as waiting. */
export function rememberJoinerNames(
  snapshot: Pick<Snapshot, 'workspace' | 'pending_joins'> | null | undefined
): void {
  const workspaceId = snapshot?.workspace?.id;
  const pending = snapshot?.pending_joins;
  if (typeof workspaceId !== 'string' || !workspaceId || !Array.isArray(pending)) return;
  for (const join of pending) {
    if (!join || typeof join.username !== 'string') continue;
    const person = joinerPerson(join.username, join.full_name);
    if (person.username && person.serverName) {
      joinerServerNames.set(joinerKey(workspaceId, person.username), person.serverName);
    }
  }
}

/** The remembered server-account name of `username` in this workspace, or `null`. */
export function joinerServerName(
  workspaceId: string | null | undefined,
  username: string | null | undefined
): string | null {
  const handle = sanitizeUsername(username);
  if (!workspaceId || !handle) return null;
  return joinerServerNames.get(joinerKey(workspaceId, handle)) ?? null;
}

/** Tests only: start from an app session that has seen nobody wait. */
export function forgetJoinerNames(): void {
  joinerServerNames.clear();
}

/**
 * A member who joined, named as the Let in dialog named them (Q4-42): `person` with the name on
 * their server account standing in for the display name they have not chosen yet — and carried as
 * its `serverName`, which marks the copy as a stand-in — so `PersonName`/`personLabel` read "Jack
 * Moreno (@crew_jack)". `null` when nothing changes: they chose a name (theirs wins), or their
 * server-account name was never seen. Render the result WITHOUT the directory: looking the person
 * up again would put the directory's copy, and its bare `@crew_jack`, back.
 */
export function joinedAsNamed(
  person: PersonRef,
  dir: PeopleDirectory | null | undefined,
  workspaceId: string | null | undefined
): CrewPerson | null {
  const resolved = resolvePerson(person, dir);
  if (!resolved || resolved.isFormer) return null;
  const layout = personLayout(resolved, 'inline');
  if (layout.kind !== 'person' || layout.lead !== 'handle') return null;
  const serverName = joinerServerName(workspaceId, resolved.username);
  return serverName ? Object.freeze({ ...resolved, displayName: serverName, serverName }) : null;
}

/** `personLabel(person, 'inline')`, with the joiner's server-account name when it stands in. */
export function joinedLabel(
  person: PersonRef,
  dir: PeopleDirectory | null | undefined,
  workspaceId: string | null | undefined
): string {
  const named = joinedAsNamed(person, dir, workspaceId);
  return named ? personLabel(named, 'inline') : personLabel(person, 'inline', dir);
}

/**
 * Directory people as a picker or a list names them: each one who joined without choosing a name
 * carries the name on their server account, as the Joined row and the toast name them ({@link
 * joinedAsNamed}); everyone else is as the directory has them.
 */
export function withJoinerNames(
  people: readonly CrewPerson[],
  workspaceId: string | null | undefined
): CrewPerson[] {
  return people.map((person) => joinedAsNamed(person, null, workspaceId) ?? person);
}

/**
 * Whether `person` carries a stand-in name ({@link joinedAsNamed}), so a renderer must draw it as
 * given rather than refresh it from the directory, which would drop the name again. Directory
 * people never carry a `serverName`; a joiner outside the directory has no principal ID.
 */
export function carriesJoinerName(person: CrewPerson): boolean {
  return person.id !== null && person.serverName !== null;
}
