import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SettledKind } from './rows';

/** No row settled: one object, so a list that is not holding keeps the same props. */
const NOTHING_SETTLED: Readonly<Record<string, never>> = Object.freeze({});

export interface HoverHoldOptions<T> {
  /**
   * A row's identity. When given, rows that arrive AFTER every held row (the next page of a list
   * read as it nears its end) are let through at once: they move nothing under the pointer, and a
   * list that held them back would stop growing while the person scrolls it.
   */
  keyOf?: (row: T) => string;
}

/** Whether `live` is `held` with rows added after its last one, by `keyOf`. */
function extendsAtTheEnd<T>(
  held: readonly T[],
  live: readonly T[],
  keyOf: (row: T) => string
): boolean {
  if (live.length <= held.length) return false;
  for (let index = 0; index < held.length; index++) {
    if (keyOf(held[index]) !== keyOf(live[index])) return false;
  }
  return true;
}

/**
 * Hold what a list shows while the pointer is over it, so no row moves under a
 * hovering pointer: a refresh, a write from another window or the agent waits
 * until `pointerleave` ("Data and live updates" in
 * `docs/dictation/dictionary.md`). A row the person acts on meanwhile says what
 * happened in place (`settled`) instead of leaving the list.
 *
 * Nothing is held while the list is empty, so a first load is never kept out.
 * The handlers, `settle` and an empty `settled` keep their identity from one
 * render to the next, so a memoized list re-renders only when what it shows
 * changes. `K` is what a settled row says (the Dictionary's `SettledKind` by
 * default; the History tab's own words there).
 */
export function useHoverHold<T, K extends string = SettledKind>(
  live: readonly T[],
  options: HoverHoldOptions<T> = {}
) {
  const { keyOf } = options;
  const [held, setHeld] = useState<readonly T[] | null>(null);
  const [settled, setSettled] = useState<Readonly<Record<string, K>>>(NOTHING_SETTLED);
  // The rows as last committed, read when the pointer comes in.
  const latest = useRef(live);
  useLayoutEffect(() => {
    latest.current = live;
  }, [live]);
  // Rows added after the held ones are let through, and the hold moves on to include them.
  const extended = held !== null && !!keyOf && extendsAtTheEnd(held, live, keyOf);
  useLayoutEffect(() => {
    if (extended) setHeld(live);
  }, [extended, live]);
  const onPointerEnter = useCallback(() => {
    if (latest.current.length > 0) setHeld(latest.current);
  }, []);
  const onPointerLeave = useCallback(() => {
    setHeld(null);
    setSettled(NOTHING_SETTLED);
  }, []);
  const settle = useCallback(
    (id: string, kind: K) => setSettled((current) => ({ ...current, [id]: kind })),
    []
  );
  const handlers = useMemo(
    () => ({ onPointerEnter, onPointerLeave }),
    [onPointerEnter, onPointerLeave]
  );
  return {
    shown: held === null || extended ? live : held,
    holding: held !== null,
    settled: held !== null ? settled : (NOTHING_SETTLED as Readonly<Record<string, K>>),
    settle,
    handlers,
  };
}
