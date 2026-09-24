import { useMemo } from 'react';
import { personDisplayName, sanitizeDisplayText, sanitizeUsername } from './displayText';
import { displayNameKey, nameKey } from './nameKey';
import type {
  CrewPeopleMap,
  CrewPerson,
  CrewPrincipalInput,
  DaemonPersonLabel,
  DaemonPersonLabels,
  PeopleSnapshotInput,
} from './types';

/**
 * The people of one workspace, resolved once per snapshot.
 *
 * `byId` covers everyone the viewer may be shown: the active principals, the
 * actor, the former principals the broker projects (S1), and the authors a
 * message result's `people` map names. Anyone else is `null`, which every
 * renderer turns into "Unknown member" — never into the ID it was asked about.
 */
export interface PeopleDirectory {
  byId(id: string | null | undefined): CrewPerson | null;
  /** Active people in snapshot order, the viewer included. */
  readonly people: readonly CrewPerson[];
  /** Former members the viewer's objects still reference. */
  readonly formerPeople: readonly CrewPerson[];
  /** The viewer, or `null` without a snapshot. */
  readonly me: CrewPerson | null;
  /** The workspace host, when the viewer can see them. */
  readonly host: CrewPerson | null;
  /** Whether the viewer hosts this workspace (display only; the broker decides authority). */
  readonly viewerIsHost: boolean;
  collides(person: CrewPerson | string | null | undefined): boolean;
  isFormer(person: CrewPerson | string | null | undefined): boolean;
  isHost(person: CrewPerson | string | null | undefined): boolean;
}

interface RawEntry {
  id: string;
  username: string;
  uid: number | null;
  name: unknown;
  avatar: unknown;
  former: boolean;
}

function isNonEmptyString(value: unknown): value is string {
  return typeof value === 'string' && value.length > 0;
}

/** The fields of an untyped principal that the display rule can use, or `null` if it has none. */
function readPrincipal(value: unknown, former: boolean): RawEntry | null {
  if (!value || typeof value !== 'object') return null;
  const p = value as Partial<CrewPrincipalInput>;
  if (!isNonEmptyString(p.id)) return null;
  const username = sanitizeUsername(p.username);
  if (!username) return null;
  return {
    id: p.id,
    username,
    uid: typeof p.uid === 'number' && Number.isFinite(p.uid) ? p.uid : null,
    name: isNonEmptyString(p.display_name) ? p.display_name : p.nickname,
    avatar: p.avatar,
    former: former || p.active === false,
  };
}

const EMPTY: readonly CrewPerson[] = Object.freeze([]);

/** The daemon's collision verdict for one principal, when its label carries a boolean one. */
function daemonCollision(
  labels: DaemonPersonLabels | null | undefined,
  principalId: string
): boolean | undefined {
  if (!labels || typeof labels !== 'object') return undefined;
  const label: unknown = (labels as Record<string, unknown>)[principalId];
  if (!label || typeof label !== 'object') return undefined;
  const collides = (label as DaemonPersonLabel).collides;
  return typeof collides === 'boolean' ? collides : undefined;
}

function asArray<T>(value: readonly T[] | null | undefined): readonly T[] {
  return Array.isArray(value) ? value : [];
}

/**
 * Builds the directory. Pure, so the controller and tests can call it directly;
 * {@link usePeopleDirectory} memoizes it for components.
 *
 * Precedence when one ID appears twice: the active principals, then the actor,
 * then the projected former principals, then the `people` map — the snapshot is
 * the verified, fuller source, and the `people` map only fills authors it does
 * not cover.
 *
 * **Display names.** The projected `display_name` is preferred over `nickname`,
 * both are sanitized ({@link personDisplayName}), and a display name whose key
 * equals ANOTHER person's username falls back to the person's own username. That
 * mirrors the broker's projection sanitizer ("looks like another principal's
 * username"), so a nickname of `alice` cannot pass for `@alice` on a daemon that
 * predates the sanitizer.
 *
 * **Collisions.** When the daemon projects `labels` for a person, its `collides`
 * is taken as is: it is computed with the confusable skeleton, which this
 * renderer cannot reproduce. Otherwise two people collide when their display
 * names share a `nameKey` (the naming design's pre-S2b rule).
 */
