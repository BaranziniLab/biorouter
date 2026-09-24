import { AgentsSection, WorkspaceAgentAccess } from '../access';
import { SignInDialog } from '../auth/SignInDialog';
import { CrewDialogs, uniqueNamesSupported } from '../dialogs';
import { OnboardingDialogs, useJoinProbe } from '../onboarding';
import { CrewSidebar } from '../sidebar';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewScreen } from '../state/crewStatus';
import type { CrewController } from '../state/types';
import { ChannelStage } from './ChannelStage';
import { SidebarSkeleton } from './CrewSkeleton';
import { MainScreen } from './MainScreen';
import { useTaskHighlight } from './useTaskHighlight';
import '../crew-app.css';
import './layout.css';

/** Which Crew sidebar the screen has: the real one, its loading shape, or none (first run). */
export function sidebarFor(
  screen: CrewScreen,
  crew: Pick<CrewController, 'connection'>
): 'sidebar' | 'skeleton' | 'none' {
  if (crew.connection) return 'sidebar';
  return screen === 'loading' ? 'skeleton' : 'none';
}

/**
 * The redesigned Crew layout (ui-redesign-spec, "Layout" and "Component architecture"): the Crew
 * sidebar, then the main area — one channel with its details pane, or the one screen
 * `deriveCrewScreen()` chose — and the dialogs, mounted once and opened through the controller's
 * intents.
 *
 * This file composes and wires; it decides nothing an area or the controller decides. React
 * authorizes nothing: every action is a request the daemon and broker judge, and a refusal comes
 * back as words in the one slot the controller's resolver picks — the surface that caused it while
 * it is mounted, otherwise the connection bar, which every screen mounts.
 *
 * Geometry lives in `crew-app.css` (`.crew-app` grid, the `crew-main` container query that pushes
 * or covers with the pane) and never in JavaScript.
 */
export function CrewLayout() {
  const crew = useCrew();
  useJoinProbe();
  const highlight = useTaskHighlight();
  const { screen } = crew;
  const sidebar = sidebarFor(screen, crew);
  const snapshot = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;

  return (
    <div
      className="crew-app"
      data-crew-sidebar={sidebar === 'none' ? 'hidden' : undefined}
      data-crew-screen={screen}
    >
      {sidebar === 'sidebar' && (
        <div className="crew-sidebar">
          <CrewSidebar
            agentsSection={<AgentsSection onShowTask={highlight.show} />}
            renameEnabled={uniqueNamesSupported(snapshot)}
          />
        </div>
      )}
      {sidebar === 'skeleton' && (
        <div className="crew-sidebar">
          <SidebarSkeleton />
        </div>
      )}
      <div className="crew-main">
        {screen === 'channel' ? (
          <ChannelStage highlight={highlight} />
        ) : (
          <MainScreen withBand={sidebar !== 'none'} />
        )}
      </div>
      <CrewDialogs agentAccess={<WorkspaceAgentAccess />} />
      <OnboardingDialogs />
      <SignInDialog />
    </div>
  );
}

export default CrewLayout;
