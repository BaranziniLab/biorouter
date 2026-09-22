import { deleteConversation } from '../../utils/deleteConversation';
import { toastError, toastSuccess } from '../../toasts';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { useCallback, useEffect, useMemo, useRef, useState, type UIEvent } from 'react';
import type { SessionSummary } from '../../api';
import { ChevronDown, Clock, Folder } from '../icons/app-icons';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { ContextMenu, ContextMenuTrigger } from '../ui/context-menu';
import { ChatRowContextMenuContent } from '../chats/ChatRowContextMenu';

const LOAD_MORE_THRESHOLD_PX = 64;
const RECENTS_EXPANDED_STORAGE_KEY = 'biorouter:sidebar-recents-expanded';

/*
 * THE WELL IS GONE (Astryx §4.1.5).
 *
 * This list used to sit on a one-off surface — `color-mix(in srgb,
 * var(--sidebar-border) 42%, transparent)` with a 12px top radius — arguing it
 * was "one calm step deeper than the rail". Every other list in the app sits
 * directly on its ground, so the well was the sidebar answering a question no
 * other surface asks, with a colour no other surface uses and a radius the
 * radius ladder does not have. Deleting it is the compaction's cheapest win: the
 * rows are already distinguishable by their hover and their selected wash.
 */

/**
 * ⚠ **The kind resolver used to live here** — a `SessionKind` union, a
 * name-regex classifier and a glyph map, all private to the sidebar. Three
 * other surfaces (History's two row components and the tab strip) drew chats
 * without any of it, so a branch looked like a branch in the sidebar and like
 * every other chat everywhere else.
 *
 * It now lives in `components/chats/chatKind.ts`, where all four read it, and
 * it prefers the real lineage fields (`diverged_from`, `parent_session_id`,
 * `session_type`) over the title regex — which this note was already asking for
 * ("replace this with the real field if the API ever exposes one"). It has.
 */

function readStoredRecentsExpanded(): boolean {
  try {
    return window.localStorage.getItem(RECENTS_EXPANDED_STORAGE_KEY) !== 'false';
  } catch {
    // Storage can be unavailable (private mode, sandboxed frame) — default open.
    return true;
  }
}

export interface RecentChatGroup {
  label: string;
  sessions: SessionSummary[];
}

function sessionActivityTime(session: SessionSummary): number {
  const updatedAt = Date.parse(session.updated_at);
  if (!Number.isNaN(updatedAt)) return updatedAt;

  const createdAt = Date.parse(session.created_at);
  return Number.isNaN(createdAt) ? 0 : createdAt;
}

export function sortRecentChats(sessions: SessionSummary[]): SessionSummary[] {
  return [...sessions].sort((left, right) => {
    const activityDifference = sessionActivityTime(right) - sessionActivityTime(left);
    return activityDifference || left.id.localeCompare(right.id);
  });
}

function localCalendarDay(timestamp: number): number {
  const date = new Date(timestamp);
  return Date.UTC(date.getFullYear(), date.getMonth(), date.getDate());
}

export function formatSessionDateLabel(updatedAt: string, now = Date.now()): string {
  const timestamp = Date.parse(updatedAt);
  if (Number.isNaN(timestamp)) return 'Unknown date';

  const dayDifference = Math.round(
    (localCalendarDay(now) - localCalendarDay(timestamp)) / 86_400_000
  );
  if (dayDifference === 0) return 'Today';
  if (dayDifference === 1) return 'Yesterday';

  const date = new Date(timestamp);
  const sameYear = date.getFullYear() === new Date(now).getFullYear();
  return new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    ...(sameYear ? {} : { year: 'numeric' }),
  }).format(date);
}

export function groupRecentChatsByDate(
  sessions: SessionSummary[],
  now = Date.now()
): RecentChatGroup[] {
  const groups: RecentChatGroup[] = [];

  for (const session of sortRecentChats(sessions)) {
    const label = formatSessionDateLabel(session.updated_at, now);
    const existingGroup = groups[groups.length - 1];
    if (existingGroup?.label === label) {
      existingGroup.sessions.push(session);
    } else {
      groups.push({ label, sessions: [session] });
    }
  }

  return groups;
}

