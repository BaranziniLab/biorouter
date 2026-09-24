import { useMemo } from 'react';
import type { Channel, CrewConnection, Invitation, PendingJoin, Snapshot, Team } from '../crewApi';
import {
  channelSlug,
  connectionNames,
  isMachineIdShaped,
  institutionLabel,
  personFromProjection,
  sanitizeDisplayText,
  teamName,
  usePeopleDirectory,
  type CrewPerson,
  type DaemonPersonLabels,
  type PeopleDirectory,
} from '../identity';
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
// Privacy
// ---------------------------------------------------------------------------------------------

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
 * Never derived from the saved connection record or the last verified copy.
 */
export function verifiedPrivacy(
  crew: Pick<CrewController, 'snapshot' | 'observedPrivacy' | 'connectionId' | 'effectivePrivacy'>
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
      institutionLabel(snapshot.workspace.institution_id) ??
      institutionLabel(observedPrivacy.institutionId),
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
  approved: boolean;
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
      approved: join.approved === true,
      otherDeviceTried:
        typeof join.mismatched_attempts === 'number' && join.mismatched_attempts > 0,
    }));
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
