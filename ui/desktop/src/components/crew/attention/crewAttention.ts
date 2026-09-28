import type { CrewMessage, CrewMessagePeople, Snapshot } from '../crewApi';
import { personDisplayName, sanitizeUsername } from '../identity/displayText';
import { channelName, workspaceName } from '../identity/objectNames';
import { AttentionThrottle } from './attentionMain';

export { AttentionThrottle, NOTIFY_INTERVAL_MS } from './attentionMain';

/**
 * The pure half of Crew's attention signals outside Crew (M2): the unread count on the app's
 * Crew item and the dock, and a system notification for new messages and mentions while the
 * person is elsewhere. Nothing here reads a clock, a window or the network; the watcher in
 * `useCrewAttention.ts` does, and hands the answers here.
 */

/** A count badge shows at most this; more reads "99+", as the Crew sidebar's rows do. */
export const ATTENTION_BADGE_CAP = 99;

/** The most recent messages read to word one notification. */
export const NOTIFY_FETCH_LIMIT = 20;

/** The count as a badge shows it. */
export function attentionBadgeText(count: number): string {
  return count > ATTENTION_BADGE_CAP ? `${ATTENTION_BADGE_CAP}+` : String(count);
}

/** A snapshot's unread map, with anything that is not a count left out. */
export function unreadCounts(unread: unknown): Map<string, number> {
  const counts = new Map<string, number>();
  if (!unread || typeof unread !== 'object' || Array.isArray(unread)) return counts;
  for (const [channelId, count] of Object.entries(unread)) {
    if (channelId && typeof count === 'number' && Number.isSafeInteger(count) && count > 0)
      counts.set(channelId, count);
  }
  return counts;
}

/** Every unread message in a workspace. */
export function unreadTotal(counts: ReadonlyMap<string, number>): number {
  let total = 0;
  for (const count of counts.values()) total += count;
  return total;
}

/** A channel whose unread count went up, and by how much. */
export interface UnreadRise {
  channelId: string;
  added: number;
}

/**
 * The channels whose unread count rose between two snapshots. The first snapshot of a watch has
 * nothing to compare with, and says nothing: opening the app is not news.
 */
export function unreadRises(
  previous: ReadonlyMap<string, number> | undefined,
  next: ReadonlyMap<string, number>
): UnreadRise[] {
  if (!previous) return [];
  const rises: UnreadRise[] = [];
  for (const [channelId, count] of next) {
    const before = previous.get(channelId) ?? 0;
    if (count > before) rises.push({ channelId, added: count - before });
  }
  return rises;
}