export function formatTimeSinceLastWorked(updatedAt: string, now = Date.now()): string {
  const timestamp = Date.parse(updatedAt);
  if (Number.isNaN(timestamp)) return 'Unknown';

  const elapsedMilliseconds = Math.max(0, now - timestamp);
  const elapsedMinutes = Math.floor(elapsedMilliseconds / 60_000);
  if (elapsedMinutes < 1) return 'Just now';
  if (elapsedMinutes < 60) return `${elapsedMinutes}m ago`;

  const elapsedHours = Math.floor(elapsedMinutes / 60);
  if (elapsedHours < 24) return `${elapsedHours}h ago`;

  const elapsedDays = Math.floor(elapsedHours / 24);
  if (elapsedDays < 7) return `${elapsedDays}d ago`;

  const elapsedWeeks = Math.floor(elapsedDays / 7);
  if (elapsedWeeks < 5) return `${elapsedWeeks}w ago`;

  const elapsedMonths = Math.floor(elapsedDays / 30);
  if (elapsedMonths < 12) return `${elapsedMonths}mo ago`;

  return `${Math.floor(elapsedDays / 365)}y ago`;
}

function formatSessionTimestamp(updatedAt: string): string {
  const timestamp = Date.parse(updatedAt);
  if (Number.isNaN(timestamp)) return '';

  return new Intl.DateTimeFormat('en-US', {
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
  }).format(timestamp);
}

function ActiveChatIndicator({ sessionId }: { sessionId: string }) {
  return (
    <span
      data-testid={`running-chat-indicator-${sessionId}`}
      aria-hidden="true"
      className="relative flex h-4 w-4 flex-shrink-0 items-center justify-center text-text-default/80"
    >
      <span className="absolute h-4 w-4 rounded-full border border-current animate-[biorouter-working-ring_1.8s_ease-out_infinite]" />
      <span className="absolute h-2.5 w-2.5 rounded-full bg-current opacity-20 animate-[biorouter-working-glow_1.8s_ease-in-out_infinite]" />
      <span className="h-1.5 w-1.5 rounded-full bg-current opacity-70" />
    </span>
  );
}

interface RecentChatRowProps {
  session: SessionSummary;
  isActive: boolean;
  isRunning: boolean;
  /**
   * `title` is the name this row is ALREADY rendering, handed to the opener so
   * the new tab is born with it. Omitted when the session is unnamed — then the
   * tab's own placeholder is the honest answer, not this row's "Untitled chat".
   */
  onOpen: (sessionId: string, title?: string, userSetName?: boolean) => void;
}

