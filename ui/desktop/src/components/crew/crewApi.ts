import type { ObserveEvent, ObserveRequest } from '../../api/types.gen';
import { client } from '../../api/client.gen';
import { userActionHeaders } from '../../utils/userAction';

export interface CrewConnection {
  remote_root?: string;
  remote_execution: boolean;
  workspace_public_key: string;
  public_key: string;
  device_id: string;
  id: string;
  name: string;
  ssh_target: string;
  port?: number;
  identity_file?: string;
  proxy_jump?: string;
  socket_path: string;
  owner_uid: number;
  workspace_id: string;
  cluster_connection_id: string;
  mode: 'private' | 'public';
  institution_id?: string | null;
  policy_epoch: number;
  status: 'disconnected' | 'connected' | 'authentication_required' | 'error';
  last_error?: string;
}
// The snapshot is forwarded by the daemon as an untyped value, so these interfaces are written by
// hand from docs/research/biorouter-crew/naming-design.md. Every field a broker before S1a/S2a does
// not send is optional; timestamps are Unix seconds, as the broker writes them.
export interface Principal {
  id: string;
  uid: number;
  username: string;
  /** The stored display name. Prefer `display_name`, which the broker sanitizes. */
  nickname: string;
  avatar?: string;
  /** Computed by the broker (S1a): the sanitized nickname, else the username. */
  display_name?: string;
  /** False for a principal that was removed from the workspace. */
  active?: boolean;
  /** Host snapshot only: this principal's UID no longer maps to its username on the server. */
  account_stale?: boolean;
}
/** A device on the viewer's own account (D16). `fingerprint` is display-grouped, never a key. */
export interface CrewDevice {
  fingerprint: string;
  added_at?: number;
  /** `bootstrap`, `token` or `invitation_code`; empty for a device added before S1a. */
  added_via?: string;
}
export interface CrewActor extends Principal {
  devices?: CrewDevice[];
}
/** An inactive principal still referenced by something the viewer can see. Display only. */
export interface FormerPrincipal {
  id: string;
  username: string;
  display_name?: string;
  avatar?: string;
  active?: false;
}
/** Naming flags the broker projects on teams and channels (S2a). Never used for authority. */
export interface CrewNameFlags {
  /** Sanitized name for display ("Untitled team" / `untitled` when nothing printable is left). */
  display_name?: string;
  /** The broker's own name key, which the resolver matches instead of recomputing it. */
  handle?: string;
  /** Another object the viewer can see has the same name key (a legacy duplicate). */
  name_conflict?: boolean;
  /** The stored name breaks the current naming rules; its owner should rename it. */
  name_invalid?: boolean;
}
export interface Team extends CrewNameFlags {
  id: string;
  name: string;
  created_by: string;
  members: string[];
  general_channel_id: string;
}
export interface Channel extends CrewNameFlags {
  id: string;
  team_id: string;
  name: string;
  created_by: string;
  owner_id: string;
  members: string[];
  archived: boolean;
  pending_owner?: string;
  classification: 'public_safe' | 'restricted';
}
export interface CrewPersonName {
  username: string;
  display_name?: string;
}
export interface Invitation {
  id: string;
  kind: 'team' | 'channel';
  target_id: string;
  principal_id: string;
  inviter_id: string;
  expires_at: number;
  /** The team's or channel's current name, projected for the invitee and the inviter (S1a). */
  target_name?: string;
  /** For a channel invitation, the name of the channel's team. */
  team_name?: string;
  inviter?: CrewPersonName;
  /** Only the inviter sees expired invitations; the broker omits them for the invitee. */
  expired?: boolean;
}
/**
 * A person the host has invited to join the workspace (S3a, manager snapshot only). Only `username`
 * is checked when a frame arrives, so treat every other field as possibly absent.
 */
