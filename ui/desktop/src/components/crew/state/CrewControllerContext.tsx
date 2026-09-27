import { createContext, useContext, useEffect, useRef, type ReactNode } from 'react';
import type { CrewController, ErrorSource, SurfaceResetListener } from './types';

const CrewControllerContext = createContext<CrewController | null>(null);

export function CrewControllerProvider({
  controller,
  children,
}: {
  controller: CrewController;
  children: ReactNode;
}) {
  return (
    <CrewControllerContext.Provider value={controller}>{children}</CrewControllerContext.Provider>
  );
}

/** The Crew controller of the nearest `CrewView`. Every Crew area reads state and acts through it. */
export function useCrew(): CrewController {
  const controller = useContext(CrewControllerContext);
  if (!controller) throw new Error('useCrew() must be called inside a CrewView.');
  return controller;
}

/**
 * Register the calling surface as the error slot for `source` while it is mounted, and answer
 * whether the current error renders here. An error whose surface is not mounted falls back to the
 * connection bar, so it still renders exactly once.
 */
export function useCrewErrorSlot(source: ErrorSource): boolean {
  const { registerErrorSlot, errorSlotFor } = useCrew();
  useEffect(() => registerErrorSlot(source), [registerErrorSlot, source]);
  return errorSlotFor(source);
}

/** Run `listener` whenever the controller resets surfaces (refresh, lost access, a mutation…). */
export function useCrewSurfaceReset(listener: SurfaceResetListener): void {
  const { subscribeSurfaceReset } = useCrew();
  const latest = useRef(listener);
  useEffect(() => {
    latest.current = listener;
  }, [listener]);
  useEffect(
    () => subscribeSurfaceReset((reason) => latest.current(reason)),
    [subscribeSurfaceReset]
  );
}