export function buildPeopleDirectory(
  snapshot: PeopleSnapshotInput | null | undefined,
  labels?: DaemonPersonLabels | null,
  people?: CrewPeopleMap | null
): PeopleDirectory {
  const entries = new Map<string, RawEntry>();
  const add = (entry: RawEntry | null) => {
    if (entry && !entries.has(entry.id)) entries.set(entry.id, entry);
  };

  asArray(snapshot?.principals).forEach((p) => add(readPrincipal(p, false)));
  const actor = snapshot ? readPrincipal(snapshot.actor, false) : null;
  add(actor);
  asArray(snapshot?.former_principals).forEach((p) => add(readPrincipal(p, true)));
  if (people && typeof people === 'object') {
    Object.entries(people).forEach(([id, entry]) => {
      if (entry && typeof entry === 'object') add(readPrincipal({ ...entry, id }, false));
    });
  }

  // Whose username each key belongs to, for the impersonation fallback.
  const usernameOwners = new Map<string, Set<string>>();
  entries.forEach((entry) => {
    const key = nameKey(entry.username);
    if (!key) return;
    const owners = usernameOwners.get(key) ?? new Set<string>();
    owners.add(entry.id);
    usernameOwners.set(key, owners);
  });

  const displayNames = new Map<string, string>();
  entries.forEach((entry) => {
    let displayName = personDisplayName(entry.name, entry.username);
    const owners = usernameOwners.get(displayNameKey(displayName));
    if (owners && [...owners].some((owner) => owner !== entry.id)) displayName = entry.username;
    displayNames.set(entry.id, displayName);
  });

  const collisionCounts = new Map<string, number>();
  displayNames.forEach((displayName) => {
    const key = displayNameKey(displayName);
    if (key) collisionCounts.set(key, (collisionCounts.get(key) ?? 0) + 1);
  });

  const rawHostPrincipalId = snapshot?.workspace?.host_principal_id;
  const rawHostUid = snapshot?.workspace?.host_uid;
  const hostPrincipalId = isNonEmptyString(rawHostPrincipalId) ? rawHostPrincipalId : null;
  const hostUid = typeof rawHostUid === 'number' ? rawHostUid : null;
  const isHostEntry = (entry: RawEntry) =>
    hostPrincipalId !== null
      ? entry.id === hostPrincipalId
      : hostUid !== null && !entry.former && entry.uid === hostUid;

  const byId = new Map<string, CrewPerson>();
  entries.forEach((entry) => {
    const displayName = displayNames.get(entry.id) ?? entry.username;
    const daemonCollides = daemonCollision(labels, entry.id);
    const avatar = sanitizeDisplayText(entry.avatar);
    byId.set(
      entry.id,
      Object.freeze({
        id: entry.id,
        username: entry.username,
        displayName,
        serverName: null,
        avatar: avatar || null,
        isFormer: entry.former,
        isHost: isHostEntry(entry),
        isYou: actor !== null && entry.id === actor.id,
        collides:
          typeof daemonCollides === 'boolean'
            ? daemonCollides
            : (collisionCounts.get(displayNameKey(displayName)) ?? 0) > 1,
      })
    );
  });

  const all = [...byId.values()];
  const active = Object.freeze(all.filter((person) => !person.isFormer));
  const former = Object.freeze(all.filter((person) => person.isFormer));
  const me = actor ? (byId.get(actor.id) ?? null) : null;
  const host = all.find((person) => person.isHost && !person.isFormer) ?? null;
  const viewerIsHost =
    actor !== null &&
    (hostPrincipalId !== null
      ? actor.id === hostPrincipalId
      : hostUid !== null && actor.uid === hostUid);

  const lookup = (person: CrewPerson | string | null | undefined): CrewPerson | null => {
    if (typeof person === 'string') return byId.get(person) ?? null;
    if (!person) return null;
    return (person.id !== null ? byId.get(person.id) : undefined) ?? person;
  };

  return {
    byId: (id) => (typeof id === 'string' ? (byId.get(id) ?? null) : null),
    people: active.length ? active : EMPTY,
    formerPeople: former.length ? former : EMPTY,
    me,
    host,
    viewerIsHost,
    collides: (person) => lookup(person)?.collides ?? false,
    isFormer: (person) => lookup(person)?.isFormer ?? false,
    isHost: (person) => lookup(person)?.isHost ?? false,
  };
}

/**
 * The people directory for a snapshot, memoized on its inputs. Pass the
 * observation frame's `labels` when the daemon projects them, and a message
 * result's `people` map when rendering messages whose authors may have left.
 */
export function usePeopleDirectory(
  snapshot: PeopleSnapshotInput | null | undefined,
  labels?: DaemonPersonLabels | null,
  people?: CrewPeopleMap | null
): PeopleDirectory {
  return useMemo(() => buildPeopleDirectory(snapshot, labels, people), [snapshot, labels, people]);
}
