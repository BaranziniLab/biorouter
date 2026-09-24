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
 *   position. It is computed once, when the channel opens — which, since the
 *   observer sends the channel one message per frame, means once enough of it
 *   has arrived that the line cannot move (`newLineDecided`, `openingProgress`).
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

function sameEntry(a: TimelineGroupEntry, b: TimelineGroupEntry): boolean {
  if (a.key !== b.key || a.head !== b.head || a.time.getTime() !== b.time.getTime()) return false;
  if (a.kind === 'message' && b.kind === 'message')
    return a.message === b.message && a.restrictedMarker === b.restrictedMarker;
  if (a.kind === 'trace' && b.kind === 'trace')
    return (
      a.messages.length === b.messages.length &&
      a.messages.every((message, index) => message === b.messages[index])
    );
  return false;
}

/** Two groups draw the same: every field a row reads, and the very same message objects. */
export function sameGroup(a: TimelineGroup, b: TimelineGroup): boolean {
  return (
    a.key === b.key &&
    a.authorId === b.authorId &&
    a.agent === b.agent &&
    a.runId === b.runId &&
    a.time.getTime() === b.time.getTime() &&
    a.entries.length === b.entries.length &&
    a.entries.every((entry, index) => sameEntry(entry, b.entries[index]))
  );
}

/**
 * `next`, with every group that draws the same as one in `previous` replaced by
 * that previous object, so a memoized group skips its render. `groupMessages`
 * builds new objects on each call, and the list changes with every message: a
 * channel streaming in one message per frame would otherwise re-render every
 * row on every frame. The observer keeps a message's object while it is
 * unchanged, which is what lets a group be recognized as unchanged.
 */
export function keepUnchangedGroups(
  previous: readonly TimelineDay[],
  next: TimelineDay[]
): TimelineDay[] {
  const before = new Map<string, TimelineGroup>();
  for (const day of previous) {
    for (const item of day.items) if (item.kind === 'group') before.set(item.key, item);
  }
  if (before.size === 0) return next;
  return next.map((day) => ({
    ...day,
    items: day.items.map((item) => {
      if (item.kind !== 'group') return item;
      const old = before.get(item.key);
      return old && sameGroup(old, item) ? old : item;
    }),
  }));
}

export interface NewLineInput {
  /** `snapshot.read_positions[channelId]`: a sequence, null (never read) or absent. */
  readPosition: string | null | undefined;
  /**
   * `snapshot.unread[channelId]`: the broker's count of messages after the read
   * position that someone else posted (`read_state` skips the viewer's own,
   * their agent's included).
   */
  unread: number | undefined;
  /** The viewer: their own posts never open the New region. */
  viewerId: string | null;
}

/** A message that can open the New region: anything but the viewer's own post as a person. */
function opensNewRegion(message: CrewMessage, viewerId: string | null): boolean {
  return !(message.actor_id === viewerId && !message.run_id);
}

/** Where the read position's message is in the list, or -1 when it is not loaded (or absent). */
function readIndex(messages: readonly CrewMessage[], readPosition: string | null | undefined) {
  if (typeof readPosition !== 'string' || !readPosition) return -1;
  return messages.findIndex((message) => message.sequence === readPosition);
}

/** The broker counted nothing unread: there is no New line, wherever the position is. */
function nothingUnread(unread: number | undefined): boolean {
  return typeof unread === 'number' && !(unread > 0);
}

/**
 * The message the New line goes before, or null for none. The read position
 * wins when its message is loaded; otherwise the last `unread` messages from
 * other people are new (the broker's own count). Sequences are opaque strings,
 * compared only for equality. The viewer's own posts at the start of the new
 * region are skipped, so a channel whose only new messages are the viewer's
 * shows no line — and a channel the broker counts nothing unread in shows none,
 * so the line never disagrees with the sidebar's count.
 */