export interface PendingJoin {
  username: string;
  /** The name on the server account, shown as "(name on the server account)". Label only. */
  full_name?: string | null;
  add_device?: boolean;
  approved?: boolean;
  created_at?: number;
  expires_at?: number;
  /** How many times a device with a different code tried to join as this person. */
  mismatched_attempts?: number;
  /** The invitation ran out before the person joined; it can no longer be let in. */
  expired?: boolean;
}
export interface CrewRun {
  id: string;
  owner_id: string;
  channel_id: string;
  status: string;
  provider?: string;
  model?: string;
}
export interface Snapshot {
  read_positions?: Record<string, string | null>;
  unread?: Record<string, number>;
  workspace: {
    id: string;
    host_uid: number;
    mode: 'private' | 'public';
    institution_id?: string | null;
    policy_epoch: number;
    /** The host's principal, injected at projection time (S1a). Prefer it over `host_uid`. */
    host_principal_id?: string;
    /** The workspace slug (S2a); absent or null for a workspace created before names. */
    name?: string | null;
  };
  actor: CrewActor;
  principals: Principal[];
  /** Inactive principals referenced by the viewer's visible objects (S1a). */
  former_principals?: FormerPrincipal[];
  teams: Team[];
  channels: Channel[];
  invitations: Invitation[];
  /** People the host invited who have not joined yet (S3a, manager only). */
  pending_joins?: PendingJoin[];
  runs: CrewRun[];
}
export interface CrewMessage {
  id: string;
  sequence: string;
  channel_id: string;
  actor_id: string;
  run_id?: string;
  body: string;
  created_at: number;
  restricted: boolean;
  source_channels: string[];
  attachments: string[];
  references?: string[];
}
/** One entry of the `people` map a message result carries beside its messages (S1a). */
export interface CrewMessageAuthor {
  username: string;
  display_name?: string;
  active?: boolean;
}
/**
 * A validated `people` map, keyed by principal ID. It has no prototype, so no key can reach one.
 * Display only: it names authors the snapshot no longer does, such as someone who left.
 */
export type CrewMessagePeople = Readonly<Record<string, CrewMessageAuthor>>;
/**
 * The result of `messages.history`, `messages.search`, `message.post` and the other calls that
 * return messages. `people` names every author once per response instead of once per message, and
 * `channel_names` covers only channels the viewer can read.
 */
export interface CrewMessageResult {
  messages: CrewMessage[];
  cursor?: string | null;
  people?: Record<string, CrewMessageAuthor>;
  channel_names?: Record<string, string>;
}
/**
 * The daemon's label for one person (S1a), keyed by principal ID on an observation `state` frame.
 * `collides` is true when two people in the workspace share a display name, so both must be shown
 * with their `@username`.
 */
export interface CrewPersonLabel {
  full: string;
  short: string;
  collides: boolean;
}
export type CrewPersonLabels = Record<string, CrewPersonLabel>;

export class CrewHttpError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code?: string,
    /** Diagnostic text for "Copy details" (for example OpenSSH's own words), never shown by default. */
    public readonly detail?: string,
    /**
     * The broker's own refusal code (`name_taken`, `forbidden`, `response_too_large`…) when the
     * daemon passed a broker refusal on; `code` is then `crew_request_refused`.
     */
    public readonly brokerCode?: string,
    /**
     * The saved connection a refusal concerns: the top-level `connection_id` the daemon's 409
     * `crew_connection_exists` / `crew_invitation_conflict` carries, so the person can open it.
     * Read it through `refusalConnectionId`, which asks the code first.
     */
    public readonly connectionId?: string
  ) {
    super(message);
    this.name = 'CrewHttpError';
  }
}

/** A broker refusal code as the daemon forwards one: a short snake_case word. */
function brokerCodeOf(value: unknown): string | undefined {
  return typeof value === 'string' && /^[a-z0-9_]{1,64}$/.test(value) ? value : undefined;
}

/**
 * A saved connection's id as a refusal names one (the daemon's are UUIDv4), or undefined. It can
 * end up in a request path, so nothing but id characters is kept, and never a dot segment.
 */
function connectionIdOf(value: unknown): string | undefined {
  if (typeof value !== 'string' || !/^[A-Za-z0-9_.:-]{1,128}$/.test(value)) return undefined;
  return /^\.+$/.test(value) ? undefined : value;
}

function crewHttpErrorFrom(result: unknown, status: number, fallback: string): CrewHttpError {
  const body =
    typeof result === 'object' && result !== null ? (result as Record<string, unknown>) : {};
  return new CrewHttpError(
    typeof body.error === 'string' ? body.error : fallback,
    status,
    typeof body.code === 'string' ? body.code : undefined,
    typeof body.detail === 'string' ? body.detail : undefined,
    brokerCodeOf(body.broker_code),
    connectionIdOf(body.connection_id)
  );
}

