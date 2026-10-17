import type { Channel, Snapshot, Team } from '../crewApi';

/**
 * Who may manage what in a workspace, as far as the verified view says (design OW-1, OW-2, OW-4,
 * OW-8), and the broker `hello` capabilities those controls wait for.
 *
 * Display only, like every gate in Crew: these decide which controls to offer, so nobody is shown
 * an action the broker would refuse them. The broker still decides each request, and refuses a
 * worker credential every one of them.
 *
 * A workspace's governance is the broker's own projection, `workspace.governance`. A broker before
 * `open_governance_v1` sends nothing, and nothing means `managed`: today's owner-only rules.
 */

/** `workspace.set_governance`, `team.join`, `channel.join` and the open rules (OW-1, OW-2). */
export const OPEN_GOVERNANCE_CAPABILITY = 'open_governance_v1';
/** `channel.delete`, `team.delete` and `workspace.delete` (OW-4, OW-5). */
export const DELETE_CAPABILITY = 'delete_v1';
/** `workspace.context.get` and `workspace.context.set` (OW-8). */
export const WORKSPACE_CONTEXT_CAPABILITY = 'workspace_context_v1';
/** A task may read up to {@link WIDE_RUN_CHANNELS} channels instead of {@link NARROW_RUN_CHANNELS}. */
export const WIDE_RUNS_CAPABILITY = 'wide_runs_v1';

/** How many source channels a task may read on a broker that advertises `wide_runs_v1`. */
export const WIDE_RUN_CHANNELS = 100;
/** The same on an older broker. */
export const NARROW_RUN_CHANNELS = 20;

/** The largest context document the broker keeps (OW-8): 64 KiB of UTF-8. */
export const CONTEXT_MAX_BYTES = 64 * 1024;

/** Whether the connected broker's signed `hello` names `capability`. Unknown reads as no. */
export function brokerSupports(
  capabilities: readonly string[] | null | undefined,
  capability: string
): boolean {
  return Array.isArray(capabilities) && capabilities.includes(capability);
}

/** How many channels a task may read on this broker, its own channel included. */
export function runChannelLimit(capabilities: readonly string[] | null | undefined): number {
  return brokerSupports(capabilities, WIDE_RUNS_CAPABILITY)
    ? WIDE_RUN_CHANNELS
    : NARROW_RUN_CHANNELS;
}

/** Whether the workspace is open: every member manages its teams and channels (OW-2). */
export function isOpenWorkspace(snapshot: Snapshot | null | undefined): boolean {
  return snapshot?.workspace?.governance === 'open';
}

function viewerId(snapshot: Snapshot | null | undefined): string | null {
  const id = snapshot?.actor?.id;
  return typeof id === 'string' && id ? id : null;
}

/** Whether `principalId` is an active member the snapshot still names. */
function activePrincipal(snapshot: Snapshot, principalId: string | undefined): boolean {
  return (snapshot.principals ?? []).some(
    (person) => person.id === principalId && person.active !== false
  );
}

/**
 * Who may rename a team, add people to it and invite to it (OW-2 R1, R4, R7): its creator while a
 * member; in an open workspace any of its members.
 */
export function managesTeam(snapshot: Snapshot | null | undefined, team: Team | null): boolean {
  const me = viewerId(snapshot);
  if (!snapshot || !team || !me || !team.members.includes(me)) return false;
  return team.created_by === me || isOpenWorkspace(snapshot);
}

/**
 * Who may rename and archive a channel and add people to it (OW-2 R2, R3, R5, R8): its owner; in
 * an open workspace any of its members. Transferring ownership stays the owner's in either.
 */
export function managesChannel(
  snapshot: Snapshot | null | undefined,
  channel: Channel | null
): boolean {
  const me = viewerId(snapshot);
  if (!snapshot || !channel || !me) return false;
  if (channel.owner_id === me) return true;
  return isOpenWorkspace(snapshot) && channel.members.includes(me);
}

/**
 * Whether to offer Delete channel… (OW-4): never for a team's `#general`, which goes only with its
 * team; in a managed workspace its owner, or the host for a channel whose owner left the
 * workspace; in an open one any of its members. Archived channels may be deleted too.
 */
export function mayDeleteChannel(
  snapshot: Snapshot | null | undefined,
  channel: Channel | null,
  options: { capabilities?: readonly string[] | null; viewerIsHost: boolean }
): boolean {
  const me = viewerId(snapshot);
  if (!snapshot || !channel || !me) return false;
  if (!brokerSupports(options.capabilities, DELETE_CAPABILITY)) return false;
  const team = snapshot.teams.find((item) => item.id === channel.team_id);
  if (team?.general_channel_id === channel.id) return false;
  if (isOpenWorkspace(snapshot)) return channel.members.includes(me);
  if (channel.owner_id === me) return true;
  return options.viewerIsHost && !activePrincipal(snapshot, channel.owner_id);
}

/**
 * Whether to offer Delete team… (OW-4): in a managed workspace its creator, or the host when the
 * creator left; in an open one any of its members.
 */
export function mayDeleteTeam(
  snapshot: Snapshot | null | undefined,
  team: Team | null,
  options: { capabilities?: readonly string[] | null; viewerIsHost: boolean }
): boolean {
  const me = viewerId(snapshot);
  if (!snapshot || !team || !me) return false;
  if (!brokerSupports(options.capabilities, DELETE_CAPABILITY)) return false;
  if (isOpenWorkspace(snapshot)) return team.members.includes(me);
  if (team.created_by === me) return true;
  return options.viewerIsHost && !activePrincipal(snapshot, team.created_by);
}

/** Whether the person may edit the context document (OW-8): anyone open, the host managed. */
export function mayEditContext(
  snapshot: Snapshot | null | undefined,
  options: { capabilities?: readonly string[] | null; viewerIsHost: boolean }
): boolean {
  if (!snapshot || !brokerSupports(options.capabilities, WORKSPACE_CONTEXT_CAPABILITY))
    return false;
  return isOpenWorkspace(snapshot) || options.viewerIsHost;
}

/** Whether Browse channels has anything to offer: an open workspace with something to join. */
export function hasJoinable(snapshot: Snapshot | null | undefined, teamId?: string): boolean {
  if (!isOpenWorkspace(snapshot)) return false;
  const channels = (snapshot?.joinable_channels ?? []).filter(
    (channel) => teamId === undefined || channel.team_id === teamId
  );
  if (channels.length > 0) return true;
  return teamId === undefined && (snapshot?.joinable_teams ?? []).length > 0;
}
