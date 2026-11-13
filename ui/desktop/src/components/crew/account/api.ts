import type * as Api from '../../../api/types.gen';
import { crewHttp, crewRequest } from '../crewApi';
import { outdatedDaemonResponse, unexpectedCrewResponse } from '../api/errors';
import { isRecord, optionalText, wireOf } from '../api/parse';

// Account ownership (docs/crew/design/account-ownership.md): the routes the desktop calls to add
// this computer again, to acknowledge and revoke the account's devices, and to list and revoke the
// chats that read a workspace as the person. Each carries the person's proof through `crewHttp`;
// the daemon and the workspace decide everything.

/** `POST …/rejoin`'s answer: who the workspace admitted, and a device it let go to make room. */
export interface CrewRejoined {
  role: Api.AccountRole;
  /** The oldest device the cap of 32 removed to make room (AO-2), grouped as Keys shows it. */
  evictedFingerprint: string | null;
}

/**
 * Add this computer again (AO-9): the daemon gives the saved connection a new device key in place,
 * keeping its ID, so chats it reached can be granted again. Refused with the daemon's code
 * (`crew_not_connected`, `crew_account_unsupported`, `crew_join_removed`…).
 */
export async function rejoinConnection(connectionId: string): Promise<CrewRejoined> {
  const answer = wireOf<Api.AccountJoined>(
    await crewHttp<unknown>(`/connections/${encodeURIComponent(connectionId)}/rejoin`, 'POST')
  );
  if (!answer) throw outdatedDaemonResponse();
  const role = optionalText(answer.role);
  if (!role) throw unexpectedCrewResponse('a rejoin answer');
  const evicted = isRecord(answer.evicted_device)
    ? optionalText(answer.evicted_device.fingerprint)
    : undefined;
  return { role: role as Api.AccountRole, evictedFingerprint: evicted ?? null };
}

/**
 * The new devices the person has seen (AO-10), by the grouped fingerprints the notice showed: only
 * those stop being reported, so a device listed after the notice was drawn is still said. The
 * route takes exactly this body (`AcknowledgeDevicesRequest`) and refuses a request without one.
 */
export async function acknowledgeNewDevices(
  connectionId: string,
  fingerprints: readonly string[]
): Promise<void> {
  const body: Api.AcknowledgeDevicesRequest = { fingerprints: [...fingerprints] };
  await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/devices/acknowledge`,
    'POST',
    body
  );
}

/**
 * Revoke one of the person's own devices by its grouped fingerprint (AO-5), this computer's
 * included. A person's signed mutation: the workspace refuses a fingerprint that is not one of
 * theirs.
 */
export async function revokeDevice(connectionId: string, fingerprint: string): Promise<void> {
  await crewRequest(connectionId, 'device.revoke', { fingerprint }, true);
}

/** A chat that may read a connection's workspace as the person (AO-12's read consent). */
export interface CrewPersonRead {
  sessionId: string;
  connectionId: string;
  /** Unix seconds. */
  grantedAt: number | null;
  /** The chat's name while it is still the chat that was given the consent. */
  sessionName: string | null;
}

/**
 * The read consents on one saved connection (`GET …/grants`'s `person_reads`). A malformed row is
 * left out: a row without a chat or a connection could only produce a Revoke aimed at nothing.
 */
export async function listPersonReads(
  connectionId: string,
  signal?: AbortSignal
): Promise<CrewPersonRead[]> {
  return personReadsFrom(
    await crewHttp<unknown>(
      `/connections/${encodeURIComponent(connectionId)}/grants`,
      'GET',
      undefined,
      signal
    ),
    connectionId
  );
}

/**
 * The read consents in one `GET …/grants` answer, read as `listPersonReads` reads them: a chat's
 * Crew bar takes them from the same answer it reads the chat's grant from, rather than asking twice.
 */
export function personReadsFrom(answer: unknown, connectionId: string): CrewPersonRead[] {
  const result = wireOf<Api.CrewGrantList>(answer);
  const rows = Array.isArray(result?.person_reads) ? result.person_reads : [];
  return rows.flatMap((row: unknown): CrewPersonRead[] => {
    const read = wireOf<Api.CrewPersonReadView>(row);
    const sessionId = optionalText(read?.session_id);
    const rowConnection = optionalText(read?.connection_id);
    if (!read || !sessionId || rowConnection !== connectionId) return [];
    return [
      {
        sessionId,
        connectionId,
        grantedAt:
          typeof read.granted_at === 'number' && Number.isFinite(read.granted_at)
            ? read.granted_at
            : null,
        sessionName: optionalText(read.session_name) ?? null,
      },
    ];
  });
}

/** Take a chat's read consent back (`DELETE /crew/sessions/{session}/reads/{connection}`). */
export async function revokePersonRead(sessionId: string, connectionId: string): Promise<void> {
  await crewHttp<unknown>(
    `/sessions/${encodeURIComponent(sessionId)}/reads/${encodeURIComponent(connectionId)}`,
    'DELETE'
  );
}
