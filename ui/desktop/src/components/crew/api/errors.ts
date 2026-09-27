import { CrewHttpError } from '../crewApi';

// Typed error codes the daemon's Crew routes answer with. The codes are the contract; the message
// text beside them is the daemon's and may change. Components map a code to their own copy.

/** Connect: the server wants a password or a verification code (open Sign in). */
export const CREW_SSH_AUTH_REQUIRED = 'crew_ssh_auth_required';
/** Connect: the server's host key is not in the person's known-hosts file. */
export const CREW_SSH_HOST_KEY_UNKNOWN = 'crew_ssh_host_key_unknown';
/** Connect: the server's host key changed. Never offer to accept it. */
export const CREW_SSH_HOST_KEY_CHANGED = 'crew_ssh_host_key_changed';
/** Connect: the server could not be resolved or reached. */
export const CREW_SSH_UNREACHABLE = 'crew_ssh_unreachable';
/** Connect: any other SSH failure; show the daemon's text. */
export const CREW_SSH_FAILED = 'crew_ssh_failed';
/** Connect: `biorouter-crew` is not installed for this account on the server. */
export const CREW_BRIDGE_MISSING = 'crew_bridge_missing';
/** Sign-in succeeded but the bridge did not start (treated like a missing bridge). */
export const CREW_HANDOFF_FAILED = 'crew_handoff_failed';
/** Connect: the server answered with a different workspace key than the one pinned. */
export const CREW_WORKSPACE_IDENTITY_MISMATCH = 'crew_workspace_identity_mismatch';

/** Every code the connect route (and the sign-in handoff) can classify a failure as. */
export const CREW_CONNECT_FAILURE_CODES = [
  CREW_SSH_AUTH_REQUIRED,
  CREW_SSH_HOST_KEY_UNKNOWN,
  CREW_SSH_HOST_KEY_CHANGED,
  CREW_SSH_UNREACHABLE,
  CREW_SSH_FAILED,
  CREW_BRIDGE_MISSING,
  CREW_HANDOFF_FAILED,
  CREW_WORKSPACE_IDENTITY_MISMATCH,
] as const;
export type CrewConnectFailureCode = (typeof CREW_CONNECT_FAILURE_CODES)[number];

/** Revoke (503): stopped on this device, but the workspace has not confirmed it yet. */
export const CREW_REVOCATION_UNCONFIRMED = 'crew_revocation_unconfirmed';
/** Revoke (404): this chat has no Crew grant. */
export const CREW_GRANT_NOT_FOUND = 'crew_grant_not_found';
/** Revoke (409): the grant belongs to a different saved connection. */
export const CREW_GRANT_OTHER_CONNECTION = 'crew_grant_other_connection';
/** Any person-gated route (403): the request carried no proof that a person asked for it. */
export const CREW_USER_ACTION_REQUIRED = 'crew_user_action_required';
/** Any person-gated route (403): this daemon was started without a way to verify a person. */
export const CREW_HUMAN_AUTHORITY_UNAVAILABLE = 'crew_human_authority_unavailable';
/** Join (S3a): the pasted text is not a Crew invitation. */
export const CREW_INVITATION_INVALID = 'crew_invitation_invalid';
/** Join (S3a): the workspace's server does not support joining by invitation. */
export const CREW_JOIN_UNSUPPORTED = 'crew_join_unsupported';
/** Join (S3a): the host approved a different device code than this computer's. */
export const CREW_JOIN_CODE_MISMATCH = 'crew_join_code_mismatch';

/**
 * Set by this renderer, never by the daemon: the daemon answered successfully, but not with a body
 * the route promises. It never means success.
 */
export const CREW_UNEXPECTED_RESPONSE = 'crew_unexpected_response';
/**
 * Set by this renderer, never by the daemon: a route this renderer knows may be newer than the
 * daemon answered with something that is not JSON at all (an older `biorouter serve` daemon returns
 * its web page for any path it does not know).
 */
export const CREW_DAEMON_OUTDATED = 'crew_daemon_outdated';

/** The copy for a background service that predates the route being called. */
export const STALE_DAEMON_MESSAGE =
  'This feature needs a newer Biorouter background service. Quit and reopen Biorouter.';

export function isConnectFailureCode(code: unknown): code is CrewConnectFailureCode {
  return (CREW_CONNECT_FAILURE_CODES as readonly unknown[]).includes(code);
}

/** The daemon's typed code for a failed Crew request, if it gave one. */
export function crewErrorCode(error: unknown): string | undefined {
  return error instanceof CrewHttpError ? error.code : undefined;
}

/**
 * The broker's own refusal code (`name_taken`, `forbidden`…) for a refusal the daemon passed on
 * from the broker, if it gave one. The daemon's own `code` is then `crew_request_refused`.
 */
export function crewBrokerCode(error: unknown): string | undefined {
  return error instanceof CrewHttpError ? error.brokerCode : undefined;
}

/** The daemon's diagnostic detail for "Copy details", if it gave one. */
export function crewErrorDetail(error: unknown): string | undefined {
  return error instanceof CrewHttpError ? error.detail : undefined;
}

export function isRevocationUnconfirmed(error: unknown): boolean {
  return crewErrorCode(error) === CREW_REVOCATION_UNCONFIRMED;
}

/**
 * Did a request to a route added after the running daemon was built fail because the daemon does
 * not have that route? A desktop daemon answers 404 with no body; a path that collides with an
 * older route's pattern answers 405; an older `biorouter serve` daemon returns its web page. Every
 * refusal a current daemon gives carries a code, so a coded 404 (for example
 * `crew_grant_not_found`) is an answer, not a missing route.
 *
 * Only meaningful for a route that may be newer than the daemon; a 404 from a route every daemon has
 * is a different problem.
 */
export function isStaleDaemon(error: unknown): boolean {
  if (!(error instanceof CrewHttpError)) return false;
  if (error.code === CREW_DAEMON_OUTDATED) return true;
  return (error.status === 404 || error.status === 405) && !error.code;
}

/** A 2xx answer whose body is not what the route promises. */
export function unexpectedCrewResponse(what: string): CrewHttpError {
  return new CrewHttpError(
    `Biorouter sent ${what} that Crew couldn't read. Retry, or quit and reopen Biorouter.`,
    200,
    CREW_UNEXPECTED_RESPONSE
  );
}

/** A 2xx answer from a newer route that is not a JSON object at all: the daemon predates it. */
export function outdatedDaemonResponse(): CrewHttpError {
  return new CrewHttpError(STALE_DAEMON_MESSAGE, 200, CREW_DAEMON_OUTDATED);
}
