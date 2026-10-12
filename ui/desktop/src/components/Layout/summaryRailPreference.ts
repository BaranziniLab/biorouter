import { useCallback, useSyncExternalStore } from 'react';
import type { SummaryRailPreference } from './yieldLadder';

/**
 * Whether this viewer wants the docked Chat summary rail: `open` (the default)
 * or `closed`.
 *
 * ONE preference for the viewer, not one per chat, for two reasons. `BaseChat`
 * remounts on every tab switch, so component state would reset each time; and
 * the person's own click must win over every later rule, the lesson the
 * sidebar's auto-collapse taught (`yieldLadder.ts`, rung 1). With `open`, the
 * rail is simply not drawn for an empty chat and appears when the first turn
 * starts; a person who closed it is never overridden, and new to-do items do not
 * force it open.
 *
 * Every mounted chat (split panes) reads the same module store through
 * `useSyncExternalStore`, and the `storage` event carries a change to a second
 * window. Every storage access is guarded: a blocked or full store reads as the
 * default and drops the write, and the in-memory value still stands.
 */
export const SUMMARY_RAIL_STORAGE_KEY = 'biorouter:summary-rail';

export const SUMMARY_RAIL_DEFAULT_PREFERENCE: SummaryRailPreference = 'open';

type PreferenceStorage = Pick<Storage, 'getItem' | 'setItem'>;

function defaultStorage(): PreferenceStorage | null {
  try {
    return typeof window === 'undefined' ? null : window.localStorage;
  } catch {
    // Accessing localStorage itself throws when site data is blocked.
    return null;
  }
}

function parsePreference(value: string | null): SummaryRailPreference {
  return value === 'closed' ? 'closed' : SUMMARY_RAIL_DEFAULT_PREFERENCE;
}

export function readSummaryRailPreference(
  storage: PreferenceStorage | null = defaultStorage()
): SummaryRailPreference {
  if (!storage) return SUMMARY_RAIL_DEFAULT_PREFERENCE;
  try {
    return parsePreference(storage.getItem(SUMMARY_RAIL_STORAGE_KEY));
  } catch {
    return SUMMARY_RAIL_DEFAULT_PREFERENCE;
  }
}

export function writeSummaryRailPreference(
  value: SummaryRailPreference,
  storage: PreferenceStorage | null = defaultStorage()
): void {
  if (!storage) return;
  try {
    storage.setItem(SUMMARY_RAIL_STORAGE_KEY, value);
  } catch {
    // Storage is full or unavailable; the in-memory preference still stands.
  }
}

// ---- The module store ------------------------------------------------------

let current: SummaryRailPreference | null = null;
const listeners = new Set<() => void>();

function snapshot(): SummaryRailPreference {
  if (current === null) current = readSummaryRailPreference();
  return current;
}

function emit(): void {
  for (const listener of listeners) listener();
}

function onStorage(event: StorageEvent): void {
  if (event.key !== null && event.key !== SUMMARY_RAIL_STORAGE_KEY) return;
  const next =
    event.key === null ? SUMMARY_RAIL_DEFAULT_PREFERENCE : parsePreference(event.newValue);
  if (next === current) return;
  current = next;
  emit();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  if (listeners.size === 1 && typeof window !== 'undefined') {
    window.addEventListener('storage', onStorage);
  }
  return () => {
    listeners.delete(listener);
    if (listeners.size === 0 && typeof window !== 'undefined') {
      window.removeEventListener('storage', onStorage);
    }
  };
}

/** Set the viewer's preference: every mounted chat, and the storage, follow. */
export function setSummaryRailPreference(value: SummaryRailPreference): void {
  if (snapshot() === value) return;
  current = value;
  writeSummaryRailPreference(value);
  emit();
}

/** The viewer's preference and its setter, shared by every mounted chat. */
export function useSummaryRailPreference(): [
  SummaryRailPreference,
  (value: SummaryRailPreference) => void,
] {
  const preference = useSyncExternalStore(
    subscribe,
    snapshot,
    () => SUMMARY_RAIL_DEFAULT_PREFERENCE
  );
  const set = useCallback((value: SummaryRailPreference) => setSummaryRailPreference(value), []);
  return [preference, set];
}

/** Tests only: forget the cached value so the next read goes to storage. */
export function resetSummaryRailPreferenceForTests(): void {
  current = null;
}
