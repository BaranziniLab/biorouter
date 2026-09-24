import { crewHttp, type CrewConnection, type CrewPersonName } from '../crewApi';
import { crewErrorCode, outdatedDaemonResponse, unexpectedCrewResponse } from './errors';
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

/** Join (S3a, 409): this computer already has the workspace, saved with other settings. */
export const CREW_CONNECTION_EXISTS = 'crew_connection_exists';
/** Join (S3a, 409): this computer pins a different identity for the same workspace. */
export const CREW_INVITATION_CONFLICT = 'crew_invitation_conflict';
/** Join status and claim (409): the connection is not connected; connect it, then ask again. */
export const CREW_NOT_CONNECTED = 'crew_not_connected';

/** What saving an invitation still needs, which neither the invitation nor the person gave. */
export type CrewInvitationMissing = 'username' | 'server' | 'institution';

const INVITATION_MISSING: readonly string[] = ['username', 'server', 'institution'];

/**
 * What an invitation says, as the daemon parsed it, before anything is saved. The labels are display
 * metadata and defaults: the workspace is trusted only after the server proves the pinned key. A
 * field an invitation (or an older `biorouter-crew status` paste) lacks is null.
 *
 * Two kinds of field sit side by side. `workspace_mode` and `workspace_institution_id` are what the
 * invitation itself states, and are null when it states nothing. `mode` and `institution_id` are
 * what saving would use with the choices sent so far; the daemon fills them in (Private by
 * default), so they must never be shown as something the workspace said.
 */
