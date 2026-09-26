import { CrewHttpError, crewHttp } from '../crewApi';
import { observeListedRevocations } from '../access/pastAccess';
import { CREW_REVOCATION_UNCONFIRMED, unexpectedCrewResponse } from './errors';
import { isRecord, nullableNumber, nullableText, optionalText, stringArray } from './parse';

// Chat and task grants (RV-R1). The daemon decides whether a grant exists and whether it may be
// revoked; these helpers only read its answer and never gate anything themselves.

/** `chat` is a conversation connected with /crew; `task` is an agent task started from Crew. */
export type CrewGrantKind = 'chat' | 'task';

/** One channel's display label, as the person saw it when granting access. */
export interface CrewGrantChannelLabel {
  channel_id?: string;
  /** `#methods`. */
  label?: string;
  /** The channel's team, when the snapshot showed it. */
  team?: string;
}

/**
 * The names of a grant's IDs, captured from the person's own snapshot when they granted access
 * (D14) and never refreshed afterwards. Display text only: every field is optional, and one that
 * is not a string is dropped rather than shown.
 */
export interface CrewGrantLabels {
  /** The workspace's name, or this device's name for the connection when it has none. */
  workspace?: string;
  /** The channel the session posts in. */
  destination?: CrewGrantChannelLabel;
  /** Every channel the session may read, the destination included. */
  sources?: CrewGrantChannelLabel[];
}

export interface CrewSessionGrant {
  session_id: string;
  run_id: string;
  connection_id: string;
  /** The channel the session may post in. */
  channel_id: string;
  /** Further channels it may read. */
  source_channels: string[];
  policy_epoch: number;
  /** Stopped on this device: revoked, or its connection was removed. */
  expired: boolean;
  /** RV-D2; absent from a daemon that predates it. */
  kind?: CrewGrantKind;
  /** The conversation's title (RV-D2); null when the daemon could not find the session. */
  session_name?: string | null;
  /** When the workspace ends the grant, in Unix seconds (RV-D2). */
  expires_at?: number | null;
  /** The names the person saw when granting; absent when the daemon recorded none. */
  labels?: CrewGrantLabels;
  /**
   * Where a stopped grant stands with the workspace (F3, D-1): `unconfirmed` while the daemon is
   * still asking the workspace to confirm a revoke (it asks again by itself whenever the
   * connection comes back), `confirmed` once it has, `ended_by_workspace` when the workspace itself
   * refused the run as ended (its policy moved since the grant). Absent from a live grant, from a
   * daemon that predates it, and for a stop whose standing the daemon does not know.
   */
  revocation?: CrewGrantRevocation;
  /**
   * When this device saw the revoke confirmed, in milliseconds: set only on the rows "Show past
   * access" remembers (`pastAccess.ts`), never by the daemon. Display only (F5).
   */
  revoked_at?: number;
}

/** A stopped grant's standing with the workspace, as the daemon's grant list says it. */
export type CrewGrantRevocation = 'unconfirmed' | 'confirmed' | 'ended_by_workspace';

const GRANT_REVOCATIONS: readonly string[] = ['unconfirmed', 'confirmed', 'ended_by_workspace'];

export interface CrewRevokeResult {
  revoked: true;
  /** Only ever true: an unconfirmed revoke is thrown as `crew_revocation_unconfirmed`. */
  remote_revocation_confirmed: true;
  session_id?: string;
  run_id?: string;
}

export type CrewGrantState = 'active' | 'expired' | 'revoked';

function channelLabelFrom(value: unknown): CrewGrantChannelLabel | undefined {
  if (!isRecord(value)) return undefined;
  const label: CrewGrantChannelLabel = {};
  const channelId = optionalText(value.channel_id);
  if (channelId) label.channel_id = channelId;
  const name = optionalText(value.label);
  if (name) label.label = name;
  const team = optionalText(value.team);
  if (team) label.team = team;
  return Object.keys(label).length ? label : undefined;
}

/** The grant's `labels`, keeping only the fields that are what they claim to be. */
export function grantLabelsFrom(value: unknown): CrewGrantLabels | undefined {
  if (!isRecord(value)) return undefined;
  const labels: CrewGrantLabels = {};
  const workspace = optionalText(value.workspace);
  if (workspace) labels.workspace = workspace;
  const destination = channelLabelFrom(value.destination);
  if (destination) labels.destination = destination;
  if (Array.isArray(value.sources)) {
    const sources = value.sources.flatMap((source) => {
      const label = channelLabelFrom(source);
      return label ? [label] : [];
    });
    if (sources.length) labels.sources = sources;
  }
  return Object.keys(labels).length ? labels : undefined;
}

function grantFrom(row: unknown): CrewSessionGrant | null {
  if (!isRecord(row)) return null;
  const session_id = optionalText(row.session_id);
  const run_id = optionalText(row.run_id);
  const connection_id = optionalText(row.connection_id);
  const channel_id = optionalText(row.channel_id);
  const source_channels = row.source_channels === undefined ? [] : stringArray(row.source_channels);
  if (
    !session_id ||
    !run_id ||
    !connection_id ||
    !channel_id ||
    !source_channels ||
    typeof row.expired !== 'boolean' ||
    typeof row.policy_epoch !== 'number' ||
    !Number.isSafeInteger(row.policy_epoch)
  )
    return null;
  const grant: CrewSessionGrant = {
    session_id,
    run_id,
    connection_id,
    channel_id,
    source_channels,
    policy_epoch: row.policy_epoch,
    expired: row.expired,
  };
  if (row.kind === 'chat' || row.kind === 'task') grant.kind = row.kind;
  const sessionName = nullableText(row.session_name);
  if (sessionName !== undefined) grant.session_name = sessionName;
  const expiresAt = nullableNumber(row.expires_at);
  if (expiresAt !== undefined) grant.expires_at = expiresAt;
  const labels = grantLabelsFrom(row.labels);
  if (labels) grant.labels = labels;
  // Only a stopped grant has a standing with the workspace; a word this renderer does not know is
  // left out rather than guessed at.
  // `remote_revocation_confirmed` says the same for the two states it covers.
  if (row.expired === true) {
    if (typeof row.revocation === 'string' && GRANT_REVOCATIONS.includes(row.revocation))
      grant.revocation = row.revocation as CrewGrantRevocation;
    else if (typeof row.remote_revocation_confirmed === 'boolean')
      grant.revocation = row.remote_revocation_confirmed ? 'confirmed' : 'unconfirmed';
  }
  return grant;
}

