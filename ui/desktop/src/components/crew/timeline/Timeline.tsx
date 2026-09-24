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
  groupMessages,
  HISTORY_PAGE_SIZE,
  newLineBeforeId,
  reachesChannelStart,
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
import { useAutoMarkRead, type AutoReadMemory } from './useAutoMarkRead';
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
 * it to the top. Older history loads by itself at the top of a full page.
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

  // ── The New line: computed once, when the channel's live tail first loads ──
  const [newLine, setNewLine] = useState<{ computed: boolean; id: string | null }>({
    computed: false,
    id: null,
  });
  if (!newLine.computed && messagesLoaded && historyBefore === null) {
    setNewLine({
      computed: true,
      id: newLineBeforeId(messages, {
        readPosition: snapshot.read_positions?.[channel.id],
        unread: snapshot.unread?.[channel.id],
        viewerId,
      }),
    });
  }

  const days = useMemo(
    () =>
      groupMessages(messages, {
        channelId: channel.id,
        channelRestricted: channel.classification === 'restricted',
        newLineBeforeId: newLine.id,
        runs,
        includeUnanchoredRuns: historyBefore === null,
        now: new Date(),
      }),
    [messages, channel.id, channel.classification, newLine.id, runs, historyBefore]
  );

  // ── Live arrivals ───────────────────────────────────────────────────────
  // What was on screen when this page (the live tail, or one older page) first
  // loaded is not an arrival; a message that appears afterwards is. It rises in
  // only while the reader follows the bottom; otherwise the live pill shows.
  const known = useRef<{ key: string; ids: Set<string> } | null>(null);
  const arriving = useRef<Set<string>>(new Set());
  const tracking = known.current;
  const fresh =
    messagesLoaded && tracking?.key === loadKey
      ? messages.filter((message) => !tracking.ids.has(message.id))
      : [];
  if (fresh.length > 0 && followingRef.current) {
    fresh.forEach((message) => arriving.current.add(message.id));
  }
  useEffect(() => {
    if (!messagesLoaded) return;
    if (known.current?.key !== loadKey) {
      known.current = { key: loadKey, ids: new Set(messages.map((message) => message.id)) };
      arriving.current = new Set();
      return;
    }
    const ids = known.current.ids;
    const added = messages.filter((message) => !ids.has(message.id));
    added.forEach((message) => ids.add(message.id));
    if (added.length > 0 && !followingRef.current && historyBefore === null) setUnseenBelow(true);
  }, [messages, messagesLoaded, loadKey, historyBefore]);

  // ── Opening at the newest message ───────────────────────────────────────
  // A channel (and an older page) opens at its newest message. A reload that
  // emptied the list lands there too — that was the jump to the top when an
  // agent started. A reload that kept the list keeps the reader's place,
  // unless they were following the bottom.
  const lastLoaded = useRef<string | null>(null);
  const emptied = useRef(false);
  /** Set by the reader scrolling up; the sentinel loads an older page only when armed. */
  const armed = useRef(false);
  if (!messagesLoaded && messages.length === 0) emptied.current = true;
  useLayoutEffect(() => {
    if (!messagesLoaded) return;
    if (lastLoaded.current !== loadKey || emptied.current || followingRef.current) {
      scrollToBottom(scroller.current, 'auto');
    }
    lastLoaded.current = loadKey;
    emptied.current = false;
    armed.current = false;
  }, [messagesLoaded, loadKey]);

  // ── Older history ───────────────────────────────────────────────────────
  const loadingPage = !messagesLoaded;
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
    enabled: !readOnly && historyBefore === null && messagesLoaded && messages.length > 0,
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

  const showSkeleton = !messagesLoaded && messages.length === 0;
  const showIntro = messagesLoaded && reachesChannelStart(messages);
  const pill: 'history' | 'live' | null =
    historyBefore !== null ? 'history' : unseenBelow && !following ? 'live' : null;

  return (
    <TimelineCopyProvider>
      <TimelineContextProvider value={context}>
        <div
          className={cn('crew-timeline', className)}
          data-readonly={readOnly ? 'true' : undefined}
        >
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
                aria-busy={loadingPage ? 'true' : undefined}
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
                {showIntro && (
                  <ChannelIntro
                    channel={channel}
                    viewerId={viewerId}
                    dir={dir}
                    readOnly={readOnly}
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
        </div>
      </TimelineContextProvider>
    </TimelineCopyProvider>
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
