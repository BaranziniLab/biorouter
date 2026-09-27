import { useState } from 'react';
import { AgentsSection, ChatAccessPane, WorkspaceAgentAccess } from '../access';
import { SignInDialog } from '../auth/SignInDialog';
import { CrewDialogs, uniqueNamesSupported } from '../dialogs';
import { OnboardingDialogs, useJoinProbe } from '../onboarding';
import { DetailsPane } from '../pane';
import { CrewSidebar } from '../sidebar';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewScreen } from '../state/crewStatus';
import type { CrewController } from '../state/types';
import { ChannelStage } from './ChannelStage';
import { SidebarSkeleton } from './CrewSkeleton';
import { MainScreen } from './MainScreen';
import { useChannelAddedToast } from './useChannelAddedToast';
import { useJoinedToast } from './useJoinedToast';
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
 * Whether the details pane may animate: only once it has opened or closed while the channel stage
 * is on screen (live QA round 4, Q4-04). A pane that MOUNTS closed — every return to Crew — played
 * its push-out and slid the channel 1068 → 1428 px; one that mounts open (its tab remembered)
 * would slide in. Neither is a change the person made, so neither moves: the root carries
 * `data-pane-still` until then (`crew-app.css`). Decided in render, so the attribute goes in the
 * same commit as the pane's own state changes: the pane reads its exit animation from the computed
 * style as it closes.
 */
export function usePaneAnimate(stageShown: boolean, paneOpen: boolean): boolean {
  const [motion, setMotion] = useState(() => ({
    stageShown,
    openAtMount: paneOpen,
    animate: false,
  }));
  let next = motion;
  if (stageShown !== motion.stageShown)
    next = { stageShown, openAtMount: paneOpen, animate: false };
  else if (stageShown && !motion.animate && paneOpen !== motion.openAtMount)
    next = { ...motion, animate: true };
  if (next !== motion) setMotion(next);
  return next.animate;
}

/**
 * The details pane beside a screen that is not a channel — an offline workspace, "Can't connect",
 * any screen the sidebar sits beside — in Chat access mode only (final acceptance F2). The Agents
 * section's chat rows open it, and a revoke there needs no connection: the daemon stops the grant
 * on this device at once and confirms it with the workspace once it is back. Before this, the row
 * set the pane's intent and nothing drew it, so the Crew view offered no Revoke during an outage.
 * The other modes are about a verified channel, which these screens do not show.
 */
function OffStageChatAccess() {
  const { ui } = useCrew();
  if (ui.pane?.mode !== 'chat-access') return null;
  return <DetailsPane chatAccess={<ChatAccessPane />} />;
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
  useJoinedToast();
  useChannelAddedToast();
  const highlight = useTaskHighlight();
  const { screen } = crew;
  const sidebar = sidebarFor(screen, crew);
  const snapshot = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const paneAnimate = usePaneAnimate(screen === 'channel', crew.ui.pane !== null);

  return (
    <div
      className="crew-app"
      data-crew-sidebar={sidebar === 'none' ? 'hidden' : undefined}
      data-crew-screen={screen}
      data-pane-still={paneAnimate ? undefined : ''}
    >
      {sidebar === 'sidebar' && (
        <div className="crew-sidebar">
          <CrewSidebar
            agentsSection={<AgentsSection onShowTask={highlight.show} />}
            renameEnabled={uniqueNamesSupported(snapshot, crew.capabilities)}
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
        ) : sidebar === 'sidebar' ? (
          // The sidebar's rows can open Chat access on any screen, so the screen shares the stage
          // with the pane (F2).
          <div className="crew-stage">
            <MainScreen withBand className="crew-stage-screen" />
            <OffStageChatAccess />
          </div>
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
