import { createElement, useMemo, type ReactNode } from 'react';
import type { Channel, CrewConnection, Invitation, PendingJoin, Snapshot, Team } from '../crewApi';
import {
  channelSlug,
  connectionNames,
  connectionServer,
  isMachineIdShaped,
  institutionLabel,
  joinerPerson,
  personFromProjection,
  personLabel,
  personLayout,
  resolvePerson,
  sanitizeDisplayText,
  sanitizeUsername,
  teamName,
  usePeopleDirectory,
  type CrewPerson,
  type DaemonPersonLabels,
  type KnownInstitution,
  type PeopleDirectory,
  type PersonRef,
} from '../identity';
import { liveInvitations } from '../dialogs/people';
import { useJoinContext } from '../onboarding/joinContext';
import { sshUsername } from '../onboarding/joinText';
import { knownInstitutions } from '../pane/presentation';
import { useConfiguredModels } from '../pane/useConfiguredModels';
import type { CrewController } from '../state/types';
import { sidebarCopy } from './copy';

/**
 * What the Crew sidebar draws, derived from the controller. Everything here is pure except
 * {@link useSidebarView}, which memoizes it for the components.
 *
 * ⚠ **Two different sources, on purpose.** The sidebar's *places* (teams, channels, unread
 * counts, invitations, the You row's name) are drawn from the verified snapshot, or — while a
 * refresh re-verifies — from the controller's presentation-only `lastVerified` copy of the same
 * connection, so the sidebar does not blank during a refresh. Its *security state* (the privacy
 * chip, the status word) is read only from verified values: an unverified mode never looks
 * verified. Actions stay disabled while the view is only the last verified copy.
 */

/** The workspace the sidebar draws, and whether it is the live verified one. */
export interface SidebarViewSource {
  snapshot: Snapshot | null;
  labels: DaemonPersonLabels | null;
  /** The snapshot is the verified one (actions allowed), not the last verified copy. */
  verified: boolean;
}

/** The verified snapshot, else the last verified one for the SAME connection, else nothing. */
export function sidebarViewSource(
  crew: Pick<CrewController, 'snapshot' | 'labels' | 'lastVerified' | 'connectionId'> & {
    observedPrivacy: CrewController['observedPrivacy'];
  }
): SidebarViewSource {
  if (crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId) {
    return {
      snapshot: crew.snapshot,
      labels: (crew.labels as DaemonPersonLabels | null) ?? null,
      verified: true,
    };
  }
  if (crew.lastVerified && crew.lastVerified.connectionId === crew.connectionId) {
    return {
      snapshot: crew.lastVerified.snapshot,
      labels: (crew.lastVerified.labels as DaemonPersonLabels | null) ?? null,
      verified: false,
    };
  }
  return { snapshot: null, labels: null, verified: false };
}

/** A name fit to show, or `''` (nothing displayable, or only an ID-shaped string). */
function displayable(raw: unknown): string {
  const text = sanitizeDisplayText(raw);
  return text && !isMachineIdShaped(text) ? text : '';
}

// ---------------------------------------------------------------------------------------------
// A workspace or channel name inside a sentence (Q4-51)
// ---------------------------------------------------------------------------------------------

/** What may touch a name on either side for it to count as the whole name, not part of a word. */
const NAME_EDGE = /[\p{L}\p{N}_-]/u;

/** Punctuation that closes the sentence or clause right after a name, kept on the name's line. */
const TRAILING = /[.,;:!?…)]/u;

/**
 * `text` with every whole occurrence of `names` in a `.crew-sidebar-name` span, so a slug in a
 * sentence never breaks at its hyphen ("…can see chen-" / "lab.", Q4-51). The text itself is
 * unchanged — the spans add no characters — so what a screen reader reads and what a test matches
 * is the sentence as the copy deck writes it.
 *
 * Only a whole occurrence is wrapped: `lab` is kept whole in "can see lab." but not inside
 * "label". The punctuation straight after a name goes in its span, so a name that moves to the
 * next line takes its full stop with it. A string with no name in it comes back as the same string.
 */
