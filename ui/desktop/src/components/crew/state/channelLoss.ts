import type { Snapshot } from '../crewApi';
import { crewObservationCopy } from './copy';

/**
 * Why the selected channel left a person's view, as far as a fresh view proves it (G7). The
 * snapshot lists only the channels the person is in, so a channel that leaves it was deleted or
 * the person is no longer in it, and the sentence that closes it may say which only when the view
 * shows it:
 *
 * - `deleted`: it no longer exists. Either its team left the view too (nobody can leave a team or
 *   be removed from one, so a team goes from a member's view only when it is deleted, with every
 *   channel in it), or an open workspace, which lists every unarchived channel of the person's
 *   teams that they are not in (`joinable_channels`), lists it nowhere while its team is still
 *   theirs.
 * - `left`: it still exists and the person is not in it: an open workspace lists it among the
 *   channels they may join.
 * - `unknown`: the view cannot tell. A managed workspace lists no channel the person is not in,
 *   so a channel deleted there and one they were removed from look the same. A list the broker
 *   cut short (`totals` larger than the list) proves nothing by what it leaves out, and neither
 *   does the absence of a channel that was archived, which is never joinable.
 */
export type ChannelLossReason = 'deleted' | 'left' | 'unknown';

/** What the decision reads of the view that no longer has the channel. */
export type ChannelLossView = Pick<
  Snapshot,
  'workspace' | 'teams' | 'channels' | 'joinable_teams' | 'joinable_channels' | 'totals'
>;

/** The broker listed every item it counted: no `total` (an older broker lists all), or no more. */
function listsAll(list: readonly unknown[] | undefined, total: number | undefined): boolean {
  return total === undefined || (list?.length ?? 0) >= total;
}

/**
 * Why `channelId` is gone from `after`. `before` is the last view that still listed it, which says
 * its team and whether it was archived; without it nothing is proved.
 */
export function channelLossReason(
  before: Pick<Snapshot, 'channels'> | null | undefined,
  after: ChannelLossView,
  channelId: string
): ChannelLossReason {
  if (!channelId || after.channels.some((item) => item.id === channelId)) return 'unknown';
  // Positive evidence: the workspace offers it to the person, so it exists and they are not in it.
  if (after.joinable_channels?.some((item) => item.id === channelId)) return 'left';
  const was = before?.channels.find((item) => item.id === channelId);
  if (!was) return 'unknown';
  const totals = after.totals;
  // The channel may only have been cut from a frame that could not hold every channel.
  if (!listsAll(after.channels, totals?.channels)) return 'unknown';
  if (!after.teams.some((team) => team.id === was.team_id)) {
    if (!listsAll(after.teams, totals?.teams)) return 'unknown';
    // A team the person may join is one they are not in, which no broker method brings about:
    // whatever happened is not the deletion the rule above infers.
    if (after.joinable_teams?.some((team) => team.id === was.team_id)) return 'unknown';
    return 'deleted';
  }
  if (after.workspace.governance !== 'open' || !after.joinable_channels) return 'unknown';
  if (was.archived) return 'unknown';
  if (!listsAll(after.joinable_channels, totals?.joinable_channels)) return 'unknown';
  return 'deleted';
}

/** The sentence that closes a channel for `reason`. `named` is its `#name`, or null. */
export function channelLossText(reason: ChannelLossReason, named: string | null): string {
  switch (reason) {
    case 'deleted':
      return named
        ? crewObservationCopy.channelDeletedNamed(named)
        : crewObservationCopy.channelDeleted;
    case 'left':
      return named ? crewObservationCopy.channelLeftNamed(named) : crewObservationCopy.channelLeft;
    default:
      return named
        ? crewObservationCopy.channelAccessLostNamed(named)
        : crewObservationCopy.channelAccessLost;
  }
}
