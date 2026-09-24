import { useCallback, useMemo, useState } from 'react';
import { crewActionCopy } from './copy';
import { failureCode, failureMessage } from './observationFailure';
import type { ActionKey, ActOptions, CrewActionError, ErrorSource } from './types';

/** Sources that always render in the connection bar. */
const BAR_SOURCES: readonly ErrorSource[] = ['observer', 'global'];

/**
 * The one slot an action error renders in: the surface that caused it while that surface is
 * mounted, otherwise the connection bar (`global`). Observer and global errors always go to the
 * bar. Nothing else decides where an error appears, so it renders exactly once.
 */
export function resolveErrorSlot(
  error: CrewActionError | null,
  mountedSlots: ReadonlyMap<ErrorSource, number>
): ErrorSource | null {
  if (!error) return null;
  if (BAR_SOURCES.includes(error.source)) return error.source;
  return (mountedSlots.get(error.source) ?? 0) > 0 ? error.source : 'global';
}

/** The error an action failure records: its message (or the fallback), code and source. */
export function actionError(failure: unknown, source: ErrorSource): CrewActionError {
  const code = failureCode(failure);
  return {
    message: failureMessage(failure, crewActionCopy.actionFallback),
    source,
    ...(code !== undefined ? { code } : {}),
  };
}

function adjust<K>(counts: ReadonlyMap<K, number>, key: K, delta: number): Map<K, number> {
  const next = new Map(counts);
  const value = (counts.get(key) ?? 0) + delta;
  if (value > 0) next.set(key, value);
  else next.delete(key);
  return next;
}

export interface CrewActions {
  error: CrewActionError | null;
  act<T>(
    source: ErrorSource,
    key: ActionKey,
    fn: () => Promise<T>,
    options?: ActOptions
  ): Promise<T | undefined>;
  reportError(message: string, source?: ErrorSource, code?: string): void;
  dismissError(): void;
  isPending(key: ActionKey): boolean;
  busy: boolean;
  errorSlotFor(source: ErrorSource): boolean;
  registerErrorSlot(source: ErrorSource): () => void;
}

/**
 * Action bookkeeping: one error with its source, and a count of pending actions per key.
 *
 * `act` replaces the old single `busy` flag. It clears the error when an action starts (unless
 * asked to keep it), records the failure's message and source, and never throws. `busy` is the
 * coarse view: true while any action is pending.
 */
export function useCrewActions(): CrewActions {
  const [error, setError] = useState<CrewActionError | null>(null);
  const [pending, setPending] = useState<ReadonlyMap<ActionKey, number>>(() => new Map());
  const [slots, setSlots] = useState<ReadonlyMap<ErrorSource, number>>(() => new Map());

  const act = useCallback(
    async <T>(
      source: ErrorSource,
      key: ActionKey,
      fn: () => Promise<T>,
      options?: ActOptions
    ): Promise<T | undefined> => {
      setPending((current) => adjust(current, key, 1));
      if (!options?.preserveError) setError(null);
      try {
        return await fn();
      } catch (failure) {
        setError(actionError(failure, source));
        return undefined;
      } finally {
        setPending((current) => adjust(current, key, -1));
      }
    },
    []
  );
  const reportError = useCallback(
    (message: string, source: ErrorSource = 'global', code?: string) =>
      setError({ message, source, ...(code !== undefined ? { code } : {}) }),
    []
  );
  const dismissError = useCallback(() => setError(null), []);
  const registerErrorSlot = useCallback((source: ErrorSource) => {
    setSlots((current) => adjust(current, source, 1));
    let registered = true;
    return () => {
      if (!registered) return;
      registered = false;
      setSlots((current) => adjust(current, source, -1));
    };
  }, []);
  const target = useMemo(() => resolveErrorSlot(error, slots), [error, slots]);
  const errorSlotFor = useCallback((source: ErrorSource) => target === source, [target]);
  const isPending = useCallback((key: ActionKey) => (pending.get(key) ?? 0) > 0, [pending]);

  return {
    error,
    act,
    reportError,
    dismissError,
    isPending,
    busy: pending.size > 0,
    errorSlotFor,
    registerErrorSlot,
  };
}