const FENCE = /^\s{0,3}(`{3,}|~{3,})/;
const INLINE_CODE = /(`+)[^`]*?\1/g;

/** A body without its code: a mention inside a code block or span is not addressed to anyone. */
function proseOf(body: string): string {
  let fence: string | null = null;
  const lines: string[] = [];
  for (const line of body.split('\n')) {
    const marker = FENCE.exec(line)?.[1];
    if (fence) {
      if (marker && marker[0] === fence[0] && marker.length >= fence.length) fence = null;
      continue;
    }
    if (marker) {
      fence = marker;
      continue;
    }
    lines.push(line);
  }
  return lines.join('\n').replace(INLINE_CODE, ' ');
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

/**
 * Whether a message body mentions `@username`: case-insensitive, word-bounded (so `@bobby` and
 * `bob@lab.org` do not mention `bob`), and outside code spans and blocks.
 */
export function mentionsUser(body: unknown, username: string): boolean {
  if (typeof body !== 'string' || !username) return false;
  const prose = proseOf(body);
  const pattern = new RegExp(
    `(^|[^\\p{L}\\p{N}_.@-])@${escapeRegExp(username)}(?![\\p{L}\\p{N}_-])`,
    'iu'
  );
  return pattern.test(prose);
}

/** What a notification says. Names only: never a message's text. */
export interface AttentionNotice {
  title: string;
  body: string;
}

/** The workspace and channel names a snapshot gives, made safe to show. */
export function namesFrom(snapshot: Pick<Snapshot, 'workspace' | 'channels' | 'actor'>): {
  workspace: string;
  username: string;
  channel(channelId: string): string;
} {
  const channels = new Map(
    (Array.isArray(snapshot.channels) ? snapshot.channels : []).map((channel) => [
      channel.id,
      channel,
    ])
  );
  return {
    workspace: workspaceName(snapshot.workspace),
    username: sanitizeUsername(snapshot.actor?.username),
    channel: (channelId) => channelName(channels.get(channelId) ?? { id: channelId }),
  };
}

/**
 * The notification for new messages in one channel: "Alice Chen mentioned you in #general" when
 * one of them mentions the person (the newest such message's author), otherwise "3 new messages
 * in chen-lab" with the channel under it. `messages` are the channel's newest, as read.
 */
export function attentionNotice(input: {
  workspace: string;
  channel: string;
  username: string;
  added: number;
  messages: readonly Pick<CrewMessage, 'actor_id' | 'body'>[];
  people?: CrewMessagePeople;
}): AttentionNotice {
  const recent = input.messages.slice(-Math.max(1, Math.min(input.added, NOTIFY_FETCH_LIMIT)));
  const mention = [...recent]
    .reverse()
    .find((message) => mentionsUser(message.body, input.username));
  if (mention) {
    const author = input.people?.[mention.actor_id];
    const username = sanitizeUsername(author?.username);
    const name = author ? personDisplayName(author.display_name, username) : '';
    return {
      title: `${name || 'Someone'} mentioned you in ${input.channel}`,
      body: input.workspace,
    };
  }
  const count = input.added;
  return {
    title: `${count === 1 ? '1 new message' : `${count} new messages`} in ${input.workspace}`,
    body: input.channel,
  };
}

// ---------------------------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------------------------

/** How often each connected workspace's snapshot is read for its unread counts. */
export const ATTENTION_POLL_MS = 10_000;
/** How often the saved connections are listed again, to follow a connect or a disconnect. */
export const CONNECTIONS_REFRESH_MS = 60_000;

/** A new-messages notification, with where clicking it goes. */
export interface AttentionNotification extends AttentionNotice {
  /** One per workspace channel: what the main process rate-limits and de-duplicates by. */
  key: string;
  connectionId: string;
  channelId: string;
}

/** Everything the watcher reaches outside itself, injected so it is tested without them. */
export interface AttentionWatcherDeps {
  listConnections(signal: AbortSignal): Promise<readonly { id: string; status: string }[]>;
  /** `workspace.snapshot` for one saved connection. */
  readSnapshot(connectionId: string, signal: AbortSignal): Promise<unknown>;
  /** The newest `limit` messages of a channel (`messages.history`, which marks nothing read). */
  readLatest(
    connectionId: string,
    channelId: string,
    limit: number,
    signal: AbortSignal
  ): Promise<{ messages?: unknown; people?: unknown }>;
  notify(notification: AttentionNotification): void;
  /** Every connected workspace's unread messages, summed. */
  onTotal(total: number): void;
  /** The person is looking at Crew in this window now: nothing to announce. */
  attended(): boolean;
  sleep(ms: number, signal: AbortSignal): Promise<void>;
  now(): number;
}

type AttentionSnapshot = Pick<Snapshot, 'workspace' | 'channels' | 'actor' | 'unread'>;

function isSnapshot(value: unknown): value is AttentionSnapshot {
  if (!value || typeof value !== 'object') return false;
  const snapshot = value as Record<string, unknown>;
  return (
    Boolean(snapshot.workspace) &&
    typeof snapshot.workspace === 'object' &&
    Array.isArray(snapshot.channels) &&
    Boolean(snapshot.actor) &&
    typeof snapshot.actor === 'object'
  );
}

function messagesOf(value: unknown): Pick<CrewMessage, 'actor_id' | 'body'>[] {
  if (!Array.isArray(value)) return [];
  return value.filter(
    (message): message is CrewMessage =>
      Boolean(message) &&
      typeof message === 'object' &&
      typeof (message as CrewMessage).actor_id === 'string' &&
      typeof (message as CrewMessage).body === 'string'
  );
}

function peopleOf(value: unknown): CrewMessagePeople | undefined {
  if (!value || typeof value !== 'object' || Array.isArray(value)) return undefined;
  const people: Record<string, { username: string; display_name?: string }> = Object.create(null);
  for (const [id, entry] of Object.entries(value)) {
    if (!entry || typeof entry !== 'object') continue;
    const author = entry as { username?: unknown; display_name?: unknown };
    if (typeof author.username !== 'string') continue;
    people[id] = {
      username: author.username,
      ...(typeof author.display_name === 'string' ? { display_name: author.display_name } : {}),
    };
  }
  return people;
}

/**
 * Reads each connected workspace's snapshot every {@link ATTENTION_POLL_MS}, keeps the unread
 * total, and announces a channel whose count rose while the person is not looking at Crew.
 *
 * A snapshot read rather than a live observation: a live one re-reads the snapshot twice every
 * two seconds and holds one of the daemon's sixteen observer slots, which every Crew view needs
 * too, for as long as the app is open. A badge and a notification are well served ten seconds
 * late, at a twentieth of the cost.
 */
export class CrewAttentionWatcher {
  private readonly controller = new AbortController();
  private readonly counts = new Map<string, Map<string, number>>();
  private readonly throttle: AttentionThrottle;
  private connections: string[] = [];
  private listedAt = Number.NEGATIVE_INFINITY;
  private wake: (() => void) | null = null;
  private lastTotal = -1;

  constructor(private readonly deps: AttentionWatcherDeps) {
    this.throttle = new AttentionThrottle();
  }

  start(): void {
    void this.run();
  }

  stop(): void {
    this.controller.abort();
    this.wake?.();
  }

  /** Read again now (the window came back to the front), and list the connections again. */
  refresh(): void {
    this.listedAt = Number.NEGATIVE_INFINITY;
    this.wake?.();
  }

  private get signal(): AbortSignal {
    return this.controller.signal;
  }

  private emitTotal(): void {
    let total = 0;
    for (const counts of this.counts.values()) total += unreadTotal(counts);
    if (total === this.lastTotal) return;
    this.lastTotal = total;
    this.deps.onTotal(total);
  }

  private async run(): Promise<void> {
    this.emitTotal();
    while (!this.signal.aborted) {
      if (this.deps.now() - this.listedAt >= CONNECTIONS_REFRESH_MS) await this.listConnections();
      for (const connectionId of this.connections) {
        if (this.signal.aborted) return;
        await this.check(connectionId);
      }
      if (this.signal.aborted) return;
      await new Promise<void>((resolve) => {
        this.wake = resolve;
        void this.deps.sleep(ATTENTION_POLL_MS, this.signal).then(resolve, resolve);
      });
      this.wake = null;
    }
  }

  private async listConnections(): Promise<void> {
    try {
      const listed = await this.deps.listConnections(this.signal);
      this.listedAt = this.deps.now();
      const connected = listed
        .filter((connection) => connection.status === 'connected')
        .map((connection) => connection.id);
      for (const known of [...this.counts.keys()])
        if (!connected.includes(known)) this.counts.delete(known);
      this.connections = connected;
      this.emitTotal();
    } catch {
      // Keep what was known; the next round asks again.
    }
  }

  private async check(connectionId: string): Promise<void> {
    let snapshot: unknown;
    try {
      snapshot = await this.deps.readSnapshot(connectionId, this.signal);
    } catch {
      return;
    }
    if (this.signal.aborted || !isSnapshot(snapshot)) return;
    const next = unreadCounts(snapshot.unread);
    const previous = this.counts.get(connectionId);
    this.counts.set(connectionId, next);
    this.emitTotal();
    if (this.deps.attended()) return;
    for (const rise of unreadRises(previous, next))
      await this.announce(connectionId, snapshot, rise);
  }

  private async announce(
    connectionId: string,
    snapshot: AttentionSnapshot,
    rise: UnreadRise
  ): Promise<void> {
    const key = `${connectionId}:${rise.channelId}`;
    if (!this.throttle.allow(key, this.deps.now())) return;
    const names = namesFrom(snapshot);
    let latest: { messages?: unknown; people?: unknown } = {};
    try {
      latest = await this.deps.readLatest(
        connectionId,
        rise.channelId,
        Math.min(rise.added, NOTIFY_FETCH_LIMIT),
        this.signal
      );
    } catch {
      // Say that something arrived, without the mention that could not be read.
    }
    if (this.signal.aborted || this.deps.attended()) return;
    const notice = attentionNotice({
      workspace: names.workspace,
      channel: names.channel(rise.channelId),
      username: names.username,
      added: rise.added,
      messages: messagesOf(latest.messages),
      people: peopleOf(latest.people),
    });
    this.deps.notify({ ...notice, key, connectionId, channelId: rise.channelId });
  }
}
