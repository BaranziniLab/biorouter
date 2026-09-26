/**
 * The Crew sidebar area (ui-redesign-spec, "The Crew sidebar"). The layout composes
 * `CrewSidebar`; the pieces are exported for tests and for a layout that places them itself.
 */
export { CrewSidebar, type CrewSidebarProps } from './CrewSidebar';
export { WorkspaceSwitcher } from './WorkspaceSwitcher';
export { WorkspaceMenu } from './WorkspaceMenu';
export { StatusRow } from './StatusRow';
export { PrivacyChip } from './PrivacyChip';
export { PrivacyPopover, PRIVACY_UPDATE_KEY } from './PrivacyPopover';
export { AttentionSections, acceptKey } from './AttentionSections';
export { TeamSections } from './TeamSections';
export { TeamSection, rowKeys, visibleTeamRows } from './TeamSection';
export { ChannelRow, MARK_READ_KEY } from './ChannelRow';
export { YouRow, devProfileName } from './YouRow';
export { YouMenu } from './YouMenu';
export { SidebarAnnouncer, useSidebarCopy } from './SidebarAnnouncer';
export { sidebarCopy, crewStatusCopy } from './copy';
export {
  invitationsToMe,
  sidebarViewSource,
  teamSections,
  unreadBadgeText,
  useSidebarView,
  verifiedPrivacy,
  waitingToJoin,
  workspaceTitle,
  type ChannelRowView,
  type InvitationRow,
  type PrivacyWhy,
  type SidebarView,
  type SidebarViewSource,
  type TeamSectionView,
  type VerifiedPrivacy,
  type WaitingRow,
} from './sidebarView';
export { COLLAPSED_TEAMS_STORAGE_KEY, useCollapsedTeams } from './useCollapsedTeams';
