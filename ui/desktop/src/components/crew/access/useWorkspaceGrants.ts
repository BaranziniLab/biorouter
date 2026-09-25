import { useEffect, useMemo } from 'react';
import { useCrew, useCrewSurfaceReset } from '../state/CrewControllerContext';
import { channelLabels } from './accessRows';
import { rememberChannelLabels } from './chatCrewAccess';
import { useCrewGrants, type CrewGrantsView } from './useCrewGrants';

/**
 * The selected connection's grants, for the surfaces inside the Crew view (the Access tab, the
 * Agent access tab, the Agents section, the header chip and the Chat access pane).
 *
 * Besides {@link useCrewGrants}'s own triggers (open, every grant action, `refetch()`), the list is
 * read again after a manual refresh of the workspace and after a task starts — the two moments the
 * daemon's list can change without anything in this module having acted. It never polls, except
 * while it lists a revoke still waiting for the workspace (`useCrewGrants`, F3).
 *
 * While it shows grants it also records the names of the channels they post in, so the ordinary
 * chat can say "Crew · #general" rather than an ID (see `chatCrewAccess.ts`).
 */
export function useWorkspaceGrants(options: { enabled?: boolean } = {}): CrewGrantsView {
  const { connectionId, snapshot, subscribeSurfaceReset } = useCrew();
  const ids = useMemo(() => (connectionId ? [connectionId] : []), [connectionId]);
  // `subscribeSurfaceReset` is created once per Crew view: the view's lists share their answers.
  const grants = useCrewGrants(ids, { ...options, cacheScope: subscribeSurfaceReset });
  const { refetch } = grants;

  useCrewSurfaceReset((reason) => {
    if (options.enabled === false) return;
    if (reason === 'refresh' || reason === 'run-started') refetch();
  });

  const labels = useMemo(() => channelLabels(snapshot), [snapshot]);
  useEffect(() => {
    if (!connectionId || grants.grants.length === 0 || labels.size === 0) return;
    rememberChannelLabels(
      connectionId,
      labels,
      grants.grants.map((grant) => grant.channel_id)
    );
  }, [connectionId, grants.grants, labels]);

  return grants;
}