export function keepNamesWhole(
  text: string,
  names: readonly (string | null | undefined)[]
): ReactNode {
  const wanted = [...new Set(names.filter((name): name is string => Boolean(name)))].sort(
    (a, b) => b.length - a.length
  );
  if (wanted.length === 0 || !text) return text;
  const parts: ReactNode[] = [];
  let plain = '';
  let index = 0;
  while (index < text.length) {
    const name = wanted.find(
      (candidate) =>
        text.startsWith(candidate, index) &&
        !NAME_EDGE.test(text.charAt(index - 1)) &&
        !NAME_EDGE.test(text.charAt(index + candidate.length))
    );
    if (!name) {
      plain += text.charAt(index);
      index += 1;
      continue;
    }
    if (plain) parts.push(plain);
    plain = '';
    let end = index + name.length;
    while (end < text.length && TRAILING.test(text.charAt(end))) end += 1;
    parts.push(
      createElement(
        'span',
        { key: `${parts.length}`, className: 'crew-sidebar-name', 'data-crew-name': '' },
        text.slice(index, end)
      )
    );
    index = end;
  }
  if (plain) parts.push(plain);
  return parts.length === 1 && typeof parts[0] === 'string' ? parts[0] : parts;
}

/**
 * The switcher's name (`switcher.name`): the workspace's own name (naming S2), else the saved
 * connection's local name (`name — server` only when two saved connections share a name).
 */
export function workspaceTitle(
  snapshot: Pick<Snapshot, 'workspace'> | null,
  connections: readonly Pick<CrewConnection, 'id' | 'name' | 'ssh_target'>[],
  connectionId: string
): string {
  const named = displayable(snapshot?.workspace?.name);
  if (named) return named;
  return connectionNames(connections).get(connectionId) ?? '';
}

// ---------------------------------------------------------------------------------------------
// The server, by the person's own name for it (D-ALIAS)
// ---------------------------------------------------------------------------------------------

/** A saved connection as the connection routes answer it, with the daemon's display label. */
export type LabelledConnection = { ssh_target?: string | null; server_label?: unknown };

/**
 * What to call the connection's server on screen (D-ALIAS): the daemon's `server_label` — the
 * person's own SSH alias for that address when one maps to it, else the host — and, from a daemon
 * that sends none, the host of the saved login. Display only: the raw address stays in Connection
 * settings, and nothing here is ever used to connect.
 */
export function serverLabel(connection: LabelledConnection | null | undefined): string {
  return (
    sanitizeDisplayText(connection?.server_label) ||
    connectionServer(connection ? { id: '', ssh_target: connection.ssh_target } : null)
  );
}

/**
 * The SSH login with its server named the same way: `crew_alice@lab-server` for a saved
 * `crew_alice@52.33.141.141` whose server the person calls `lab-server`. Without a label it is
 * the saved login exactly as saved.
 */
export function loginLabel(connection: LabelledConnection | null | undefined): string {
  const target = sanitizeDisplayText(connection?.ssh_target);
  const label = sanitizeDisplayText(connection?.server_label);
  if (!label) return target;
  const at = target.lastIndexOf('@');
  return at > 0 ? `${target.slice(0, at)}@${label}` : label;
}

/**
 * The person's username on this connection, when anything names it: the verified directory's
 * `me`, else the saved login's user part, else what the join remembered (Q2-43). A joiner the
 * host has not let in yet has no snapshot, and their username is still known.
 */
export function knownUsername(
  me: Pick<CrewPerson, 'username'> | null | undefined,
  connection: Pick<CrewConnection, 'ssh_target'> | null | undefined,
  remembered?: string | null
): string | null {
  return (
    sanitizeDisplayText(me?.username) ||
    sshUsername(connection?.ssh_target) ||
    sanitizeDisplayText(remembered) ||
    null
  );
}

// ---------------------------------------------------------------------------------------------
// A join the host has not let in yet
// ---------------------------------------------------------------------------------------------

/**
 * Who a joiner waits for, as `personLabel(…, 'inline')`: the host the invitation named, which the
 * Join dialog remembers for this connection (and the join screen updates once the workspace names
 * its inviter) — the workspace itself says nothing to a non-member. `null` when nothing named
 * anyone, so each sentence says "your host" instead. `username` is the joiner's own username as
 * the join remembered it, for a login that does not carry one.
 */
export function usePendingHost(crew: Pick<CrewController, 'connectionId'>): {
  host: string | null;
  username: string | null;
} {
  const context = useJoinContext(crew.connectionId);
  const inviter = context.hostUsername
    ? personFromProjection({
        username: context.hostUsername,
        display_name: context.hostDisplayName,
      })
    : null;
  return {
    host: inviter ? personLabel(inviter, 'inline') : null,
    username: sanitizeDisplayText(context.username) || null,
  };
}

