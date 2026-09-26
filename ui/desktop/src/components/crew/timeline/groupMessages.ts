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
 * - A task status row follows the task's result: the last message carrying its
 *   `run_id` once one of them, besides the "Task: …" post, is not a tool update.
 *   With no result yet it sits right after the "Task: …" post (the first message
 *   carrying the `run_id`), and moves under the result when it lands (Q2-62).
 *   The task's own posts stay one group above it. With none of its messages
 *   loaded it goes at the end of the live log — there only while the task is
 *   live, or when it started no earlier than the oldest message loaded
 *   (`started_at`), so an old task whose post has scrolled out of the tail does
 *   not pile up below the newest message.
 * - When the first unread message is also its day's first, the New line is not
 *   a second rule 30px under the day's: "New" goes on the day's own rule
 *   (`TimelineDay.newOnRule`, Q2-53).
 * - Rows of one author (as a person, or as their agent) posted in the same
 *   minute read the same "Bob Lee's message, 10:02 AM" in their action names,
 *   so each carries its place among them (`sameMinute`, ", 2 of 2", Q2-57).
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
 * channel's start; a full one may have older messages behind it. The observer
 * asks for less when the broker answers `response_too_large`, and says so
 * (`page_size`, the controller's `pageSize`): the functions below take that size.
 */
export const HISTORY_PAGE_SIZE = 200;

/**
 * Run statuses in which a task is still going or waits on its owner. The
 * daemon lists every such run in a `state` frame, beside only the newest
 * finished ones (`routes/crew_observation.rs`, `run_is_live`).
 */
export const LIVE_RUN_STATUSES: readonly string[] = [
  'starting',
  'running',
  'waiting_for_approval',
  'cancellation_pending',
  'cancellation_unconfirmed',
];

/** `started_at` when the daemon recorded one (Unix milliseconds), else null. */
export function runStartedAt(run: ObservedRun): number | null {
  const startedAt = run.started_at;
  return typeof startedAt === 'number' && Number.isFinite(startedAt) && startedAt >= 0
    ? startedAt
    : null;
}

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
  /**
   * Where this row stands among the author's rows posted in the same minute, when there is more
   * than one: `{ index: 2, count: 2 }` reads ", 2 of 2" after the time in its action names.
   */
  sameMinute?: { index: number; count: number };
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
  /**
   * The New line falls on this day's first message: its "New" goes on the day's own rule
   * instead of a second rule under it. The day's items then hold no `new` item.
   */
  newOnRule?: boolean;
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
   * tail does; an older history page shows only the runs it anchors. Even then,
   * only a live run, or one that started no earlier than the oldest loaded
   * message, is shown, oldest first.
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
  /** Each run's first loaded message: its "Task: …" post, where its title comes from. */
  const anchorOf = new Map<string, number>();
  /** Each run's last loaded message, and whether a result (not a tool update) came after its post. */
  const lastOf = new Map<string, number>();
  const hasResult = new Set<string>();
  messages.forEach((message, index) => {
    const runId = message.run_id;
    if (!runId) return;
    if (!anchorOf.has(runId)) anchorOf.set(runId, index);
    else if (!isTraceMessage(message)) hasResult.add(runId);
    lastOf.set(runId, index);
  });
  /** The message the run's status row follows: its result, else its "Task: …" post. */
  const rowAfter = (runId: string) =>
    hasResult.has(runId) ? lastOf.get(runId) : anchorOf.get(runId);

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
      // The day's first message: "New" rides on the day's own rule rather than a second one.
      if (day.items.length === 0) day.newOnRule = true;
      else day.items.push({ kind: 'new', key: 'new' });
      group = null;
    }

    const runId = message.run_id || null;
    const agent = runId !== null;
    const anchor = runId !== null && anchorOf.get(runId) === index;
    const statusRowHere = runId !== null && rowAfter(runId) === index;
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

    const run = statusRowHere && runId ? runById.get(runId) : undefined;
    if (run) {
      const first = messages[anchorOf.get(run.run_id) ?? index];
      day.items.push({
        kind: 'task',
        key: `task-${run.run_id}`,
        run,
        title: taskTitle(first.body),
        anchored: true,
      });
      placed.add(run.run_id);
      group = null;
    }
  });

  numberSameMinuteRows(days);

  if (options.includeUnanchoredRuns) {
    const oldest = messages.length > 0 ? messageTime(messages[0].created_at).getTime() : null;
    const unplaced = runs
      .filter((run) => {
        if (placed.has(run.run_id)) return false;
        if (LIVE_RUN_STATUSES.includes(run.status)) return true;
        const startedAt = runStartedAt(run);
        return startedAt !== null && (oldest === null || startedAt >= oldest);
      })
      .map((run, index) => ({ run, index, startedAt: runStartedAt(run) }))
      // Oldest first, so the newest task sits lowest; an undated one after every dated one.
      .sort(
        (a, b) =>
          (a.startedAt ?? Number.POSITIVE_INFINITY) - (b.startedAt ?? Number.POSITIVE_INFINITY) ||
          a.index - b.index
      )
      .map(({ run }) => run);
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

/**
 * Gives each message row that shares its author (as a person or as their agent) and its minute
 * with another row its place among them, in reading order (`sameMinute`). Their action names are
 * otherwise identical ("Copy text of Bob Lee's message, 10:02 AM"), which a screen reader's
 * button list cannot tell apart. A day never spans a minute, so rows are counted per day.
 */
function numberSameMinuteRows(days: TimelineDay[]): void {
  for (const day of days) {
    const byMinute = new Map<string, TimelineMessageEntry[]>();
    for (const item of day.items) {
      if (item.kind !== 'group') continue;
      for (const entry of item.entries) {
        if (entry.kind !== 'message') continue;
        const minute = Math.floor(entry.time.getTime() / 60_000);
        const key = `${item.authorId}\u0000${item.agent ? 'agent' : 'person'}\u0000${minute}`;
        const rows = byMinute.get(key);
        if (rows) rows.push(entry);
        else byMinute.set(key, [entry]);
      }
    }
    for (const rows of byMinute.values()) {
      if (rows.length < 2) continue;
      rows.forEach((entry, position) => {
        entry.sameMinute = { index: position + 1, count: rows.length };
      });
    }
  }
}

function sameEntry(a: TimelineGroupEntry, b: TimelineGroupEntry): boolean {
  if (a.key !== b.key || a.head !== b.head || a.time.getTime() !== b.time.getTime()) return false;
  if (a.kind === 'message' && b.kind === 'message')
    return (
      a.message === b.message &&
      a.restrictedMarker === b.restrictedMarker &&
      a.sameMinute?.index === b.sameMinute?.index &&
      a.sameMinute?.count === b.sameMinute?.count
    );
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
 * and the snapshot alone — for a daemon that does not mark the end of it
 * (`remaining`, the controller's `backlogComplete`, which the timeline prefers):
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
  input: NewLineInput,
  pageSize: number = HISTORY_PAGE_SIZE
): OpeningProgress {
  if (messages.length === 0 || messages.length >= pageSize) return 'complete';
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
export function reachesChannelStart(
  messages: readonly CrewMessage[],
  pageSize: number = HISTORY_PAGE_SIZE
): boolean {
  return messages.length < pageSize;
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
