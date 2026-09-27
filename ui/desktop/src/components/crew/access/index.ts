/**
 * Chat access, revoke, the access lists and the Agents sidebar section (ui-redesign-spec, "Revoke").
 *
 * Every component here reads the Crew controller through `useCrew()` and is placed by the layout:
 *
 * - `ChatConnectNote` — the composer's note slot, when the route carries `?sessionId=`.
 * - `ChatAccessPane` — the details pane's body in `chat-access` mode (the pane owns its header).
 * - `AccessTab` — the details pane's Access tab.
 * - `WorkspaceAgentAccess` — Workspace settings → Agent access.
 * - `AgentsSection` — the Crew sidebar's `agentsSection` slot.
 * - `useAgentAccessCount` — the channel header's agent-access chip.
 *
 * Outside Crew, `useChatCrewAccess` + `ChatCrewAccessBar` give an ordinary chat its Crew state, and
 * `useChatCrewAccessState` lets the chat's extension menu read it without fetching.
 */
export { AccessList, type AccessListProps } from './AccessList';
export { AccessTab, type AccessTabProps } from './AccessTab';
export { AgentsSection, type AgentsSectionProps } from './AgentsSection';
export { ChatAccessPane, type ChatAccessPaneProps } from './ChatAccessPane';
export { ChatConnectNote, type ChatConnectNoteProps } from './ChatConnectNote';
export { ChatCrewAccessBar, type ChatCrewAccessBarProps } from './ChatCrewAccessBar';
export { WorkspaceAgentAccess, type WorkspaceAgentAccessProps } from './WorkspaceAgentAccess';
export {
  accessRow,
  accessRows,
  accessStatusOf,
  agentAccessCount,
  formatExpiry,
  grantKind,
  splitAccessRows,
  type AccessRow,
  type AccessStatus,
  type AgentAccessCount,
} from './accessRows';
export {
  isCrewExtensionName,
  useChatCrewAccess,
  useChatCrewAccessState,
  type ChatCrewAccess,
  type ChatCrewAccessState,
} from './chatCrewAccess';
export { accessCopy } from './copy';
export { useAgentAccessCount } from './useAgentAccessCount';
export {
  CREW_GRANTS_CHANGED_EVENT,
  announceGrantsChanged,
  revokeGrant,
  revokeOutcomeFrom,
  useCrewGrants,
  type CrewGrantsView,
  type RevokeOutcome,
} from './useCrewGrants';
export { useWorkspaceGrants } from './useWorkspaceGrants';
