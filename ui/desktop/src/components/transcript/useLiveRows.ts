import { useMemo, useRef } from 'react';

/**
 * At most this many rows may arrive in one render and still count as live. A
 * live turn appends one or two (the question, then the answer as it starts); a
 * history read appends the whole chat at once.
 */
const LIVE_BATCH_MAX = 3;

/**
 * Which rows of a transcript were appended live, so only they play the `.br-enter`
 * entrance (spec 3.6 "Entrances", motion B2 and J6).
 *
 * A row is live when it was not in the list the component first rendered, it
 * arrived in a small batch, and the chat was not loading at the time. So a
 * history load, a tab switch (the list remounts) and a reload (the chat is
 * `loading` while its rows arrive) all draw at rest, and a read-only replay,
 * which passes no live state, never animates.
 *
 * The answer for a key never changes once given: a row that entered live keeps
 * its class, so re-renders do not restart its animation, and a row that did
 * not never gains one.
 */
export function useLiveRows(keys: readonly string[], interactive: boolean, loading: boolean) {
  const seen = useRef<Set<string> | null>(null);
  const live = useRef<Set<string>>(new Set());
  return useMemo(() => {
    if (seen.current === null) {
      seen.current = new Set(keys);
    } else {
      const fresh = keys.filter((key) => !seen.current!.has(key));
      if (interactive && !loading && fresh.length > 0 && fresh.length <= LIVE_BATCH_MAX) {
        for (const key of fresh) live.current.add(key);
      }
      for (const key of fresh) seen.current.add(key);
    }
    return live.current;
    // `keys` is compared by content: a new array with the same rows is no change.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [keys.join('\0'), interactive, loading]);
}