// ---------------------------------------------------------------------------------------------
// Privacy
// ---------------------------------------------------------------------------------------------

/**
 * The institutions configured providers publish names for, so `ucsf` reads as `UCSF` wherever a
 * provider publishes that name for it (Q2-38), and as itself everywhere else.
 */
export function useKnownInstitutions(): KnownInstitution[] {
  const { providers } = useConfiguredModels();
  return useMemo(() => knownInstitutions(providers), [providers]);
}

export type PrivacyWhy = 'both' | 'connection' | 'workspace' | 'public';

export interface VerifiedPrivacy {
  /** The effective mode for this person in this workspace. */
  effective: 'private' | 'public';
  /** This connection's verified mode. */
  connectionMode: 'private' | 'public';
  /** The workspace's baseline. */
  workspaceMode: 'private' | 'public';
  /**
   * The institution label in force: the workspace's, else this connection's; `null` when neither
   * has one. Already worded for display (the ID, or a name the model registry publishes for it).
   */
  institution: string | null;
  why: PrivacyWhy;
}

/**
 * The privacy the observer VERIFIED for the selected connection, or `null` while it has not.
 * Never derived from the saved connection record or the last verified copy. `known` words the
 * institution by the name a configured provider publishes for it (Q2-38).
 */
export function verifiedPrivacy(
  crew: Pick<CrewController, 'snapshot' | 'observedPrivacy' | 'connectionId' | 'effectivePrivacy'>,
  known?: readonly KnownInstitution[] | null
): VerifiedPrivacy | null {
  const { snapshot, observedPrivacy, connectionId, effectivePrivacy } = crew;
  if (!snapshot || !observedPrivacy || observedPrivacy.connectionId !== connectionId) return null;
  if (effectivePrivacy !== 'private' && effectivePrivacy !== 'public') return null;
  const connectionMode = observedPrivacy.mode;
  const workspaceMode = snapshot.workspace.mode === 'public' ? 'public' : 'private';
  const why: PrivacyWhy =
    effectivePrivacy === 'public'
      ? 'public'
      : connectionMode === 'private' && workspaceMode === 'private'
        ? 'both'
        : connectionMode === 'private'
          ? 'connection'
          : 'workspace';
  return {
    effective: effectivePrivacy,
    connectionMode,
    workspaceMode,
    institution:
      institutionLabel(snapshot.workspace.institution_id, known) ??
      institutionLabel(observedPrivacy.institutionId, known),
    why,
  };
}

// ---------------------------------------------------------------------------------------------
// Attention: invitations to me and people waiting to join
// ---------------------------------------------------------------------------------------------

export interface InvitationRow {
  id: string;
  /** The team as typed, `#slug` for a channel, or `null` when the broker did not name it. */
  target: string | null;
  inviter: CrewPerson | null;
}

/**
 * The invitations addressed to the viewer, in the broker's order. The broker omits expired ones
 * for the invitee and marks them `expired` for the inviter; this trusts that flag rather than the
 * local clock, which can be skewed against the broker's.
 */
export function invitationsToMe(
  snapshot: Pick<Snapshot, 'actor' | 'invitations'> | null,
  dir: PeopleDirectory
): InvitationRow[] {
  if (!snapshot || !Array.isArray(snapshot.invitations)) return [];
  const me = snapshot.actor?.id;
  return snapshot.invitations
    .filter(
      (invitation): invitation is Invitation =>
        Boolean(invitation) &&
        typeof invitation.id === 'string' &&
        invitation.principal_id === me &&
        invitation.expired !== true
    )
    .map((invitation) => {
      const name = displayable(invitation.target_name);
      const target = !name
        ? null
        : invitation.kind === 'channel'
          ? `#${channelSlug({ id: invitation.target_id, name })}`
          : name;
      const inviter =
        dir.byId(invitation.inviter_id) ??
        personFromProjection(invitation.inviter ? { ...invitation.inviter } : null);
      return { id: invitation.id, target, inviter };
    });
}

