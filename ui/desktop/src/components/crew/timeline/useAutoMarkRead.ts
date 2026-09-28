import { useEffect, useRef, useState, type MutableRefObject } from 'react';

/** A message must stay on screen this long before the channel is marked read up to it. */
export const AUTO_READ_DWELL_MS = 1000;
/** At most one automatic mark-read per channel in this long. */
export const AUTO_READ_MIN_INTERVAL_MS = 5000;

/** The last automatic write per channel. Owned by the caller, so it survives a channel switch. */
export type AutoReadMemory = Map<string, { sequence: string; at: number }>;

export interface AutoMarkReadInput {
  channelId: string;
  /** The newest loaded message's sequence: a new one starts the look again. */
  latestSequence: string | null;
  /** `snapshot.read_positions[channelId]`: a sequence, null (never read) or absent. */
  readPosition: string | null | undefined;
  /** `snapshot.unread[channelId]`. */
  unread: number | undefined;
  /**
   * Asked once a dwell: the newest message on screen right now, measured, when it is one the read
   * position has not passed; else null. The channel is marked read up to a message only when two
   * looks a dwell apart both find it, so it was on screen for the whole dwell.
   */
  seen: () => string | null;
  /** Off for a window that does not reach its unread, a read-only view, or before messages load. */
  enabled: boolean;
  /** `controller.markRead`: `channel.read`, never a refresh (L12). */
  markRead(channelId: string, sequence: string): Promise<void>;
  memory: MutableRefObject<AutoReadMemory>;
}

function windowIsActive(): boolean {
  if (typeof document === 'undefined') return false;
  if (document.visibilityState !== 'visible') return false;
  return typeof document.hasFocus === 'function' ? document.hasFocus() : true;
}

/**
 * A channel the person is reading becomes read by itself, up to the newest message they have
 * actually had on screen (QA M7): opening a busy channel at its newest message used to mark every
 * unread message read a second later, the ones never loaded included, and the broker keeps one
 * watermark, so that signal could not be had back. The channel menu keeps "Mark as read".
 *
 * Gated as the spec's risk note says: the window is focused and visible, the message has been on
 * screen for a second (two looks a dwell apart find it), a channel is written at most once every
 * five seconds, and never twice for the same message. It sends `channel.read` to that message's
 * sequence and never refreshes. A failure is silent: nothing the person did failed, and the next
 * look tries again.
 */
export function useAutoMarkRead({
  channelId,
  latestSequence,
  readPosition,
  unread,
  seen,
  enabled,
  markRead,
  memory,
}: AutoMarkReadInput): void {
  // The controller hands out a new `markRead` each render; the looks must not restart with it, or
  // a busy channel would never dwell long enough to fire.
  const write = useRef(markRead);
  useEffect(() => {
    write.current = markRead;
  }, [markRead]);
  const measure = useRef(seen);
  useEffect(() => {
    measure.current = seen;
  }, [seen]);

  // Focus and visibility are not React state; a change re-runs the gate.
  const [activation, setActivation] = useState(0);
  useEffect(() => {
    const bump = () => setActivation((value) => value + 1);
    window.addEventListener('focus', bump);
    document.addEventListener('visibilitychange', bump);
    return () => {
      window.removeEventListener('focus', bump);
      document.removeEventListener('visibilitychange', bump);
    };
  }, []);

  const needsRead =
    (typeof unread === 'number' && unread > 0) ||
    (readPosition !== undefined && readPosition !== latestSequence);

  useEffect(() => {
    if (!enabled || !needsRead) return;
    let previous = measure.current();
    let timer: number | undefined;
    const look = () => {
      timer = window.setTimeout(() => {
        // Re-armed by the focus and visibility listeners above.
        if (!windowIsActive()) return;
        const now = measure.current();
        const last = memory.current.get(channelId);
        const since = last ? Date.now() - last.at : Number.POSITIVE_INFINITY;
        if (
          now !== null &&
          now === previous &&
          now !== readPosition &&
          last?.sequence !== now &&
          since >= AUTO_READ_MIN_INTERVAL_MS
        ) {
          memory.current.set(channelId, { sequence: now, at: Date.now() });
          write.current(channelId, now).catch(() => {
            // Automatic, so silent: "Mark as read" in the channel menu is the visible path.
          });
        }
        previous = now;
        look();
      }, AUTO_READ_DWELL_MS);
    };
    look();
    return () => window.clearTimeout(timer);
  }, [enabled, latestSequence, needsRead, readPosition, channelId, memory, activation]);
}