function RecentChatRow({ session, isActive, isRunning, onOpen }: RecentChatRowProps) {
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const deletePending = useRef(false);
  const handleDelete = async () => {
    if (deletePending.current) return;
    deletePending.current = true;
    setDeleting(true);
    try {
      await deleteConversation(session.id);
      setConfirmDelete(false);
      toastSuccess({
        title: 'Chat deleted',
        msg: `"${session.name}" was removed from chat history.`,
      });
    } catch (error) {
      toastError({
        title: 'Failed to delete chat',
        msg: error instanceof Error ? error.message : 'Please try again.',
      });
    } finally {
      deletePending.current = false;
      setDeleting(false);
    }
  };
  const title = session.name.trim() || 'Untitled chat';
  const accessibleLabel = `${isRunning ? 'Open ongoing chat' : 'Open chat'}: ${title}`;
  const messageLabel = `${session.message_count} ${session.message_count === 1 ? 'message' : 'messages'}`;
  const sessionTimestamp = formatSessionTimestamp(session.updated_at);

  /* #114: these rows were click-only, so the conversation id — the handle every
     Workspace tool and Chat Recall's exact load already take — could not be got
     out of the sidebar at all. `openInNewTab` is this row's OWN `onOpen`, name
     and all, so the menu opens a tab exactly the way clicking does. */
  const target = {
    sessionId: session.id,
    workingDir: session.working_dir,
    openInNewTab: () => onOpen(session.id, session.name.trim() || undefined, session.user_set_name),
  };

  return (
    <ContextMenu>
      <Tooltip>
        {/* ⚠ Both triggers are `asChild` onto the SAME button, nested. Each Radix
          trigger merges its handlers onto its single child, so the button ends
          up carrying the tooltip's hover/focus listeners and the menu's
          `contextmenu` listener at once — which is what keeps the row one
          element. Wrapping the button in a div for the second trigger would put
          a box inside the 2px row rhythm and break `space-y-0.5`. */}
        <TooltipTrigger asChild>
          <ContextMenuTrigger asChild>
            <button
              type="button"
              data-testid={`recent-chat-${session.id}`}
              // One click, one real tab. There is no preview/double-click-to-pin
              // gesture: an already-open chat is deduped by the reducer, so clicking
              // around Recents can never replace the chat you are reading.
              // The name goes WITH the click. We are rendering it right here, so
              // there is no reason for the tab to open on a placeholder and wait
              // for BaseChat to fetch a session we already listed.
              onClick={() =>
                onOpen(session.id, session.name.trim() || undefined, session.user_set_name)
              }
              aria-label={accessibleLabel}
              aria-current={isActive ? 'page' : undefined}
              className={`relative flex h-8 w-full min-w-0 max-w-full items-center gap-2 overflow-hidden rounded-lg px-3 text-left text-sm transition-colors duration-150 before:absolute before:inset-y-2 before:left-0 before:w-0.5 before:bg-transparent hover:bg-sidebar-hover ${
                isActive ? 'bg-sidebar-active font-medium before:bg-accent-bar' : ''
              }`}
            >
              {/* ⚠ One glyph, two facts. This row used to draw an identical bubble
              for every kind of chat plus a separate dense dot for privacy — so
              the icon column carried no information and the tier needed its own
              mark. `ChatKindIcon` folds both in: shape says what the chat IS,
              and a private plain chat gets the padlocked bubble.

              The active tint still wins over the tier ink, because "this is the
              chat you are in" is what the sidebar is for. */}
              <ChatKindIcon
                session={session}
                tier={session.privacy_tier}
                testId={`recent-chat-glyph-${session.id}`}
                isActive={isActive}
                className={`h-3.5 w-3.5 ${isActive ? 'text-accent-bar' : 'text-text-subtle'}`}
              />
              <span className="min-w-0 flex-1 truncate leading-5">{title}</span>
              {isRunning && <ActiveChatIndicator sessionId={session.id} />}
            </button>
          </ContextMenuTrigger>
        </TooltipTrigger>
        <TooltipContent
          side="right"
          align="start"
          sideOffset={8}
          className="w-56 max-w-[min(14rem,calc(100vw-16px))] px-2 py-1.5 font-normal"
        >
          <div data-testid={`recent-chat-summary-${session.id}`}>
            <p className="line-clamp-2 font-medium text-text-inverse">{title}</p>
            <div className="mt-2 flex items-start gap-1.5 text-text-inverse/80">
              <Folder className="mt-0.5 size-3.5 shrink-0" />
              <span className="sr-only">Working folder: </span>
              <span className="min-w-0 break-all font-mono text-xs leading-4">
                {session.working_dir}
              </span>
            </div>
            <div className="mt-2 flex items-center gap-1.5 text-text-inverse/80">
              <Clock className="size-3.5 shrink-0" />
              <div>
                <p>Last worked {formatTimeSinceLastWorked(session.updated_at)}</p>
                {sessionTimestamp && <p className="mt-0.5 text-xs">{sessionTimestamp}</p>}
              </div>
            </div>
            <p className="mt-2 text-xs text-text-inverse/80">{messageLabel}</p>
          </div>
        </TooltipContent>
      </Tooltip>
      <ChatRowContextMenuContent target={target} onDelete={() => setConfirmDelete(true)} />
      <ConfirmationModal
        isOpen={confirmDelete}
        title="Delete chat?"
        message={`Are you sure you want to permanently delete the chat "${title}"? This action cannot be undone.`}
        confirmLabel="Delete"
        isSubmitting={deleting}
        cancelLabel="Cancel"
        confirmVariant="destructive"
        onConfirm={() => void handleDelete()}
        onCancel={() => {
          if (!deleting) setConfirmDelete(false);
        }}
      />
    </ContextMenu>
  );
}

interface RecentChatsProps {
  sessions: SessionSummary[];
  activeSessionId?: string | null;
  runningSessionIds: ReadonlySet<string>;
  hasMore: boolean;
  isLoadingMore: boolean;
  onLoadMore: () => void;
  onOpen: (sessionId: string, title?: string, userSetName?: boolean) => void;
  onViewAll: () => void;
}

