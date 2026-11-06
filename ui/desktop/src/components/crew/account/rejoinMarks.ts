import { useSyncExternalStore } from 'react';

/**
 * The saved connections whose own device the person removed from Keys (AO-5): until this computer
 * is added again, the connection bar offers Add this computer again for them, before any request
 * comes back refused. Memory only, per window: a reload learns it from the daemon's codes instead
 * (`crew_device_not_recognized`, `crew_device_key_missing`). Display only; the workspace decides.
 */
const marks = new Set<string>();
const listeners = new Set<() => void>();
let version = 0;

function changed(): void {
  version += 1;
  for (const listener of listeners) listener();
}

/** This computer's device was removed from the connection's account: it needs adding again. */
export function markNeedsRejoin(connectionId: string): void {
  if (!connectionId || marks.has(connectionId)) return;
  marks.add(connectionId);
  changed();
}

/** This computer was added again (or the connection is gone). */
export function clearNeedsRejoin(connectionId: string): void {
  if (marks.delete(connectionId)) changed();
}

export function needsRejoin(connectionId: string): boolean {
  return marks.has(connectionId);
}

export function resetRejoinMarksForTests(): void {
  marks.clear();
  changed();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Whether `connectionId` is marked, re-rendering when the marks change. */
export function useNeedsRejoin(connectionId: string): boolean {
  useSyncExternalStore(
    subscribe,
    () => version,
    () => version
  );
  return Boolean(connectionId) && marks.has(connectionId);
}
