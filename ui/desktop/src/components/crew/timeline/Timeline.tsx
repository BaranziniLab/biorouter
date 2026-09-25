import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type FocusEvent,
  type KeyboardEvent,
  type MutableRefObject,
} from 'react';
import { ScrollArea, type ScrollAreaHandle } from '../../ui/scroll-area';
import { cn } from '../../../utils';
import type { Channel, CrewMessage, CrewMessagePeople, ObservedRun, Snapshot } from '../crewApi';
import { channelSlug, usePeopleDirectory } from '../identity';
import { crewActionCopy } from '../state/copy';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewController, CrewFrameLabels } from '../state/types';
import { ChannelIntro } from './ChannelIntro';
import { timelineCopy } from './copy';
import { DayDivider } from './DayDivider';
import {
  canBePageBefore,
  GROUP_GAP_MS,
  groupMessages,
  HISTORY_PAGE_SIZE,
  isTraceMessage,
  keepUnchangedGroups,
  newLineBeforeId,
  newLineDecided,
  openingProgress,
  reachesChannelStart,
  type NewLineInput,
  type TimelineDay,
  type TimelineItem,
} from './groupMessages';
import { HistorySentinel } from './HistorySentinel';
import { JumpPill } from './JumpPill';
import { MessageGroup } from './MessageGroup';
import { PendingPostRow } from './MessageRow';
import { NewDivider } from './NewDivider';
import { TaskStatusRow } from './TaskStatusRow';
import {
  TimelineContextProvider,
  type OwnAgentChat,
  type RenderAttachments,
  type TimelineContextValue,
} from './TimelineContext';
import { TimelineCopyProvider } from './TimelineCopy';
import { TimelineSkeleton } from './TimelineSkeleton';
import { messageTime } from './timelineTime';
import { useAutoMarkRead, type AutoReadMemory } from './useAutoMarkRead';
import { useOpening } from './useOpening';
import '../crew-app.css';
import './timeline.css';

/** The channel data a timeline draws. The controller's verified state by default. */
export interface TimelineView {
  snapshot: Snapshot;
  channel: Channel;
  messages: CrewMessage[];
  /** False while the channel's messages (or an older page) are loading. */
  messagesLoaded: boolean;
  /** The viewer's own runs (`state.runs` is owner-scoped). */
  runs: ObservedRun[];
  labels: CrewFrameLabels | null;
  /** The sequence an older page is shown before, or null for the live tail. */
  historyBefore: string | null;
  /** The size of a full page of this view. Absent: `HISTORY_PAGE_SIZE`. */
  pageSize?: number;
  /**
   * The observer's word on the live tail's backlog (`controller.backlogComplete`).
   * Absent when it gives none: the stream is then timed (`useOpening`).
   */
  backlogComplete?: boolean;
  /** Authors the message pages named, including people who have left. Display only. */
  people?: CrewMessagePeople | null;
}

export interface TimelineProps {
  /**
   * Draw this view instead of the controller's verified one — the layout passes
   * the last verified view (`controller.lastVerified`) during re-verification,
   * together with `readOnly`.
   */
  view?: TimelineView | null;
  /** Presentation only: no action is enabled and nothing is marked read. */
  readOnly?: boolean;
  /**
   * Attachments and server paths under a message body. The files area renders
   * them. Pass a stable callback (`useCallback`): a new one re-renders every row.
   * It is told whether its row is the active one, so its controls are Tab stops
   * only then (Q3-05).
   */
  renderAttachments?: RenderAttachments;
  /**
   * The viewer's own chats that hold Crew access, by the run they post as: those
   * posts read "Your agent · {chat title}" (Q3-22). The layout builds it from
   * this device's grants, so it holds only the viewer's chats. Pass a stable map.
   */
  ownAgentChats?: ReadonlyMap<string, OwnAgentChat>;
  /**
   * A task to bring into view and wash with the highlight once (a new task,
   * "Show task in channel", an Agents row). Cleared by `onHighlightDone`.
   */
  highlightRunId?: string | null;
  onHighlightDone?: () => void;
  /** Layout only. */
  className?: string;
}

