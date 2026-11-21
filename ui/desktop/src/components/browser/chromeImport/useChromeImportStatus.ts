import { useEffect, useSyncExternalStore } from 'react';
import type { ChromeImportStatus } from '../../../browser/types';

/**
 * The Chrome import's status (BB-21, BB-25), shared by every browser tab and Settings: one read
 * per window until something changes it, so a second tab does not ask main again and a dismissal
 * or an import in one place hides the first use card everywhere.
 */

let current: ChromeImportStatus | null = null;
let pending: Promise<void> | null = null;
const listeners = new Set<() => void>();

function emit() {
  for (const listener of listeners) listener();
}

export function chromeImportBridge() {
  return typeof window === 'undefined' ? undefined : window.electron?.browserImport;
}

/** Read the status again from main. */
export function refreshChromeImportStatus(): Promise<void> {
  const bridge = chromeImportBridge();
  if (!bridge) return Promise.resolve();
  const request = bridge
    .status()
    .then((status) => {
      current = status;
      emit();
    })
    .catch(() => {})
    .finally(() => {
      if (pending === request) pending = null;
    });
  pending = request;
  return request;
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function useChromeImportStatus(): ChromeImportStatus | null {
  const snapshot = () => current;
  // The server snapshot is the same: a first render on a static page simply has no status yet.
  const status = useSyncExternalStore(subscribe, snapshot, snapshot);
  useEffect(() => {
    if (!current && !pending) void refreshChromeImportStatus();
  }, []);
  return status;
}

/** Tests only: forget what was read. */
export function resetChromeImportStatusForTests() {
  current = null;
  pending = null;
  emit();
}
