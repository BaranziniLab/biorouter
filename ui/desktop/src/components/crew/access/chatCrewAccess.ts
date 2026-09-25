import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import {
  findSessionGrant,
  grantDestinationLabel,
  sessionGrantState,
  type CrewSessionGrant,
} from '../api/grants';
import { isRecord, optionalText } from '../api/parse';
import { CrewHttpError, crewHttp } from '../crewApi';
import { sanitizeDisplayText } from '../identity';
import {
  CONNECT_FAILURE_CODES,
  classifyConnectFailure,
  isMembershipEnded,
} from '../state/connectFailure';
import { accessCopy } from './copy';
import { isUnconfirmedRevocation, onGrantsChanged } from './useCrewGrants';

/**
 * A chat's Crew access, seen from the ordinary chat (outside the Crew view).
 *
 * The daemon already refuses every turn of a chat whose grant was revoked or ran out ("Crew run was
 * revoked; request a fresh human grant"), and it keeps refusing whatever this module says. What was
 * missing was a person-readable reason in the chat itself, so the chat can say why before the next
 * turn fails, offer the two ways forward, and show — while access is active — that the chat is
 * connected and where Revoke is. Nothing here authorizes anything: a lookup that fails renders
 * nothing and blocks nothing.
 */

export type ChatCrewAccessState =
  /** Not looked up yet, or the lookup failed: show nothing, block nothing. */
  | 'unknown'
  /** No saved Crew connection holds a grant for this chat. */
  | 'none'
  | 'active'
  /**
   * The grant is active, but the connection it was made on is disconnected: the chat's next turn
   * fails with "Crew connection is disconnected" until the person connects in Crew (Q2-08). It
   * holds nothing — connecting is all it takes — and for the extension menu it is still active.
   */
  | 'offline'
  | 'revoked'
  | 'expired'
  /**
   * A task's grant after the task: it ended with the task (T-25), which is how a task that did its
   * work ends, so it is not "revoked" and nothing went wrong (Q2-09). The chat is held all the same.
   */
  | 'finished';

export interface ChatCrewAccess {
  sessionId: string | null;
  state: ChatCrewAccessState;
  grant: CrewSessionGrant | null;
  /**
   * Where the chat posts: the channel's name as the person saw it when granting, else as the Crew
   * view last showed it, else "a channel in {workspace}", else "a Crew channel".
   */
  destination: string;
  /**
   * Why an `offline` chat's connection is down: `network` when the daemon's last answer (or the
   * computer itself) says the network failed — the daemon dials such a drop again by itself once
   * the network is back (Q4-01) — else `other`. `null` unless `state` is `offline`.
   */
  offlineCause: 'network' | 'other' | null;
  /** The grant was stopped on this device and the workspace has not confirmed it yet. */
  unconfirmed: boolean;
  /**
   * The daemon refuses this chat's turns: hold the composer with the reason beside it. True for
   * `revoked`, `expired` and `finished`, including the moment after a revoke this window saw
   * confirmed and before the daemon's list says so.
   */
  blocksComposer: boolean;
  /** Look the grant up again now. */
  refetch(): void;
}

// ── Channel names seen in the Crew view ──────────────────────────────────────────────────────
// The grant list names a channel by ID only, and the ordinary chat has no workspace snapshot to
// read its name from. The Crew view records the labels of channels that hold grants while it shows
// them; the chat reads them back. Process memory, never persisted: a channel name is workspace
// content, and after a restart the chat names the workspace instead until Crew is opened again.

const channelLabelMemory = new Map<string, string>();
const MAX_REMEMBERED_LABELS = 200;
const labelKey = (connectionId: string, channelId: string) => `${connectionId}\n${channelId}`;