const NO_IDS: ReadonlySet<string> = new Set<string>();
const NO_AGENT_CHATS: ReadonlyMap<string, OwnAgentChat> = new Map();

/**
 * Whether a post the viewer sends now would land in the group the log ends with: their own
 * message (as a person, not their agent), within the grouping window. Only then does the
 * "Sending…" row go without the viewer's avatar and name (Q3-20).
 */
function continuesOwnGroup(days: readonly TimelineDay[], viewerId: string | null, now: number) {
  if (viewerId === null) return false;
  const lastDay = days[days.length - 1];
  const last = lastDay?.items[lastDay.items.length - 1];
  if (!last || last.kind !== 'group' || last.agent || last.authorId !== viewerId) return false;
  const lastEntry = last.entries[last.entries.length - 1];
  return Boolean(lastEntry) && now - lastEntry.time.getTime() <= GROUP_GAP_MS;
}

/**
 * How long a "Sending…" row waits for the observer to deliver its message
 * before it goes quietly. The broker accepted the post, so the message is
 * coming; this only keeps a stalled observation from leaving the row for good.
 */
export const PENDING_POST_TIMEOUT_MS = 30_000;

/** A post the broker has accepted, from this timeline's composer. */
interface PendingPost {
  body: string;
  /** Messages already on screen when it was sent: the delivered one is not among them. */
  before: ReadonlySet<string>;
}

/** The draft as a send began: what to recognize the delivered message by. */
interface PostAttempt extends PendingPost {
  attachments: readonly string[];
}

/**
 * Whether the send that just settled was accepted. The controller's `send()`
 * says nothing (and `state/*` is not this area's to change), so it is read the
 * way the composer sees it: an accepted post clears exactly what was sent from
 * the draft, and a refused one leaves the draft and records a composer error. A
 * post whose only trouble was the kept upload record still went out. A draft
 * cleared because the verified view was dropped proves nothing either way.
 */
function postAccepted(attempt: PostAttempt, crew: CrewController): boolean {
  const { error, draft, snapshot } = crew;
  if (error?.source === 'composer' && error.message !== crewActionCopy.sendTransferRecordKept) {
    return false;
  }
  // A reset that dropped the verified view (and cleared the draft with it) is not an answer.
  if (!snapshot) return false;
  const bodyCleared = !attempt.body.trim() || !draft.body.trim();
  const filesCleared = attempt.attachments.every(
    (id) => !draft.attachments.some((file) => file.id === id)
  );
  return bodyCleared && filesCleared;
}

function isDelivery(post: PendingPost, message: CrewMessage, viewerId: string | null): boolean {
  return (
    viewerId !== null &&
    !post.before.has(message.id) &&
    message.actor_id === viewerId &&
    !message.run_id &&
    message.body.trim() === post.body.trim()
  );
}

/**
 * The post between Send and its arrival (T-37). The send is not optimistic —
 * the draft stays until the broker answers — but once it has answered the draft
 * is cleared, and the message arrived only when the observer next delivered it,
 * seconds later, with nothing on screen in between. So from the answer until
 * the message is in the list, a dimmed "Sending…" row stands in for it. It is
 * matched by who posted it and its words, among messages that were not already
 * on screen; `send()` returns no message ID to match by.
 */
function usePendingPost(
  crew: CrewController,
  messages: readonly CrewMessage[],
  viewerId: string | null
): PendingPost | null {
  const posting = crew.isPending('send');
  const [pending, setPending] = useState<PendingPost | null>(null);
  const attempt = useRef<PostAttempt | null>(null);
  const wasPosting = useRef(false);
  const latest = useRef({ crew, messages });
  latest.current = { crew, messages };
  useEffect(() => {
    const was = wasPosting.current;
    wasPosting.current = posting;
    const { crew: now, messages: list } = latest.current;
    if (posting && !was) {
      attempt.current = {
        body: now.draft.body,
        attachments: now.draft.attachments.map((file) => file.id),
        before: new Set(list.map((message) => message.id)),
      };
    } else if (!posting && was) {
      const sent = attempt.current;
      attempt.current = null;
      setPending(sent && postAccepted(sent, now) ? { body: sent.body, before: sent.before } : null);
    }
  }, [posting]);
  const delivered =
    pending !== null && messages.some((message) => isDelivery(pending, message, viewerId));
  useEffect(() => {
    if (delivered) setPending(null);
  }, [delivered]);
  useEffect(() => {
    if (!pending) return;
    const timer = window.setTimeout(() => setPending(null), PENDING_POST_TIMEOUT_MS);
    return () => window.clearTimeout(timer);
  }, [pending]);
  return pending && !delivered ? pending : null;
}