async function crewHeaders(hasJsonBody: boolean): Promise<Headers> {
  const headers = new Headers(client.getConfig().headers as HeadersInit);
  headers.set('X-Secret-Key', await window.electron.getSecretKey());
  Object.entries(await userActionHeaders()).forEach(([key, value]) => headers.set(key, value));
  if (hasJsonBody) {
    headers.set('Content-Type', 'application/json');
  } else {
    headers.delete('Content-Type');
  }
  return headers;
}

export async function crewHttp<T>(
  path: string,
  method = 'GET',
  body?: unknown,
  signal?: AbortSignal
): Promise<T> {
  const config = client.getConfig();
  const headers = await crewHeaders(body !== undefined);
  const response = await fetch(`${config.baseUrl ?? ''}/crew${path}`, {
    method,
    headers,
    signal,
    body: body === undefined ? undefined : JSON.stringify(body),
  });
  const result = await response.json().catch(() => null);
  if (!response.ok)
    throw crewHttpErrorFrom(result, response.status, `Crew request failed (${response.status})`);
  return result as T;
}

export function crewRequest<T>(
  connectionId: string,
  method: string,
  params: Record<string, unknown> = {},
  mutation = false,
  signal?: AbortSignal
): Promise<T> {
  const key = mutation
    ? typeof params.idempotency_key === 'string'
      ? params.idempotency_key
      : crypto.randomUUID()
    : undefined;
  return crewHttp<T>(
    `/connections/${encodeURIComponent(connectionId)}/request`,
    'POST',
    {
      method,
      params: key ? { ...params, idempotency_key: key } : params,
      request_id: key,
    },
    signal
  );
}

export interface ObservedRun {
  run_id: string;
  channel_id: string;
  session_id: string;
  status: string;
  error?: string;
  /** When this device admitted the task, in Unix milliseconds. Absent for an older run. */
  started_at?: number;
}
// The generated union owns the wire contract; these refinements describe validated payloads.
type ObservationPayload<T> = T extends { type: 'state' }
  ? Omit<T, 'snapshot' | 'runs' | 'labels' | 'capabilities'> & {
      snapshot: Snapshot;
      runs: ObservedRun[];
      labels?: CrewPersonLabels;
      /** What the connected broker says it supports (for example `unique_names_v1`). */
      capabilities?: string[];
    }
  : T extends { type: 'messages' }
    ? Omit<T, 'messages' | 'remaining' | 'page_size' | 'people' | 'channel_names'> & {
        messages: CrewMessage[];
        /** How many messages of this page are still to come; `0` ends the opening backlog. */
        remaining?: number;
        /** The page size the observer asks the broker for now. */
        page_size?: number;
        people?: CrewMessagePeople;
        channel_names?: Readonly<Record<string, string>>;
      }
    : T;
export type CrewObservation = ObservationPayload<ObserveEvent>;

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function nonEmptyText(value: unknown): value is string {
  return typeof value === 'string' && value.trim().length > 0;
}

// Labels, former members, pending joins and devices are display-only projections. A malformed one is
// dropped, so the interface falls back to what it can compute itself, rather than either failing the
// whole observation or handing an unchecked value to a renderer.
function validatedLabels(value: unknown): CrewPersonLabels | undefined {
  if (!isRecord(value)) return undefined;
  // fromEntries defines own properties, so a "__proto__" key cannot reach the prototype.
  return Object.fromEntries(
    Object.entries(value).flatMap(([principalId, label]): [string, CrewPersonLabel][] =>
      principalId &&
      isRecord(label) &&
      nonEmptyText(label.full) &&
      nonEmptyText(label.short) &&
      typeof label.collides === 'boolean'
        ? [[principalId, { full: label.full, short: label.short, collides: label.collides }]]
        : []
    )
  );
}

/** Keys a map from the wire never gets, whatever prototype it has. */
const RESERVED_KEYS = new Set(['__proto__', 'constructor', 'prototype']);

/**
 * A `people` map (a message result's, or an observation frame's), keeping only entries a renderer
 * can name someone from. Built on a null prototype, so no key reaches `Object.prototype`.
 */
export function validatedPeople(value: unknown): CrewMessagePeople | undefined {
  if (!isRecord(value)) return undefined;
  const people: Record<string, CrewMessageAuthor> = Object.create(null);
  for (const [principalId, entry] of Object.entries(value)) {
    if (!principalId || RESERVED_KEYS.has(principalId)) continue;
    if (!isRecord(entry) || !nonEmptyText(entry.username)) continue;
    const author: CrewMessageAuthor = { username: entry.username };
    if (typeof entry.display_name === 'string') author.display_name = entry.display_name;
    if (typeof entry.active === 'boolean') author.active = entry.active;
    people[principalId] = author;
  }
  return people;
}

