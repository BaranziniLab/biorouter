import { useCallback, useSyncExternalStore } from 'react';
import { listTransfers, type CrewTransfer } from '../crewTransfers';
import { transferStatePresentation } from '../state/crewStatus';

/** How often the one poller asks again while a transfer is moving. */
export const TRANSFER_POLL_MS = 2000;

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
  timer: ReturnType<typeof setTimeout> | null;
  /** The newest list request; an older answer never overwrites a newer one. */
  sequence: number;
  disposed: boolean;
}

/**
 * One entry per connection, shared by every surface that shows a transfer: the composer's
 * upload chips, each attachment card, the Files tab. That is what makes the poller ONE per
 * view (L13): the legacy layout ran a 2s interval per attachment card, forever. Here the first
 * subscriber lists once, the list is asked again every 2s only while a transfer is active, and
 * the last subscriber to leave stops everything and forgets the records.
 *
 * Transfer records are this computer's receipts. They move only while a transfer is active or
 * after an action on this computer, and every action here asks for a fresh list, so nothing
 * polls while everything is at rest.
 */
const entries = new Map<string, Entry>();

/** Moving bytes or finishing: offer Pause, and keep the poller running. */
export function isTransferActive(transfer: CrewTransfer): boolean {
  return transferStatePresentation(transfer).active;
}

function publish(entry: Entry, state: CrewTransfersState) {
  entry.state = state;
  for (const listener of [...entry.listeners]) listener();
}

function schedule(connectionId: string, entry: Entry) {
  if (entry.disposed || entry.timer) return;
  if (!entry.state.transfers.some(isTransferActive)) return;
  entry.timer = setTimeout(() => {
    entry.timer = null;
    void fetchTransfers(connectionId, entry);
  }, TRANSFER_POLL_MS);
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
      publish(entry, { transfers, loaded: true, error: '' });
      schedule(connectionId, entry);
    },
    (failure: unknown) => {
      if (!current()) return;
      publish(entry, { ...entry.state, loaded: true, error: failureText(failure) });
      schedule(connectionId, entry);
    }
  );
}

function subscribeTransfers(connectionId: string, listener: () => void): () => void {
  let entry = entries.get(connectionId);
  if (!entry) {
    entry = { state: IDLE, listeners: new Set(), timer: null, sequence: 0, disposed: false };
    entries.set(connectionId, entry);
    void fetchTransfers(connectionId, entry);
  }
  const subscribed = entry;
  subscribed.listeners.add(listener);
  return () => {
    subscribed.listeners.delete(listener);
    if (subscribed.listeners.size > 0) return;
    subscribed.disposed = true;
    if (subscribed.timer) clearTimeout(subscribed.timer);
    subscribed.timer = null;
    if (entries.get(connectionId) === subscribed) entries.delete(connectionId);
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

/** This connection's transfer records, from the one shared poller. */
export function useCrewTransfers(connectionId: string): CrewTransfersView {
  const subscribe = useCallback(
    (listener: () => void) =>
      connectionId ? subscribeTransfers(connectionId, listener) : () => undefined,
    [connectionId]
  );
  const read = useCallback(
    () => (connectionId ? (entries.get(connectionId)?.state ?? IDLE) : IDLE),
    [connectionId]
  );
  const state = useSyncExternalStore(subscribe, read, read);
  const refresh = useCallback(() => refreshCrewTransfers(connectionId), [connectionId]);
  return { ...state, refresh };
}
