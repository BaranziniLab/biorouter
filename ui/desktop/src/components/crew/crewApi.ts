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
export interface Principal {
  id: string;
  uid: number;
  username: string;
  nickname: string;
  avatar?: string;
}
export interface Team {
  id: string;
  name: string;
  created_by: string;
  members: string[];
  general_channel_id: string;
}
export interface Channel {
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
export interface Invitation {
  id: string;
  kind: 'team' | 'channel';
  target_id: string;
  principal_id: string;
  status?: string;
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
  };
  actor: Principal;
  principals: Principal[];
  teams: Team[];
  channels: Channel[];
  invitations: Invitation[];
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

export class CrewHttpError extends Error {
  constructor(
    message: string,
    public readonly status: number,
    public readonly code?: string
  ) {
    super(message);
    this.name = 'CrewHttpError';
  }
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
    throw new CrewHttpError(
      typeof result?.error === 'string' ? result.error : `Crew request failed (${response.status})`,
      response.status,
      typeof result?.code === 'string' ? result.code : undefined
    );
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
}
// The generated union owns the wire contract; these refinements describe validated payloads.
type ObservationPayload<T> = T extends { type: 'state' }
  ? Omit<T, 'snapshot' | 'runs'> & { snapshot: Snapshot; runs: ObservedRun[] }
  : T extends { type: 'messages' }
    ? Omit<T, 'messages'> & { messages: CrewMessage[] }
    : T;
export type CrewObservation = ObservationPayload<ObserveEvent>;

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
    return frame;
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
    return frame;
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
    throw new CrewHttpError(
      typeof result?.error === 'string'
        ? result.error
        : `Crew observer failed (${response.status}).`,
      response.status,
      result?.code
    );
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