function rowsOf(result: unknown, list: 'grants' | 'replaced_grants', connectionId: string) {
  const rows = isRecord(result) && Array.isArray(result[list]) ? result[list] : [];
  return rows
    .map(grantFrom)
    .filter(
      (grant): grant is CrewSessionGrant => grant !== null && grant.connection_id === connectionId
    );
}

/**
 * The grants the daemon holds for one saved connection. A missing or malformed list reads as no
 * grants, and a malformed row is left out, because a row without a session or a connection could
 * only produce a Revoke aimed at nothing.
 *
 * Every answer is also shown to "Show past access" (`observeListedRevocations`), with the earlier
 * stops the daemon still asks the workspace about (`replaced_grants`), so a revoke the daemon
 * confirmed by itself after a 503 is dated when this window first reads it confirmed (NEW-3).
 * Display only: what this resolves with is `grants`, exactly as before.
 */
export async function listSessionGrants(
  connectionId: string,
  signal?: AbortSignal
): Promise<CrewSessionGrant[]> {
  const result = await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/grants`,
    'GET',
    undefined,
    signal
  );
  const grants = rowsOf(result, 'grants', connectionId);
  if (!signal?.aborted)
    observeListedRevocations(connectionId, grants, rowsOf(result, 'replaced_grants', connectionId));
  return grants;
}

/**
 * The grant held for one conversation, looked up across every saved connection (a session has at
 * most one). Lists that fail are tolerated only when another connection holds the grant: without a
 * match, a failure is rethrown, because "no grant" would be a guess.
 */
export async function findSessionGrant(
  connectionIds: string[],
  sessionId: string,
  signal?: AbortSignal
): Promise<CrewSessionGrant | null> {
  const ids = [...new Set(connectionIds)];
  const listed = await Promise.allSettled(ids.map((id) => listSessionGrants(id, signal)));
  for (const outcome of listed) {
    if (outcome.status !== 'fulfilled') continue;
    const grant = outcome.value.find((candidate) => candidate.session_id === sessionId);
    if (grant) return grant;
  }
  const failure = listed.find(
    (outcome): outcome is PromiseRejectedResult => outcome.status === 'rejected'
  );
  if (failure) throw failure.reason;
  return null;
}

/**
 * Revoke a conversation's or task's Crew grant. The request has no body, so it carries no
 * `Content-Type`. It resolves only when the daemon confirmed the revoke; everything else throws a
 * `CrewHttpError`, so a caller can show success exactly when this resolves:
 *
 * - 503 `crew_revocation_unconfirmed`: stopped on this device, not yet confirmed by the workspace;
 * - any other status: not revoked, with the daemon's own message.
 */
export async function revokeSessionGrant(
  connectionId: string,
  sessionId: string
): Promise<CrewRevokeResult> {
  const result = await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/sessions/${encodeURIComponent(sessionId)}/revoke`,
    'POST'
  );
  if (!isRecord(result) || result.revoked !== true) throw unexpectedCrewResponse('a revoke answer');
  // RV-D1 answers 503 when the workspace did not confirm. A 2xx that says so anyway is still not a
  // confirmed revoke. A daemon from before RV-D1 omits the flag; it revoked remotely before
  // answering 200, so its success is confirmed.
  if (result.remote_revocation_confirmed === false)
    throw new CrewHttpError(
      optionalText(result.error) ??
        'Stopped on this device. The workspace has not confirmed yet; reconnect and retry.',
      200,
      CREW_REVOCATION_UNCONFIRMED
    );
  const revoked: CrewRevokeResult = { revoked: true, remote_revocation_confirmed: true };
  const session = optionalText(result.session_id);
  if (session) revoked.session_id = session;
  const run = optionalText(result.run_id);
  if (run) revoked.run_id = run;
  return revoked;
}

/**
 * The destination's name as the person saw it when granting (`#methods`), or null. A label that
 * names a different channel than the grant posts in is not this grant's, so it is not used.
 */
export function grantDestinationLabel(
  grant: Pick<CrewSessionGrant, 'channel_id' | 'labels'>
): string | null {
  const destination = grant.labels?.destination;
  if (!destination?.label) return null;
  if (destination.channel_id !== undefined && destination.channel_id !== grant.channel_id)
    return null;
  return destination.label;
}

/**
 * How a grant reads at rest. `expired` on the wire means stopped on this device, which the person
 * reads as revoked — unless the workspace itself ended the run (`ended_by_workspace`, D-1: its
 * policy moved since the grant), which nobody revoked, so it reads as expired. A grant past its
 * `expires_at` has run out. `now` is in milliseconds.
 */
export function sessionGrantState(grant: CrewSessionGrant, now = Date.now()): CrewGrantState {
  if (grant.expired) return grant.revocation === 'ended_by_workspace' ? 'expired' : 'revoked';
  if (typeof grant.expires_at === 'number' && grant.expires_at * 1000 <= now) return 'expired';
  return 'active';
}