export function newLineBeforeId(
  messages: readonly CrewMessage[],
  input: NewLineInput
): string | null {
  if (nothingUnread(input.unread)) return null;
  const read = readIndex(messages, input.readPosition);
  let start = read >= 0 ? read + 1 : -1;
  if (start < 0) {
    const unread =
      typeof input.unread === 'number' && input.unread > 0 ? Math.floor(input.unread) : 0;
    if (unread === 0) return null;
    // Count back over other people's messages only, as the broker does. More
    // unread than loaded: everything loaded is new.
    start = 0;
    let counted = 0;
    for (let index = messages.length - 1; index >= 0; index -= 1) {
      if (messages[index].actor_id === input.viewerId) continue;
      counted += 1;
      if (counted === unread) {
        start = index;
        break;
      }
    }
  }
  const first = messages.slice(start).find((message) => opensNewRegion(message, input.viewerId));
  return first?.id ?? null;
}

/**
 * The New line's place can no longer move, however much of the channel is still
 * on its way. The observer sends a channel's messages one per frame, oldest
 * first, so a list that has only begun to arrive must not fix the line
 * (ui-redesign-spec: "Computed when the channel opens; fixed until the channel
 * changes"). It is decided early when the broker counts nothing unread, or when
 * the read position's message is loaded with a message after it that opens the
 * New region — the first such message is final once it has arrived, since
 * nothing arrives out of order. Every other case waits for `openingProgress`.
 */
export function newLineDecided(messages: readonly CrewMessage[], input: NewLineInput): boolean {
  if (nothingUnread(input.unread)) return true;
  const read = readIndex(messages, input.readPosition);
  return (
    read >= 0 && messages.slice(read + 1).some((message) => opensNewRegion(message, input.viewerId))
  );
}

/**
 * How far the live tail's opening backlog has provably arrived, from the list
 * and the snapshot alone (the observer marks no end of it):
 *
 * - `complete`: the list itself proves it — nothing at all (the observer's
 *   first frame was empty), or a full page, which is the most a tail holds.
 * - `caught-up`: every message the broker counted unread has arrived — the read
 *   position's message and `unread` messages from others after it (or, never
 *   read, `unread` of them anywhere). The viewer's own later posts may follow.
 * - `streaming`: messages the snapshot names have not arrived yet: the read
 *   position's message (a tail shorter than a page holds it, since the tail is
 *   the channel's newest messages and the position is one of the channel's),
 *   or some of the unread after it.
 * - `unknown`: the snapshot carries no read state for the channel.
 */
export type OpeningProgress = 'complete' | 'caught-up' | 'streaming' | 'unknown';

export function openingProgress(
  messages: readonly CrewMessage[],
  input: NewLineInput
): OpeningProgress {
  if (messages.length === 0 || messages.length >= HISTORY_PAGE_SIZE) return 'complete';
  const { readPosition, unread, viewerId } = input;
  let from: number;
  if (readPosition === null) from = 0;
  else if (typeof readPosition === 'string' && readPosition) {
    const read = readIndex(messages, readPosition);
    if (read < 0) return 'streaming';
    from = read + 1;
  } else return 'unknown';
  if (typeof unread !== 'number') return 'unknown';
  const others = messages.slice(from).filter((message) => message.actor_id !== viewerId).length;
  return others >= unread ? 'caught-up' : 'streaming';
}

/** The channel's start is loaded: the list is shorter than a full page. */
export function reachesChannelStart(messages: readonly CrewMessage[]): boolean {
  return messages.length < HISTORY_PAGE_SIZE;
}

/**
 * The list can be the page shown before `historyBefore`: it holds no message at
 * that sequence (the live tail, `historyBefore === null`, always can). Loading
 * an older page names the first message on screen as the page's boundary and
 * clears the list only afterwards, so for one render the previous list is drawn
 * under the new page's boundary — and it is the list that holds that message.
 * Sequences are compared only for equality.
 */
export function canBePageBefore(
  messages: readonly CrewMessage[],
  historyBefore: string | null
): boolean {
  return historyBefore === null || !messages.some((message) => message.sequence === historyBefore);
}
