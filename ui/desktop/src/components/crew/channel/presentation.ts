import type { Channel, ObservedRun, Snapshot, Team } from '../crewApi';
import {
  connectionNames,
  identityCopy,
  isMachineIdShaped,
  sanitizeDisplayText,
  usePeopleDirectory,
  type PeopleDirectory,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { CANCELLABLE_RUN_STATUSES } from '../state/crewStatus';
import type { CrewController } from '../state/types';

/**
 * What the channel area draws, read from the controller.
 *
 * The snapshot is the verified one, or during re-verification the last verified one for this
 * connection (`keepLastVerifiedView`), so the header never blinks away during a manual refresh.
 * It is presentation only: `verified` says whether the view is current, and nothing here
 * authorizes an action — the daemon and broker decide every one.
 */
export interface ChannelPresentation {
  crew: CrewController;
  /** A verified snapshot and observed privacy exist for the selected connection. */
  verified: boolean;
  snapshot: Snapshot | null;
  channel: Channel | null;
  team: Team | null;
  dir: PeopleDirectory;
  /** The viewer owns the selected channel (display only; the broker decides authority). */
  isOwner: boolean;
  /** The workspace as the switcher names it: its own name, else the saved connection's. */
  workspace: string;
}

export function workspaceLabel(crew: CrewController, snapshot: Snapshot | null): string {
  const named = sanitizeDisplayText(snapshot?.workspace.name);
  if (named && !isMachineIdShaped(named)) return named;
  return connectionNames(crew.connections).get(crew.connectionId) ?? identityCopy.unnamedWorkspace;
}

export function useChannelPresentation(): ChannelPresentation {
  const crew = useCrew();
  const verified = Boolean(
    crew.snapshot && crew.observedPrivacy?.connectionId === crew.connectionId
  );
  const snapshot = crew.snapshot ?? crew.lastVerified?.snapshot ?? null;
  const labels = crew.snapshot ? crew.labels : (crew.lastVerified?.labels ?? null);
  const people = crew.snapshot ? crew.people : crew.lastVerified?.people;
  const dir = usePeopleDirectory(snapshot, labels, people ?? null);
  const channel = snapshot?.channels.find((item) => item.id === crew.channelId) ?? null;
  const team = channel
    ? (snapshot?.teams.find((item) => item.id === channel.team_id) ?? null)
    : null;
  return {
    crew,
    verified,
    snapshot,
    channel,
    team,
    dir,
    isOwner: Boolean(snapshot && channel && channel.owner_id === snapshot.actor.id),
    workspace: workspaceLabel(crew, snapshot),
  };
}

/** The viewer's own tasks in `channelId` that may still post there (Stop is still offered). */
export function activeTasksIn(runs: readonly ObservedRun[], channelId: string): number {
  return runs.filter(
    (run) => run.channel_id === channelId && CANCELLABLE_RUN_STATUSES.includes(run.status)
  ).length;
}

/** Copy a value; Crew never toasts a copy. A failure is silent here: the value is still on screen. */
export async function copyText(value: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(value);
    return true;
  } catch {
    return false;
  }
}

/** A calendar date the way the timeline's day divider names one: "September 22" or with a year. */
export function calendarDate(unixSeconds: number, now: Date = new Date()): string {
  const date = new Date(unixSeconds * 1000);
  return date.toLocaleDateString('en-US', {
    month: 'long',
    day: 'numeric',
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: 'numeric' }),
  });
}
