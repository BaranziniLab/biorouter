import type { CrewMessage, ObservedRun } from '../crewApi';
import { dayKey, dayLabel, messageTime } from './timelineTime';

/**
 * How a channel's messages become what the timeline draws: days, the New line,
 * message groups, an agent's folded tool updates and the owner's task rows.
 * Pure and table-tested; the components only render what this returns.
 *
 * ui-redesign-spec, "The timeline":
 *
 * - A group breaks on a new author, a gap over five minutes, a day divider, the
 *   New line, or human versus agent. It also breaks between two tasks of the
 *   same owner, and after a task's anchor, where the task's status row goes.
 * - The New line goes before the first message after the channel's read
 *   position, or before the last `unread` messages when there is no usable
 *   position. It is computed once, when the channel opens.
 * - A task status row is anchored after the first message carrying its
 *   `run_id` (the agent's "Task: …" post), else at the end of the live log.
 *
 * Supplement (baseline critique): an agent's step-by-step tool updates ("Using
 * crew__request", "Tool failed: blob.read. …") are folded behind one "Show
 * details" control instead of reading as a stream of posts.
 */

/** A group breaks when the next message is more than this much later. */
export const GROUP_GAP_MS = 5 * 60 * 1000;

/**
 * The size of a full page: the live tail keeps at most this many messages, and
 * `messages.history` returns at most this many. A shorter list reaches the
 * channel's start; a full one may have older messages behind it.
 */
export const HISTORY_PAGE_SIZE = 200;

const TASK_PREFIX = 'Task: ';

/**
 * The daemon's run-projection lines (`routes/crew.rs`, `project_run_event` and
 * `ToolActivity::messages`). Only an agent's message (one carrying `run_id`,
 * which the broker sets for a run actor and never for a person) is tested, so a
 * person cannot fold their own post by typing one of these.
 */
const TRACE_LINES: readonly RegExp[] = [
  /^Using [\w.:-]+$/,
  /^Tool response received: [\w.:-]+\.$/,
  /^Tool failed: [\w.:-]+\. Inspect the task conversation for details\.$/,
  /^Requested remote\.[\w.]+: [^\n]*$/,
  /^remote\.execute returned a job receipt\. Check remote\.job_status for its outcome\.$/,
];

/** The broker's projection status on a message, when it sends one. */
function projectionStatus(message: CrewMessage): string | null {
  const status = (message as { status?: unknown }).status;
  return typeof status === 'string' ? status : null;
}

/** An agent's tool-update line: shown folded, never as a post of its own. */
export function isTraceMessage(message: CrewMessage): boolean {
  if (!message.run_id) return false;
  const status = projectionStatus(message);
  if (status !== null && status !== 'progress') return false;
  const body = typeof message.body === 'string' ? message.body.trim() : '';
  return TRACE_LINES.some((line) => line.test(body));
}

/** The first line of a "Task: …" post, without the prefix; null for any other body. */
export function taskTitle(body: string): string | null {
  if (typeof body !== 'string' || !body.startsWith(TASK_PREFIX)) return null;
  const line = body.slice(TASK_PREFIX.length).split('\n')[0].trim();
  return line || null;
}

export interface TimelineMessageEntry {
  kind: 'message';
  key: string;
  message: CrewMessage;
  /** The group's first row, which carries the avatar, the author and the time. */
  head: boolean;
  /** The message is restricted and the channel is not: show the muted "Restricted". */
  restrictedMarker: boolean;
  time: Date;
}

/** Consecutive tool updates of one agent, folded into one row. */
export interface TimelineTraceEntry {
  kind: 'trace';
  key: string;
  messages: CrewMessage[];
  head: boolean;
  time: Date;
}

export type TimelineGroupEntry = TimelineMessageEntry | TimelineTraceEntry;

export interface TimelineGroup {
  kind: 'group';
  key: string;
  /** The author's principal ID (for an agent's post, the owner's). A key, never rendered. */
  authorId: string;
  /** The posts are an agent's, carrying a `run_id`. */
  agent: boolean;
  runId: string | null;
  time: Date;
  entries: TimelineGroupEntry[];
}

export interface TimelineTask {
  kind: 'task';
  key: string;
  run: ObservedRun;
  /** The task's first line, from its "Task: …" post when that is loaded. */
  title: string | null;
  /** Placed after its first message, rather than at the end of the log. */
  anchored: boolean;
}

export interface TimelineNewLine {
  kind: 'new';
  key: 'new';
}

export type TimelineItem = TimelineGroup | TimelineTask | TimelineNewLine;

export interface TimelineDay {
  key: string;
  /** Null only for task rows with no message loaded at all. */
  label: string | null;
  items: TimelineItem[];
}

