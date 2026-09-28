import { useCallback, useSyncExternalStore } from 'react';
import { resetBetweenTests } from './draftStash';

/**
 * The channel whose sidebar row takes the focus once it is drawn and no dialog is open: the one
 * Create channel just made (UXN-7). Closing the dialog handed focus back to Add channel, its opener,
 * a step away from the channel the person had just made and was now looking at. Module scope, as
 * the dialog and the row are mounted apart; one channel at a time, the newest request winning.
 */
let pending: string | null = null;
const listeners = new Set<() => void>();

function notify(): void {
  for (const listener of [...listeners]) listener();
}

function subscribe(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

/** Focus `channelId`'s row once it is drawn and no dialog is open. */
export function focusChannelRowWhenShown(channelId: string): void {
  pending = channelId || null;
  notify();
}

/** The row took the focus it was asked to (or the request no longer applies). */
export function settleChannelRowFocus(channelId: string): void {
  if (pending !== channelId) return;
  pending = null;
  notify();
}

/** Whether `channelId`'s row is asked to take the focus. */
export function useChannelRowFocusRequested(channelId: string): boolean {
  const read = useCallback(() => pending !== null && pending === channelId, [channelId]);
  return useSyncExternalStore(subscribe, read, read);
}

resetBetweenTests(() => {
  pending = null;
});
