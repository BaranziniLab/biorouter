import { useCrew } from '../state/CrewControllerContext';
import type { CrewScreen } from '../state/crewStatus';
import {
  ConnectingCard,
  NoChannelState,
  NoTeamState,
  OfflineState,
  SignInNeededState,
  WorkspaceDeletedState,
} from './EmptyStates';
import { HostDialog } from './HostDialog';
import { JoinDialog } from './JoinDialog';
import { JoinStatusCard } from './JoinStatusCard';
import { NotSetUpPane } from './NotSetUpPane';
import { OpenWorkspaceDialog } from './OpenWorkspaceDialog';
import { TrustPane } from './TrustPane';
import { Welcome } from './Welcome';

/** The main-area screens this area draws. `loading`, `checking`, `updates-paused` and `channel` are the layout's. */
export const ONBOARDING_SCREENS: readonly CrewScreen[] = [
  'welcome',
  'connecting',
  'offline',
  'sign-in',
  'trust',
  'not-set-up',
  'join',
  'no-team',
  'no-channel',
  'workspace-deleted',
];

/** Whether `screen` is one this area draws. */
export function isOnboardingScreen(screen: CrewScreen): boolean {
  return ONBOARDING_SCREENS.includes(screen);
}

/**
 * The main area for the controller's `screen` when it is an onboarding, connection-problem or
 * empty state; nothing otherwise. The screen is decided by `deriveCrewScreen()` alone, so this
 * never guesses from an error's words.
 */
export function OnboardingScreen() {
  const { screen, connectionId } = useCrew();
  switch (screen) {
    case 'welcome':
      return <Welcome />;
    case 'connecting':
      return <ConnectingCard />;
    case 'offline':
      return <OfflineState />;
    case 'sign-in':
      return <SignInNeededState />;
    case 'trust':
      return <TrustPane />;
    case 'not-set-up':
      return <NotSetUpPane />;
    case 'join':
      // Keyed by connection: what the card polled and whether Join was pressed belong to one
      // connection, so another connection starts a fresh card and never joins on a press made
      // for the first (`ready` joins only when the person started the flow for that workspace).
      return <JoinStatusCard key={connectionId ?? ''} />;
    case 'no-team':
      return <NoTeamState />;
    case 'no-channel':
      return <NoChannelState />;
    case 'workspace-deleted':
      return <WorkspaceDeletedState />;
    default:
      return null;
  }
}

/**
 * The Join, Host and Open a workspace on a server dialogs, each opened by its dialog intent
 * (`{kind: 'join' | 'host' | 'open-workspace'}`).
 */
export function OnboardingDialogs() {
  return (
    <>
      <JoinDialog />
      <HostDialog />
      <OpenWorkspaceDialog />
    </>
  );
}
