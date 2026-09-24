import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type MutableRefObject,
  type ReactNode,
} from 'react';
import { ScrollArea, type ScrollAreaHandle } from '../../ui/scroll-area';
import { cn } from '../../../utils';
import type { Channel, CrewMessage, ObservedRun, Snapshot } from '../crewApi';
import { channelSlug, usePeopleDirectory } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import type { CrewFrameLabels } from '../state/types';
import { ChannelIntro } from './ChannelIntro';
import { timelineCopy } from './copy';
import { DayDivider } from './DayDivider';
import {
  canBePageBefore,
  groupMessages,
  HISTORY_PAGE_SIZE,
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
import { NewDivider } from './NewDivider';
import { TaskStatusRow } from './TaskStatusRow';
import { TimelineContextProvider, type TimelineContextValue } from './TimelineContext';
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
   */
  renderAttachments?: (message: CrewMessage) => ReactNode;
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
  const dir = usePeopleDirectory(snapshot, labels);
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
  const onScrollChange = useCallback((atBottom: boolean) => {
    followingRef.current = atBottom;
    setFollowing(atBottom);
    if (atBottom) setUnseenBelow(false);
  }, []);
  const anchorBottom = useCallback(() => followingRef.current, []);

  // ── The opening: the live tail streams in, one message per frame ────────
  // The observer sends the channel's newest messages oldest first, one per
  // frame, each behind a few broker round trips (routes/crew_observation.rs), so
  // over a remote link the tail takes seconds to arrive and `messagesLoaded` is
  // true from its first message. Nothing that describes the whole channel may be
  // decided from what has arrived so far: the New line waits until its place
  // cannot move, and the intro, the automatic mark-read, `aria-busy` and live
  // arrivals wait until the opening has arrived (`useOpening`). An older page
  // lands whole.
  const readState: NewLineInput = {
    readPosition: snapshot.read_positions?.[channel.id],
    unread: snapshot.unread?.[channel.id],
    viewerId,
  };
  const progress = historyBefore === null ? openingProgress(messages, readState) : 'complete';
  const reloading = !messagesLoaded && messages.length === 0;
  const opened = useOpening({
    loadKey,
    pageReady,
    reloading,
    progress,
    size: messages.length,
    newestId: messages[messages.length - 1]?.id ?? null,
  });

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
          includeUnanchoredRuns: historyBefore === null,
          now,
        })
      ),
    [messages, channel.id, channel.classification, newLine.id, runs, historyBefore, now]
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
    if (added.length > 0 && !followingRef.current && historyBefore === null) setUnseenBelow(true);
  }, [messages, pageReady, loadKey, historyBefore]);

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

  // ── Older history ───────────────────────────────────────────────────────
  const loadingPage = !pageReady;
  // Drawn from the list on screen, so the row stays put (as "Loading…") while
  // the previous page is still drawn.
  const hasOlder = messagesLoaded && messages.length >= HISTORY_PAGE_SIZE;
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
  const latest = messages[messages.length - 1];
  useAutoMarkRead({
    channelId: channel.id,
    latestSequence: typeof latest?.sequence === 'string' ? latest.sequence : null,
    readPosition: snapshot.read_positions?.[channel.id],
    unread: snapshot.unread?.[channel.id],
    atBottom: following,
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

  // ── Keyboard: ↑/↓ move between rows, Home/End to the ends ────────────────
  const [activeRow, setActiveRow] = useState<string | null>(null);
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
  const intro: 'shown' | 'pending' | null = !reachesChannelStart(messages)
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
                aria-live="polite"
                aria-label={timelineCopy.logLabel(slug)}
                aria-busy={opened ? undefined : 'true'}
                tabIndex={0}
                className="crew-timeline-log biorouter-focus-region"
                onKeyDown={onLogKeyDown}
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
                    {day.label && <DayDivider label={day.label} />}
                    {day.items.map((item) => (
                      <TimelineEntry key={item.key} item={item} />
                    ))}
                  </section>
                ))}
              </div>
            </div>
          </ScrollArea>
          {pill && (
            <div className="crew-timeline-pill-slot">
              <JumpPill
                mode={pill}
                disabled={readOnly}
                onJump={
                  pill === 'history'
                    ? crew.jumpToLatest
                    : () => {
                        setUnseenBelow(false);
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