/** A `channel_names` map: channel ID to its name, strings only, on a null prototype. */
export function validatedChannelNames(
  value: unknown
): Readonly<Record<string, string>> | undefined {
  if (!isRecord(value)) return undefined;
  const names: Record<string, string> = Object.create(null);
  for (const [channelId, name] of Object.entries(value)) {
    if (channelId && !RESERVED_KEYS.has(channelId) && nonEmptyText(name)) names[channelId] = name;
  }
  return names;
}

/** A non-negative integer count from the wire, or undefined for anything else. */
function count(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isSafeInteger(value) && value >= 0 ? value : undefined;
}

function recordsWith<T>(value: unknown, keys: string[]): T[] | undefined {
  if (!Array.isArray(value)) return undefined;
  return value.filter(
    (item): item is T => isRecord(item) && keys.every((key) => nonEmptyText(item[key]))
  );
}

function assignOptional(target: Record<string, unknown>, key: string, value: unknown) {
  if (value === undefined) delete target[key];
  else target[key] = value;
}

/** A run as observed, without a `started_at` that is not a time. */
function validatedRun(run: unknown): unknown {
  if (!isRecord(run) || !('started_at' in run)) return run;
  const { started_at: startedAt, ...rest } = run;
  return count(startedAt) === undefined ? rest : run;
}

/** A pending join, without an `expired` that is not a flag. */
function validatedPendingJoin(join: PendingJoin): PendingJoin {
  if (!('expired' in join) || typeof join.expired === 'boolean') return join;
  const { expired: _expired, ...rest } = join;
  return rest;
}

function stateWithValidatedProjections(frame: Record<string, unknown>): CrewObservation {
  const { labels, capabilities, ...state } = frame;
  const snapshot = { ...(frame.snapshot as Record<string, unknown>) };
  if (isRecord(snapshot.actor)) {
    const actor = { ...snapshot.actor };
    assignOptional(actor, 'devices', recordsWith<CrewDevice>(actor.devices, ['fingerprint']));
    snapshot.actor = actor;
  }
  assignOptional(
    snapshot,
    'former_principals',
    recordsWith<FormerPrincipal>(snapshot.former_principals, ['id', 'username'])
  );
  assignOptional(
    snapshot,
    'pending_joins',
    recordsWith<PendingJoin>(snapshot.pending_joins, ['username'])?.map(validatedPendingJoin)
  );
  state.snapshot = snapshot;
  state.runs = (frame.runs as unknown[]).map(validatedRun);
  assignOptional(state, 'labels', validatedLabels(labels));
  assignOptional(
    state,
    'capabilities',
    Array.isArray(capabilities)
      ? capabilities.filter((capability): capability is string => nonEmptyText(capability))
      : undefined
  );
  return state as unknown as CrewObservation;
}

// The backlog marker, the page size and the names beside a message are display projections too:
// a malformed one is dropped and the frame is kept.
function messagesWithValidatedProjections(frame: Record<string, unknown>): CrewObservation {
  const { remaining, page_size: pageSize, people, channel_names: channelNames, ...rest } = frame;
  const messages: Record<string, unknown> = rest;
  assignOptional(messages, 'remaining', count(remaining));
  const size = count(pageSize);
  assignOptional(messages, 'page_size', size !== undefined && size > 0 ? size : undefined);
  assignOptional(messages, 'people', validatedPeople(people));
  assignOptional(messages, 'channel_names', validatedChannelNames(channelNames));
  return messages as unknown as CrewObservation;
}

