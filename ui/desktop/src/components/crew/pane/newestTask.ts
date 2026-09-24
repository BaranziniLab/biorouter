import type { CrewMessage, ObservedRun } from '../crewApi';

/**
 * The viewer's newest task in a channel, for "Show task in channel".
 *
 * `state.runs` is owner-scoped but carries no time, and the daemon lists it from a map, so its
 * order says nothing about age. The channel's messages are the only clock the renderer has: a task
 * is as new as the first loaded message it posted, which is the message the timeline anchors its
 * task row at. So the newest task is the one the timeline draws lowest among its anchored rows.
 *
 * A task with no loaded message (still setting up, or older than the loaded page) is chosen only
 * when no task in the channel has one. Among those the last listed wins, because that is the row
 * the timeline draws last.
 */
export function newestTaskIn(
  runs: readonly ObservedRun[],
  messages: readonly CrewMessage[],
  channelId: string
): ObservedRun | null {
  const tasks = runs.filter((run) => run.channel_id === channelId);
  if (tasks.length === 0) return null;
  const anchors = new Map<string, number>();
  messages.forEach((message, index) => {
    if (message.channel_id === channelId && message.run_id && !anchors.has(message.run_id)) {
      anchors.set(message.run_id, index);
    }
  });
  let newest: ObservedRun | null = null;
  let newestAnchor = -1;
  for (const task of tasks) {
    const anchor = anchors.get(task.run_id);
    if (anchor !== undefined && anchor > newestAnchor) {
      newest = task;
      newestAnchor = anchor;
    }
  }
  return newest ?? tasks[tasks.length - 1];
}
