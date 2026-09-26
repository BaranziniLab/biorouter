import { useMemo } from 'react';
import { useCrew } from '../state/CrewControllerContext';
import { agentAccessCount, type AgentAccessCount } from './accessRows';
import { useWorkspaceGrants } from './useWorkspaceGrants';

/**
 * The channel header's agent-access chip (ui-redesign-spec, "The channel header and channel menu"):
 * how many chats and tasks can post in the channel right now, with the chip's label ("2 chats",
 * "1 task", "3 agents") and its accessible name ("3 chats or agents can post here"). `total` is 0
 * when nothing can post, and the chip is not shown.
 *
 * Defaults to the selected channel. The grants are read when the header mounts and after every
 * grant action; the tasks come from the observer's owned runs, so a started task counts at once.
 */
export function useAgentAccessCount(channelId?: string): AgentAccessCount {
  const { runs, channelId: selected } = useCrew();
  const grants = useWorkspaceGrants();
  const target = channelId ?? selected;
  return useMemo(
    () => agentAccessCount({ grants: grants.grants, runs, channelId: target }),
    [grants.grants, runs, target]
  );
}