export default function RecentChats({
  sessions,
  activeSessionId,
  runningSessionIds,
  hasMore,
  isLoadingMore,
  onLoadMore,
  onOpen,
  onViewAll,
}: RecentChatsProps) {
  const groups = useMemo(() => groupRecentChatsByDate(sessions), [sessions]);
  const [isExpanded, setIsExpanded] = useState(readStoredRecentsExpanded);
  const scrollContainerRef = useRef<HTMLDivElement>(null);

  const toggleExpanded = useCallback(() => {
    setIsExpanded((wasExpanded) => {
      const nextExpanded = !wasExpanded;
      try {
        window.localStorage.setItem(RECENTS_EXPANDED_STORAGE_KEY, String(nextExpanded));
      } catch {
        // Persisting is best-effort; the session still collapses.
      }
      return nextExpanded;
    });
  }, []);

  const handleScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      if (!hasMore || isLoadingMore) return;

      const container = event.currentTarget;
      const remainingScroll = container.scrollHeight - container.scrollTop - container.clientHeight;
      if (remainingScroll <= LOAD_MORE_THRESHOLD_PX) onLoadMore();
    },
    [hasMore, isLoadingMore, onLoadMore]
  );

  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || !hasMore || isLoadingMore || container.clientHeight === 0) return;
    if (container.scrollHeight <= container.clientHeight) onLoadMore();
  }, [hasMore, isLoadingMore, onLoadMore, sessions.length]);

  return (
    <div
      className={`flex min-h-0 w-full min-w-0 flex-col ${isExpanded ? 'flex-1' : 'shrink-0'}`}
      data-testid="recent-chats"
    >
      <div className="flex h-8 shrink-0 items-center justify-between gap-2 px-5">
        <button
          type="button"
          data-testid="recents-disclosure"
          aria-expanded={isExpanded}
          aria-controls="recent-chat-scroll"
          onClick={toggleExpanded}
          className="flex min-w-0 items-center gap-1.5 rounded-sm text-[11px] font-semibold uppercase tracking-[0.08em] text-text-subtle transition-colors duration-150 hover:text-text-default"
        >
          <ChevronDown
            aria-hidden="true"
            className={`size-[11px] shrink-0 transition-transform duration-150 ${
              isExpanded ? '' : '-rotate-90'
            }`}
          />
          <span>Recents</span>
        </button>
        <button
          type="button"
          data-testid="view-all-chat-history"
          onClick={onViewAll}
          className="shrink-0 rounded-sm text-[11px] font-medium text-text-subtle transition-colors duration-150 hover:text-text-default"
        >
          See all
        </button>
      </div>

      <div
        ref={scrollContainerRef}
        id="recent-chat-scroll"
        data-testid="recent-chat-scroll"
        hidden={!isExpanded}
        className="mx-1 min-h-0 min-w-0 shrink overflow-y-auto overflow-x-hidden p-1"
        onScroll={handleScroll}
      >
        {groups.length === 0 ? (
          <p className="px-3 py-3 text-xs text-text-subtle">
            {isLoadingMore ? 'Loading chats…' : 'No recent chats yet'}
          </p>
        ) : (
          groups.map((group) => (
            <section key={group.label} aria-label={group.label} className="mb-2 min-w-0 last:mb-0">
              <p className="px-3 pb-0.5 text-xs font-normal leading-4 text-text-subtle">
                {group.label}
              </p>
              {/* 2px between rows, matching the nav group above. The rail has one
                  rhythm or it has none: with the destinations separated and the
                  history flush, the two halves of the same column read as two
                  different lists. */}
              <div className="min-w-0 space-y-0.5">
                {group.sessions.map((session) => (
                  <RecentChatRow
                    key={session.id}
                    session={session}
                    isActive={activeSessionId === session.id}
                    isRunning={runningSessionIds.has(session.id)}
                    onOpen={onOpen}
                  />
                ))}
              </div>
            </section>
          ))
        )}
        {isLoadingMore && groups.length > 0 && (
          <p
            role="status"
            data-testid="recent-chat-loading"
            className="px-3 py-2 text-xs text-text-subtle"
          >
            Loading more chats…
          </p>
        )}
      </div>
    </div>
  );
}
