import { identityCopy } from './copy';
import { cleanName, isMachineIdShaped, nameKey } from './nameKey';
import { sanitizeDisplayText } from './displayText';
import type { CrewPerson } from './types';

/**
 * How workspaces, teams, channels and saved connections are named on screen
 * (ui-redesign-spec, "Identity and naming display rules", rule 7).
 *
 * - **Teams** render as typed, never upper-cased.
 * - **Channels** render `#slug`; in a list that spans teams, `{team} / #slug`
 *   only where two teams share a slug.
 * - **Workspaces** render their name, else "{host display name}'s workspace"
 *   for a legacy unnamed one.
 * - **Saved connections** render their local name, as `name — server` only when
 *   two saved connections share a name.
 *
 * None of these ever falls back to an ID. A legacy name that has nothing
 * displayable left, or that reads as a UUID or a 64-hex digest, is shown as
 * "Untitled team", `#untitled` or "Unnamed workspace" instead; the ID stays in
 * the object's "Copy … ID" menu item.
 */

/** A team as the snapshot projects it. `display_name` is the broker's sanitized name (S2). */
export interface TeamNameInput {
  id: string;
  name?: string | null;
  display_name?: string | null;
}

/** A channel as the snapshot projects it. */
export interface ChannelNameInput {
  id: string;
  team_id?: string | null;
  name?: string | null;
  display_name?: string | null;
}

/** The fields of a saved connection its label reads. */
export interface ConnectionNameInput {
  id: string;
  name?: string | null;
  ssh_target?: string | null;
}

/** A name fit to display, or `''` when nothing displayable (or only an ID-shaped string) is left. */
function displayable(raw: unknown): string {
  const text = sanitizeDisplayText(raw);
  return text && !isMachineIdShaped(text) ? text : '';
}

function preferredName(
  object: { name?: string | null; display_name?: string | null } | null | undefined
) {
  if (!object) return '';
  return displayable(object.display_name) || displayable(object.name);
}

/** A team's name, as typed. */
export function teamName(team: TeamNameInput | null | undefined): string {
  return preferredName(team) || identityCopy.untitledTeam;
}

/** A channel's slug, without the `#`. A leading `#` in a stored name is not doubled. */
export function channelSlug(channel: ChannelNameInput | null | undefined): string {
  const name = preferredName(channel).replace(/^#+/, '').trim();
  return name || identityCopy.untitledChannel;
}

/** A channel as `#slug`. */
export function channelName(channel: ChannelNameInput | null | undefined): string {
  return `#${channelSlug(channel)}`;
}

/**
 * Labels for a list of channels that may span teams (Agents, Also read): each is
 * `#slug`, except that a slug two or more teams share is qualified as
 * `{team} / #slug` in every team that has it. Slugs are compared by `nameKey`,
 * so `#Methods` and `#methods` count as the same slug. Keyed by channel ID.
 */
export function channelNamesAcrossTeams(
  channels: readonly ChannelNameInput[],
  teams: readonly TeamNameInput[]
): Map<string, string> {
  const teamsById = new Map<string, TeamNameInput>();
  teams.forEach((team) => {
    if (team && typeof team.id === 'string') teamsById.set(team.id, team);
  });

  const teamsBySlug = new Map<string, Set<string>>();
  channels.forEach((channel) => {
    if (!channel) return;
    const key = nameKey(channelSlug(channel));
    const set = teamsBySlug.get(key) ?? new Set<string>();
    set.add(channel.team_id ?? '');
    teamsBySlug.set(key, set);
  });

  const labels = new Map<string, string>();
  channels.forEach((channel) => {
    if (!channel || typeof channel.id !== 'string') return;
    const name = channelName(channel);
    const shared = (teamsBySlug.get(nameKey(channelSlug(channel)))?.size ?? 0) > 1;
    const team = channel.team_id ? teamsById.get(channel.team_id) : undefined;
    labels.set(channel.id, shared ? `${teamName(team)} / ${name}` : name);
  });
  return labels;
}

/**
 * A workspace's name: the name its host gave it (S2), else "{host}'s workspace"
 * from the host's display name, else "Unnamed workspace".
 */
export function workspaceName(
  workspace: { name?: string | null } | null | undefined,
  host?: CrewPerson | null
): string {
  const name = displayable(workspace?.name);
  if (name) return name;
  const hostName = displayable(host?.displayName) || displayable(host?.username);
  return hostName ? identityCopy.hostsWorkspace(hostName) : identityCopy.unnamedWorkspace;
}

/**
 * The server a saved connection reaches, for telling two same-named connections
 * apart: the SSH target without its `user@` part.
 */
export function connectionServer(connection: ConnectionNameInput | null | undefined): string {
  const target = sanitizeDisplayText(connection?.ssh_target);
  const at = target.lastIndexOf('@');
  return at >= 0 ? target.slice(at + 1) : target;
}

/**
 * Labels for the saved connections, keyed by ID: each connection's local name,
 * as `name — server` only where two connections share a name (compared by
 * `nameKey`). A connection with no usable name is labelled by its server.
 */
export function connectionNames(connections: readonly ConnectionNameInput[]): Map<string, string> {
  const nameOf = (connection: ConnectionNameInput) =>
    displayable(connection.name) || connectionServer(connection) || identityCopy.unnamedWorkspace;

  const counts = new Map<string, number>();
  connections.forEach((connection) => {
    if (!connection) return;
    const key = nameKey(cleanName(nameOf(connection)));
    counts.set(key, (counts.get(key) ?? 0) + 1);
  });

  const labels = new Map<string, string>();
  connections.forEach((connection) => {
    if (!connection || typeof connection.id !== 'string') return;
    const name = nameOf(connection);
    const server = connectionServer(connection);
    const shared = (counts.get(nameKey(cleanName(name))) ?? 0) > 1;
    labels.set(connection.id, shared && server && server !== name ? `${name} — ${server}` : name);
  });
  return labels;
}
