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
  read_positions?: Record<string, number>;
  unread?: Record<string, number>;
  workspace: { id: string; host_uid: number; mode: 'private' | 'public'; policy_epoch: number };
  actor: Principal;
  principals: Principal[];
  teams: Team[];
  channels: Channel[];
  invitations: Invitation[];
  runs: CrewRun[];
}
export interface CrewMessage {
  id: string;
  sequence: number;
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

export async function crewHttp<T>(path: string, method = 'GET', body?: unknown): Promise<T> {
  const config = client.getConfig();
  const headers = new Headers(config.headers as HeadersInit);
  headers.set('X-Secret-Key', await window.electron.getSecretKey());
  Object.entries(await userActionHeaders()).forEach(([key, value]) => headers.set(key, value));
  if (method !== 'GET') {
    headers.set('Content-Type', 'application/json');
  }
  const response = await fetch(`${config.baseUrl ?? ''}/crew${path}`, {
    method,
    headers,
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
  mutation = false
): Promise<T> {
  const key = mutation
    ? typeof params.idempotency_key === 'string'
      ? params.idempotency_key
      : crypto.randomUUID()
    : undefined;
  return crewHttp<T>(`/connections/${encodeURIComponent(connectionId)}/request`, 'POST', {
    method,
    params: key ? { ...params, idempotency_key: key } : params,
    request_id: key,
  });
}
