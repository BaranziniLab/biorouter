import { useEffect, useState } from 'react';
import type { OpeningProgress } from './groupMessages';

/**
 * The live tail counts as arrived once it has stopped growing for this long.
 * The observer sends the backlog one message per frame, each behind a few broker
 * round trips, and once caught up it waits two seconds between polls — so a
 * pause this long means the backlog is in, on any link but a very slow one.
 */
export const OPENING_QUIET_MS = 1500;
/**
 * …or for this long, while the snapshot proves messages are still on their way
 * (`openingProgress` says `streaming`). Only a stalled stream reaches it: the
 * list then settles with what it holds rather than staying busy for good.
 */
export const OPENING_STALL_MS = 10_000;

export interface OpeningInput {
  /** The page being shown: the live tail, or an older page's boundary. */
  loadKey: string;
  /** The list on screen is this page's, loaded (`pageReady` in the timeline). */
  pageReady: boolean;
  /** The list was emptied for a reload: what the load had arrived at no longer holds. */
  reloading: boolean;
  /** `openingProgress` for the live tail; `complete` for an older page, which lands whole. */
  progress: OpeningProgress;
  /** The list's length and its newest message: what changes when a message arrives. */
  size: number;
  newestId: string | null;
}

/**
 * Whether the page's opening has arrived: an older page as soon as it lands,
 * the live tail once `openingProgress` proves it complete or once it has
 * stopped growing (`OPENING_QUIET_MS`, or `OPENING_STALL_MS` while messages are
 * provably still to come). Latched per load: live messages that change the
 * snapshot afterwards cannot take it back, and a reload that empties the list
 * starts it over.
 *
 * What waits for it (ui-redesign-spec, "The timeline"): the channel intro, the
 * automatic mark-read, `aria-busy` coming off, and live arrivals rising in. The
 * observer marks no end of the backlog, so without this the first streamed
 * message stood in for the whole channel.
 */
export function useOpening({
  loadKey,
  pageReady,
  reloading,
  progress,
  size,
  newestId,
}: OpeningInput): boolean {
  const [arrived, setArrived] = useState<string | null>(null);
  if (reloading && arrived !== null) setArrived(null);
  const complete = pageReady && (arrived === loadKey || progress === 'complete');
  if (complete && arrived !== loadKey) setArrived(loadKey);

  useEffect(() => {
    if (!pageReady || complete) return;
    const timer = window.setTimeout(
      () => setArrived(loadKey),
      progress === 'streaming' ? OPENING_STALL_MS : OPENING_QUIET_MS
    );
    return () => window.clearTimeout(timer);
    // `size` and `newestId` restart the wait whenever a message arrives.
  }, [pageReady, complete, progress, loadKey, size, newestId]);

  return complete;
}