/** Record the label of each channel a grant names, as the Crew view shows it. */
export function rememberChannelLabels(
  connectionId: string,
  labels: ReadonlyMap<string, string>,
  channelIds: Iterable<string>
): void {
  for (const channelId of channelIds) {
    const label = labels.get(channelId);
    if (!label) continue;
    const key = labelKey(connectionId, channelId);
    channelLabelMemory.delete(key);
    channelLabelMemory.set(key, label);
  }
  while (channelLabelMemory.size > MAX_REMEMBERED_LABELS) {
    const oldest = channelLabelMemory.keys().next().value;
    if (oldest === undefined) break;
    channelLabelMemory.delete(oldest);
  }
}

export function rememberedChannelLabel(connectionId: string, channelId: string): string | null {
  return channelLabelMemory.get(labelKey(connectionId, channelId)) ?? null;
}

/** For tests: forget every remembered channel label. */
export function forgetChannelLabels(): void {
  channelLabelMemory.clear();
}

// ── The per-chat answer, shared with controls that must not fetch on their own ──────────────
// The chat's extension menu reads this so its Crew row can refuse to look switched off while the
// grant stays active. It only ever reads what the chat's own lookup published.

type Listener = () => void;
const published = new Map<string, { token: object; state: ChatCrewAccessState }>();

/**
 * What the extension menu is told: whether the grant stands, not how the bar words it. An offline
 * chat's grant is still active — switching the Crew extension off would not end it — so the menu
 * keeps the Crew row locked on.
 */
function publishedState(state: ChatCrewAccessState): ChatCrewAccessState {
  return state === 'offline' ? 'active' : state;
}
const listeners = new Set<Listener>();

function publish(sessionId: string, token: object, state: ChatCrewAccessState | null) {
  const current = published.get(sessionId);
  if (state === null) {
    if (current?.token !== token) return;
    published.delete(sessionId);
  } else {
    if (current?.token === token && current.state === state) return;
    published.set(sessionId, { token, state });
  }
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: Listener) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/**
 * The Crew access state the chat for `sessionId` last looked up, or `null` when no chat for it is
 * open. Never fetches.
 */
export function useChatCrewAccessState(
  sessionId: string | null | undefined
): ChatCrewAccessState | null {
  return useSyncExternalStore(
    subscribe,
    () => (sessionId ? (published.get(sessionId)?.state ?? null) : null),
    () => null
  );
}

/** The extension key the Crew platform extension is registered under. */
export function isCrewExtensionName(name: string): boolean {
  return name.trim().toLowerCase() === 'crew';
}

// ── The lookup ───────────────────────────────────────────────────────────────────────────────

interface SavedConnection {
  id: string;
  name: string;
  /** The daemon's word for the connection now (`connected`, `disconnected`), when it gave one. */
  status?: string;
  /** Why it is down, in the daemon's words (`last_error`). Never shown: only classified. */
  lastError?: string;
  /** The typed reason beside `last_error`, when the daemon has one (`last_error_code`). */
  lastErrorCode?: string;
}

function savedConnections(result: unknown): SavedConnection[] {
  const rows = isRecord(result) && Array.isArray(result.connections) ? result.connections : [];
  return rows.flatMap((row): SavedConnection[] => {
    if (!isRecord(row)) return [];
    const id = optionalText(row.id);
    if (!id) return [];
    const connection: SavedConnection = { id, name: sanitizeDisplayText(row.name) };
    const status = optionalText(row.status);
    if (status) connection.status = status;
    const lastError = optionalText(row.last_error);
    if (lastError) connection.lastError = lastError;
    const lastErrorCode = optionalText(row.last_error_code);
    if (lastErrorCode) connection.lastErrorCode = lastErrorCode;
    return [connection];
  });
}

/** Whether the daemon says the connection `connectionId` is down (only an explicit answer). */
function isDisconnected(connections: readonly SavedConnection[], connectionId: string): boolean {
  return connections.find((item) => item.id === connectionId)?.status === 'disconnected';
}

