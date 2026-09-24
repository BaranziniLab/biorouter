import { crewHttp, type CrewConnection, type CrewPersonName } from '../crewApi';
import { outdatedDaemonResponse, unexpectedCrewResponse } from './errors';
import { isRecord, nullableNumber, nullableText, optionalText } from './parse';

// Joining a workspace by invitation and device code (S3a). Only the daemon parses an invitation and
// only the joiner's own daemon computes the device code; these helpers carry the person's request
// and check the shape of the answer. None of them decides anything.

/** Connection settings a joiner may change before saving. Everything else comes from the invitation. */
export interface CrewInvitationAdvanced {
  name?: string;
  port?: number;
  identity_file?: string;
  proxy_jump?: string;
  remote_root?: string;
  remote_execution?: boolean;
}

export interface CrewInvitationOverrides {
  /** The joiner's login on the server; defaults to the invited username. */
  username?: string;
  /** Defaults to the workspace's mode, which the joiner confirms. */
  mode?: 'private' | 'public';
  institution_id?: string | null;
  advanced?: CrewInvitationAdvanced;
}

/**
 * What an invitation says, as the daemon parsed it, before anything is saved. The labels are display
 * metadata and defaults: the workspace is trusted only after the server proves the pinned key. A
 * field an invitation (or an older `biorouter-crew status` paste) lacks is null.
 */
export interface CrewInvitationPreview {
  workspace_id: string;
  workspace_name: string | null;
  workspace_public_key: string | null;
  /** The fingerprint to compare with the host's. */
  workspace_key_fingerprint: string | null;
  host_username: string | null;
  host_display_name: string | null;
  mode: 'private' | 'public' | null;
  institution_id: string | null;
  ssh_host: string | null;
  ssh_port: number | null;
  proxy_jump: string | null;
  /** The username the host invited, used to prefill "Your username on {server}". */
  invitee_username: string | null;
}

/** The message a host sends a joiner, and the `brcrew1:` line inside it. */
export interface CrewInvitationMessage {
  message: string;
  line: string;
}

export type CrewJoinState =
  | 'invited'
  | 'approved'
  | 'code_mismatch'
  | 'not_invited'
  | 'expired'
  | 'joined'
  /** The workspace's server cannot join by invitation: offer the invitation-token path. */
  | 'unsupported';

const JOIN_STATES: readonly string[] = [
  'invited',
  'approved',
  'code_mismatch',
  'not_invited',
  'expired',
  'joined',
  'unsupported',
];

export interface CrewJoinStatus {
  status: CrewJoinState;
  /**
   * This computer's device code, 16 Crockford base-32 characters without separators, computed by
   * the joiner's own daemon from its saved key. Always present for `invited` and `code_mismatch`.
   * Display it with `groupDeviceCode`; the broker never supplies it.
   */
  code?: string;
  inviter?: CrewPersonName;
  workspace_name?: string | null;
  /** Unix seconds. */
  expires_at?: number | null;
  /** The invitation adds this computer to an existing member's account. */
  add_device?: boolean;
}

export interface CrewJoinClaim {
  joined: true;
  /** Who the workspace admitted this computer as. */
  principal?: CrewPersonName;
}

/**
 * The broker's answer to the host's `enrollment.invite {username}` (S3a), sent through the
 * connection's `/request` route. `join_id` is a machine ID: never show it.
 */
export interface CrewEnrollmentInvite {
  username: string;
  /** The name on the server account, shown as "(name on the server account)". */
  full_name?: string | null;
  add_device: boolean;
  join_id: string;
  expires_at: number;
}

const DEVICE_CODE = /^[0-9ABCDEFGHJKMNPQRSTVWXYZ]{16}$/;

/**
 * A device code in the daemon's canonical form, or undefined. Only separators and case are
 * normalized: a daemon never emits the ambiguous letters I, L, O or U, so one that does is refused
 * rather than repaired.
 */
function deviceCodeFrom(value: unknown): string | undefined {
  if (typeof value !== 'string') return undefined;
  const code = value.replace(/[\s-]/g, '').toUpperCase();
  return DEVICE_CODE.test(code) ? code : undefined;
}

/** `7QK2M9XA3JTPWZ4D` → `7QK2-M9XA-3JTP-WZ4D`, for display only; copy the ungrouped value. */
export function groupDeviceCode(code: string): string {
  return (code.match(/.{1,4}/g) ?? []).join('-');
}

function personFrom(value: unknown): CrewPersonName | undefined {
  if (!isRecord(value)) return undefined;
  const username = optionalText(value.username);
  if (!username) return undefined;
  const displayName = optionalText(value.display_name);
  return displayName ? { username, display_name: displayName } : { username };
}

function orNull<T>(value: T | null | undefined): T | null {
  return value ?? null;
}

