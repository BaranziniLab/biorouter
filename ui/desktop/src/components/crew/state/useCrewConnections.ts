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
import { forgetConnectionDrafts, forgetLastChannel, resetBetweenTests } from './draftStash';
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
   * Resolves with the list the daemon answered (undefined when the result was dropped), so a
   * caller can decide by the fresh record rather than wait for a render.
   */
  loadConnections(signal?: AbortSignal, current?: number): Promise<CrewConnection[] | undefined>;
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
      if (signal?.aborted || (current !== undefined && generation.current !== current))
        return undefined;
      setConnections(result.connections);
      setConnectionsState('loaded');
      setConnectionId((old) =>
        result.connections.some((item) => item.id === old) ? old : (result.connections[0]?.id ?? '')
      );
      return result.connections;
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
      forgetConnectionMemory(id);
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

// ---------------------------------------------------------------------------------------------
// What this app session remembers about each connection (live QA round 2, Q2-01 and Q2-18)
// ---------------------------------------------------------------------------------------------
//
// SECURITY-SENSITIVE (human review). A connection whose live updates ended is never connected by
// the renderer. A window's memory cannot know about a Disconnect made anywhere else — `biorouter
// crew disconnect` in a terminal on the same daemon, the Disconnect button in a second window, an
// edit (which disconnects) — and the daemon ends observation with the same `observation_refused`
// for all of them and for a dropped bridge. So a renderer that connected "a dropped connection"
// would undo exactly the Disconnects the daemon's own re-dial deliberately honours
// (`disarm_idle_redial`). Re-dialling a dropped bridge is the daemon's alone (D-KEEPALIVE,
// `crew/keepalive.rs`); the renderer only reads the saved record again, observes again when the
// daemon holds a bridge, and leaves every connect to the person.

/** When each connection was last observed again quietly after a loss, in this window. */
const quietReobserves = new Map<string, number[]>();
/** Connections a verified view (or a `joined` answer) showed this app session. */
const verifiedConnections = new Set<string>();

/**
 * The least time since the previous quiet re-observation of a connection before the next one:
 * none for the first, then 20 s, then 60 s — a growing back-off, as the daemon's own re-dial
 * grows. Its length is also the cap: at most this many in `QUIET_REOBSERVE_WINDOW_MS`.
 */
export const QUIET_REOBSERVE_GAPS_MS: readonly number[] = [0, 20_000, 60_000];
/**
 * The window quiet re-observations are counted in. Past the cap, or sooner than the gap, a loss
 * is shown with Retry at once: a bridge that keeps dropping is a real failure, not an idle drop.
 */
export const QUIET_REOBSERVE_WINDOW_MS = 10 * 60_000;

/**
 * After a loss the daemon now reports as disconnected, the gaps between the reads of the saved
 * record that follow it: 30 s, then 60, 90 and 120 s (cumulative 30 s, 1.5, 3 and 5 min). A read
 * is only `GET /connections`; it connects nothing. It catches the daemon's own re-dial of a
 * network failure (tried again 20, 60 and 180 s apart) without a click: when the record says
 * connected again, the observation, whose error is on show, observes again by itself.
 */
export const DAEMON_REDIAL_FOLLOW_MS: readonly number[] = [30_000, 60_000, 90_000, 120_000];

/**
 * Whether a connection that ended while the daemon still (or again) calls it connected may be
 * observed again quietly now, recorded as taken when it may. Observing is read-only and verified
 * as ever; this only limits how often a failure is hidden behind "Reconnecting…": the gaps grow
 * (`QUIET_REOBSERVE_GAPS_MS`) and the count is capped per `QUIET_REOBSERVE_WINDOW_MS`. A person's
 * Retry does not reset it — a bridge that keeps dropping keeps being shown.
 */
export function takeQuietReobserve(connectionId: string, now: number): boolean {
  if (!connectionId) return false;
  const recent = (quietReobserves.get(connectionId) ?? []).filter(
    (at) => now - at < QUIET_REOBSERVE_WINDOW_MS
  );
  const last = recent[recent.length - 1];
  const gap = QUIET_REOBSERVE_GAPS_MS[recent.length];
  const allowed = gap !== undefined && (last === undefined || now - last >= gap);
  if (allowed) recent.push(now);
  if (recent.length) quietReobserves.set(connectionId, recent);
  else quietReobserves.delete(connectionId);
  return allowed;
}

/** This app session saw `connectionId` verified: its computer was known to the workspace. */
export function noteConnectionVerified(connectionId: string): void {
  if (connectionId) verifiedConnections.add(connectionId);
}

/** Whether this app session saw `connectionId` verified. */
export function connectionVerifiedThisSession(connectionId: string): boolean {
  return verifiedConnections.has(connectionId);
}

