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
