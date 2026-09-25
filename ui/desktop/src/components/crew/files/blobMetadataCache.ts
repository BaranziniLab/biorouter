import type { CrewBlob } from './AttachmentCard';

/**
 * What `blob.status` said about a finished shared file, kept for the renderer's lifetime so a card
 * that mounts again — the Files tab reopened, a message re-rendered as the page streams in, a
 * posted file's card replacing its "Sending…" stand-in — starts from the file's name and size
 * instead of a nameless "Attachment" with a dimmed Save (Q4-17).
 *
 * Only COMPLETE files are kept: a finished upload's record does not change (its name, size,
 * checksum, type and channel are fixed when it is published), so a kept answer cannot go stale. An
 * unfinished one can, and is always asked for again. The card still asks for every file it shows,
 * and a failed answer drops the kept one, so this only decides what is drawn while it asks.
 *
 * Keyed by connection AND file, so one connection's answer never names another's file. Bounded
 * (oldest out first). Display only: nothing here decides what may be saved, previewed or posted.
 */

const LIMIT = 500;
const entries = new Map<string, CrewBlob>();

const keyOf = (connectionId: string, blobId: string) => `${connectionId}:${blobId}`;

/** The kept answer for this file on this connection, or null. */
export function cachedBlob(connectionId: string, blobId: string): CrewBlob | null {
  return entries.get(keyOf(connectionId, blobId)) ?? null;
}

/** Keep a `blob.status` answer, if the file is complete and the answer is about it. */
export function rememberBlob(connectionId: string, blobId: string, blob: CrewBlob): void {
  if (!blob || blob.complete !== true || blob.id !== blobId) return;
  const key = keyOf(connectionId, blobId);
  // Re-inserted, so the newest answer is the last one out.
  entries.delete(key);
  entries.set(key, blob);
  while (entries.size > LIMIT) {
    const oldest = entries.keys().next().value;
    if (oldest === undefined) break;
    entries.delete(oldest);
  }
}

/** Drop the kept answer: asking again failed, so it is no longer known to hold. */
export function forgetBlob(connectionId: string, blobId: string): void {
  entries.delete(keyOf(connectionId, blobId));
}

/** Tests only: start from nothing. */
export function clearBlobCache(): void {
  entries.clear();
}

/** Tests only: how many answers are kept. */
export function blobCacheSize(): number {
  return entries.size;
}