/** Forget everything kept for a removed connection, drafts and last channel included. */
export function forgetConnectionMemory(connectionId: string): void {
  quietReobserves.delete(connectionId);
  verifiedConnections.delete(connectionId);
  forgetConnectionDrafts(connectionId);
  forgetLastChannel(connectionId);
}

// ---------------------------------------------------------------------------------------------
// Connect on arrival from a chat (live QA round 3, Q3-08)
// ---------------------------------------------------------------------------------------------
//
// SECURITY-SENSITIVE (human review). A chat's "Connect in Crew" navigates here with the connection
// to connect (`crewConnect`) beside its one-hop intent id. The click in the chat is the person's,
// so Crew connects that connection once, as the person, when `arrivalConnectDecision` allows it.
// "Once" is per intent id and survives a remount and a page reload of the same history entry
// (session storage), so Back to that entry, or a reload after a later Disconnect, never connects
// again. The renderer still never connects anything a person did not ask for.

/** The route-state key naming the connection a chat's "Connect in Crew" asks to connect. */
export const CREW_CONNECT_ROUTE_KEY = 'crewConnect';
/**
 * The one-hop intent id's key: `chatAccessRouteState()`'s, in `access/ChatConnectNote.tsx`. A test
 * pins the two spellings together.
 */
export const CHAT_ACCESS_INTENT_ROUTE_KEY = 'crewOpenChatAccess';
/** Where the consumed intent ids are kept across a reload of this window. */
export const ARRIVAL_CONNECT_STORAGE_KEY = 'crew:consumedConnectIntents';
/** The most consumed intent ids kept in storage; the oldest go first. */
const MAX_STORED_ARRIVAL_CONNECTS = 32;
/** An intent id is a UUID; anything much longer is not one of ours. */
const MAX_INTENT_ID_LENGTH = 128;

export interface ArrivalConnectIntent {
  intentId: string;
  connectionId: string;
}

/** The arriving "connect this" request in `state`, when it carries both an intent id and a connection. */
export function arrivalConnectIntent(state: unknown): ArrivalConnectIntent | null {
  if (!state || typeof state !== 'object') return null;
  const record = state as Record<string, unknown>;
  const intentId = record[CHAT_ACCESS_INTENT_ROUTE_KEY];
  const connectionId = record[CREW_CONNECT_ROUTE_KEY];
  if (typeof intentId !== 'string' || !intentId || intentId.length > MAX_INTENT_ID_LENGTH)
    return null;
  if (typeof connectionId !== 'string' || !connectionId) return null;
  return { intentId, connectionId };
}

const consumedArrivalConnects = new Set<string>();

function storedArrivalConnects(): string[] {
  try {
    const parsed: unknown = JSON.parse(
      window.sessionStorage.getItem(ARRIVAL_CONNECT_STORAGE_KEY) ?? '[]'
    );
    return Array.isArray(parsed)
      ? parsed.filter(
          (item): item is string =>
            typeof item === 'string' && item.length > 0 && item.length <= MAX_INTENT_ID_LENGTH
        )
      : [];
  } catch {
    return [];
  }
}

/** Whether the "connect this" request with `intentId` was already honoured (or declined). */
export function arrivalConnectConsumed(intentId: string): boolean {
  return consumedArrivalConnects.has(intentId) || storedArrivalConnects().includes(intentId);
}

/** Spend the "connect this" request with `intentId`: it never connects again. */
export function consumeArrivalConnect(intentId: string): void {
  if (!intentId) return;
  consumedArrivalConnects.add(intentId);
  try {
    const kept = storedArrivalConnects().filter((item) => item !== intentId);
    kept.push(intentId);
    window.sessionStorage.setItem(
      ARRIVAL_CONNECT_STORAGE_KEY,
      JSON.stringify(kept.slice(-MAX_STORED_ARRIVAL_CONNECTS))
    );
  } catch {
    // Storage refused: the in-memory copy still covers this page.
  }
}

/** Forget what this module remembers. Tests share one module instance per file. */
export function resetConnectionMemoryForTests(): void {
  quietReobserves.clear();
  verifiedConnections.clear();
  consumedArrivalConnects.clear();
  try {
    window.sessionStorage.removeItem(ARRIVAL_CONNECT_STORAGE_KEY);
  } catch {
    // No storage to clear.
  }
}
resetBetweenTests(resetConnectionMemoryForTests);

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
  loadConnections(): Promise<unknown>;
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
 * with a password or code prompt opens Sign in by itself, once per attempt. Connect resolves true
 * once the daemon accepted it, and never throws.
 *
 * The loss of live updates never calls `connect` (`useCrewController`'s loss handler):
 * re-dialling a dropped bridge is the daemon's (D-KEEPALIVE), which never follows a Disconnect,
 * wherever it was made.
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
  const connect = async (opts?: { userInitiated?: boolean }): Promise<boolean> => {
    const target = connectionId;
    const accepted = await act('connect', 'connect', async () => {
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
      return true;
    });
    return accepted === true;
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