/**
 * Why a disconnected connection is down, for the chat's offline bar (live QA round 4, Q4-06).
 * `network` only when something says the network failed: the daemon's last answer classifies as
 * unreachable (`classifyConnectFailure`, the Crew view's own reading), or this computer has no
 * network at all (`navigator.onLine`) and the answer carries no typed reason of another kind. The
 * daemon dials such a drop again by itself for up to an hour (Q4-01).
 *
 * `other` for everything nothing will reconnect by itself, so the bar keeps "until you connect": a
 * connection with no last answer (a person's Disconnect, a restart), a membership the workspace
 * ended, and every sign-in, host-key or other failure. The daemon's saved answer for an SSH drop is
 * its untyped message ("Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]…"), which the
 * classifier reads as sign-in, as it must for a daemon without codes: a network drop is recognised
 * from it only by a typed `crew_ssh_unreachable` or by the computer being offline.
 */
export function offlineCauseOf(
  connection: Pick<SavedConnection, 'lastError' | 'lastErrorCode'> | undefined,
  online: boolean
): 'network' | 'other' {
  if (!connection?.lastError) return 'other';
  const code = connection.lastErrorCode;
  if (isMembershipEnded({ last_error_code: code })) return 'other';
  const failure = classifyConnectFailure(new CrewHttpError(connection.lastError, 0, code));
  if (failure.kind === 'unreachable') return 'network';
  const typed =
    code !== undefined && Object.prototype.hasOwnProperty.call(CONNECT_FAILURE_CODES, code);
  return !online && !typed ? 'network' : 'other';
}

function subscribeOnline(listener: () => void) {
  window.addEventListener('online', listener);
  window.addEventListener('offline', listener);
  return () => {
    window.removeEventListener('online', listener);
    window.removeEventListener('offline', listener);
  };
}

const readOnline = () => typeof navigator === 'undefined' || navigator.onLine !== false;

/** Whether this computer says it has a network: an answer that changes, so it is watched. */
function useOnline(): boolean {
  return useSyncExternalStore(subscribeOnline, readOnline, () => true);
}

/** A revoke this window saw land, for one grant: its chat, connection and run. */
interface SeenRevoke {
  sessionId: string;
  connectionId: string;
  runId: string;
}

interface Lookup {
  sessionId: string;
  failed: boolean;
  connections: SavedConnection[];
  grant: CrewSessionGrant | null;
}

/**
 * Where a grant posts, as precisely as this computer can name it: the label the daemon recorded
 * when the person granted access, then the label the Crew view last showed for the channel, then
 * the workspace (the recorded name first, then the saved connection's), then "a Crew channel".
 */
export function chatDestination(
  grant: CrewSessionGrant | null,
  connections: readonly SavedConnection[] = []
): string {
  if (!grant) return accessCopy.chatDestinationUnknown;
  const recorded = sanitizeDisplayText(grantDestinationLabel(grant));
  if (recorded) return recorded;
  const remembered = rememberedChannelLabel(grant.connection_id, grant.channel_id);
  if (remembered) return remembered;
  const workspace =
    sanitizeDisplayText(grant.labels?.workspace) ||
    connections.find((connection) => connection.id === grant.connection_id)?.name;
  return workspace
    ? accessCopy.chatDestinationWorkspace(workspace)
    : accessCopy.chatDestinationUnknown;
}

const MAX_TIMER_MS = 2_147_483_647;

/**
 * How often a watched chat that holds a grant re-reads the saved connections (Q3-04): an outage
 * shows as the offline bar within this long, with no focus event needed.
 */
export const CONNECTION_WATCH_MS = 15_000;

/** Whether two reads of the saved connections say the same thing about every connection. */
function sameConnections(a: readonly SavedConnection[], b: readonly SavedConnection[]): boolean {
  return (
    a.length === b.length &&
    a.every(
      (item, index) =>
        item.id === b[index].id &&
        item.name === b[index].name &&
        item.status === b[index].status &&
        item.lastError === b[index].lastError &&
        item.lastErrorCode === b[index].lastErrorCode
    )
  );
}