/** How close to the bottom edge the newest row counts as on screen for mark-read. */
const BOTTOM_TOLERANCE_PX = 4;

/**
 * `ScrollAreaHandle.scrollToBottom`, where the element can scroll that way. A
 * viewport without `Element.scrollTo` (jsdom) is set directly, so a test — and
 * the integration tests that mount this layout — never trip over it.
 */
function scrollToBottom(handle: ScrollAreaHandle | null, behavior: 'auto' | 'smooth') {
  const viewport = handle?.viewportRef.current;
  if (!handle || !viewport) return;
  if (typeof viewport.scrollTo === 'function') handle.scrollToBottom(behavior);
  else viewport.scrollTop = viewport.scrollHeight;
}

/**
 * Run `callback` once a frame has been drawn after now: two animation frames,
 * so the frame between them has been painted and its accessibility tree sent.
 * Returns the cancel.
 */
function afterAFrame(callback: () => void): () => void {
  if (typeof window.requestAnimationFrame !== 'function') {
    const timer = window.setTimeout(callback, 32);
    return () => window.clearTimeout(timer);
  }
  let second: number | null = null;
  const first = window.requestAnimationFrame(() => {
    second = window.requestAnimationFrame(callback);
  });
  return () => {
    window.cancelAnimationFrame(first);
    if (second !== null) window.cancelAnimationFrame(second);
  };
}

function prefersReducedMotion(): boolean {
  return (
    typeof window.matchMedia === 'function' &&
    window.matchMedia('(prefers-reduced-motion: reduce)').matches
  );
}

/**
 * A channel's messages, read like Slack (ui-redesign-spec, "The timeline").
 *
 * `ScrollArea` — the chat transcript's primitive, following the bottom and
 * keeping it anchored when the viewport changes height — around a
 * `role="log"` in the 760px chat column. It opens at the newest message,
 * follows new posts while the reader is at the bottom, and keeps its place when
 * they are not (a "Jump to latest" pill appears instead). A refresh never sends
 * it to the top. Older history loads by itself at the top of a full page. The
 * live tail streams in one message per frame, so what describes the whole
 * channel — the New line, the intro, mark-read, `aria-busy`, arrivals — waits
 * until enough of it has arrived (`openingProgress`, `useOpening`).
 *
 * Messages are grouped by author and time, broken by day dividers and a fixed
 * New line; agents read as agents; an agent's tool updates fold behind "Show
 * details"; the viewer's own tasks are status rows at the point they were
 * requested. Nothing animates except a live arrival while following, the pills
 * and the highlight wash.
 *
 * Reads `useCrew()`. Imports no other area: attachments come through
 * `renderAttachments`, and Stop opens the confirmation through a dialog intent.
 */
export function Timeline({ view, ...props }: TimelineProps) {
  const crew = useCrew();
  // One memory for every channel, so switching away and back cannot write twice in five seconds.
  const readMemory = useRef<AutoReadMemory>(new Map());
  const current: TimelineView | null =
    view ??
    (crew.snapshot && crew.channel
      ? {
          snapshot: crew.snapshot,
          channel: crew.channel,
          messages: crew.messages,
          messagesLoaded: crew.messagesLoaded,
          runs: crew.runs,
          labels: crew.labels,
          historyBefore: crew.historyBefore,
          pageSize: crew.pageSize,
          backlogComplete: crew.backlogComplete,
          people: crew.people,
        }
      : null);
  if (!current) return null;
  // Keyed per channel: scroll state, the New line and arrivals start fresh in each one.
  return (
    <ChannelTimeline key={current.channel.id} view={current} readMemory={readMemory} {...props} />
  );
}

