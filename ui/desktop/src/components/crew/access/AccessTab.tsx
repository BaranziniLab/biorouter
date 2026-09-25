import { useMemo } from 'react';
import { channelName } from '../identity';
import { cn } from '../../../utils';
import { useCrew } from '../state/CrewControllerContext';
import { AccessList } from './AccessList';
import { accessRows } from './accessRows';
import { accessCopy } from './copy';
import { usePastAccess } from './pastAccess';
import { useAccessActions } from './useAccessActions';
import { isUnconfirmedRevocation } from './useCrewGrants';
import { useWorkspaceGrants } from './useWorkspaceGrants';

export interface AccessTabProps {
  /** Layout only. */
  className?: string;
}

/**
 * The details pane's Agent access tab — the same name as Workspace settings' tab and the channel
 * menu's item, since what it lists is agents, not people: every chat and task that can post in or
 * read the selected channel, with Revoke on active chats and Stop on running tasks. The list is
 * read when the tab opens and after every action; it never polls.
 *
 * It has no heading of its own: the tab above it already says "Agent access", and its panel is
 * named by that tab, so a heading repeated the name on screen and to a screen reader (live QA
 * round 3, Q3-30).
 *
 * "Show past access" also holds the revokes this device remembers (`pastAccess.ts`): the daemon
 * lists one grant per chat, so a chat granted again had lost its revoked row (live QA round 4,
 * Q4-12). Its words start at the tab panel's own edge, as About's do (Q4-29): the list is `flush`.
 */
export function AccessTab({ className }: AccessTabProps) {
  const { snapshot, runs, channelId, channel, connectionId, connections } = useCrew();
  const grants = useWorkspaceGrants();
  const past = usePastAccess(connectionId);
  const { onOpen, onStop } = useAccessActions();
  const rows = useMemo(
    () =>
      accessRows(grants.grants, {
        snapshot,
        runs,
        channelId,
        isUnconfirmed: isUnconfirmedRevocation,
        // Only once the daemon's list is read: a remembered row is merged unless the list holds
        // its run, which an unread list cannot say.
        pastAccess:
          connectionId && grants.status === 'loaded' ? { connectionId, entries: past } : undefined,
      }),
    [grants.grants, grants.status, snapshot, runs, channelId, connectionId, past]
  );
  return (
    <div className={cn('flex flex-col gap-2', className)} data-testid="crew-access-tab">
      <AccessList
        flush
        rows={rows}
        status={grants.status}
        error={grants.error}
        onRetryLoad={grants.refetch}
        emptyText={accessCopy.empty(channel ? channelName(channel) : accessCopy.unknownChannel)}
        onOpen={onOpen}
        onRevoke={(row) => grants.revoke(row.connectionId, row.sessionId)}
        onStop={onStop}
        isConnected={(id) =>
          connections.some((item) => item.id === id && item.status === 'connected')
        }
      />
    </div>
  );
}
