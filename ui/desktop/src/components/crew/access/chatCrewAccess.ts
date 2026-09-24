import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from 'react';
import {
  findSessionGrant,
  grantDestinationLabel,
  sessionGrantState,
  type CrewSessionGrant,
} from '../api/grants';
import { isRecord, optionalText } from '../api/parse';
import { crewHttp } from '../crewApi';
import { sanitizeDisplayText } from '../identity';
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
  | 'revoked'
  | 'expired';

export interface ChatCrewAccess {
  sessionId: string | null;
  state: ChatCrewAccessState;
  grant: CrewSessionGrant | null;
  /**
   * Where the chat posts: the channel's name as the person saw it when granting, else as the Crew
   * view last showed it, else "a channel in {workspace}", else "a Crew channel".
   */
  destination: string;
  /** The grant was stopped on this device and the workspace has not confirmed it yet. */
  unconfirmed: boolean;
  /** The daemon refuses this chat's turns: hold the composer with the reason beside it. */
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
}

function savedConnections(result: unknown): SavedConnection[] {
  const rows = isRecord(result) && Array.isArray(result.connections) ? result.connections : [];
  return rows.flatMap((row): SavedConnection[] => {
    if (!isRecord(row)) return [];
    const id = optionalText(row.id);
    return id ? [{ id, name: sanitizeDisplayText(row.name) }] : [];
  });
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
 * Look up the Crew grant of one chat across the saved connections: when the chat opens, whenever a
 * grant changes through any Biorouter surface, when a turn ends while the chat holds a grant, and
 * when the window regains focus (a grant changed from the CLI). It never polls; the one timer it
 * sets flips an active grant to expired at the moment the daemon's `expires_at` passes.
 */
export function useChatCrewAccess(sessionId: string | null | undefined): ChatCrewAccess {
  const id = sessionId || null;
  const [lookup, setLookup] = useState<Lookup | null>(null);
  const [nonce, setNonce] = useState(0);
  const [now, setNow] = useState(() => Date.now());
  const [, setMarks] = useState(0);
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

  useEffect(() => {
    if (!id) return;
    return onGrantsChanged((detail) => {
      if (detail.sessionId !== id) return;
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
  const grantState = grant ? sessionGrantState(grant, now) : null;
  const expiresAt = grant?.expires_at;

  useEffect(() => {
    if (grantState !== 'active' || typeof expiresAt !== 'number') return;
    const wait = expiresAt * 1000 - Date.now();
    if (wait > MAX_TIMER_MS) return;
    const timer = window.setTimeout(() => setNow(Date.now()), Math.max(0, wait) + 50);
    return () => window.clearTimeout(timer);
  }, [grantState, expiresAt]);

  const state: ChatCrewAccessState =
    !id || !current || current.failed ? 'unknown' : (grantState ?? 'none');
  const unconfirmed =
    state === 'revoked' && grant
      ? isUnconfirmedRevocation(grant.connection_id, grant.session_id)
      : false;

  useEffect(() => {
    if (!id) return;
    const owner = token.current;
    publish(id, owner, state);
    return () => publish(id, owner, null);
  }, [id, state]);

  const destination = useMemo(
    () => chatDestination(grant, current?.connections ?? []),
    [grant, current]
  );

  return {
    sessionId: id,
    state,
    grant,
    destination,
    unconfirmed,
    blocksComposer: state === 'revoked' || state === 'expired',
    refetch,
  };
}