export interface GroupMessagesOptions {
  channelId: string;
  /** Restricted channels mark nothing; a public-safe channel marks its restricted messages. */
  channelRestricted: boolean;
  /** The message the New line goes before, as computed when the channel opened. */
  newLineBeforeId: string | null;
  /** The viewer's own runs (`state.runs` is owner-scoped). Other channels' runs are ignored. */
  runs: readonly ObservedRun[];
  /**
   * Show runs whose first message is not loaded at the end of the log. The live
   * tail does; an older history page shows only the runs it anchors.
   */
  includeUnanchoredRuns: boolean;
  now: Date;
}

export function groupMessages(
  messages: readonly CrewMessage[],
  options: GroupMessagesOptions
): TimelineDay[] {
  const runs = options.runs.filter((run) => run.channel_id === options.channelId);
  const runById = new Map(runs.map((run) => [run.run_id, run]));
  const anchorOf = new Map<string, number>();
  messages.forEach((message, index) => {
    if (message.run_id && !anchorOf.has(message.run_id)) anchorOf.set(message.run_id, index);
  });

  const days: TimelineDay[] = [];
  const placed = new Set<string>();
  let day: TimelineDay | null = null;
  let group: TimelineGroup | null = null;
  let previousTime = 0;

  messages.forEach((message, index) => {
    const time = messageTime(message.created_at);
    const key = `day-${dayKey(time)}`;
    if (!day || day.key !== key) {
      day = { key, label: dayLabel(time, options.now), items: [] };
      days.push(day);
      group = null;
    }
    if (message.id === options.newLineBeforeId) {
      day.items.push({ kind: 'new', key: 'new' });
      group = null;
    }

    const runId = message.run_id || null;
    const agent = runId !== null;
    const anchor = runId !== null && anchorOf.get(runId) === index;
    if (
      !group ||
      group.authorId !== message.actor_id ||
      group.agent !== agent ||
      group.runId !== runId ||
      time.getTime() - previousTime > GROUP_GAP_MS
    ) {
      group = {
        kind: 'group',
        key: `group-${message.id}`,
        authorId: message.actor_id,
        agent,
        runId,
        time,
        entries: [],
      };
      day.items.push(group);
    }
    previousTime = time.getTime();

    const head = group.entries.length === 0;
    const last = group.entries[group.entries.length - 1];
    if (!anchor && isTraceMessage(message)) {
      if (last?.kind === 'trace') last.messages.push(message);
      else
        group.entries.push({
          kind: 'trace',
          key: `trace-${message.id}`,
          messages: [message],
          head,
          time,
        });
    } else {
      group.entries.push({
        kind: 'message',
        key: message.id,
        message,
        head,
        restrictedMarker: message.restricted === true && !options.channelRestricted,
        time,
      });
    }

    const run = anchor && runId ? runById.get(runId) : undefined;
    if (run) {
      day.items.push({
        kind: 'task',
        key: `task-${run.run_id}`,
        run,
        title: taskTitle(message.body),
        anchored: true,
      });
      placed.add(run.run_id);
      group = null;
    }
  });

  if (options.includeUnanchoredRuns) {
    const unplaced = runs.filter((run) => !placed.has(run.run_id));
    if (unplaced.length > 0) {
      let tail = days[days.length - 1];
      if (!tail) {
        tail = { key: 'day-none', label: null, items: [] };
        days.push(tail);
      }
      for (const run of unplaced) {
        tail.items.push({
          kind: 'task',
          key: `task-${run.run_id}`,
          run,
          title: null,
          anchored: false,
        });
      }
    }
  }
  return days;
}

export interface NewLineInput {
  /** `snapshot.read_positions[channelId]`: a sequence, null (never read) or absent. */
  readPosition: string | null | undefined;
  /** `snapshot.unread[channelId]`. */
  unread: number | undefined;
  /** The viewer: their own posts never open the New region. */
  viewerId: string | null;
}

/**
 * The message the New line goes before, or null for none. The read position
 * wins when its message is loaded; otherwise the last `unread` messages are new.
 * Sequences are opaque strings, compared only for equality. The viewer's own
 * posts at the start of the new region are skipped, so a channel whose only new
 * messages are the viewer's shows no line.
 */
export function newLineBeforeId(
  messages: readonly CrewMessage[],
  input: NewLineInput
): string | null {
  let start = -1;
  if (typeof input.readPosition === 'string' && input.readPosition) {
    const read = messages.findIndex((message) => message.sequence === input.readPosition);
    if (read >= 0) start = read + 1;
  }
  if (start < 0) {
    const unread =
      typeof input.unread === 'number' && input.unread > 0 ? Math.floor(input.unread) : 0;
    if (unread === 0) return null;
    start = Math.max(0, messages.length - unread);
  }
  const first = messages
    .slice(start)
    .find((message) => !(message.actor_id === input.viewerId && !message.run_id));
  return first?.id ?? null;
}

/** The channel's start is loaded: the list is shorter than a full page. */
export function reachesChannelStart(messages: readonly CrewMessage[]): boolean {
  return messages.length < HISTORY_PAGE_SIZE;
}
