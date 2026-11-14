import { useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react';
import type { SettledKind } from './rows';

/** No row settled: one object, so a list that is not holding keeps the same props. */
const NOTHING_SETTLED: Readonly<Record<string, SettledKind>> = Object.freeze({});

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
 * changes.
 */
export function useHoverHold<T>(live: readonly T[]) {
  const [held, setHeld] = useState<readonly T[] | null>(null);
  const [settled, setSettled] = useState<Readonly<Record<string, SettledKind>>>(NOTHING_SETTLED);
  // The rows as last committed, read when the pointer comes in.
  const latest = useRef(live);
  useLayoutEffect(() => {
    latest.current = live;
  }, [live]);
  const onPointerEnter = useCallback(() => {
    if (latest.current.length > 0) setHeld(latest.current);
  }, []);
  const onPointerLeave = useCallback(() => {
    setHeld(null);
    setSettled(NOTHING_SETTLED);
  }, []);
  const settle = useCallback(
    (id: string, kind: SettledKind) => setSettled((current) => ({ ...current, [id]: kind })),
    []
  );
  const handlers = useMemo(
    () => ({ onPointerEnter, onPointerLeave }),
    [onPointerEnter, onPointerLeave]
  );
  return {
    shown: held ?? live,
    holding: held !== null,
    settled: held !== null ? settled : NOTHING_SETTLED,
    settle,
    handlers,
  };
}
