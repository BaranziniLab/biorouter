import { useMemo } from 'react';
import { channelName } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { AccessList } from './AccessList';
import { accessRows } from './accessRows';
import { accessCopy } from './copy';
import { useAccessActions } from './useAccessActions';
import { isUnconfirmedRevocation } from './useCrewGrants';
import { useWorkspaceGrants } from './useWorkspaceGrants';

export interface AccessTabProps {
  /** Layout only. */
  className?: string;
}

/**
 * The details pane's Access tab ("Chats and agents with access"): every chat and task that can post
 * in or read the selected channel, with Revoke on active chats and Stop on running tasks. The list is
 * read when the tab opens and after every action; it never polls.
 */
export function AccessTab({ className }: AccessTabProps) {
  const { snapshot, runs, channelId, channel } = useCrew();
  const grants = useWorkspaceGrants();
  const { onOpen, onStop } = useAccessActions();
  const rows = useMemo(
    () =>
      accessRows(grants.grants, {
        snapshot,
        runs,
        channelId,
        isUnconfirmed: isUnconfirmedRevocation,
      }),
    [grants.grants, snapshot, runs, channelId]
  );
  return (
    <section className={className} aria-label={accessCopy.tabTitle} data-testid="crew-access-tab">
      <AccessList
        rows={rows}
        status={grants.status}
        error={grants.error}
        onRetryLoad={grants.refetch}
        emptyText={accessCopy.empty(channel ? channelName(channel) : accessCopy.unknownChannel)}
        onOpen={onOpen}
        onRevoke={(row) => grants.revoke(row.connectionId, row.sessionId)}
        onStop={onStop}
      />
    </section>
  );
}