/**
 * Look up the Crew grant of one chat across the saved connections: when the chat opens, whenever a
 * grant changes through any Biorouter surface, when a turn ends while the chat holds a grant, and
 * when the window regains focus (a grant changed from the CLI). The one timer that flips an active
 * grant to expired fires at the moment the daemon's `expires_at` passes.
 *
 * While the chat holds a grant that stands (active or offline) and the window is visible, it also
 * re-reads the saved connections — only `GET /crew/connections`, never the grants — every
 * {@link CONNECTION_WATCH_MS}, and at once when the network comes or goes or the window becomes
 * visible again. Live QA round 3 (Q3-04) left an open, focused chat showing its live "Crew ·
 * #general" chip and Revoke for six minutes into an outage, because nothing read the connection
 * again until the window was refocused. A chat without a grant, or a hidden window, reads nothing
 * on a timer. A read that fails changes nothing: a missed read is not an outage.
 */
export function useChatCrewAccess(sessionId: string | null | undefined): ChatCrewAccess {
  const id = sessionId || null;
  const [lookup, setLookup] = useState<Lookup | null>(null);
  const [nonce, setNonce] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [, setMarks] = useState(0);
  const [seenRevoke, setSeenRevoke] = useState<SeenRevoke | null>(null);
  const shownGrant = useRef<CrewSessionGrant | null>(null);
  const token = useRef({});
  const refetch = useCallback(() => setNonce((value) => value + 1), []);

  useEffect(() => {
    if (!id) return;
    const controller = new AbortController();
    void (async () => {
      let next: Lookup;
      try {
        const connections = savedConnections(
          await crewHttp<unknown>('/connections', 'GET', undefined, controller.signal)
        );
        const grant = connections.length
          ? await findSessionGrant(
              connections.map((connection) => connection.id),
              id,
              controller.signal
            )
          : null;
        next = { sessionId: id, failed: false, connections, grant };
      } catch {
        next = { sessionId: id, failed: true, connections: [], grant: null };
      }
      if (controller.signal.aborted) return;
      setNow(Date.now());
      setLookup(next);
    })();
    return () => controller.abort();
  }, [id, nonce]);

  const current = lookup && lookup.sessionId === id ? lookup : null;
  const hasConnections = Boolean(current && current.connections.length > 0);
  const hasGrant = Boolean(current?.grant);

  // A revoke of this chat's grant, from this chat's own bar or any other surface, holds the chat
  // at once: the daemon's list is read again, but until it answers the chat must not look usable,
  // and Enter must say why (live QA round 2, Q2-73). Only a revoke that stopped the grant counts —
  // a refused one leaves it active — and only for the grant shown when it landed: a grant made
  // afterwards is a new run, and a 'granted' announcement forgets it outright.
  useEffect(() => {
    if (!id) return;
    return onGrantsChanged((detail) => {
      if (detail.sessionId !== id) return;
      if (detail.change === 'revoked' || detail.change === 'unconfirmed') {
        const shown = shownGrant.current;
        if (shown && shown.connection_id === detail.connectionId)
          setSeenRevoke({ sessionId: id, connectionId: shown.connection_id, runId: shown.run_id });
      } else if (detail.change === 'granted') {
        setSeenRevoke(null);
      }
      setMarks((value) => value + 1);
      refetch();
    });
  }, [id, refetch]);

  useEffect(() => {
    if (!id || !hasConnections) return;
    const onFocus = () => refetch();
    window.addEventListener('focus', onFocus);
    return () => window.removeEventListener('focus', onFocus);
  }, [id, hasConnections, refetch]);

  useEffect(() => {
    if (!id || !hasGrant) return;
    const onTurnFinished = () => refetch();
    window.addEventListener('message-stream-finished', onTurnFinished);
    return () => window.removeEventListener('message-stream-finished', onTurnFinished);
  }, [id, hasGrant, refetch]);

  const grant = current?.grant ?? null;
  const grantConnectionId = grant?.connection_id ?? null;
  useEffect(() => {
    shownGrant.current = grant;
  }, [grant]);
  const revokedHere = Boolean(
    grant &&
    seenRevoke &&
    seenRevoke.sessionId === id &&
    seenRevoke.connectionId === grant.connection_id &&
    seenRevoke.runId === grant.run_id
  );
  const listedState = grant ? sessionGrantState(grant, now) : null;
  const grantState = listedState === 'active' && revokedHere ? 'revoked' : listedState;
  const expiresAt = grant?.expires_at;

  useEffect(() => {
    if (grantState !== 'active' || typeof expiresAt !== 'number') return;
    const wait = expiresAt * 1000 - Date.now();
    if (wait > MAX_TIMER_MS) return;
    const timer = window.setTimeout(() => setNow(Date.now()), Math.max(0, wait) + 50);
    return () => window.clearTimeout(timer);
  }, [grantState, expiresAt]);

  const unconfirmed =
    grantState === 'revoked' && grant
      ? isUnconfirmedRevocation(grant.connection_id, grant.session_id)
      : false;
  let state: ChatCrewAccessState;
  if (!id || !current || current.failed) state = 'unknown';
  else if (!grant || !grantState) state = 'none';
  else if (grantState === 'active')
    state = isDisconnected(current.connections, grant.connection_id) ? 'offline' : 'active';
  // A task's grant that stopped only on this device is still a revoke to confirm, with Retry.
  else if (grant.kind === 'task' && !unconfirmed) state = 'finished';
  else state = grantState;

  // Watch the connection while the grant stands and the chat is watched (Q3-04).
  const watching = Boolean(id) && (state === 'active' || state === 'offline');
  useEffect(() => {
    if (!id || !watching || !grantConnectionId) return;
    let timer: number | undefined;
    let reading: AbortController | null = null;
    const visible = () => document.visibilityState === 'visible';
    const read = () => {
      reading?.abort();
      const request = new AbortController();
      reading = request;
      void crewHttp<unknown>('/connections', 'GET', undefined, request.signal).then(
        (result) => {
          if (request.signal.aborted) return;
          const connections = savedConnections(result);
          // The grant's connection is gone from the list: read the grant again, as on open.
          if (!connections.some((item) => item.id === grantConnectionId)) {
            refetch();
            return;
          }
          setLookup((previous) =>
            previous &&
            previous.sessionId === id &&
            !previous.failed &&
            !sameConnections(previous.connections, connections)
              ? { ...previous, connections }
              : previous
          );
        },
        () => undefined
      );
    };
    const stop = () => {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
    };
    const start = () => {
      stop();
      if (visible()) timer = window.setInterval(read, CONNECTION_WATCH_MS);
    };
    const onVisibility = () => {
      if (visible()) {
        read();
        start();
      } else {
        stop();
        reading?.abort();
      }
    };
    const onNetwork = () => {
      if (visible()) read();
    };
    start();
    document.addEventListener('visibilitychange', onVisibility);
    window.addEventListener('online', onNetwork);
    window.addEventListener('offline', onNetwork);
    return () => {
      stop();
      reading?.abort();
      document.removeEventListener('visibilitychange', onVisibility);
      window.removeEventListener('online', onNetwork);
      window.removeEventListener('offline', onNetwork);
    };
  }, [id, watching, grantConnectionId, refetch]);

  const shared = publishedState(state);
  useEffect(() => {
    if (!id) return;
    const owner = token.current;
    publish(id, owner, shared);
    return () => publish(id, owner, null);
  }, [id, shared]);

  const destination = useMemo(
    () => chatDestination(grant, current?.connections ?? []),
    [grant, current]
  );
  const online = useOnline();
  const offlineCause =
    state === 'offline' && grant
      ? offlineCauseOf(
          current?.connections.find((item) => item.id === grant.connection_id),
          online
        )
      : null;

  return {
    sessionId: id,
    state,
    grant,
    destination,
    offlineCause,
    unconfirmed,
    blocksComposer: state === 'revoked' || state === 'expired' || state === 'finished',
    refetch,
  };
}
