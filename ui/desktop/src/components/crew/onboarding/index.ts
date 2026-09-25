/**
 * Crew onboarding (ui-redesign-spec, "Onboarding: join, host and admit", "Main-area states outside
 * a channel", "SSH failure classification"): first run, joining by invitation, hosting a new
 * workspace, the join states, the trust and "not set up" panes, and the empty states. Every
 * component reads the controller through `useCrew()`; none authorizes anything.
 *
 * Mount once in the layout: `useJoinProbe()` (it reports a not-yet-member to the controller),
 * `<OnboardingDialogs />` and, for the main area, `<OnboardingScreen />` whenever
 * `isOnboardingScreen(screen)`. `<SetupChecklist />` and `<NameSuggestionNote />` may also sit
 * above a channel.
 */
export { Welcome } from './Welcome';
export { JoinDialog, type JoinDialogProps } from './JoinDialog';
export { HostDialog, type HostDialogProps } from './HostDialog';
export { JoinStatusCard, JOIN_POLL_INTERVAL_MS, LEGACY_JOIN_STATUS } from './JoinStatusCard';
export { LegacyJoinForm } from './LegacyJoinForm';
export { SetupChecklist } from './SetupChecklist';
export { TrustPane } from './TrustPane';
export { NotSetUpPane } from './NotSetUpPane';
export {
  ConnectingCard,
  NoChannelState,
  NoTeamState,
  OfflineState,
  SignInNeededState,
  useFocusHold,
  type FocusHoldProps,
} from './EmptyStates';
export { NameSuggestionNote } from './NameSuggestionNote';
export {
  OnboardingDialogs,
  OnboardingScreen,
  ONBOARDING_SCREENS,
  isOnboardingScreen,
} from './OnboardingScreen';
export { useJoinProbe } from './useJoinProbe';
export { forgetJoinContext, readJoinContext, updateJoinContext } from './joinContext';
export * from './copy';
