import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { client } from '../../../api/client.gen';
import { CrewHttpError, crewHttp, observeCrew } from '../crewApi';
import {
  CREW_BRIDGE_MISSING,
  CREW_CONNECT_FAILURE_CODES,
  CREW_DAEMON_OUTDATED,
  CREW_GRANT_NOT_FOUND,
  CREW_HANDOFF_FAILED,
  CREW_REVOCATION_UNCONFIRMED,
  CREW_SSH_AUTH_REQUIRED,
  CREW_SSH_FAILED,
  CREW_SSH_HOST_KEY_CHANGED,
  CREW_SSH_HOST_KEY_UNKNOWN,
  CREW_SSH_UNREACHABLE,
  CREW_UNEXPECTED_RESPONSE,
  CREW_USER_ACTION_REQUIRED,
  CREW_WORKSPACE_IDENTITY_MISMATCH,
  STALE_DAEMON_MESSAGE,
  crewErrorCode,
  crewErrorDetail,
  isConnectFailureCode,
  isRevocationUnconfirmed,
  isStaleDaemon,
  outdatedDaemonResponse,
  unexpectedCrewResponse,
} from './errors';

vi.mock('../../../utils/userAction', () => ({
  userActionHeaders: async () => ({ 'X-User-Action': 'errors-test-proof' }),
}));

async function failureOf(promise: Promise<unknown>): Promise<CrewHttpError> {
  const failure: unknown = await promise.then(
    () => undefined,
    (error: unknown) => error
  );
  if (!(failure instanceof CrewHttpError))
    throw new Error(`expected a CrewHttpError, got ${String(failure)}`);
  return failure;
}

function jsonResponse(status: number, body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { 'content-type': 'application/json' },
  });
}

describe('CrewHttpError capture', () => {
  beforeEach(() => {
    vi.restoreAllMocks();
    client.setConfig({ baseUrl: 'http://crew-errors.test', headers: {} });
    Object.defineProperty(window, 'electron', {
      configurable: true,
      writable: true,
      value: { getSecretKey: vi.fn().mockResolvedValue('errors-secret') },
    });
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('keeps the daemon message, code and detail of a refusal', async () => {
    const detail = 'Host key verification failed.\nSHA256:offered';
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse(400, {
          code: CREW_SSH_HOST_KEY_UNKNOWN,
          error: 'Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]',
          detail,
        })
      )
    );

    const failure = await failureOf(crewHttp('/connections/conn-1/connect', 'POST'));

    expect(failure.message).toBe('Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]');
    expect(failure.status).toBe(400);
    expect(failure.code).toBe(CREW_SSH_HOST_KEY_UNKNOWN);
    expect(failure.detail).toBe(detail);
    expect(crewErrorCode(failure)).toBe(CREW_SSH_HOST_KEY_UNKNOWN);
    expect(crewErrorDetail(failure)).toBe(detail);
  });

  it('falls back to the status when the body is not a typed refusal', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(jsonResponse(502, { error: 7, code: ['x'], detail: { x: 1 } }))
    );

    const failure = await failureOf(crewHttp('/connections'));

    expect(failure.message).toBe('Crew request failed (502)');
    expect(failure.code).toBeUndefined();
    expect(failure.detail).toBeUndefined();
  });

  it('reads a body that is not JSON as an uncoded failure', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('', { status: 404 })));

    const failure = await failureOf(crewHttp('/resolve', 'POST', { selectors: [] }));

    expect(failure.status).toBe(404);
    expect(failure.code).toBeUndefined();
    expect(isStaleDaemon(failure)).toBe(true);
  });

  it('captures the code and detail of a refused observation', async () => {
    vi.stubGlobal(
      'fetch',
      vi.fn().mockResolvedValue(
        jsonResponse(403, {
          code: CREW_USER_ACTION_REQUIRED,
          error: 'Authorize this action in the Crew panel',
          detail: 'no proof',
        })
      )
    );

    const failure = await failureOf(
      observeCrew('conn-1', undefined, null, new AbortController().signal, vi.fn())
    );

    expect(failure.status).toBe(403);
    expect(failure.code).toBe(CREW_USER_ACTION_REQUIRED);
    expect(failure.detail).toBe('no proof');
  });

  it('does not invent a code for an observation refused without one', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(jsonResponse(500, { code: 12 })));

    const failure = await failureOf(
      observeCrew('conn-1', undefined, null, new AbortController().signal, vi.fn())
    );

    expect(failure.message).toBe('Crew observer failed (500).');
    expect(failure.code).toBeUndefined();
  });
});

