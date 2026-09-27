import type { ComponentType } from 'react';
import { CrewControllerProvider } from './state/CrewControllerContext';
import { useCrewController } from './state/useCrewController';
import type { CrewControllerOptions } from './state/types';

export interface CrewViewProps {
  /** The layout to render; it reads the controller with `useCrew()`. */
  layout: ComponentType;
  controllerOptions?: CrewControllerOptions;
}

/**
 * The Crew controller root: it creates the one Crew controller, provides it, and renders the
 * layout it is given. All state and actions live in `state/`; all markup lives in the layout.
 *
 * The `/crew` route renders `CrewApp`, which is this root with the redesigned layout and its
 * options. `CrewView` is what the test harnesses mount (`access/testing.tsx`,
 * `channel/crewTestHarness.tsx`, `state/useCrewController.test.tsx`) to drive the real controller
 * around a small layout of their own, so `layout` has no default.
 */
export default function CrewView({ layout: Layout, controllerOptions }: CrewViewProps) {
  const controller = useCrewController(controllerOptions);
  return (
    <CrewControllerProvider controller={controller}>
      <Layout />
    </CrewControllerProvider>
  );
}