function previewFrom(value: unknown): CrewInvitationPreview | null {
  // Accept the summary bare or inside a `preview` envelope.
  const body = isRecord(value) && isRecord(value.preview) ? value.preview : value;
  if (!isRecord(body)) return null;
  const workspaceId = optionalText(body.workspace_id);
  if (!workspaceId) return null;
  const port = nullableNumber(body.ssh_port);
  return {
    workspace_id: workspaceId,
    workspace_name: orNull(nullableText(body.workspace_name)),
    workspace_public_key: orNull(nullableText(body.workspace_public_key)),
    workspace_key_fingerprint: orNull(nullableText(body.workspace_key_fingerprint)),
    host_username: orNull(nullableText(body.host_username)),
    host_display_name: orNull(nullableText(body.host_display_name)),
    mode: body.mode === 'private' || body.mode === 'public' ? body.mode : null,
    institution_id: orNull(nullableText(body.institution_id)),
    ssh_host: orNull(nullableText(body.ssh_host)),
    ssh_port: typeof port === 'number' && Number.isSafeInteger(port) && port > 0 ? port : null,
    proxy_jump: orNull(nullableText(body.proxy_jump)),
    invitee_username: orNull(nullableText(body.invitee_username)),
  };
}

function invitationBody(invitation: string, overrides: CrewInvitationOverrides, preview: boolean) {
  return preview ? { ...overrides, invitation, preview: true } : { ...overrides, invitation };
}

/**
 * Parse pasted text (the whole message, the bare `brcrew1:` line, or an older
 * `biorouter-crew status` output) without saving anything. A paste that is not an invitation is
 * refused with `crew_invitation_invalid`.
 */
export async function previewInvitation(
  invitation: string,
  overrides: CrewInvitationOverrides = {},
  signal?: AbortSignal
): Promise<CrewInvitationPreview> {
  const result = await crewHttp<unknown>(
    '/connections/from-invitation',
    'POST',
    invitationBody(invitation, overrides, true),
    signal
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  const preview = previewFrom(result);
  if (!preview) throw unexpectedCrewResponse('an invitation summary');
  return preview;
}

/** Save the connection exactly as the invitation pins it, with the joiner's confirmed choices. */
export async function saveFromInvitation(
  invitation: string,
  overrides: CrewInvitationOverrides = {}
): Promise<CrewConnection> {
  const result = await crewHttp<unknown>(
    '/connections/from-invitation',
    'POST',
    invitationBody(invitation, overrides, false)
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  // Accept the connection bare, as `POST /crew/connections` returns it, or inside an envelope.
  const connection = isRecord(result.connection) ? result.connection : result;
  if (!optionalText(connection.id) || typeof connection.name !== 'string')
    throw unexpectedCrewResponse('a saved connection');
  return connection as unknown as CrewConnection;
}

/**
 * The invitation message the host sends a joiner, built by the host's daemon from its own verified
 * connection. `invitee` is the invited `@username`, which the message names for the joiner.
 */
export async function getInvitation(
  connectionId: string,
  invitee?: string,
  signal?: AbortSignal
): Promise<CrewInvitationMessage> {
  const query = invitee ? `?${new URLSearchParams({ invitee }).toString()}` : '';
  const result = await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/invitation${query}`,
    'GET',
    undefined,
    signal
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  const message = optionalText(result.message);
  const line = optionalText(result.line);
  // The joiner pastes the whole message, so it must carry the line the daemon parses.
  if (!message || !line || !line.startsWith('brcrew1:') || !message.includes(line))
    throw unexpectedCrewResponse('an invitation');
  return { message, line };
}

/**
 * Where this computer's join stands. The daemon asks the workspace's server, and computes the device
 * code itself when the person is invited; poll it while the join screen is visible.
 */
export async function joinStatus(
  connectionId: string,
  signal?: AbortSignal
): Promise<CrewJoinStatus> {
  const result = await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/join`,
    'GET',
    undefined,
    signal
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  if (typeof result.status !== 'string' || !JOIN_STATES.includes(result.status))
    throw unexpectedCrewResponse('a join status');
  const status: CrewJoinStatus = { status: result.status as CrewJoinState };
  const code = deviceCodeFrom(result.code);
  if (result.code !== undefined && result.code !== null && !code)
    throw unexpectedCrewResponse('a join status');
  if ((status.status === 'invited' || status.status === 'code_mismatch') && !code)
    throw unexpectedCrewResponse('a join status');
  if (code) status.code = code;
  const inviter = personFrom(result.inviter);
  if (inviter) status.inviter = inviter;
  const workspaceName = nullableText(result.workspace_name);
  if (workspaceName !== undefined) status.workspace_name = workspaceName;
  const expiresAt = nullableNumber(result.expires_at);
  if (expiresAt !== undefined) status.expires_at = expiresAt;
  if (typeof result.add_device === 'boolean') status.add_device = result.add_device;
  return status;
}

/**
 * Ask the daemon to finish joining once the host approved this computer's code. The daemon signs the
 * join with the saved device key; a refusal (for example `crew_join_code_mismatch`) throws.
 */
export async function claimJoin(connectionId: string): Promise<CrewJoinClaim> {
  const result = await crewHttp<unknown>(
    `/connections/${encodeURIComponent(connectionId)}/join`,
    'POST'
  );
  if (!isRecord(result)) throw outdatedDaemonResponse();
  if (result.joined === false || (result.status !== undefined && result.status !== 'joined'))
    throw unexpectedCrewResponse('a join answer');
  const claim: CrewJoinClaim = { joined: true };
  const principal = personFrom(result.principal);
  if (principal) claim.principal = principal;
  return claim;
}
