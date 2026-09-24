import {
  useCallback,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import { crewHttp, type CrewConnection } from '../crewApi';
import type { PreparedDevice, SaveConnectionInput } from './types';

/**
 * The full connection body a privacy change PATCHes (L18: the daemon replaces the whole record,
 * so every field is sent, in this order).
 */
export function connectionUpdateBody(connection: CrewConnection): SaveConnectionInput {
  return {
    name: connection.name,
    ssh_target: connection.ssh_target,
    port: connection.port,
    identity_file: connection.identity_file,
    proxy_jump: connection.proxy_jump,
    socket_path: connection.socket_path,
    owner_uid: connection.owner_uid,
    workspace_id: connection.workspace_id,
    workspace_public_key: connection.workspace_public_key,
    cluster_connection_id: connection.cluster_connection_id,
    remote_root: connection.remote_root,
    remote_execution: connection.remote_execution,
    mode: connection.mode,
    institution_id: connection.institution_id ?? null,
  };
}

export interface CrewConnections {
  connections: CrewConnection[];
  setConnections: Dispatch<SetStateAction<CrewConnection[]>>;
  connectionId: string;
  setConnectionId: Dispatch<SetStateAction<string>>;
  connectionsState: 'loading' | 'loaded' | 'failed';
  markConnectionsFailed(): void;
  /**
   * `GET /crew/connections`; keeps the selection when it still exists, else selects the first.
   * With a signal and a generation, a result that arrives after either moved on is dropped.
   */
  loadConnections(signal?: AbortSignal, current?: number): Promise<void>;
  saveConnection(input: SaveConnectionInput): Promise<CrewConnection>;
  updateConnection(id: string, input: SaveConnectionInput): Promise<CrewConnection>;
  removeConnection(id: string): Promise<void>;
  prepareHostingDevice(): Promise<PreparedDevice>;
}

/** The saved connections and the selected one. Every call here throws on failure. */
export function useCrewConnections(generation: MutableRefObject<number>): CrewConnections {
  const [connections, setConnections] = useState<CrewConnection[]>([]);
  const [connectionId, setConnectionId] = useState('');
  const [connectionsState, setConnectionsState] = useState<'loading' | 'loaded' | 'failed'>(
    'loading'
  );

  const loadConnections = useCallback(
    async (signal?: AbortSignal, current?: number) => {
      const result = await crewHttp<{ connections: CrewConnection[] }>(
        '/connections',
        'GET',
        undefined,
        signal
      );
      if (signal?.aborted || (current !== undefined && generation.current !== current)) return;
      setConnections(result.connections);
      setConnectionsState('loaded');
      setConnectionId((old) =>
        result.connections.some((item) => item.id === old) ? old : (result.connections[0]?.id ?? '')
      );
    },
    [generation]
  );
  const markConnectionsFailed = useCallback(
    () => setConnectionsState((state) => (state === 'loaded' ? state : 'failed')),
    []
  );
  const saveConnection = useCallback(
    async (input: SaveConnectionInput) => {
      const saved = await crewHttp<CrewConnection>('/connections', 'POST', input);
      await loadConnections();
      setConnectionId(saved.id);
      return saved;
    },
    [loadConnections]
  );
  const updateConnection = useCallback(
    async (id: string, input: SaveConnectionInput) => {
      const saved = await crewHttp<CrewConnection>(`/connections/${id}`, 'PATCH', input);
      await loadConnections();
      return saved;
    },
    [loadConnections]
  );
  const removeConnection = useCallback(
    async (id: string) => {
      await crewHttp(`/connections/${id}`, 'DELETE');
      await loadConnections();
    },
    [loadConnections]
  );
  const prepareHostingDevice = useCallback(
    () => crewHttp<PreparedDevice>('/devices/prepare', 'POST', {}),
    []
  );

  return {
    connections,
    setConnections,
    connectionId,
    setConnectionId,
    connectionsState,
    markConnectionsFailed,
    loadConnections,
    saveConnection,
    updateConnection,
    removeConnection,
    prepareHostingDevice,
  };
}
