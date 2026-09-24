import type { CrewMessage, ObservedRun } from '../crewApi';

/** `started_at` when the daemon recorded one (Unix milliseconds), else null. */
function startedAtOf(run: ObservedRun): number | null {
  const startedAt = run.started_at;
  return typeof startedAt === 'number' && Number.isFinite(startedAt) && startedAt >= 0
    ? startedAt
    : null;
}

/**
 * The viewer's newest task in a channel, for "Show task in channel".
 *
 * `state.runs` is owner-scoped and the daemon records when each task started (`started_at`), so
 * the task that started last is the newest. A run recorded before the daemon kept that time has
 * none, and every dated task is newer than it.
 *
 * Among undated tasks the channel's messages are the only clock: a task is as new as the first
 * loaded message it posted, which is the message the timeline anchors its task row at, so the
 * newest is the one the timeline draws lowest among its anchored rows. A task with no loaded
 * message is chosen only when no undated task has one; among those the last listed wins.
 */
export function newestTaskIn(
  runs: readonly ObservedRun[],
  messages: readonly CrewMessage[],
  channelId: string
): ObservedRun | null {
  const tasks = runs.filter((run) => run.channel_id === channelId);
  if (tasks.length === 0) return null;

  let latest: ObservedRun | null = null;
  let latestStart = -1;
  for (const task of tasks) {
    const startedAt = startedAtOf(task);
    if (startedAt !== null && startedAt >= latestStart) {
      latest = task;
      latestStart = startedAt;
    }
  }
  if (latest) return latest;

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
