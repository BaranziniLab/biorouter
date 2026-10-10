import { useEffect, useState } from 'react';
import { useElapsedMs } from '../../hooks/useElapsedMs';
import { formatElapsed } from '../../utils/formatElapsed';

/** Below this a duration flickers in and out (the activity line's own rule). */
export const THOUGHT_REVEAL_MS = 2000;

/**
 * How long a thought took, measured where it can be: from the moment this view
 * saw the model start thinking to the moment the answer began. A thought loaded
 * from history was never watched, so it has no duration and says none (a
 * number made up from message times would be a guess).
 *
 * Returns the start while the thought is live and the frozen length once it is
 * over.
 */
export function useThoughtDuration(thinking: boolean): {
  since: number | undefined;
  tookMs: number | null;
} {
  const [since, setSince] = useState<number | undefined>(undefined);
  const [tookMs, setTookMs] = useState<number | null>(null);
  useEffect(() => {
    if (thinking && since === undefined) setSince(Date.now());
    if (!thinking && since !== undefined && tookMs === null) setTookMs(Date.now() - since);
  }, [thinking, since, tookMs]);
  return { since, tookMs };
}

/**
 * The live "· 8s" on a thinking row. Its own component, mounted only while the
 * thought is live, because the shared 1 Hz clock re-renders every subscriber:
 * a finished thought must not keep it ticking.
 */
export function ThinkingClock({ since }: { since: number }) {
  const ms = useElapsedMs(since);
  if (ms === null || ms < THOUGHT_REVEAL_MS) return null;
  return <span className="tabular-nums">· {formatElapsed(ms)}</span>;
}

/** The frozen "· 8s" once the answer has started, or nothing for a short thought. */
export function thoughtTook(ms: number | null): string | null {
  return ms !== null && ms >= THOUGHT_REVEAL_MS ? `· ${formatElapsed(ms)}` : null;
}
