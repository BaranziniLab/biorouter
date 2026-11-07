/**
 * The private draft flag (DD-71, "The private draft flag" in
 * `docs/dictation/surfaces.md`).
 *
 * A composer's draft is flagged when a transcript inserted into it reports
 * `bias_scope: "private"`: by the composer for text that went into its field,
 * and by `dictationSession.ts` for text that went into the stored draft of a
 * composer that unmounted while the person spoke. The flag is cleared when the
 * draft is sent or emptied (the box and the stored draft both). Before a flagged draft leaves for a chat whose model is public, the
 * composer asks MSG-PRIVATE-DRAFT; before the model switcher moves an unsent
 * chat holding one to a public model, it asks MSG-PRIVATE-DRAFT-SWITCH. The
 * person's say so proceeds. No agent path reads or answers either question:
 * the flag lives in this window's memory, keyed by the composer's draft key,
 * and is never written to disk, sent to the daemon or logged.
 *
 * `bias_scope: "private"` means private entries were eligible for that
 * recording, not that a private term is in the text, and the messages say so.
 */
import { useCallback, useSyncExternalStore } from 'react';

const flagged = new Set<string>();
const listeners = new Set<() => void>();

function changed(): void {
  for (const listener of [...listeners]) listener();
}

/** A transcript dictated with private scope went into `key`'s draft. */
export function markPrivateDraft(key: string): void {
  if (flagged.has(key)) return;
  flagged.add(key);
  changed();
}

/** The draft was sent, or emptied. */
export function clearPrivateDraft(key: string): void {
  if (!flagged.delete(key)) return;
  changed();
}

export function isPrivateDraft(key: string | undefined | null): boolean {
  return !!key && flagged.has(key);
}

export function subscribePrivateDraft(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Whether `key`'s draft is flagged, for a component; the same reader serves a server render. */
export function usePrivateDraft(key: string | undefined | null): boolean {
  const read = useCallback(() => isPrivateDraft(key), [key]);
  return useSyncExternalStore(subscribePrivateDraft, read, read);
}

/**
 * Whether a send to a model of `tier` must ask first. A tier that is not known
 * to be private asks: the stricter reading of "a public model", because an
 * unresolved tier is not evidence of a private one.
 */
export function privateDraftNeedsAsking(
  key: string | undefined | null,
  tier: 'public' | 'private' | undefined
): boolean {
  return isPrivateDraft(key) && tier !== 'private';
}

/** For tests. */
export function resetPrivateDraftsForTests(): void {
  flagged.clear();
  listeners.clear();
}