function observationFrame(line: string): CrewObservation {
  const frame = JSON.parse(line);
  if (!frame || typeof frame !== 'object')
    throw new Error('The daemon returned an invalid Crew observation. Retry to reconnect.');
  const cursorValid = frame.cursor === null || typeof frame.cursor === 'string';
  if (
    frame.type === 'state' &&
    typeof frame.connection_id === 'string' &&
    (frame.connection_mode === 'private' || frame.connection_mode === 'public') &&
    Number.isSafeInteger(frame.connection_policy_epoch) &&
    frame.connection_policy_epoch >= 0 &&
    (frame.connection_institution_id === null ||
      (typeof frame.connection_institution_id === 'string' &&
        frame.connection_institution_id.length >= 1 &&
        frame.connection_institution_id.length <= 64 &&
        /^[a-z0-9]/.test(frame.connection_institution_id) &&
        !/[^a-z0-9_-]/.test(frame.connection_institution_id))) &&
    frame.snapshot?.actor &&
    frame.snapshot?.workspace &&
    Array.isArray(frame.snapshot.principals) &&
    Array.isArray(frame.snapshot.invitations) &&
    Array.isArray(frame.snapshot.runs) &&
    Array.isArray(frame.snapshot.channels) &&
    Array.isArray(frame.snapshot.teams) &&
    Array.isArray(frame.runs)
  )
    return stateWithValidatedProjections(frame);
  if (
    frame.type === 'messages' &&
    typeof frame.channel_id === 'string' &&
    Array.isArray(frame.messages) &&
    frame.messages.length <= 200 &&
    cursorValid &&
    typeof frame.reset === 'boolean' &&
    frame.messages.every(
      (message: CrewMessage) =>
        message !== null &&
        typeof message.id === 'string' &&
        typeof message.sequence === 'string' &&
        message.channel_id === frame.channel_id
    )
  )
    return messagesWithValidatedProjections(frame);
  if (frame.type === 'reconnect' && cursorValid) return frame;
  if (
    frame.type === 'error' &&
    frame.clear === true &&
    typeof frame.code === 'string' &&
    typeof frame.error === 'string'
  )
    return frame;
  throw new Error(
    'The daemon returned an invalid Crew observation. Retry after checking the daemon.'
  );
}

export async function observeCrew(
  connectionId: string,
  channelId: string | undefined,
  after: string | null,
  signal: AbortSignal,
  receive: (frame: CrewObservation) => void
): Promise<'reconnect' | 'terminal'> {
  const request: ObserveRequest = {
    channel_id: channelId,
    after: after ?? undefined,
    initial: 'latest',
  };
  const response = await fetch(
    `${client.getConfig().baseUrl ?? ''}/crew/connections/${encodeURIComponent(connectionId)}/observe`,
    {
      method: 'POST',
      headers: await crewHeaders(true),
      signal,
      body: JSON.stringify(request),
    }
  );
  if (!response.ok) {
    const result = await response.json().catch(() => null);
    throw crewHttpErrorFrom(result, response.status, `Crew observer failed (${response.status}).`);
  }
  if (
    response.headers.get('content-type')?.split(';')[0].trim().toLowerCase() !==
    'application/x-ndjson'
  ) {
    await response.body?.cancel();
    throw new Error(
      'The daemon returned an unsupported Crew stream format. Retry after updating the daemon.'
    );
  }
  if (!response.body) throw new Error('The daemon did not provide a Crew observation stream.');
  const reader = response.body.getReader();
  const decoder = new TextDecoder('utf-8', { fatal: true });
  let pending = new Uint8Array(0);
  const consume = (bytes: Uint8Array): 'reconnect' | 'terminal' | undefined => {
    const line = decoder.decode(bytes);
    if (!line.trim()) return undefined;
    const frame = observationFrame(line);
    if (!signal.aborted) receive(frame);
    if (frame.type === 'reconnect') return 'reconnect';
    if (frame.type === 'error') return 'terminal';
    return undefined;
  };
  try {
    while (!signal.aborted) {
      const { value, done } = await reader.read();
      if (signal.aborted) return 'terminal';
      if (value) {
        let start = 0;
        while (start < value.length && !signal.aborted) {
          const newline = value.indexOf(10, start);
          const end = newline < 0 ? value.length : newline;
          const length = pending.length + end - start;
          if (length + 1 > 1048576) throw new Error('Crew observation exceeds its frame limit.');
          const bytes = new Uint8Array(length);
          bytes.set(pending);
          bytes.set(value.subarray(start, end), pending.length);
          pending = bytes;
          if (newline < 0) break;
          const result = consume(pending);
          pending = new Uint8Array(0);
          if (result) return result;
          start = newline + 1;
        }
      }
      if (done) {
        if (pending.length)
          throw new Error('Crew updates ended with an incomplete frame. Retry to reconnect.');
        throw new Error('Crew updates disconnected. Retry to reconnect to the daemon.');
      }
    }
    return 'terminal';
  } catch (failure) {
    if (signal.aborted) return 'terminal';
    throw failure;
  } finally {
    await reader.cancel().catch(() => {});
    reader.releaseLock();
  }
}