export interface CrewInvitationPreview {
  workspace_id: string;
  workspace_name: string | null;
  workspace_public_key: string | null;
  /** The fingerprint to compare with the host's (SHA-256 of the key, lowercase hex). */
  workspace_key_fingerprint: string | null;
  /** The daemon's short form of the fingerprint for comparing by eye: `3F2A 9C1E 77B0 D4E1`. */
  fingerprint: string | null;
  host_username: string | null;
  host_display_name: string | null;
  /** The workspace's privacy as the invitation states it; null when it doesn't say. */
  workspace_mode: 'private' | 'public' | null;
  /** The workspace's institution as the invitation states it; null when it doesn't say. */
  workspace_institution_id: string | null;
  /** The privacy saving would use: a default, not the workspace's word. */
  mode: 'private' | 'public' | null;
  /** The institution saving would use: a default, not the workspace's word. */
  institution_id: string | null;
  ssh_host: string | null;
  ssh_port: number | null;
  proxy_jump: string | null;
  /** The username the host invited, used to prefill "Your username on {server}". */
  invitee_username: string | null;
  /**
   * The broker socket and the host's numeric UID the invitation pins. Machine fields: never shown.
   * The Host flow saves them with its prepared identity (`POST /crew/connections`), because a host
   * pastes `biorouter-crew`'s own output, which cannot name the SSH host. Not every daemon's
   * preview carries them: null means ask the person, never refuse the paste.
   */
  socket_path: string | null;
  owner_uid: number | null;
  /**
   * A connection on this computer that already pins this workspace. Saving again returns it (and
   * changes nothing) when the settings match, and is refused otherwise: offer to open it instead.
   */
  existing_connection_id: string | null;
  /** What saving still needs; empty when it can save. */
  missing: CrewInvitationMissing[];
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

/** What `POST …/join` answers once this computer is a member (the daemon's `JoinClaimed`). */
export interface CrewJoinClaim {
  joined: true;
  /** Who invited this computer, as the workspace named them. Display only. */
  inviter?: CrewPersonName;
  workspace_name?: string | null;
  /** This computer was added to an existing member's account. */
  add_device?: boolean;
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

function modeFrom(value: unknown): 'private' | 'public' | null {
  return value === 'private' || value === 'public' ? value : null;
}

function missingFrom(value: unknown): CrewInvitationMissing[] {
  if (!Array.isArray(value)) return [];
  return [
    ...new Set(
      value.filter(
        (item): item is CrewInvitationMissing =>
          typeof item === 'string' && INVITATION_MISSING.includes(item)
      )
    ),
  ];
}

function previewFrom(value: unknown): CrewInvitationPreview | null {
  // Accept the summary bare or inside a `preview` envelope.
  const body = isRecord(value) && isRecord(value.preview) ? value.preview : value;
  if (!isRecord(body)) return null;
  const workspaceId = optionalText(body.workspace_id);
  if (!workspaceId) return null;
  const port = nullableNumber(body.ssh_port);
  const ownerUid = nullableNumber(body.owner_uid);
  return {
    workspace_id: workspaceId,
    workspace_name: orNull(nullableText(body.workspace_name)),
    workspace_public_key: orNull(nullableText(body.workspace_public_key)),
    workspace_key_fingerprint: orNull(nullableText(body.workspace_key_fingerprint)),
    fingerprint: orNull(nullableText(body.fingerprint)),
    host_username: orNull(nullableText(body.host_username)),
    host_display_name: orNull(nullableText(body.host_display_name)),
    workspace_mode: modeFrom(body.workspace_mode),
    workspace_institution_id: orNull(nullableText(body.workspace_institution_id)),
    mode: modeFrom(body.mode),
    institution_id: orNull(nullableText(body.institution_id)),
    ssh_host: orNull(nullableText(body.ssh_host)),
    ssh_port: typeof port === 'number' && Number.isSafeInteger(port) && port > 0 ? port : null,
    proxy_jump: orNull(nullableText(body.proxy_jump)),
    invitee_username: orNull(nullableText(body.invitee_username)),
    socket_path: orNull(nullableText(body.socket_path)),
    owner_uid:
      typeof ownerUid === 'number' && Number.isSafeInteger(ownerUid) && ownerUid >= 0
        ? ownerUid
        : null,
    existing_connection_id: optionalText(body.existing_connection_id) ?? null,
    missing: missingFrom(body.missing),
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
 * The saved connection a join refusal concerns: the `connection_id` a 409
 * `crew_connection_exists` or `crew_invitation_conflict` carries, so the person can open it.
 *
 * The id is read from the refusal's JSON body, wherever the error exposes it (`body`, or as a
 * field of its own); when it exposes neither, `fallback` (the preview's `existing_connection_id`)
 * is used. Any other failure concerns no saved connection and answers null.
 */
export function refusalConnectionId(error: unknown, fallback?: string | null): string | null {
  const code = crewErrorCode(error);
  if (code !== CREW_CONNECTION_EXISTS && code !== CREW_INVITATION_CONFLICT) return null;
  const carrier = error as unknown as Record<string, unknown>;
  const body = isRecord(carrier.body) ? carrier.body : {};
  return (
    optionalText(body.connection_id) ??
    optionalText(carrier.connection_id) ??
    optionalText(carrier.connectionId) ??
    optionalText(fallback) ??
    null
  );
}

/**
 * The ids of the connections this computer has saved right now, straight from the daemon. A join
 * reads them before it saves, so it can tell a connection it created from one that already
 * existed. Throws when the daemon's answer can't be read: a caller must not guess "none".
 */
export async function savedConnectionIds(signal?: AbortSignal): Promise<string[]> {
  const result = await crewHttp<unknown>('/connections', 'GET', undefined, signal);
  if (!isRecord(result) || !Array.isArray(result.connections))
    throw unexpectedCrewResponse('a connection list');
  return result.connections.flatMap((row) => {
    const id = isRecord(row) ? optionalText(row.id) : undefined;
    return id ? [id] : [];
  });
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
  const inviter = personFrom(result.inviter);
  if (inviter) claim.inviter = inviter;
  const workspaceName = nullableText(result.workspace_name);
  if (workspaceName !== undefined) claim.workspace_name = workspaceName;
  if (typeof result.add_device === 'boolean') claim.add_device = result.add_device;
  return claim;
}