export interface WaitingRow {
  username: string;
  serverName: string | null;
  /** When the invitation runs out, in Unix SECONDS as the broker stamps it; null when unsaid. */
  expiresAt: number | null;
  approved: boolean;
  /**
   * The invitation ran out before the person joined (the broker's `expired`, not the local clock,
   * which can be skewed against the broker's). Such a join cannot be let in; it is invited again.
   */
  expired: boolean;
  /** A device with a different code tried to join as this person. */
  otherDeviceTried: boolean;
}

/** People the host invited who have not joined yet (S3a, host snapshot only). */
export function waitingToJoin(snapshot: Pick<Snapshot, 'pending_joins'> | null): WaitingRow[] {
  const pending = snapshot?.pending_joins;
  if (!Array.isArray(pending)) return [];
  return pending
    .filter((join): join is PendingJoin => Boolean(join) && typeof join.username === 'string')
    .map((join) => ({
      username: join.username,
      serverName: typeof join.full_name === 'string' ? join.full_name : null,
      expiresAt:
        typeof join.expires_at === 'number' &&
        Number.isFinite(join.expires_at) &&
        join.expires_at > 0
          ? join.expires_at
          : null,
      approved: join.approved === true,
      expired: join.expired === true,
      otherDeviceTried:
        typeof join.mismatched_attempts === 'number' && join.mismatched_attempts > 0,
    }));
}

/** "Sat 1:41 AM": the day and the time an invitation runs out, in the viewer's own locale. */
export function expiryWhen(expiresAtSeconds: number): string {
  return new Intl.DateTimeFormat(undefined, {
    weekday: 'short',
    hour: 'numeric',
    minute: '2-digit',
  }).format(new Date(expiresAtSeconds * 1000));
}

/**
 * When a pending join's invitation runs out, as the row says it (Q4-36): "expires Sat 1:41 AM",
 * "expired" once that moment has passed on this computer's clock, or `null` when the broker named
 * no expiry. ⚠ `expires_at` is Unix SECONDS (`created_at + PENDING_JOIN_LIFETIME_SECS` in the
 * broker), so it is compared with `nowMs / 1000`, never with `Date.now()` itself. Display only: the
 * broker's own `expired` flag, not this clock, decides whether the join can still be let in.
 */
export function expiryText(
  expiresAtSeconds: number | null,
  nowMs: number = Date.now()
): string | null {
  if (expiresAtSeconds === null) return null;
  return expiresAtSeconds * 1000 <= nowMs
    ? sidebarCopy.waiting.expiredShort
    : sidebarCopy.waiting.expires(expiryWhen(expiresAtSeconds));
}

// ---------------------------------------------------------------------------------------------
// The name on a joiner's server account, kept past the join (Q4-42)
// ---------------------------------------------------------------------------------------------

/**
 * The name on each joiner's server account, by workspace and username, as the host's verified
 * snapshots listed it while they waited. The `pending_joins` row is the only place that name comes
 * from, and it is gone in the very snapshot that shows them joined — so without this, the Let in
 * dialog said "Jack joined wong-lab" while the toast and the "Joined, not in your teams" row said
 * "@crew_jack" about the same event (Q4-42).
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
 * A member who just joined, named once for the event (Q4-42): `person` with the name on their
 * server account standing in for the display name they have not chosen yet, so
 * `PersonName`/`personLabel` read "Jack Moreno (@crew_jack)", as the Let in dialog does. `null`
 * when nothing changes — they chose a name (theirs wins), or their server-account name was never
 * seen. Render the result WITHOUT the directory: looking the person up again would put the
 * directory's copy, and its bare `@crew_jack`, back.
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
  return serverName ? { ...resolved, displayName: serverName } : null;
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

// ---------------------------------------------------------------------------------------------
// People who joined and are in none of the host's teams (Q3-52)
// ---------------------------------------------------------------------------------------------

export interface JoinedRow {
  id: string;
  person: CrewPerson;
}

/**
 * The people a host should add somewhere: active members who are in none of the teams, and none
 * of the channels, the host's snapshot holds, and whom the host has not already invited to one.
 * A joiner the host let in and then walked away from — as the Let in dialog says they can — used
 * to be announced once by a toast and then never again (Q3-52).
 *
 * ⚠ **Only the host's own teams.** The broker's snapshot lists the teams the viewer is a member
 * of, the host included, so "in no team" cannot be known here; "in none of yours" can. The
 * section says exactly that. A channel the host can see counts too: a channel invitation admits
 * someone to a channel without its team (`invitation.accept`), and that person has somewhere to
 * be. A live invitation from the host counts as acted on: the team's header already says
 * "· 1 invited". Display only: membership is the broker's.
 */
