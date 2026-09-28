import Markdown from 'react-markdown';
import remarkBreaks from 'remark-breaks';
import remarkGfm from 'remark-gfm';
import type { CrewMessage, CrewMessagePeople, Snapshot } from '../crewApi';
import { personDisplayName, sanitizeUsername } from '../identity/displayText';
import { channelName, workspaceName } from '../identity/objectNames';
import { mentionPattern, rehypeCrewBodyText, type BodyNode } from '../timeline/bodyText';
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

/**
 * The remark plugins `MessageBody` parses a body with (`REMARK_PLUGINS` in
 * `timeline/MessageBody.tsx`), so a body is read here as the timeline reads it. The agreement table
 * in `crewAttention.test.ts` renders every case through `MessageBody` as well.
 */
const MESSAGE_REMARK_PLUGINS = [remarkGfm, remarkBreaks];

/** The ID the timeline's step gives its "mentions you" label when run from here. */
const MENTION_PROBE_ID = 'crew-attention-mentions-you';

/**
 * Whether the timeline marks `body` as mentioning `@username` (QA M2), decided by the timeline's own
 * rule rather than a second copy of it: the body is parsed with `MessageBody`'s markdown plugins and
 * run through `rehypeCrewBodyText`, the step that draws the mention chip, and the answer is whether
 * that step added its "mentions you" label. So a mention here is exactly one the timeline marks:
 * any case, a whole name (not `@bobby`, not `@bob.lee`, not `bob@lab.org`), not against a hidden
 * character (`@bob\u200Bx`), and outside code, code blocks and a link's words. A notification that
 * says "mentioned you" while the channel marks nothing is the failure this prevents.
 *
 * `username` is the viewer's username as the snapshot gives it; one that is not a valid username
 * mentions nobody, as in the timeline.
 */
export function mentionsUser(body: unknown, username: string | null | undefined): boolean {
  if (typeof body !== 'string' || typeof username !== 'string' || !mentionPattern(username))
    return false;
  // Every mention holds `@username` in some case (the timeline's pattern trims the name and
  // matches ASCII case-insensitively): most bodies are answered without parsing.
  if (!body.toLowerCase().includes(`@${username.trim().toLowerCase()}`)) return false;
  let mentioned = false;
  const probe = () => (tree: BodyNode) => {
    const last = tree.children?.[tree.children.length - 1];
    mentioned = last?.type === 'element' && last.properties?.id === MENTION_PROBE_ID;
    // Nothing is drawn from here: an empty tree leaves react-markdown nothing to build.
    tree.children = [];
  };
  try {
    Markdown({
      children: body,
      remarkPlugins: MESSAGE_REMARK_PLUGINS,
      rehypePlugins: [
        // The step reads and writes only the node fields it declares; the casts are to unified's tree.
        [rehypeCrewBodyText as never, { mention: username, mentionLabelId: MENTION_PROBE_ID }],
        probe as never,
      ],
    });
  } catch {
    return false;
  }
  return mentioned;
}

/** What a notification says. Names only: never a message's text. */
export interface AttentionNotice {
  title: string;
  body: string;
}

/** The workspace and channel names a snapshot gives, made safe to show. */
export function namesFrom(snapshot: Pick<Snapshot, 'workspace' | 'channels' | 'actor'>): {
  workspace: string;
  /** The viewer's username and principal ID, taken as the timeline takes them (`Timeline.tsx`). */
  username: string | null;
  viewerId: string | null;
  channel(channelId: string): string;
} {
  const channels = new Map(
    (Array.isArray(snapshot.channels) ? snapshot.channels : []).map((channel) => [
      channel.id,
      channel,
    ])
  );
  const actor = snapshot.actor as { id?: unknown; username?: unknown } | undefined;
  return {
    workspace: workspaceName(snapshot.workspace),
    username: typeof actor?.username === 'string' && actor.username ? actor.username : null,
    viewerId: typeof actor?.id === 'string' ? actor.id : null,
    channel: (channelId) => channelName(channels.get(channelId) ?? { id: channelId }),
  };
}

/** A message as the notice reads it: who posted it, whether their agent did, and its words. */
export type NoticeMessage = Pick<CrewMessage, 'actor_id' | 'body'> & { run_id?: string };

/**
 * Whether the timeline marks `message` as mentioning the viewer. Its rule, in `MessageRow.tsx`:
 * the viewer's own words never mention them, their agent's may (an agent's post carries `run_id`).
 */
function mentionsViewer(
  message: NoticeMessage,
  username: string | null,
  viewerId: string | null
): boolean {
  if (!message.run_id && viewerId !== null && message.actor_id === viewerId) return false;
  return mentionsUser(message.body, username);
}

/**
 * The notification for new messages in one channel: "Alice Chen mentioned you in #general" when
 * one of them mentions the person (the newest such message's author), otherwise "3 new messages
 * in chen-lab" with the channel under it. `messages` are the channel's newest, as read.
 */
export function attentionNotice(input: {
  workspace: string;
  channel: string;
  username: string | null;
  /** The viewer's principal ID, so their own words are never read as mentioning them. */
  viewerId?: string | null;
  added: number;
  messages: readonly NoticeMessage[];
  people?: CrewMessagePeople;
}): AttentionNotice {
  const recent = input.messages.slice(-Math.max(1, Math.min(input.added, NOTIFY_FETCH_LIMIT)));
  const mention = [...recent]
    .reverse()
    .find((message) => mentionsViewer(message, input.username, input.viewerId ?? null));
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

function messagesOf(value: unknown): NoticeMessage[] {
  if (!Array.isArray(value)) return [];
  return value
    .filter(
      (message): message is CrewMessage =>
        Boolean(message) &&
        typeof message === 'object' &&
        typeof (message as CrewMessage).actor_id === 'string' &&
        typeof (message as CrewMessage).body === 'string'
    )
    .map((message) => ({
      actor_id: message.actor_id,
      body: message.body,
      ...(typeof message.run_id === 'string' && message.run_id ? { run_id: message.run_id } : {}),
    }));
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
      viewerId: names.viewerId,
      added: rise.added,
      messages: messagesOf(latest.messages),
      people: peopleOf(latest.people),
    });
    this.deps.notify({ ...notice, key, connectionId, channelId: rise.channelId });
  }
}
