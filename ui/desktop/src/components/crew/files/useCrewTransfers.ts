import { useCallback, useSyncExternalStore } from 'react';
import { listTransfers, type CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';

/** How often the one poller asks again while a transfer is moving. */
export const TRANSFER_POLL_MS = 2000;
/**
 * How often it asks while nothing moves but a transfer is paused. A paused transfer is this
 * computer's, but the command line can resume, pause or remove it too (`biorouter crew files`),
 * and the list used to keep "Paused 17%" until the view mounted again (R-8).
 */
export const TRANSFER_PAUSED_POLL_MS = 5000;
/**
 * How often it asks while a surface watches every record (the Files tab) and nothing moves or is
 * paused. The command line on this computer can resume, forget or start a transfer, which nothing
 * here hears: a list that asked only while a transfer moved kept showing "Failed" while the
 * command line uploaded the same file (RES2-N3).
 */
export const TRANSFER_WATCH_POLL_MS = 10_000;

export interface CrewTransfersState {
  /** Every transfer record this computer keeps for the connection, both directions. */
  transfers: readonly CrewTransfer[];
  /** True once the first list answered (or failed). */
  loaded: boolean;
  /** The last list failure, cleared by the next success. The records keep their last value. */
  error: string;
}

const IDLE: CrewTransfersState = { transfers: [], loaded: false, error: '' };

interface Entry {
  state: CrewTransfersState;
  listeners: Set<() => void>;
  /** How many subscribers watch every record, so the list is asked again even when idle. */
  watchers: number;
  timer: ReturnType<typeof setTimeout> | null;
  /** The newest list request; an older answer never overwrites a newer one. */
  sequence: number;
  disposed: boolean;
}

/**
 * One entry per connection, shared by every surface that shows a transfer: the composer's
 * upload chips, each attachment card, the Files tab. That is what makes the poller ONE per
 * view (L13): the legacy layout ran a 2s interval per attachment card, forever. Here the first
 * subscriber lists once, the list is asked again every 2s while a transfer is active and every
 * 5s while one is only paused, and the last subscriber to leave stops everything and forgets the
 * records.
 *
 * Transfer records are this computer's receipts. They move while a transfer is active, after an
 * action here (each asks for a fresh list), and after an action from the command line on this
 * computer, which nothing here hears: so a paused transfer keeps the poller going slowly, the
 * window coming back to the front asks again (R-8), and a surface that watches every record (the
 * Files tab) keeps it going slower still (RES2-N3). Nothing else polls once every transfer has
 * finished.
 */
const entries = new Map<string, Entry>();

/** Moving bytes or finishing: offer Pause, and keep the poller running. */
export function isTransferActive(transfer: CrewTransfer): boolean {
  return transferStatePresentation(transfer).active;
}

/** Stopped but resumable, from here or from the command line: keep the poller running slowly. */
function isTransferPaused(transfer: CrewTransfer): boolean {
  return transferStatePresentation(transfer).key === 'paused';
}

/**
 * When the poller asks next, or `null` when nothing can change without an action here and no
 * surface watches every record.
 */
function pollDelay(transfers: readonly CrewTransfer[], watched: boolean): number | null {
  if (transfers.some(isTransferActive)) return TRANSFER_POLL_MS;
  if (transfers.some(isTransferPaused)) return TRANSFER_PAUSED_POLL_MS;
  return watched ? TRANSFER_WATCH_POLL_MS : null;
}

/** The window came back to the front: every connection shown asks again. */
function refreshAllOnReturn() {
  if (typeof document !== 'undefined' && document.visibilityState === 'hidden') return;
  for (const [connectionId, entry] of entries) void fetchTransfers(connectionId, entry);
}

let listeningForReturn = false;
function listenForReturn(listen: boolean) {
  if (typeof window === 'undefined' || listen === listeningForReturn) return;
  listeningForReturn = listen;
  if (listen) {
    window.addEventListener('focus', refreshAllOnReturn);
    document.addEventListener('visibilitychange', refreshAllOnReturn);
  } else {
    window.removeEventListener('focus', refreshAllOnReturn);
    document.removeEventListener('visibilitychange', refreshAllOnReturn);
  }
}

function publish(entry: Entry, state: CrewTransfersState) {
  entry.state = state;
  for (const listener of [...entry.listeners]) listener();
}

function schedule(connectionId: string, entry: Entry) {
  if (entry.disposed || entry.timer) return;
  const delay = pollDelay(entry.state.transfers, entry.watchers > 0);
  if (delay === null) return;
  entry.timer = setTimeout(() => {
    entry.timer = null;
    void fetchTransfers(connectionId, entry);
  }, delay);
}

function failureText(failure: unknown): string {
  return failure instanceof Error && failure.message ? failure.message : 'Transfer list failed';
}

function fetchTransfers(connectionId: string, entry: Entry): Promise<void> {
  if (entry.timer) {
    clearTimeout(entry.timer);
    entry.timer = null;
  }
  const sequence = ++entry.sequence;
  const current = () => !entry.disposed && sequence === entry.sequence;
  return listTransfers(connectionId).then(
    (transfers) => {
      if (!current()) return;
      // An answer that is not a list is a failed list, not an empty one: keep the last records.
      if (!Array.isArray(transfers)) {
        publish(entry, { ...entry.state, loaded: true, error: failureText(null) });
      } else {
        const records = transfers.filter(
          (item): item is CrewTransfer =>
            typeof item === 'object' && item !== null && typeof item.id === 'string'
        );
        publish(entry, { transfers: records, loaded: true, error: '' });
      }
      schedule(connectionId, entry);
    },
    (failure: unknown) => {
      if (!current()) return;
      publish(entry, { ...entry.state, loaded: true, error: failureText(failure) });
      schedule(connectionId, entry);
    }
  );
}

function subscribeTransfers(
  connectionId: string,
  listener: () => void,
  watch: boolean
): () => void {
  let entry = entries.get(connectionId);
  const fresh = !entry;
  if (!entry) {
    entry = {
      state: IDLE,
      listeners: new Set(),
      watchers: 0,
      timer: null,
      sequence: 0,
      disposed: false,
    };
    entries.set(connectionId, entry);
    listenForReturn(true);
  }
  const subscribed = entry;
  subscribed.listeners.add(listener);
  if (watch) subscribed.watchers += 1;
  if (fresh) void fetchTransfers(connectionId, subscribed);
  // A watcher joining an idle list starts its slow poll; the first list's answer schedules it.
  else if (watch && subscribed.state.loaded) schedule(connectionId, subscribed);
  return () => {
    subscribed.listeners.delete(listener);
    if (watch) subscribed.watchers -= 1;
    if (subscribed.listeners.size > 0) return;
    subscribed.disposed = true;
    if (subscribed.timer) clearTimeout(subscribed.timer);
    subscribed.timer = null;
    if (entries.get(connectionId) === subscribed) entries.delete(connectionId);
    if (entries.size === 0) listenForReturn(false);
  };
}

/**
 * Ask for a fresh list now (after starting, pausing, resuming or removing a transfer). It also
 * restarts the poller when the answer shows an active transfer. Resolves once the answer is
 * applied. Nothing happens for a connection no surface is showing.
 */
export function refreshCrewTransfers(connectionId: string): Promise<void> {
  const entry = connectionId ? entries.get(connectionId) : undefined;
  return entry ? fetchTransfers(connectionId, entry) : Promise.resolve();
}

export interface CrewTransfersView extends CrewTransfersState {
  refresh(): Promise<void>;
}

/**
 * This connection's transfer records, from the one shared poller. `watch`: the surface shows every
 * record, so the list is asked again every {@link TRANSFER_WATCH_POLL_MS} even while nothing moves
 * (the Files tab, RES2-N3).
 */
export function useCrewTransfers(
  connectionId: string,
  options: { watch?: boolean } = {}
): CrewTransfersView {
  const watch = options.watch === true;
  const subscribe = useCallback(
    (listener: () => void) =>
      connectionId ? subscribeTransfers(connectionId, listener, watch) : () => undefined,
    [connectionId, watch]
  );
  const read = useCallback(
    () => (connectionId ? (entries.get(connectionId)?.state ?? IDLE) : IDLE),
    [connectionId]
  );
  const state = useSyncExternalStore(subscribe, read, read);
  const refresh = useCallback(() => refreshCrewTransfers(connectionId), [connectionId]);
  return { ...state, refresh };
}
