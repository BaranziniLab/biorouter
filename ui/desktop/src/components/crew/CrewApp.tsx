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
 * It is `CrewView` with `layout={CrewLayout}` and these options, written out so the route's root
 * names its layout and options in one place. `CrewView` stays as the controller root the test
 * harnesses mount around layouts of their own.
 *
 * The old layout (`crew/legacy/`) and its global stylesheet (`crew/crew.css`) are deleted, and
 * `integration/legacyStylesheet.test.ts` keeps them that way: it fails if either path exists
 * again or if any source file imports or mocks them.
 */
export default function CrewApp() {
  const controller = useCrewController(CREW_APP_OPTIONS);
  return (
    <CrewControllerProvider controller={controller}>
      <CrewLayout />
    </CrewControllerProvider>
  );
}
