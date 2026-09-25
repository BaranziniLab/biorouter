import { CrewHttpError } from '../crewApi';
import { crewActionCopy } from './copy';
import type { LastConnectFailure } from './types';

/** The daemon's SSH failure codes, without their prefix, plus `unknown`. */
export type ConnectFailureKind =
  | 'auth_required'
  | 'host_key_unknown'
  | 'host_key_changed'
  | 'unreachable'
  | 'bridge_missing'
  | 'handoff_failed'
  | 'workspace_identity_mismatch'
  | 'ssh_failed'
  | 'unknown';

/** Each typed code the connect and sign-in routes return, and the surface kind it maps to. */
export const CONNECT_FAILURE_CODES: Readonly<Record<string, ConnectFailureKind>> = {
  crew_ssh_auth_required: 'auth_required',
  crew_ssh_host_key_unknown: 'host_key_unknown',
  crew_ssh_host_key_changed: 'host_key_changed',
  crew_ssh_unreachable: 'unreachable',
  crew_bridge_missing: 'bridge_missing',
  crew_handoff_failed: 'handoff_failed',
  crew_workspace_identity_mismatch: 'workspace_identity_mismatch',
  crew_ssh_failed: 'ssh_failed',
};

/** Failures that mean the server or workspace could not be verified. Only a typed code says so. */
export const TRUST_FAILURE_KINDS: readonly ConnectFailureKind[] = [
  'host_key_unknown',
  'host_key_changed',
  'workspace_identity_mismatch',
];
/** Failures that mean `biorouter-crew` is not running for this account on the server. */
export const NOT_SET_UP_FAILURE_KINDS: readonly ConnectFailureKind[] = [
  'bridge_missing',
  'handoff_failed',
];

export function isTrustFailure(kind: ConnectFailureKind | undefined): boolean {
  return kind !== undefined && TRUST_FAILURE_KINDS.includes(kind);
}
export function isNotSetUpFailure(kind: ConnectFailureKind | undefined): boolean {
  return kind !== undefined && NOT_SET_UP_FAILURE_KINDS.includes(kind);
}

/**
 * The daemon's `last_error_code` on a saved connection the workspace no longer admits: this
 * computer or its person was removed (Q3-12, Q3-50). The daemon's keepalive stops re-dialling it.
 */
export const MEMBERSHIP_ENDED_CODE = 'crew_membership_ended';

/** Whether the saved connection's last answer was that its membership ended. */
export function isMembershipEnded(
  connection: { last_error_code?: string | null } | null | undefined
): boolean {
  return connection?.last_error_code === MEMBERSHIP_ENDED_CODE;
}

/** What an arriving "connect this" request (Q3-08) does now. */
export type ArrivalConnectDecision = 'connect' | 'wait' | 'skip';

export interface ArrivalConnectInput {
  /** The saved record of the connection the request names; null when there is none. */
  connection: { status: string; last_error_code?: string | null } | null;
  /** The classified failure of the most recent connect or sign-in for that connection. */
  lastConnectFailure: { kind: ConnectFailureKind } | null;
  /** A connect for it is already running. */
  connecting: boolean;
  /** Sign in is open, or a sign-in is running. */
  signInPending: boolean;
}

/**
 * SECURITY-SENSITIVE (human review). Whether Crew connects, on arrival, the connection a chat's
 * "Connect in Crew" named (Q3-08). The click in the chat is the person's own action, so this is
 * the person's connect, one screen later — but only for a saved connection the daemon calls
 * `disconnected`, and never for an answer that is final: a server or workspace that could not be
 * verified, a server that wants a password or a code (Sign in is the person's), or a membership
 * the workspace ended (`MEMBERSHIP_ENDED_CODE`). A final answer decides even while a connect runs;
 * otherwise `wait` while one is running, and the caller decides once it settles.
 */
export function arrivalConnectDecision(input: ArrivalConnectInput): ArrivalConnectDecision {
  const { connection, lastConnectFailure, connecting, signInPending } = input;
  if (!connection) return 'skip';
  const failure = lastConnectFailure?.kind;
  if (
    isMembershipEnded(connection) ||
    isTrustFailure(failure) ||
    failure === 'auth_required' ||
    connection.status === 'authentication_required' ||
    signInPending
  )
    return 'skip';
  if (connecting) return 'wait';
  return connection.status === 'disconnected' ? 'connect' : 'skip';
}

// A daemon without typed codes still names the SSH child's outcome in its text, for example
// "Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]". A missing remote command is
// the more specific cause, so exit 127 is checked before the generic end-of-file.
const BRIDGE_MISSING_TEXT = /\bexit_127\b/;
const AUTH_REQUIRED_TEXT = /\bssh_eof\b|\bexit_255\b/;

/**
 * Classify a failed connect (or sign-in) for the connection-problem surfaces.
 *
 * A typed daemon code decides. Without one (a daemon that predates the codes answers every
 * failure as `crew_request_refused`), `exit_127` in the text means Crew isn't set up, and
 * `ssh_eof` or `exit_255` means the server wants a password or a code: the sign-in terminal then
 * shows OpenSSH's own words, host-key refusals included. Nothing else is ever labelled a host-key
 * problem; the old `/host|SSH|key|authentication/i` match sent every SSH failure to the trust
 * screen.
 */
export function classifyConnectFailure(failure: unknown): LastConnectFailure {
  const message = failure instanceof Error ? failure.message : crewActionCopy.actionFallback;
  const code = failure instanceof CrewHttpError ? failure.code : undefined;
  const detailValue =
    failure !== null && typeof failure === 'object'
      ? (failure as { detail?: unknown }).detail
      : undefined;
  const detail = typeof detailValue === 'string' && detailValue ? detailValue : undefined;
  const typed =
    code !== undefined && Object.prototype.hasOwnProperty.call(CONNECT_FAILURE_CODES, code)
      ? CONNECT_FAILURE_CODES[code]
      : undefined;
  const kind: ConnectFailureKind =
    typed ??
    (BRIDGE_MISSING_TEXT.test(message)
      ? 'bridge_missing'
      : AUTH_REQUIRED_TEXT.test(message)
        ? 'auth_required'
        : 'unknown');
  return {
    kind,
    message,
    ...(code !== undefined ? { code } : {}),
    ...(detail !== undefined ? { detail } : {}),
  };
}
