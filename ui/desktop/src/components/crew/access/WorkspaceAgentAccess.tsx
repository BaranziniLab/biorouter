import { useId, useMemo } from 'react';
import { usePeopleDirectory, workspaceName, type DaemonPersonLabels } from '../identity';
import { cn } from '../../../utils';
import { useCrew } from '../state/CrewControllerContext';
import { AccessList } from './AccessList';
import { accessRows } from './accessRows';
import { accessCopy } from './copy';
import { usePastAccess } from './pastAccess';
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
 *
 * "Show past access" holds the revokes this device remembers (`pastAccess.ts`), exactly as the
 * Access tab's does: the daemon lists one grant per chat, so without them a chat revoked and
 * granted again lost its revoked row here while the channel's tab still showed it (live QA round
 * 4, Q4-12).
 */
export function WorkspaceAgentAccess({ className }: WorkspaceAgentAccessProps) {
  const { snapshot, runs, labels, connection, connectionId, connections } = useCrew();
  const grants = useWorkspaceGrants();
  const past = usePastAccess(connectionId);
  const headingId = useId();
  const { onOpen, onStop } = useAccessActions();
  const dir = usePeopleDirectory(snapshot, labels as DaemonPersonLabels | null);
  const rows = useMemo(
    () =>
      accessRows(grants.grants, {
        snapshot,
        runs,
        isUnconfirmed: isUnconfirmedRevocation,
        // Only once the daemon's list is read, as in the Access tab: a remembered row is merged
        // unless the list holds its run, which an unread list cannot say.
        pastAccess:
          connectionId && grants.status === 'loaded' ? { connectionId, entries: past } : undefined,
      }),
    [grants.grants, grants.status, snapshot, runs, connectionId, past]
  );
  const workspace = snapshot
    ? workspaceName(snapshot.workspace, dir.host)
    : (connection?.name ?? accessCopy.chatDestinationUnknown);
  return (
    <section
      className={cn('flex flex-col gap-2', className)}
      aria-labelledby={headingId}
      data-testid="crew-workspace-agent-access"
    >
      <h3 id={headingId} className="text-label text-text-default">
        {accessCopy.tabTitle}
      </h3>
      <AccessList
        rows={rows}
        status={grants.status}
        error={grants.error}
        onRetryLoad={grants.refetch}
        emptyText={accessCopy.emptyWorkspace(workspace)}
        onOpen={onOpen}
        onRevoke={(row) => grants.revoke(row.connectionId, row.sessionId)}
        onStop={onStop}
        isConnected={(id) =>
          connections.some((item) => item.id === id && item.status === 'connected')
        }
      />
    </section>
  );
}