describe('isStaleDaemon', () => {
  it('recognizes a missing route: 404 or 405 with no code', () => {
    expect(isStaleDaemon(new CrewHttpError('Crew request failed (404)', 404))).toBe(true);
    expect(isStaleDaemon(new CrewHttpError('Crew request failed (405)', 405))).toBe(true);
    expect(isStaleDaemon(outdatedDaemonResponse())).toBe(true);
    expect(outdatedDaemonResponse().message).toBe(STALE_DAEMON_MESSAGE);
    expect(outdatedDaemonResponse().code).toBe(CREW_DAEMON_OUTDATED);
  });

  it('reads a coded 404 as the daemon answering, not as a missing route', () => {
    expect(isStaleDaemon(new CrewHttpError('No grant', 404, CREW_GRANT_NOT_FOUND))).toBe(false);
  });

  it('never reads other failures as a stale daemon', () => {
    expect(isStaleDaemon(new CrewHttpError('Crew request failed (400)', 400))).toBe(false);
    expect(isStaleDaemon(new CrewHttpError('Crew request failed (500)', 500))).toBe(false);
    expect(isStaleDaemon(unexpectedCrewResponse('a join status'))).toBe(false);
    expect(isStaleDaemon(new Error('Crew request failed (404)'))).toBe(false);
    expect(isStaleDaemon({ status: 404 })).toBe(false);
    expect(isStaleDaemon(undefined)).toBe(false);
  });
});

describe('error codes', () => {
  it('lists every connect failure code the spec classifies', () => {
    expect([...CREW_CONNECT_FAILURE_CODES].sort()).toEqual(
      [
        'crew_bridge_missing',
        'crew_handoff_failed',
        'crew_ssh_auth_required',
        'crew_ssh_failed',
        'crew_ssh_host_key_changed',
        'crew_ssh_host_key_unknown',
        'crew_ssh_unreachable',
        'crew_workspace_identity_mismatch',
      ].sort()
    );
    for (const code of [
      CREW_SSH_AUTH_REQUIRED,
      CREW_SSH_HOST_KEY_UNKNOWN,
      CREW_SSH_HOST_KEY_CHANGED,
      CREW_SSH_UNREACHABLE,
      CREW_SSH_FAILED,
      CREW_BRIDGE_MISSING,
      CREW_HANDOFF_FAILED,
      CREW_WORKSPACE_IDENTITY_MISMATCH,
    ])
      expect(isConnectFailureCode(code)).toBe(true);
    expect(isConnectFailureCode(CREW_REVOCATION_UNCONFIRMED)).toBe(false);
    expect(isConnectFailureCode(undefined)).toBe(false);
  });

  it('matches the daemon spelling of the revoke and person codes', () => {
    expect(CREW_REVOCATION_UNCONFIRMED).toBe('crew_revocation_unconfirmed');
    expect(CREW_GRANT_NOT_FOUND).toBe('crew_grant_not_found');
    expect(CREW_USER_ACTION_REQUIRED).toBe('crew_user_action_required');
  });

  it('reads codes and details only from Crew failures', () => {
    const failure = new CrewHttpError('x', 503, CREW_REVOCATION_UNCONFIRMED, 'why');
    expect(isRevocationUnconfirmed(failure)).toBe(true);
    expect(crewErrorCode(new Error('x'))).toBeUndefined();
    expect(crewErrorDetail('x')).toBeUndefined();
    expect(unexpectedCrewResponse('an invitation').code).toBe(CREW_UNEXPECTED_RESPONSE);
    expect(isRevocationUnconfirmed(unexpectedCrewResponse('a revoke answer'))).toBe(false);
  });
});