export function joinedWithoutTeam(
  snapshot: Pick<
    Snapshot,
    'actor' | 'workspace' | 'principals' | 'teams' | 'channels' | 'invitations'
  > | null,
  dir: PeopleDirectory,
  nowSeconds?: number
): JoinedRow[] {
  if (!snapshot || !Array.isArray(snapshot.principals)) return [];
  const me = snapshot.actor?.id ?? null;
  const host = snapshot.workspace?.host_principal_id ?? null;
  const placed = new Set<string>();
  for (const group of [snapshot.teams, snapshot.channels]) {
    if (!Array.isArray(group)) continue;
    for (const item of group) {
      if (item && Array.isArray(item.members)) item.members.forEach((id) => placed.add(id));
    }
  }
  const invited = new Set(
    liveInvitations(Array.isArray(snapshot.invitations) ? snapshot.invitations : [], nowSeconds)
      .filter((invitation) => me !== null && invitation.inviter_id === me)
      .map((invitation) => invitation.principal_id)
  );
  const rows: JoinedRow[] = [];
  for (const principal of snapshot.principals) {
    if (!principal || typeof principal.id !== 'string') continue;
    if (principal.active === false || principal.id === me || principal.id === host) continue;
    if (placed.has(principal.id) || invited.has(principal.id)) continue;
    const person = dir.byId(principal.id) ?? personFromProjection(principal);
    if (person) rows.push({ id: principal.id, person });
  }
  return rows;
}

// ---------------------------------------------------------------------------------------------
// Teams and channels
// ---------------------------------------------------------------------------------------------

export interface ChannelRowView {
  id: string;
  teamId: string;
  /** The slug, without `#`. */
  name: string;
  unread: number;
  archived: boolean;
}

export interface TeamSectionView {
  id: string;
  name: string;
  channels: ChannelRowView[];
  archived: ChannelRowView[];
}

/** Teams in snapshot order, each with its open channels and its archived ones. */
export function teamSections(
  snapshot: Pick<Snapshot, 'teams' | 'channels' | 'unread'> | null
): TeamSectionView[] {
  if (!snapshot || !Array.isArray(snapshot.teams)) return [];
  const channels = Array.isArray(snapshot.channels) ? snapshot.channels : [];
  return snapshot.teams
    .filter((team): team is Team => Boolean(team) && typeof team.id === 'string')
    .map((team) => {
      const rows = channels
        .filter((channel): channel is Channel => Boolean(channel) && channel.team_id === team.id)
        .map((channel) => ({
          id: channel.id,
          teamId: team.id,
          name: channelSlug(channel),
          unread: unreadCount(snapshot.unread?.[channel.id]),
          archived: channel.archived === true,
        }));
      return {
        id: team.id,
        name: teamName(team),
        channels: rows.filter((row) => !row.archived),
        archived: rows.filter((row) => row.archived),
      };
    });
}

function unreadCount(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? Math.floor(value) : 0;
}

/** The neutral count badge's text: the count, capped at `99+`. */
export function unreadBadgeText(count: number): string {
  return count > sidebarCopy.channel.unreadCap ? sidebarCopy.channel.unreadOverCap : String(count);
}

// ---------------------------------------------------------------------------------------------
// The hook
// ---------------------------------------------------------------------------------------------

export interface SidebarView extends SidebarViewSource {
  dir: PeopleDirectory;
  /** The switcher's name for the selected workspace. */
  title: string;
}

/** The sidebar's view of the controller, memoized. */
export function useSidebarView(crew: CrewController): SidebarView {
  const { snapshot, labels, lastVerified, connectionId, observedPrivacy, connections } = crew;
  const source = useMemo(
    () => sidebarViewSource({ snapshot, labels, lastVerified, connectionId, observedPrivacy }),
    [snapshot, labels, lastVerified, connectionId, observedPrivacy]
  );
  const dir = usePeopleDirectory(source.snapshot, source.labels);
  const title = useMemo(
    () => workspaceTitle(source.snapshot, connections, connectionId),
    [source.snapshot, connections, connectionId]
  );
  return { ...source, dir, title };
}