function ChannelTimeline({
  view,
  readOnly = false,
  renderAttachments,
  ownAgentChats = NO_AGENT_CHATS,
  highlightRunId = null,
  onHighlightDone,
  className,
  readMemory,
}: Omit<TimelineProps, 'view'> & {
  view: TimelineView;
  readMemory: MutableRefObject<AutoReadMemory>;
}) {
  const crew = useCrew();
  const { snapshot, channel, messages, messagesLoaded, runs, labels, historyBefore } = view;
  const pageSize =
    typeof view.pageSize === 'number' && view.pageSize > 0 ? view.pageSize : HISTORY_PAGE_SIZE;
  // The observer's word on the backlog, for the live tail only: an older page lands whole.
  const backlogComplete = historyBefore === null ? view.backlogComplete : undefined;
  const dir = usePeopleDirectory(snapshot, labels, view.people ?? null);
  const viewerId = typeof snapshot.actor?.id === 'string' ? snapshot.actor.id : null;
  const slug = channelSlug(channel);
  const scroller = useRef<ScrollAreaHandle>(null);
  const loadKey = historyBefore ?? 'live';

  // ── Is the list on screen this page's? ──────────────────────────────────
  // Loading an older page moves the boundary first and clears the list a
  // render later (useCrewController `loadOlder`, then useCrewObservation's
  // history effect), so for one render the previous page is drawn under the new
  // page's key. That list is drawn as it is, but it is not the new page: it
  // seeds no arrivals, opens nothing at its bottom, draws no New line and marks
  // nothing read, and the log is busy. It is recognized as the very list drawn
  // under another key, or — for an older page — by holding the boundary message.
  // An empty list is never taken for the previous page: one shared empty array
  // must not hold a new key busy.
  const drawn = useRef<{ key: string; messages: readonly CrewMessage[] } | null>(null);
  const previousPage =
    messagesLoaded &&
    messages.length > 0 &&
    ((drawn.current !== null &&
      drawn.current.key !== loadKey &&
      drawn.current.messages === messages) ||
      !canBePageBefore(messages, historyBefore));
  /** The list on screen is this page, loaded. Everything that acts on a page asks this. */
  const pageReady = messagesLoaded && !previousPage;
  useLayoutEffect(() => {
    if (pageReady) drawn.current = { key: loadKey, messages };
  }, [pageReady, loadKey, messages]);

  // ── Following the bottom ────────────────────────────────────────────────
  const [following, setFollowing] = useState(true);
  const followingRef = useRef(true);
  const [unseenBelow, setUnseenBelow] = useState(false);
  /** How many messages from others arrived below while the reader was scrolled up (Q3-27). */
  const [unseenCount, setUnseenCount] = useState(0);
  const onScrollChange = useCallback((atBottom: boolean) => {
    followingRef.current = atBottom;
    setFollowing(atBottom);
    if (atBottom) {
      setUnseenBelow(false);
      setUnseenCount(0);
    }
  }, []);
  const anchorBottom = useCallback(() => followingRef.current, []);

  // ── The opening: the live tail streams in, one message per frame ────────
  // The observer sends the channel's newest messages oldest first, one per
  // frame, each behind a few broker round trips (routes/crew_observation.rs), so
  // over a remote link the tail takes seconds to arrive and `messagesLoaded` is
  // true from its first message. Nothing that describes the whole channel may be
  // decided from what has arrived so far: the New line waits until its place
  // cannot move, and the intro, the automatic mark-read, `aria-busy` and live
  // arrivals wait until the opening has arrived (`useOpening`) — which the
  // observer says itself (`remaining`, `backlogComplete`) when it is new enough
  // to. An older page lands whole.
  const readState: NewLineInput = {
    readPosition: snapshot.read_positions?.[channel.id],
    unread: snapshot.unread?.[channel.id],
    viewerId,
  };
  const progress =
    historyBefore !== null || backlogComplete === true
      ? 'complete'
      : openingProgress(messages, readState, pageSize);
  const reloading = !messagesLoaded && messages.length === 0;
  const newestId = messages[messages.length - 1]?.id ?? null;
  const opened = useOpening({
    loadKey,
    pageReady,
    reloading,
    progress,
    size: messages.length,
    newestId,
    backlogComplete,
  });

  // ── What the log announces ──────────────────────────────────────────────
  // A polite log reads out what is inserted into it, so while a page streams or
  // lands in (the opening, an older page) it is `aria-live="off"` as well as
  // busy: VoiceOver does not honour `aria-busy` reliably, and read the opening
  // out as a flood of messages. It turns polite only after a frame has been
  // drawn with the page in it, so the insertions that opened it are already
  // behind it when the region starts listening.
  //
  // Once live, the attribute is left off rather than set to "polite": `role="log"` is polite by
  // itself, and a modal's `hideOthers` (aria-hidden) keeps every element that carries an
  // `aria-live` attribute, and its ancestors, in the accessibility tree. With the attribute on,
  // every Crew dialog left the whole log and its buttons reachable behind it (Q2-13).
  const [liveKey, setLiveKey] = useState<string | null>(null);
  useEffect(() => {
    if (!opened) return;
    return afterAFrame(() => setLiveKey(loadKey));
  }, [opened, loadKey]);
  const live = opened && liveKey === loadKey;

  // ── The New line: fixed once, as soon as the live tail decides its place ──
  const [newLine, setNewLine] = useState<{ computed: boolean; id: string | null }>({
    computed: false,
    id: null,
  });
  if (
    !newLine.computed &&
    pageReady &&
    historyBefore === null &&
    (opened || progress === 'caught-up' || newLineDecided(messages, readState))
  ) {
    setNewLine({ computed: true, id: newLineBeforeId(messages, readState) });
  }

  // "Today" becomes "Yesterday" at midnight even when nothing new arrives.
  const [now, setNow] = useState(() => new Date());
  useEffect(() => {
    const midnight = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1);
    const timer = window.setTimeout(
      () => setNow(new Date()),
      midnight.getTime() - Date.now() + 1000
    );
    return () => window.clearTimeout(timer);
  }, [now]);

  // Groups that did not change keep their objects, so only the group a new
  // message joins re-renders — not every row on every frame of a stream.
  const drawnDays = useRef<TimelineDay[]>([]);
  const days = useMemo(
    () =>
      keepUnchangedGroups(
        drawnDays.current,
        groupMessages(messages, {
          channelId: channel.id,
          channelRestricted: channel.classification === 'restricted',
          newLineBeforeId: newLine.id,
          runs,
          // Not while the page loads: a task row with nothing under it would stand above the
          // skeleton, and then jump below the messages when they land (Q2-62).
          includeUnanchoredRuns: historyBefore === null && pageReady,
          now,
        })
      ),
    [messages, channel.id, channel.classification, newLine.id, runs, historyBefore, pageReady, now]
  );
  useEffect(() => {
    drawnDays.current = days;
  }, [days]);

  // ── Live arrivals ───────────────────────────────────────────────────────
  // A live arrival is a message posted after this page opened that appears once
  // the opening has arrived. It rises in only while the reader follows the
  // bottom; otherwise the live pill shows. The rest of the backlog, still
  // streaming in, was posted before the page opened: that — not whether it is
  // on screen yet — is what keeps it still. Both tests hold together, so neither
  // a misjudged end of the stream nor a skewed broker clock animates history.
  // Only the page's own list seeds what was there: seeded from the previous
  // page, every message of an older page would count as new.
  const [openedAt, setOpenedAt] = useState(() => ({ key: loadKey, at: Date.now() }));
  if (openedAt.key !== loadKey) setOpenedAt({ key: loadKey, at: Date.now() });
  const known = useRef<{ key: string; ids: Set<string> } | null>(null);
  const arriving = useRef<Set<string>>(new Set());
  const tracking = known.current;
  const fresh =
    pageReady && opened && openedAt.key === loadKey && tracking?.key === loadKey
      ? messages.filter(
          (message) =>
            !tracking.ids.has(message.id) && messageTime(message.created_at).getTime() > openedAt.at
        )
      : [];
  if (fresh.length > 0 && followingRef.current) {
    fresh.forEach((message) => arriving.current.add(message.id));
  }
  useEffect(() => {
    if (!pageReady) return;
    if (known.current?.key !== loadKey) {
      known.current = { key: loadKey, ids: new Set(messages.map((message) => message.id)) };
      arriving.current = new Set();
      return;
    }
    const ids = known.current.ids;
    const added = messages.filter((message) => !ids.has(message.id));
    added.forEach((message) => ids.add(message.id));
    if (added.length > 0 && !followingRef.current && historyBefore === null) {
      setUnseenBelow(true);
      // What the pill counts is what a person would call a new message: not their own post, and
      // not an agent's folded tool update.
      const counted = added.filter(
        (message) => !isTraceMessage(message) && !(message.actor_id === viewerId && !message.run_id)
      ).length;
      if (counted > 0) setUnseenCount((count) => count + counted);
    }
  }, [messages, pageReady, loadKey, historyBefore, viewerId]);

  // ── Opening at the newest message ───────────────────────────────────────
  // A channel (and an older page) opens at its newest message. A reload that
  // emptied the list lands there too — that was the jump to the top when an
  // agent started. A reload that kept the list keeps the reader's place,
  // unless they were following the bottom. The previous page, still on screen
  // while an older one is requested, is not opened: the reader stays at the
  // top, where they asked for more, until the page lands.
  const lastLoaded = useRef<string | null>(null);
  const emptied = useRef(false);
  /** Set by the reader scrolling up; the sentinel loads an older page only when armed. */
  const armed = useRef(false);
  if (reloading) emptied.current = true;
  useLayoutEffect(() => {
    if (!pageReady) return;
    if (lastLoaded.current !== loadKey || emptied.current || followingRef.current) {
      scrollToBottom(scroller.current, 'auto');
    }
    lastLoaded.current = loadKey;
    emptied.current = false;
    armed.current = false;
  }, [pageReady, loadKey]);

  // ── Following the live tail ─────────────────────────────────────────────
  // A full tail keeps its last page (useCrewObservation drops the oldest
  // message as the newest arrives), so an arrival need not grow the content:
  // the scroll area, which follows growth, then leaves the newest message below
  // the fold. So a new newest message on the page already open is followed
  // here, while the reader follows the bottom. Opening a page is the effect
  // above's, and is not scrolled twice.
  const followed = useRef<{ key: string; id: string | null } | null>(null);
  useLayoutEffect(() => {
    const previous = followed.current;
    followed.current = pageReady ? { key: loadKey, id: newestId } : null;
    if (!pageReady || historyBefore !== null || !followingRef.current) return;
    if (!previous || previous.key !== loadKey || previous.id === newestId) return;
    scrollToBottom(scroller.current, 'auto');
  }, [newestId, pageReady, loadKey, historyBefore]);

  // ── Older history ───────────────────────────────────────────────────────
  const loadingPage = !pageReady;
  // Drawn from the list on screen, so the row stays put (as "Loading…") while
  // the previous page is still drawn.
  const hasOlder = messagesLoaded && messages.length >= pageSize;
  const lastTop = useRef(0);
  const onViewportScroll = useCallback((viewport: HTMLDivElement) => {
    // Scrolling UP is what arms the automatic load, so a page that lands with
    // the sentinel in view does not chain into the next one by itself.
    if (viewport.scrollTop < lastTop.current) armed.current = true;
    lastTop.current = viewport.scrollTop;
  }, []);
  const loadOlder = crew.loadOlder;
  const onSentinelReached = useCallback(() => {
    if (!armed.current || readOnly || loadingPage) return;
    armed.current = false;
    loadOlder();
  }, [readOnly, loadingPage, loadOlder]);
  const sentinelRoot = useCallback(() => scroller.current?.viewportRef.current ?? null, []);

  // ── Automatic mark-read ─────────────────────────────────────────────────
  // `following` is the scroll area's last verdict; the newest row must also be
  // on screen, measured, when the dwell ends.
  const newestOnScreen = useCallback(() => {
    const handle = scroller.current;
    const viewport = handle?.viewportRef.current;
    if (!handle || !viewport) return false;
    return (
      handle.isAtBottom() &&
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= BOTTOM_TOLERANCE_PX
    );
  }, []);
  const latest = messages[messages.length - 1];
  useAutoMarkRead({
    channelId: channel.id,
    latestSequence: typeof latest?.sequence === 'string' ? latest.sequence : null,
    readPosition: snapshot.read_positions?.[channel.id],
    unread: snapshot.unread?.[channel.id],
    atBottom: following,
    isAtBottom: newestOnScreen,
    // Not while the tail streams in: the newest message so far is not the channel's.
    enabled: !readOnly && historyBefore === null && opened && messages.length > 0,
    markRead: crew.markRead,
    memory: readMemory,
  });

  // ── Task highlight ──────────────────────────────────────────────────────
  const taskRows = useRef(new Map<string, HTMLElement>());
  const registerTaskRow = useCallback((runId: string, element: HTMLElement | null) => {
    if (element) taskRows.current.set(runId, element);
    else taskRows.current.delete(runId);
  }, []);
  const [highlighted, setHighlighted] = useState<string | null>(null);
  const handledHighlight = useRef<string | null>(null);
  useEffect(() => {
    if (!highlightRunId) {
      handledHighlight.current = null;
      return;
    }
    if (handledHighlight.current === highlightRunId) return;
    const row = taskRows.current.get(highlightRunId);
    if (!row) return;
    handledHighlight.current = highlightRunId;
    row.scrollIntoView({ block: 'center', behavior: prefersReducedMotion() ? 'auto' : 'smooth' });
    setHighlighted(highlightRunId);
  }, [highlightRunId, days]);
  // The callback is read through a ref, so an inline one from the layout does
  // not change the context every render.
  const highlightDone = useRef(onHighlightDone);
  useEffect(() => {
    highlightDone.current = onHighlightDone;
  }, [onHighlightDone]);
  const onHighlightEnd = useCallback((runId: string) => {
    setHighlighted((current) => (current === runId ? null : current));
    highlightDone.current?.();
  }, []);

  // ── The post between Send and its arrival ───────────────────────────────
  const pendingPost = usePendingPost(crew, messages, viewerId);
  const showPending = pendingPost !== null && !readOnly && historyBefore === null;
  const pendingHead = showPending && !continuesOwnGroup(days, viewerId, Date.now());

  // ── Keyboard: ↑/↓ move between rows, Home/End to the ends ────────────────
  // The active row's actions are Tab stops only while focus is inside the log. Once focus leaves
  // it (Tab onward, a click elsewhere, the window losing focus), no row is active and Tab from the
  // header goes straight through the log to the composer: a row focused once used to keep its
  // Copy text in the Tab order, so a person typing after a Tab pressed it with a space (Q2-12).
  const [activeRow, setActiveRow] = useState<string | null>(null);
  const onLogBlur = (event: FocusEvent<HTMLDivElement>) => {
    const next = event.relatedTarget;
    if (next instanceof Node && event.currentTarget.contains(next)) return;
    setActiveRow(null);
  };
  const onLogKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (!['ArrowUp', 'ArrowDown', 'Home', 'End'].includes(event.key)) return;
    const log = event.currentTarget;
    const rows = Array.from(log.querySelectorAll<HTMLElement>('[data-crew-row]'));
    const target = event.target as HTMLElement;
    const index = target === log ? -1 : rows.indexOf(target);
    // Inside a control (a link, a button, an open disclosure) the keys are its own.
    if (rows.length === 0 || (target !== log && index < 0)) return;
    event.preventDefault();
    let next: number;
    if (event.key === 'Home') next = 0;
    else if (event.key === 'End') next = rows.length - 1;
    else if (event.key === 'ArrowUp') next = index < 0 ? rows.length - 1 : Math.max(0, index - 1);
    else next = index < 0 ? 0 : Math.min(rows.length - 1, index + 1);
    rows[next].focus();
    rows[next].scrollIntoView({ block: 'nearest' });
  };

  // Stable between renders that change nothing here: the controller is a new
  // object on every composer keystroke, and the rows (memoized per group) must
  // not re-render with it. `arriving` keeps one identity while arrivals are
  // added to it; a row that arrives is a new component and reads it on mount.
  const arrivingSet = arriving.current.size > 0 ? arriving.current : NO_IDS;
  const context = useMemo<TimelineContextValue>(
    () => ({
      dir,
      viewerId,
      readOnly,
      renderAttachments,
      ownAgentChats,
      activeRow,
      setActiveRow,
      arriving: arrivingSet,
      registerTaskRow,
      highlightedRunId: highlighted,
      onHighlightEnd,
    }),
    [
      dir,
      viewerId,
      readOnly,
      renderAttachments,
      ownAgentChats,
      activeRow,
      arrivingSet,
      registerTaskRow,
      highlighted,
      onHighlightEnd,
    ]
  );

  const showSkeleton = reloading;
  // The intro claims the channel's start is loaded, which only the whole tail
  // can show: while it streams in, the list is short whatever the channel's
  // size. Its place is kept meanwhile (hidden, named nothing), so a short
  // channel's messages do not move down when it appears; a full page removes it.
  const intro: 'shown' | 'pending' | null = !reachesChannelStart(messages, pageSize)
    ? null
    : opened
      ? 'shown'
      : pageReady && historyBefore === null && messages.length > 0
        ? 'pending'
        : null;
  const pill: 'history' | 'live' | null =
    historyBefore !== null ? 'history' : unseenBelow && !following ? 'live' : null;

  // The copy announcer's live region sits inside the timeline's own box, beside
  // (never inside) the log, so an announcement is not read as a message.
  return (
    <div className={cn('crew-timeline', className)} data-readonly={readOnly ? 'true' : undefined}>
      <TimelineCopyProvider>
        <TimelineContextProvider value={context}>
          <ScrollArea
            ref={scroller}
            className="crew-timeline-scroll biorouter-scroll-fade-top"
            autoScroll
            anchorBottomOnResize={anchorBottom}
            onScrollChange={onScrollChange}
            handleScroll={onViewportScroll}
          >
            <div className="crew-timeline-column max-w-measure-chat mx-auto">
              <div
                role="log"
                aria-live={live ? undefined : 'off'}
                aria-label={timelineCopy.logLabel(slug)}
                aria-description={timelineCopy.logDescription}
                aria-busy={opened && !loadingPage ? undefined : 'true'}
                tabIndex={0}
                className="crew-timeline-log biorouter-focus-region"
                onKeyDown={onLogKeyDown}
                onBlur={onLogBlur}
              >
                {hasOlder && (
                  <HistorySentinel
                    loading={loadingPage}
                    disabled={readOnly}
                    onLoad={loadOlder}
                    onReached={onSentinelReached}
                    root={sentinelRoot}
                  />
                )}
                {intro && (
                  <ChannelIntro
                    channel={channel}
                    viewerId={viewerId}
                    dir={dir}
                    readOnly={readOnly}
                    pending={intro === 'pending'}
                  />
                )}
                {showSkeleton && (
                  <TimelineSkeleton
                    label={
                      historyBefore !== null
                        ? timelineCopy.loadingOlder
                        : timelineCopy.loadingMessages
                    }
                  />
                )}
                {days.map((day) => (
                  <section key={day.key} className="crew-day">
                    {day.label && <DayDivider label={day.label} newOnRule={day.newOnRule} />}
                    {day.items.map((item) => (
                      <TimelineEntry key={item.key} item={item} />
                    ))}
                  </section>
                ))}
                {showPending && <PendingPostRow body={pendingPost.body} head={pendingHead} />}
              </div>
            </div>
          </ScrollArea>
          {pill && (
            <div className="crew-timeline-pill-slot">
              <JumpPill
                mode={pill}
                disabled={readOnly}
                count={pill === 'live' ? unseenCount : 0}
                onJump={
                  pill === 'history'
                    ? crew.jumpToLatest
                    : () => {
                        setUnseenBelow(false);
                        setUnseenCount(0);
                        scrollToBottom(
                          scroller.current,
                          prefersReducedMotion() ? 'auto' : 'smooth'
                        );
                      }
                }
              />
            </div>
          )}
        </TimelineContextProvider>
      </TimelineCopyProvider>
    </div>
  );
}

function TimelineEntry({ item }: { item: TimelineItem }) {
  switch (item.kind) {
    case 'new':
      return <NewDivider />;
    case 'task':
      return <TaskStatusRow task={item} />;
    case 'group':
      return <MessageGroup group={item} />;
  }
}
