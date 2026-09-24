import { useMemo } from 'react';
import { usePeopleDirectory, workspaceName, type DaemonPersonLabels } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { AccessList } from './AccessList';
import { accessRows } from './accessRows';
import { accessCopy } from './copy';
import { useAccessActions } from './useAccessActions';
import { isUnconfirmedRevocation } from './useCrewGrants';
import { useWorkspaceGrants } from './useWorkspaceGrants';

export interface WorkspaceAgentAccessProps {
  /** Layout only. */
  className?: string;
}

/**
 * Workspace settings → Agent access: every chat and task with access anywhere in this workspace —
 * the same rows and actions as the Access tab, without its channel filter. The content of the
 * Workspace settings dialog's Agent access tab slot.
 */
export function WorkspaceAgentAccess({ className }: WorkspaceAgentAccessProps) {
  const { snapshot, runs, labels, connection } = useCrew();
  const grants = useWorkspaceGrants();
  const { onOpen, onStop } = useAccessActions();
  const dir = usePeopleDirectory(snapshot, labels as DaemonPersonLabels | null);
  const rows = useMemo(
    () => accessRows(grants.grants, { snapshot, runs, isUnconfirmed: isUnconfirmedRevocation }),
    [grants.grants, snapshot, runs]
  );
  const workspace = snapshot
    ? workspaceName(snapshot.workspace, dir.host)
    : (connection?.name ?? accessCopy.chatDestinationUnknown);
  return (
    <section
      className={className}
      aria-label={accessCopy.tabTitle}
      data-testid="crew-workspace-agent-access"
    >
      <AccessList
        rows={rows}
        status={grants.status}
        error={grants.error}
        onRetryLoad={grants.refetch}
        emptyText={accessCopy.emptyWorkspace(workspace)}
        onOpen={onOpen}
        onRevoke={(row) => grants.revoke(row.connectionId, row.sessionId)}
        onStop={onStop}
      />
    </section>
  );
}
