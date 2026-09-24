import { CrewLayout } from './layout/CrewLayout';
import { CrewControllerProvider } from './state/CrewControllerContext';
import { useCrewController } from './state/useCrewController';
import type { CrewControllerOptions } from './state/types';

/**
 * The new layout's controller options: Sign in opens by itself when a connect the person started
 * finds the server wants a password or a code, and a refresh keeps a presentation-only copy of the
 * last verified view so re-verifying never blanks the page.
 */
export const CREW_APP_OPTIONS: Readonly<CrewControllerOptions> = {
  autoOpenSignIn: true,
  keepLastVerifiedView: true,
};

/**
 * The `/crew` route: the redesigned Crew layout over the one Crew controller.
 *
 * It is `CrewView` with `layout={CrewLayout}` and these options, written out rather than rendered
 * through `CrewView`, for one reason. `CrewView` imports the legacy layout as its default, and the
 * legacy layout imports `crew.css`, which is a global stylesheet: routing through `CrewView` would
 * load it here too, and its `.crew-main`, `.crew-channel`, `.crew-timeline`, `.crew-message-meta`
 * and `.crew-message-body` rules (padding, 13–14px type, a hover ground on the whole channel
 * column) would restyle the new layout, which uses those names. Nothing on this route's import
 * graph reaches `crew/legacy/` or `crew.css`; `integration/legacyStylesheet.test.ts` holds that.
 */
export default function CrewApp() {
  const controller = useCrewController(CREW_APP_OPTIONS);
  return (
    <CrewControllerProvider controller={controller}>
      <CrewLayout />
    </CrewControllerProvider>
  );
}
