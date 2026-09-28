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
import { Button } from '../../ui/button';
import { ScrollArea, type ScrollAreaHandle } from '../../ui/scroll-area';
import { cn } from '../../../utils';
import type { Channel, CrewMessage, CrewMessagePeople, ObservedRun, Snapshot } from '../crewApi';
import { channelSlug, usePeopleDirectory } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewFrameLabels } from '../state/types';
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
import { usePendingPost, type PendingPost } from './pendingPost';
import { TaskStatusRow } from './TaskStatusRow';
import {
  TimelineContextProvider,
  type OwnAgentChat,
  type RenderAttachments,
  type TimelineContextValue,
} from './TimelineContext';
import { TimelineCopyProvider } from './TimelineCopy';
import { TimelineSkeleton } from './TimelineSkeleton';
import { dayKey, messageTime } from './timelineTime';
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
  /**
   * The window reaches the channel's first message: an older page came back short. Absent: the
   * list's own length decides (`reachesChannelStart`).
   */
  reachesStart?: boolean;
  /** A page being added to the window, if one is on its way. */
  historyLoading?: 'older' | 'newer' | null;
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

export { PENDING_POST_TIMEOUT_MS } from './pendingPost';

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
 * `role="log"` in the 760px chat column. It opens at the newest message, or at
 * the New line when there is one to read (QA M7), follows new posts while the
 * reader is at the bottom, and keeps its place when they are not (a "Jump to
 * latest" pill appears instead). A refresh never sends it to the top. Older
 * history is added above by itself at the top of a full page, with the reader's
 * place kept, and "Newer messages" follows a window that no longer reaches the
 * newest message (QA M6). The
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
          reachesStart: crew.reachesStart,
          historyLoading: crew.historyLoading,
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
  const viewerUsername =
    typeof snapshot.actor?.username === 'string' && snapshot.actor.username
      ? snapshot.actor.username
      : null;
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

  // ── The post between Send and its arrival ───────────────────────────────
  const pendingPost = usePendingPost(crew, messages, viewerId);

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
  // Posting here reads the channel to its end — the post already sends `channel.read` — so the
  // rule goes with it rather than standing above the viewer's own reply until they leave (Q4-10).
  const [postedHere, setPostedHere] = useState(false);
  if (pendingPost !== null && !readOnly && !postedHere) setPostedHere(true);
  const newLineId = postedHere ? null : newLine.id;

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
          newLineBeforeId: newLineId,
          runs,
          // Not while the page loads: a task row with nothing under it would stand above the
          // skeleton, and then jump below the messages when they land (Q2-62).
          includeUnanchoredRuns: historyBefore === null && pageReady,
          now,
        })
      ),
    [messages, channel.id, channel.classification, newLineId, runs, historyBefore, pageReady, now]
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
    // Only what arrived after the newest known message is new below: an older page added above
    // is history (QA M6).
    let lastKnown = -1;
    messages.forEach((message, index) => {
      if (ids.has(message.id)) lastKnown = index;
    });
    const added = messages.filter((message) => !ids.has(message.id));
    const below = messages.slice(lastKnown + 1).filter((message) => !ids.has(message.id));
    added.forEach((message) => ids.add(message.id));
    if (below.length > 0 && !followingRef.current && historyBefore === null) {
      setUnseenBelow(true);
      // What the pill counts is what a person would call a new message: not their own post, and
      // not an agent's folded tool update.
      const counted = below.filter(
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
  // The ids drawn at the last page that was ready: a list that shares one is the same window
  // moved (an older page added above, its newest end giving way), never a page to open afresh.
  const drawnIds = useRef<ReadonlySet<string> | null>(null);
  const movedWindow =
    drawnIds.current !== null && messages.some((message) => drawnIds.current?.has(message.id));
  useLayoutEffect(() => {
    if (!pageReady) return;
    const opening = lastLoaded.current !== loadKey && !movedWindow;
    if (opening || emptied.current || followingRef.current) {
      scrollToBottom(scroller.current, 'auto');
    }
    lastLoaded.current = loadKey;
    emptied.current = false;
    if (opening) armed.current = false;
  }, [pageReady, loadKey]); // eslint-disable-line react-hooks/exhaustive-deps -- a move is read as the page changes, not on its own
  useLayoutEffect(() => {
    if (pageReady) drawnIds.current = new Set(messages.map((message) => message.id));
    else if (reloading) drawnIds.current = null;
  }, [pageReady, reloading, messages]);

  // ── The rows, by message ────────────────────────────────────────────────
  // Rows register themselves (`registerRow`), so no message ID is ever written into the DOM.
  const rowsByMessage = useRef(new Map<string, HTMLElement>());
  const messageOfRow = useRef(new WeakMap<Element, string>());
  const registerRow = useCallback((messageId: string, element: HTMLElement | null) => {
    const rows = rowsByMessage.current;
    const before = rows.get(messageId);
    if (element) {
      rows.set(messageId, element);
      messageOfRow.current.set(element, messageId);
    } else if (before) {
      rows.delete(messageId);
    }
  }, []);
  /** The message rows on screen, in document order. */
  const messageRows = useCallback((viewport: HTMLElement) => {
    const rows: { id: string; element: HTMLElement }[] = [];
    viewport.querySelectorAll<HTMLElement>('[data-crew-row]').forEach((element) => {
      const id = messageOfRow.current.get(element);
      if (id) rows.push({ id, element });
    });
    return rows;
  }, []);

  // ── Keeping the reader's place when the window moves ────────────────────
  // Rows added above, or rows dropped from the top as the window slides, move everything under
  // them: the first row the reader could see is put back where it was (QA M6). It is measured as
  // the new list renders, while the DOM still shows the old one, and restored once the new one is
  // committed. Not while following the bottom, which is its own anchor. Scroll anchoring is off in
  // the stylesheet, so the browser never does it a second time.
  const placedList = useRef(messages);
  const place = useRef<{ id: string; top: number } | null>(null);
  if (placedList.current !== messages) {
    placedList.current = messages;
    const viewport = scroller.current?.viewportRef.current;
    place.current =
      viewport && !followingRef.current ? firstVisibleRow(viewport, messageRows(viewport)) : null;
  }
  useLayoutEffect(() => {
    const viewport = scroller.current?.viewportRef.current;
    const held = place.current;
    place.current = null;
    if (!viewport || !held || followingRef.current) return;
    const row = rowsByMessage.current.get(held.id);
    if (!row || !viewport.contains(row)) return;
    const shift = row.getBoundingClientRect().top - viewport.getBoundingClientRect().top - held.top;
    if (Math.abs(shift) >= 1) viewport.scrollTop += shift;
  }, [messages]);

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
  const loadingOlder = loadingPage || view.historyLoading === 'older';
  // The window reaches the channel's start: an older page came back short, or the live tail is
  // shorter than a page. Drawn from the list on screen, so the row stays put (as "Loading…")
  // while a page is on its way.
  const reachesStart = view.reachesStart ?? reachesChannelStart(messages, pageSize);
  const hasOlder = messagesLoaded && !reachesStart;
  const lastTop = useRef(0);
  const onViewportScroll = useCallback((viewport: HTMLDivElement) => {
    // Scrolling UP is what arms the automatic load, so a page that lands with
    // the sentinel in view does not chain into the next one by itself.
    if (viewport.scrollTop < lastTop.current) armed.current = true;
    lastTop.current = viewport.scrollTop;
  }, []);
  const loadOlder = crew.loadOlder;
  const onSentinelReached = useCallback(() => {
    if (!armed.current || readOnly || loadingOlder) return;
    armed.current = false;
    loadOlder();
  }, [readOnly, loadingOlder, loadOlder]);
  const sentinelRoot = useCallback(() => scroller.current?.viewportRef.current ?? null, []);

  // ── The unread start, and opening at the New line ───────────────────────
  // A channel with unread messages opens where they start rather than at the newest, once the New
  // line's place is fixed (QA M7): once per channel, and only while the reader has not moved away
  // from where it opened. When more is unread than the window holds, the first unread message is
  // not loaded: it opens at the newest as before, nothing is marked read (the watermark would take
  // the messages never shown with it), and a pill offers to load back to them.
  const firstUnreadOutside =
    historyBefore === null && !reachesStart && unreadBeyond(messages, readState);
  const logRef = useRef<HTMLDivElement>(null);
  const placedAtNew = useRef(false);
  useLayoutEffect(() => {
    if (placedAtNew.current || readOnly || historyBefore !== null || !newLine.computed) return;
    if (firstUnreadOutside) return;
    placedAtNew.current = true;
    if (!newLine.id || postedHere || !followingRef.current) return;
    const target = logRef.current?.querySelector<HTMLElement>('[data-crew-new-line]');
    if (!target) return;
    target.scrollIntoView({ block: 'start' });
    const atBottom = scroller.current?.isAtBottom() ?? true;
    followingRef.current = atBottom;
    setFollowing(atBottom);
  }, [newLine, readOnly, historyBefore, postedHere, days, firstUnreadOutside]);

  // Jump to first unread: older pages are added until the read position is in the window, the
  // channel's start is, or the window is full; then the New line is placed again and shown.
  //
  // A page that adds nothing ends the search where it is: one that failed (the connection bar says
  // why, and the observer leaves the window as it was), or one that held only messages already on
  // screen. Asking again at once would ask for as long as the failure lasted, with a new error each
  // time. The pill stays, so the person can try again.
  const [seeking, setSeeking] = useState(false);
  /** The page the search asked for: the window's first message then, and whether it started. */
  const seekPage = useRef<{ firstId: string | null; started: boolean } | null>(null);
  useEffect(() => {
    if (!seeking) {
      seekPage.current = null;
      return;
    }
    if (!firstUnreadOutside || readOnly) {
      seekPage.current = null;
      setSeeking(false);
      setNewLine({ computed: true, id: newLineBeforeId(messages, readState) });
      placedAtNew.current = false;
      followingRef.current = true;
      return;
    }
    const asked = seekPage.current;
    if (loadingOlder) {
      if (asked) asked.started = true;
      return;
    }
    const firstId = messages[0]?.id ?? null;
    if (asked) {
      // Its answer is on its way.
      if (!asked.started) return;
      if (asked.firstId === firstId) {
        seekPage.current = null;
        setSeeking(false);
        return;
      }
    }
    seekPage.current = { firstId, started: false };
    loadOlder();
  }, [seeking, firstUnreadOutside, loadingOlder, loadOlder, readOnly]); // eslint-disable-line react-hooks/exhaustive-deps -- the list is read as a page lands

  // ── Automatic mark-read ─────────────────────────────────────────────────
  // Up to the newest message on screen, measured, and only one the read position has not passed
  // (QA M7): opening a busy channel used to mark everything read a second later.
  const latestMessages = useRef(messages);
  latestMessages.current = messages;
  const readPosition = snapshot.read_positions?.[channel.id];
  const latestReadPosition = useRef(readPosition);
  latestReadPosition.current = readPosition;
  const newestSeen = useCallback(() => {
    const handle = scroller.current;
    const viewport = handle?.viewportRef.current;
    if (!handle || !viewport) return null;
    const list = latestMessages.current;
    let index: number;
    if (
      handle.isAtBottom() &&
      viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight <= BOTTOM_TOLERANCE_PX
    ) {
      index = list.length - 1;
    } else {
      const id = lastVisibleMessageId(viewport, messageRows(viewport));
      index = id ? list.findIndex((message) => message.id === id) : -1;
    }
    if (index < 0) return null;
    const read = latestReadPosition.current;
    const readIndex =
      typeof read === 'string' ? list.findIndex((message) => message.sequence === read) : -1;
    if (index <= readIndex) return null;
    const sequence = list[index].sequence;
    return typeof sequence === 'string' && sequence ? sequence : null;
  }, [messageRows]);
  const latest = messages[messages.length - 1];
  useAutoMarkRead({
    channelId: channel.id,
    latestSequence: typeof latest?.sequence === 'string' ? latest.sequence : null,
    readPosition,
    unread: snapshot.unread?.[channel.id],
    seen: newestSeen,
    // Not while the tail streams in (the newest message so far is not the channel's), nor while
    // the unread start is outside the window.
    enabled:
      !readOnly && historyBefore === null && opened && messages.length > 0 && !firstUnreadOutside,
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

  // ── Where the post on its way is drawn ─────────────────────────────────
  // As the message will be once it lands, so nothing moves then (Q4-19): under today's band when
  // it is the day's first message, heading a group of its own unless it continues the viewer's.
  const showPending = pendingPost !== null && !readOnly && historyBefore === null;
  const pendingNewDay = showPending && days[days.length - 1]?.key !== `day-${dayKey(now)}`;
  const pendingHead =
    showPending && (pendingNewDay || !continuesOwnGroup(days, viewerId, Date.now()));

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
      viewerUsername,
      readOnly,
      renderAttachments,
      ownAgentChats,
      activeRow,
      setActiveRow,
      arriving: arrivingSet,
      registerTaskRow,
      registerRow,
      highlightedRunId: highlighted,
      onHighlightEnd,
    }),
    [
      dir,
      viewerId,
      viewerUsername,
      readOnly,
      renderAttachments,
      ownAgentChats,
      activeRow,
      arrivingSet,
      registerTaskRow,
      registerRow,
      highlighted,
      onHighlightEnd,
    ]
  );

  const showSkeleton = reloading;
  // The intro claims the channel's start is loaded, which only the whole tail
  // can show: while it streams in, the list is short whatever the channel's
  // size. Its place is kept meanwhile (hidden, named nothing), so a short
  // channel's messages do not move down when it appears; a full page removes it.
  const intro: 'shown' | 'pending' | null = !reachesStart
    ? null
    : opened
      ? 'shown'
      : pageReady && historyBefore === null && messages.length > 0
        ? 'pending'
        : null;
  const pill: 'history' | 'live' | 'unread' | null =
    historyBefore !== null
      ? 'history'
      : unseenBelow && !following
        ? 'live'
        : firstUnreadOutside && opened && !readOnly
          ? 'unread'
          : null;

  // The copy announcer's live region sits inside the timeline's own box, beside
  // (never inside) the log, so an announcement is not read as a message.
  return (
    <div
      className={cn('crew-timeline', className)}
      data-readonly={readOnly ? 'true' : undefined}
      // A pill stands over the log's end: the log keeps room for it (QA M6).
      data-pill={pill ?? undefined}
    >
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
                ref={logRef}
                className="crew-timeline-log biorouter-focus-region"
                onKeyDown={onLogKeyDown}
                onBlur={onLogBlur}
              >
                {hasOlder && (
                  <HistorySentinel
                    loading={loadingOlder}
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
                {historyBefore !== null && messagesLoaded && (
                  <NewerMessages
                    loading={view.historyLoading === 'newer'}
                    disabled={readOnly || !crew.loadNewer}
                    onLoad={() => crew.loadNewer?.()}
                  />
                )}
                {showPending &&
                  (pendingNewDay ? (
                    <section className="crew-day" data-pending="true" aria-hidden="true" inert>
                      <DayDivider label={timelineCopy.today} />
                      <PendingPostGroup post={pendingPost} head />
                    </section>
                  ) : (
                    <PendingPostGroup post={pendingPost} head={pendingHead} />
                  ))}
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
                    : pill === 'unread'
                      ? () => setSeeking(true)
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

/**
 * The post on its way, in the box the delivered message will take: a group of its own (and its
 * 8px above) when it heads one, or the bare row when it continues the viewer's group.
 */
function PendingPostGroup({ post, head }: { post: PendingPost; head: boolean }) {
  const row = <PendingPostRow post={post} head={head} />;
  return head ? (
    <div className="crew-message-group" data-pending="true">
      {row}
    </div>
  ) : (
    row
  );
}

/** "Newer messages": the page after a window that no longer reaches the newest message (QA M6). */
function NewerMessages({
  loading,
  disabled,
  onLoad,
}: {
  loading: boolean;
  disabled: boolean;
  onLoad(): void;
}) {
  return (
    <div className="crew-history-sentinel crew-history-newer">
      <Button
        type="button"
        variant="ghost"
        size="sm"
        className="text-text-muted"
        disabled={disabled || loading}
        onClick={onLoad}
      >
        {loading ? timelineCopy.loadingNewer : timelineCopy.newer}
      </Button>
    </div>
  );
}

/** A message row on screen: the message it draws, and its element. */
interface MessageRowElement {
  id: string;
  element: HTMLElement;
}

/**
 * The first message row whose bottom is below the viewport's top, and where its top is, relative
 * to the viewport: what the reader sees first. Rows are in document order, so it is found by
 * halving.
 */
export function firstVisibleRow(
  viewport: HTMLElement,
  rows: readonly MessageRowElement[]
): { id: string; top: number } | null {
  if (rows.length === 0) return null;
  const top = viewport.getBoundingClientRect().top;
  let low = 0;
  let high = rows.length - 1;
  while (low < high) {
    const middle = Math.floor((low + high) / 2);
    if (rows[middle].element.getBoundingClientRect().bottom > top) high = middle;
    else low = middle + 1;
  }
  const row = rows[low];
  return { id: row.id, top: row.element.getBoundingClientRect().top - top };
}

/** The last message row whose end is inside the viewport: the newest message read to its end. */
export function lastVisibleMessageId(
  viewport: HTMLElement,
  rows: readonly MessageRowElement[]
): string | null {
  const box = viewport.getBoundingClientRect();
  for (let index = rows.length - 1; index >= 0; index -= 1) {
    const rect = rows[index].element.getBoundingClientRect();
    if (rect.bottom <= box.bottom + 1 && rect.bottom > box.top) return rows[index].id;
  }
  return null;
}

/**
 * More is unread than the list holds (QA M7): the read position's message is not in it, or, for a
 * channel never read, the broker counts more unread messages from others than the list has.
 */
export function unreadBeyond(messages: readonly CrewMessage[], input: NewLineInput): boolean {
  if (!(typeof input.unread === 'number' && input.unread > 0) || messages.length === 0)
    return false;
  const { readPosition } = input;
  if (typeof readPosition === 'string' && readPosition)
    return !messages.some((message) => message.sequence === readPosition);
  if (readPosition === null)
    return messages.filter((message) => message.actor_id !== input.viewerId).length < input.unread;
  return false;
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
