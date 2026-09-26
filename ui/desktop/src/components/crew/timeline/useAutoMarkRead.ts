import { useEffect, useRef, useState, type MutableRefObject } from 'react';

/** The bottom of the log must stay in view this long before the channel is marked read. */
export const AUTO_READ_DWELL_MS = 1000;
/** At most one automatic mark-read per channel in this long. */
export const AUTO_READ_MIN_INTERVAL_MS = 5000;

/** The last automatic write per channel. Owned by the caller, so it survives a channel switch. */
export type AutoReadMemory = Map<string, { sequence: string; at: number }>;

export interface AutoMarkReadInput {
  channelId: string;
  /** The newest loaded message's sequence. */
  latestSequence: string | null;
  /** `snapshot.read_positions[channelId]`: a sequence, null (never read) or absent. */
  readPosition: string | null | undefined;
  /** `snapshot.unread[channelId]`. */
  unread: number | undefined;
  /** The reader is at the bottom of the live log, as last reported by the scroll area. */
  atBottom: boolean;
  /**
   * Asked when the dwell ends: is the newest message on screen right now? The
   * reported `atBottom` is a cached verdict that a full tail can outlive — the
   * newest row lands below the fold without a scroll the scroll area would see —
   * so it is never enough on its own. While this answers false the check is
   * asked again after another dwell.
   */
  isAtBottom?: () => boolean;
  /** Off for a history page, a read-only view, or before messages load. */
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
 * A channel the person is looking at, scrolled to its newest message, becomes
 * read by itself (baseline critique: unread badges cleared only through "Mark
 * read", and that reloaded the channel). The channel menu keeps "Mark as read".
 *
 * Gated as the spec's risk note says: the window is focused and visible, the
 * bottom has been in view for a second (and is, measured, when the second
 * ends), and a channel is written at most once every five seconds — and never
 * twice for the same newest message. It sends
 * `channel.read` to that message's sequence and never refreshes. A failure is
 * silent: nothing the person did failed, and the next new message tries again.
 */
export function useAutoMarkRead({
  channelId,
  latestSequence,
  readPosition,
  unread,
  atBottom,
  isAtBottom,
  enabled,
  markRead,
  memory,
}: AutoMarkReadInput): void {
  // The controller hands out a new `markRead` each render; the timer must not
  // restart with it, or a busy channel would never dwell long enough to fire.
  const write = useRef(markRead);
  useEffect(() => {
    write.current = markRead;
  }, [markRead]);
  const measure = useRef(isAtBottom);
  useEffect(() => {
    measure.current = isAtBottom;
  }, [isAtBottom]);

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
    if (!enabled || !atBottom || !latestSequence || !needsRead) return;
    const last = memory.current.get(channelId);
    if (last?.sequence === latestSequence) return;
    const since = last ? Date.now() - last.at : Number.POSITIVE_INFINITY;
    const wait = Math.max(AUTO_READ_DWELL_MS, AUTO_READ_MIN_INTERVAL_MS - since);
    const fire = () => {
      // Re-armed by the focus and visibility listeners above.
      if (!windowIsActive()) return;
      // The newest message is not on screen after all: look again after another dwell.
      if (measure.current && !measure.current()) {
        timer = window.setTimeout(fire, AUTO_READ_DWELL_MS);
        return;
      }
      memory.current.set(channelId, { sequence: latestSequence, at: Date.now() });
      write.current(channelId, latestSequence).catch(() => {
        // Automatic, so silent: "Mark as read" in the channel menu is the visible path.
      });
    };
    let timer = window.setTimeout(fire, wait);
    return () => window.clearTimeout(timer);
  }, [enabled, atBottom, latestSequence, needsRead, channelId, memory, activation]);
}
