import { CrewHttpError, crewHttp } from '../crewApi';
import { CREW_REVOCATION_UNCONFIRMED, unexpectedCrewResponse } from './errors';
import { isRecord, nullableNumber, nullableText, optionalText, stringArray } from './parse';

// Chat and task grants (RV-R1). The daemon decides whether a grant exists and whether it may be
// revoked; these helpers only read its answer and never gate anything themselves.

/** `chat` is a conversation connected with /crew; `task` is an agent task started from Crew. */
export type CrewGrantKind = 'chat' | 'task';

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
}

export interface CrewRevokeResult {
  revoked: true;
  /** Only ever true: an unconfirmed revoke is thrown as `crew_revocation_unconfirmed`. */
  remote_revocation_confirmed: true;
  session_id?: string;
  run_id?: string;
}

export type CrewGrantState = 'active' | 'expired' | 'revoked';

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
  return grant;
}

/**
 * The grants the daemon holds for one saved connection. A missing or malformed list reads as no
 * grants, and a malformed row is left out, because a row without a session or a connection could
 * only produce a Revoke aimed at nothing.
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
  const rows = isRecord(result) && Array.isArray(result.grants) ? result.grants : [];
  return rows
    .map(grantFrom)
    .filter(
      (grant): grant is CrewSessionGrant => grant !== null && grant.connection_id === connectionId
    );
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
 * How a grant reads at rest. `expired` on the wire means stopped on this device, which the person
 * reads as revoked; a grant past its `expires_at` has run out. `now` is in milliseconds.
 */
export function sessionGrantState(grant: CrewSessionGrant, now = Date.now()): CrewGrantState {
  if (grant.expired) return 'revoked';
  if (typeof grant.expires_at === 'number' && grant.expires_at * 1000 <= now) return 'expired';
  return 'active';
}
