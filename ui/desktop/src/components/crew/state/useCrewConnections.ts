import {
  useCallback,
  useEffect,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import { crewHttp, type CrewConnection } from '../crewApi';
import { classifyConnectFailure } from './connectFailure';
import type {
  ActionKey,
  ActOptions,
  ErrorSource,
  LastConnectFailure,
  PreparedDevice,
  SaveConnectionInput,
} from './types';

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

/**
 * How long focus and visibility changes settle before the list is read again: switching windows
 * fires `focus` and `visibilitychange` together, and one read answers both.
 */
export const CONNECTIONS_RELOAD_DEBOUNCE_MS = 250;

/**
 * The saved connections and the selected one. Every call here throws on failure.
 *
 * The list is read on mount and after every change made here, and again whenever the window comes
 * back into view (focus, or the page becoming visible): a connection saved or connected from the
 * terminal (`biorouter crew …`) shares the daemon but not this page, and used to stay invisible
 * until a reload (T-51). A failed background read keeps the list it has.
 */
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
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const reload = () => {
      if (document.visibilityState === 'hidden') return;
      if (timer !== undefined) clearTimeout(timer);
      timer = setTimeout(() => {
        timer = undefined;
        void loadConnections().catch(() => undefined);
      }, CONNECTIONS_RELOAD_DEBOUNCE_MS);
    };
    window.addEventListener('focus', reload);
    document.addEventListener('visibilitychange', reload);
    return () => {
      if (timer !== undefined) clearTimeout(timer);
      window.removeEventListener('focus', reload);
      document.removeEventListener('visibilitychange', reload);
    };
  }, [loadConnections]);
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

/** The classified failure of the most recent connect or sign-in, remembered per connection. */
export interface CrewConnectFailures {
  failure: (LastConnectFailure & { connectionId: string }) | null;
  /** Classify `failure` and remember it for `connectionId`. Returns the classification. */
  record(connectionId: string, failure: unknown): LastConnectFailure;
  /** Forget the failure of `connectionId` (it connected, verified, or was disconnected on purpose). */
  clear(connectionId: string): void;
}

export function useCrewConnectFailures(): CrewConnectFailures {
  const [failure, setFailure] = useState<(LastConnectFailure & { connectionId: string }) | null>(
    null
  );
  const record = useCallback((connectionId: string, thrown: unknown) => {
    const classified = classifyConnectFailure(thrown);
    setFailure({ connectionId, ...classified });
    return classified;
  }, []);
  const clear = useCallback(
    (connectionId: string) =>
      setFailure((current) => (current?.connectionId === connectionId ? null : current)),
    []
  );
  return { failure, record, clear };
}

export interface CrewConnectionLifecycleContext {
  connectionId: string;
  failures: CrewConnectFailures;
  autoOpenSignIn: boolean;
  openSignIn(reason: 'user' | 'auto'): void;
  loadConnections(): Promise<void>;
  refresh(): Promise<void>;
  stopObserving(): void;
  act<T>(
    source: ErrorSource,
    key: ActionKey,
    fn: () => Promise<T>,
    options?: ActOptions
  ): Promise<T | undefined>;
}

/**
 * Connect and disconnect the selected connection.
 *
 * Connect is `POST …/connect`, then reload the list, then refresh (the order the old view used).
 * A failure is classified and remembered for the connection, and also recorded as an error from
 * the `connect` source. With `autoOpenSignIn`, a user-initiated connect that the server answered
 * with a password or code prompt opens Sign in by itself, once per attempt.
 */
export function createConnectionLifecycle(context: CrewConnectionLifecycleContext) {
  const {
    connectionId,
    failures,
    autoOpenSignIn,
    openSignIn,
    loadConnections,
    refresh,
    stopObserving,
    act,
  } = context;
  const connect = async (opts?: { userInitiated?: boolean }) => {
    const target = connectionId;
    await act('connect', 'connect', async () => {
      try {
        await crewHttp(`/connections/${target}/connect`, 'POST', {});
      } catch (failure) {
        const classified = failures.record(target, failure);
        if (autoOpenSignIn && opts?.userInitiated && classified.kind === 'auth_required')
          openSignIn('auto');
        throw failure;
      }
      failures.clear(target);
      await loadConnections();
      await refresh();
    });
  };
  const disconnect = async () => {
    const target = connectionId;
    await act('global', 'disconnect', async () => {
      await crewHttp(`/connections/${target}/disconnect`, 'POST', {});
      stopObserving();
      failures.clear(target);
      await loadConnections();
    });
  };
  return { connect, disconnect };
}
