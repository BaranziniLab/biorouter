import type { Channel, Snapshot, Team } from '../crewApi';
import {
  connectionNames,
  identityCopy,
  isMachineIdShaped,
  sanitizeDisplayText,
  usePeopleDirectory,
  type PeopleDirectory,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController } from '../state/types';

/**
 * What the pane draws, read from the controller.
 *
 * The snapshot is the verified one, or while a refresh re-verifies the last verified one for this
 * connection, so the pane's content — and a Task being written in it — survives a manual refresh.
 * It is presentation only: `verified` says whether the view is current, and nothing here
 * authorizes anything; the daemon and broker decide every action.
 */
export interface PanePresentation {
  crew: CrewController;
  verified: boolean;
  snapshot: Snapshot | null;
  channel: Channel | null;
  team: Team | null;
  dir: PeopleDirectory;
  isOwner: boolean;
  /** The workspace as the switcher names it: its own name, else the saved connection's. */
  workspace: string;
}

export function usePanePresentation(): PanePresentation {
  const crew = useCrew();
  const verified = Boolean(
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
  );
  const snapshot = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const labels = crew.snapshot ? crew.labels : (crew.lastVerified?.labels ?? null);
  const dir = usePeopleDirectory(snapshot, labels);
  const channel = snapshot?.channels.find((item) => item.id === crew.channelId) ?? null;
  const team = channel
    ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
    : null;
  const named = sanitizeDisplayText(snapshot?.workspace.name);
  return {
    crew,
    verified,
    snapshot,
    channel,
    team,
    dir,
    isOwner: Boolean(snapshot && channel && channel.owner_id === snapshot.actor.id),
    workspace:
      named && !isMachineIdShaped(named)
        ? named
        : (connectionNames(crew.connections).get(crew.connectionId) ??
          identityCopy.unnamedWorkspace),
  };
}

/** Copy a value; Crew never toasts a copy. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
}
