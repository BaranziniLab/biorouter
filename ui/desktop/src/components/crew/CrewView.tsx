import type { ComponentType } from 'react';
import LegacyCrewLayout from './legacy/LegacyCrewLayout';
import { CrewControllerProvider } from './state/CrewControllerContext';
import { useCrewController } from './state/useCrewController';
import type { CrewControllerOptions } from './state/types';

export interface CrewViewProps {
  /** The layout to render; it reads the controller with `useCrew()`. Default: the legacy layout. */
  layout?: ComponentType;
  controllerOptions?: CrewControllerOptions;
}

/**
 * The Crew route's root: it creates the one Crew controller, provides it, and renders a layout.
 * All state and actions live in `state/`; all markup lives in the layout.
 */
export default function CrewView({
  layout: Layout = LegacyCrewLayout,
  controllerOptions,
}: CrewViewProps) {
  const controller = useCrewController(controllerOptions);
  return (
    <CrewControllerProvider controller={controller}>
      <Layout />
    </CrewControllerProvider>
  );
}
