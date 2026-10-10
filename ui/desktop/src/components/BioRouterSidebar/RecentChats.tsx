import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type UIEvent,
} from 'react';

import type { SessionSummary } from '../../api';
import { useDiverge } from '../../hooks/useDiverge';
import { toastError, toastSuccess } from '../../toasts';
import { deleteConversation } from '../../utils/deleteConversation';
import { exportConversation } from '../../utils/exportConversation';
import { renameSessionOptimistically } from '../../utils/sessionNameSync';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { chatKindOf } from '../chats/chatKind';
import {
  ChatRowContextMenuContent,
  ChatRowDropdownMenuContent,
  type ChatRowMenuHandlers,
} from '../chats/ChatRowContextMenu';
import { ChatRowRenameInput } from '../chats/ChatRowRenameInput';
import { chatRowCopy } from '../chats/copy';
import { ChevronRight, Folder, History, MoreHorizontal, Plus } from '../icons/app-icons';
import { Button } from '../ui/button';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { ContextMenu, ContextMenuTrigger } from '../ui/context-menu';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { isContextMenuKey, openContextMenuFromKeyboard } from '../ui/keyboardContextMenu';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { useRovingRows } from '../ui/useRovingRows';
import { sidebarCopy } from './copy';
import SidebarViewMenu from './SidebarViewMenu';
import {
  applyHeldArrangement,
  arrangeSidebarChats,
  holdArrangement,
  tildePath,
  type HeldArrangement,
  type SidebarChatGroup,
  type SidebarChatView,
} from './sidebarChatView';
import { useCollapsedFolders } from './useSidebarChatView';
import './sidebar.css';

const LOAD_MORE_THRESHOLD_PX = 64;
const RECENTS_EXPANDED_STORAGE_KEY = 'biorouter:sidebar-recents-expanded';
/** A folder group shows this many chats, then a "Show more" row. */
export const FOLDER_PREVIEW_COUNT = 5;
/** The hover card waits for intent (Astryx §4.1 item 6). */
export const CHAT_HOVER_CARD_DELAY_MS = 700;

/*
 * The kind resolver lives in `components/chats/chatKind.ts`, where the sidebar,
 * History and the tab strip all read it; it prefers the real lineage fields
 * (`diverged_from`, `parent_session_id`, `session_type`) and the Crew task
 * folder over the title.
 */

function readStoredRecentsExpanded(): boolean {
  try {
    return window.localStorage.getItem(RECENTS_EXPANDED_STORAGE_KEY) !== 'false';
  } catch {
    // Storage can be unavailable (private mode, sandboxed frame) — default open.
    return true;
  }
}

function isMacPlatform(): boolean {
  return typeof window !== 'undefined' && window.electron?.platform === 'darwin';
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

/** The hover card's second line: `~/path · 3h ago · 12 messages`. */
export function chatHoverDetail(
  session: SessionSummary,
  homeDir?: string | null,
  now = Date.now()
): string {
  return [
    session.working_dir ? tildePath(session.working_dir, homeDir) : null,
    formatTimeSinceLastWorked(session.updated_at, now),
    sidebarCopy.chats.messages(session.message_count),
  ]
    .filter(Boolean)
    .join(' · ');
}

const chatKey = (sessionId: string) => `chat:${sessionId}`;
const folderKey = (group: SidebarChatGroup) => `folder:${group.key}`;
const moreKey = (group: SidebarChatGroup) => `more:${group.key}`;

/** What a row's menu offers, decided from the chat itself (History's menu is the reference). */
function canDiverge(session: SessionSummary): boolean {
  const kind = chatKindOf(session);
  return (kind === 'chat' || kind === 'branch' || kind === 'crew') && session.message_count > 1;
}

// ── One chat row ──────────────────────────────────────────────────────────

interface RecentChatRowProps {
  session: SessionSummary;
  isActive: boolean;
  isRunning: boolean;
  homeDir?: string | null;
  tabIndex: 0 | -1;
  onRowFocus: (key: string) => void;
  /**
   * `title` is the name this row is ALREADY rendering, handed to the opener so
   * the new tab is born with it. Omitted when the session is unnamed — then the
   * tab's own placeholder is the honest answer, not this row's "Untitled chat".
   */
  onOpen: (sessionId: string, title?: string, userSetName?: boolean) => void;
  onStartRename: (sessionId: string) => void;
  onRequestDelete: (session: SessionSummary) => void;
  onDiverge: (sessionId: string) => void;
  /** Set while this row is being renamed: the row turns into its editor. */
  editing: { draft: string } | null;
  onDraftChange: (draft: string) => void;
  onCommitRename: (value: string) => void;
  onCancelRename: () => void;
}

function RecentChatRow({
  session,
  isActive,
  isRunning,
  homeDir,
  tabIndex,
  onRowFocus,
  onOpen,
  onStartRename,
  onRequestDelete,
  onDiverge,
  editing,
  onDraftChange,
  onCommitRename,
  onCancelRename,
}: RecentChatRowProps) {
  const title = session.name.trim() || sidebarCopy.chats.untitled;
  const accessibleLabel = isRunning
    ? sidebarCopy.chats.openRunningChat(title)
    : sidebarCopy.chats.openChat(title);
  const key = chatKey(session.id);
  const labelRef = useRef<HTMLSpanElement>(null);
  const [titleIsCut, setTitleIsCut] = useState(false);
  const [menuOpen, setMenuOpen] = useState(false);
  // Rename was chosen in a menu: start editing once the menu has let go of
  // focus, or it would hand focus back to this row over the new input.
  const renameAfterMenu = useRef(false);

  /* #114: `openInNewTab` is this row's OWN `onOpen`, name and all, so the menu
     opens a tab exactly the way clicking does. */
  const target = {
    sessionId: session.id,
    workingDir: session.working_dir,
    openInNewTab: () => onOpen(session.id, session.name.trim() || undefined, session.user_set_name),
  };
  const handlers: ChatRowMenuHandlers = {
    onRename: () => {
      renameAfterMenu.current = true;
    },
    onDiverge: canDiverge(session) ? () => onDiverge(session.id) : undefined,
    onExport: () => void exportConversation(session.id, title, { scope: 'app' }),
    onDelete: () => onRequestDelete(session),
  };
  const onMenuCloseAutoFocus = (event: Event) => {
    if (!renameAfterMenu.current) return;
    renameAfterMenu.current = false;
    event.preventDefault();
    onStartRename(session.id);
  };

  if (editing) {
    return (
      <li className="br-chat-row-item" data-row-item="">
        <div className="br-nav-row br-chat-row" data-editing="true" data-row={key}>
          <ChatKindIcon
            session={session}
            tier={session.privacy_tier}
            testId={`recent-chat-glyph-${session.id}`}
            isActive={isActive}
          />
          <ChatRowRenameInput
            title={title}
            value={editing.draft}
            onChange={onDraftChange}
            onCommit={onCommitRename}
            onCancel={onCancelRename}
          />
          <span className="br-chat-row-trailing" aria-hidden="true" />
        </div>
      </li>
    );
  }

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>) => {
    if (event.key === 'F2' && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      onStartRename(session.id);
      return;
    }
    // Shift+F10 / the Menu key: Chromium on macOS sends no `contextmenu` for
    // them, so the row dispatches it (F-01). Preventing the keydown stops
    // Chromium's own dispatch elsewhere, so the menu opens once.
    if (isContextMenuKey(event)) {
      event.preventDefault();
      openContextMenuFromKeyboard(event.currentTarget, labelRef.current ?? event.currentTarget);
    }
  };

  return (
    <li className="br-chat-row-item" data-row-item="" data-menu-open={menuOpen || undefined}>
      <ContextMenu onOpenChange={setMenuOpen}>
        <Tooltip
          delayDuration={CHAT_HOVER_CARD_DELAY_MS}
          onOpenChange={(open) => {
            if (open && labelRef.current) {
              setTitleIsCut(labelRef.current.scrollWidth > labelRef.current.clientWidth);
            }
          }}
        >
          {/* ⚠ Both triggers are `asChild` onto the SAME button, nested: the
              button carries the tooltip's hover/focus listeners and the menu's
              `contextmenu` listener at once, which keeps the row one element in
              the 2px rhythm. */}
          <TooltipTrigger asChild>
            <ContextMenuTrigger asChild>
              <button
                type="button"
                className="br-nav-row br-chat-row no-drag"
                data-testid={`recent-chat-${session.id}`}
                data-row={key}
                tabIndex={tabIndex}
                onFocus={() => onRowFocus(key)}
                onKeyDown={onKeyDown}
                // One click, one real tab; an open chat is deduped by the
                // reducer. The name goes WITH the click so the tab is born
                // titled. Double-click is two opens (pinned by a test), never a
                // rename.
                onClick={() =>
                  onOpen(session.id, session.name.trim() || undefined, session.user_set_name)
                }
                aria-label={accessibleLabel}
                aria-current={isActive ? 'page' : undefined}
              >
                {/* One glyph, two facts: the shape says what the chat IS, and a
                    private chat carries the lock badge. It never hides. */}
                <ChatKindIcon
                  session={session}
                  tier={session.privacy_tier}
                  testId={`recent-chat-glyph-${session.id}`}
                  isActive={isActive}
                />
                <span ref={labelRef} className="br-nav-row-label">
                  {title}
                </span>
                <span className="br-chat-row-trailing" aria-hidden="true">
                  {isRunning ? (
                    <span
                      className="br-chat-row-ring"
                      data-testid={`running-chat-indicator-${session.id}`}
                    />
                  ) : null}
                </span>
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
              {titleIsCut ? <p className="line-clamp-2">{title}</p> : null}
              <p>{chatHoverDetail(session, homeDir)}</p>
            </div>
          </TooltipContent>
        </Tooltip>
        <ChatRowContextMenuContent
          target={target}
          {...handlers}
          onCloseAutoFocus={onMenuCloseAutoFocus}
        />
      </ContextMenu>
      <DropdownMenu onOpenChange={setMenuOpen}>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="xs"
                className="br-chat-row-more no-drag text-text-muted hover:text-text-default"
                aria-label={sidebarCopy.chats.moreActions(title)}
                data-testid={`recent-chat-more-${session.id}`}
                tabIndex={tabIndex}
                onFocus={() => onRowFocus(key)}
              >
                <MoreHorizontal className="size-4" aria-hidden />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent>{sidebarCopy.chats.moreActions(title)}</TooltipContent>
        </Tooltip>
        <ChatRowDropdownMenuContent
          target={target}
          {...handlers}
          onCloseAutoFocus={onMenuCloseAutoFocus}
        />
      </DropdownMenu>
    </li>
  );
}

// ── A folder group's header ───────────────────────────────────────────────

interface FolderHeaderProps {
  group: SidebarChatGroup;
  collapsed: boolean;
  tabIndex: 0 | -1;
  onRowFocus: (key: string) => void;
  onToggle: () => void;
}

function FolderHeader({ group, collapsed, tabIndex, onRowFocus, onToggle }: FolderHeaderProps) {
  const key = folderKey(group);
  const [menuOpen, setMenuOpen] = useState(false);
  const canStartChat =
    !group.isCrew && Boolean(group.workingDir) && Boolean(window.electron?.createChatWindow);

  const copyPath = async () => {
    const path = group.workingDir ?? '';
    try {
      if (!navigator.clipboard?.writeText) throw new Error('clipboard unavailable');
      await navigator.clipboard.writeText(path);
      toastSuccess({
        title: sidebarCopy.folder.pathCopied,
        msg: path,
        toastOptions: { autoClose: 2000 },
      });
    } catch {
      toastError({ title: sidebarCopy.folder.copyFailed, msg: path });
    }
  };

  return (
    <div
      className="br-sidebar-folder"
      data-row-item=""
      data-menu-open={menuOpen || undefined}
      data-testid={`sidebar-folder-${group.key}`}
    >
      <Tooltip delayDuration={CHAT_HOVER_CARD_DELAY_MS}>
        <TooltipTrigger asChild>
          <button
            type="button"
            className="br-sidebar-folder-toggle no-drag"
            data-row={key}
            tabIndex={tabIndex}
            onFocus={() => onRowFocus(key)}
            aria-expanded={!collapsed}
            onClick={onToggle}
          >
            <ChevronRight className="br-nav-chevron" aria-hidden />
            <Folder className="br-nav-row-icon" aria-hidden />
            <span className="br-nav-row-label">{group.label}</span>
          </button>
        </TooltipTrigger>
        {group.path ? <TooltipContent side="right">{group.path}</TooltipContent> : null}
      </Tooltip>
      <div className="br-sidebar-actions">
        {canStartChat ? (
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="xs"
                className="no-drag text-text-muted hover:text-text-default"
                aria-label={sidebarCopy.folder.newChat(group.label)}
                tabIndex={tabIndex}
                onFocus={() => onRowFocus(key)}
                onClick={() => window.electron?.createChatWindow?.(undefined, group.workingDir)}
              >
                <Plus className="size-4" aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{sidebarCopy.folder.newChat(group.label)}</TooltipContent>
          </Tooltip>
        ) : null}
        {group.workingDir ? (
          <DropdownMenu onOpenChange={setMenuOpen}>
            <Tooltip>
              <TooltipTrigger asChild>
                <DropdownMenuTrigger asChild>
                  <Button
                    type="button"
                    variant="ghost"
                    shape="round"
                    size="xs"
                    className="no-drag text-text-muted hover:text-text-default"
                    aria-label={sidebarCopy.folder.moreActions(group.label)}
                    tabIndex={tabIndex}
                    onFocus={() => onRowFocus(key)}
                  >
                    <MoreHorizontal className="size-4" aria-hidden />
                  </Button>
                </DropdownMenuTrigger>
              </TooltipTrigger>
              <TooltipContent>{sidebarCopy.folder.moreActions(group.label)}</TooltipContent>
            </Tooltip>
            <DropdownMenuContent align="end" className="w-56">
              <DropdownMenuItem onSelect={() => void copyPath()}>
                {sidebarCopy.folder.copyPath}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        ) : null}
      </div>
    </div>
  );
}

// ── The list ──────────────────────────────────────────────────────────────

interface RecentChatsProps {
  sessions: SessionSummary[];
  activeSessionId?: string | null;
  runningSessionIds: ReadonlySet<string>;
  hasMore: boolean;
  isLoadingMore: boolean;
  /** True while a non-default view is still reading every page. */
  isLoadingAll?: boolean;
  onLoadMore: () => void;
  onOpen: (sessionId: string, title?: string, userSetName?: boolean) => void;
  onViewAll: () => void;
  view: SidebarChatView;
  onViewChange: (view: SidebarChatView) => void;
  /** The person's home folder, so folders read `~/…`. */
  homeDir?: string | null;
}

export default function RecentChats({
  sessions,
  activeSessionId,
  runningSessionIds,
  hasMore,
  isLoadingMore,
  isLoadingAll = false,
  onLoadMore,
  onOpen,
  onViewAll,
  view,
  onViewChange,
  homeDir,
}: RecentChatsProps) {
  const [isExpanded, setIsExpanded] = useState(readStoredRecentsExpanded);
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const { diverge } = useDiverge();

  const arranged = useMemo(
    () => arrangeSidebarChats(sessions, view, Date.now(), { homeDir }),
    [sessions, view, homeDir]
  );

  // Rename in place. The draft lives here, keyed by session id, so a head
  // refresh that moves or re-renders the row keeps what the person typed.
  const [editing, setEditing] = useState<{ sessionId: string; draft: string } | null>(null);
  const focusAfterEdit = useRef<string | null>(null);

  // Principle 9: while the pointer is over the list (or a row is being renamed)
  // the rows hold still; the live order applies when it leaves.
  const [pointerInside, setPointerInside] = useState(false);
  const [held, setHeld] = useState<HeldArrangement | null>(null);
  const holding = (pointerInside || editing !== null) && !isLoadingAll;
  useEffect(() => {
    if (!holding) setHeld(null);
  }, [holding]);
  const captureHold = () => setHeld((current) => current ?? holdArrangement(arranged, view));
  const groups = useMemo(
    () =>
      holding && held && held.view.groupBy === view.groupBy && held.view.sortBy === view.sortBy
        ? applyHeldArrangement(held, arranged)
        : arranged,
    [holding, held, arranged, view]
  );

  // A view change re-renders the list at rest and fades it in from 0.6.
  const [viewGeneration, setViewGeneration] = useState(0);
  const lastView = useRef(view);
  useEffect(() => {
    if (lastView.current.groupBy === view.groupBy && lastView.current.sortBy === view.sortBy) {
      return;
    }
    lastView.current = view;
    setViewGeneration((generation) => generation + 1);
  }, [view]);

  const [collapsedFolders, toggleFolder] = useCollapsedFolders();
  const [expandedFolders, setExpandedFolders] = useState<ReadonlySet<string>>(new Set());

  // What each group actually shows: a collapsed folder shows no rows (unless it
  // holds the current chat, which opens it without touching the stored choice),
  // and an open folder shows five rows before "Show more".
  const shown = useMemo(
    () =>
      groups.map((group) => {
        if (group.kind !== 'folder') {
          return {
            group,
            collapsed: false,
            rows: group.sessions,
            more: null as null | 'more' | 'less',
          };
        }
        const activeIndex = group.sessions.findIndex((session) => session.id === activeSessionId);
        const collapsed = collapsedFolders.has(group.key) && activeIndex < 0;
        if (collapsed) return { group, collapsed, rows: [], more: null };
        const long = group.sessions.length > FOLDER_PREVIEW_COUNT;
        const expanded = expandedFolders.has(group.key) || activeIndex >= FOLDER_PREVIEW_COUNT;
        return {
          group,
          collapsed,
          rows: long && !expanded ? group.sessions.slice(0, FOLDER_PREVIEW_COUNT) : group.sessions,
          more: !long ? null : expanded ? ('less' as const) : ('more' as const),
        };
      }),
    [groups, collapsedFolders, expandedFolders, activeSessionId]
  );

  // One tab stop for the list (spec 3.4, F-18): ↑/↓/Home/End move between rows.
  const rowKeys = useMemo(
    () =>
      shown.flatMap(({ group, rows, more }) => [
        ...(group.kind === 'folder' ? [folderKey(group)] : []),
        ...rows.map((session) => chatKey(session.id)),
        ...(more ? [moreKey(group)] : []),
      ]),
    [shown]
  );
  const roving = useRovingRows(rowKeys, activeSessionId ? chatKey(activeSessionId) : null);

  const toggleExpanded = useCallback(() => {
    setIsExpanded((wasExpanded) => {
      const nextExpanded = !wasExpanded;
      try {
        window.localStorage.setItem(RECENTS_EXPANDED_STORAGE_KEY, String(nextExpanded));
      } catch {
        // Persisting is best-effort; the list still collapses.
      }
      return nextExpanded;
    });
  }, []);

  const startRename = useCallback(
    (sessionId: string) => {
      const session = sessions.find((candidate) => candidate.id === sessionId);
      if (!session) return;
      setEditing({ sessionId, draft: session.name.trim() || sidebarCopy.chats.untitled });
    },
    [sessions]
  );

  const finishRename = (sessionId: string) => {
    focusAfterEdit.current = sessionId;
    setEditing(null);
  };

  const commitRename = (sessionId: string, value: string) => {
    const session = sessions.find((candidate) => candidate.id === sessionId);
    finishRename(sessionId);
    if (!session) return;
    void renameSessionOptimistically(sessionId, value, {
      name: session.name,
      userSetName: session.user_set_name ?? false,
    });
  };

  // Focus returns to the row button after a rename ends.
  useLayoutEffect(() => {
    const sessionId = focusAfterEdit.current;
    if (!sessionId || editing) return;
    focusAfterEdit.current = null;
    scrollContainerRef.current
      ?.querySelector<HTMLElement>(`[data-row="${CSS.escape(chatKey(sessionId))}"]`)
      ?.focus();
  }, [editing]);

  // ⌘⌥R (Ctrl+Alt+R) renames the chat you are in, when its row is listed.
  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      const mod = isMacPlatform() ? event.metaKey : event.ctrlKey;
      if (!mod || !event.altKey || event.shiftKey || event.code !== 'KeyR') return;
      if (!activeSessionId || !sessions.some((session) => session.id === activeSessionId)) return;
      event.preventDefault();
      setIsExpanded(true);
      startRename(activeSessionId);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [activeSessionId, sessions, startRename]);

  // Delete, behind one confirmation for the list.
  const [pendingDelete, setPendingDelete] = useState<SessionSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const deletePending = useRef(false);
  const handleDelete = async () => {
    const session = pendingDelete;
    if (!session || deletePending.current) return;
    deletePending.current = true;
    setDeleting(true);
    const title = session.name.trim() || sidebarCopy.chats.untitled;
    try {
      await deleteConversation(session.id);
      setPendingDelete(null);
      toastSuccess({ title: 'Chat deleted', msg: `"${title}" was removed from chat history.` });
    } catch (error) {
      toastError({
        title: chatRowCopy.deleteDialog.failed,
        msg:
          typeof error === 'string' && error.trim()
            ? error
            : error instanceof Error
              ? error.message
              : 'Please try again.',
      });
    } finally {
      deletePending.current = false;
      setDeleting(false);
    }
  };

  const handleScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      if (!hasMore || isLoadingMore || isLoadingAll) return;
      const container = event.currentTarget;
      const remainingScroll = container.scrollHeight - container.scrollTop - container.clientHeight;
      if (remainingScroll <= LOAD_MORE_THRESHOLD_PX) onLoadMore();
    },
    [hasMore, isLoadingMore, isLoadingAll, onLoadMore]
  );

  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || !hasMore || isLoadingMore || isLoadingAll || container.clientHeight === 0) {
      return;
    }
    if (container.scrollHeight <= container.clientHeight) onLoadMore();
  }, [hasMore, isLoadingMore, isLoadingAll, onLoadMore, sessions.length]);

  // The running rings pause off screen and while the window is hidden.
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    const onVisibility = () =>
      container.setAttribute('data-document-hidden', String(document.hidden));
    onVisibility();
    document.addEventListener('visibilitychange', onVisibility);
    if (typeof IntersectionObserver === 'undefined') {
      return () => document.removeEventListener('visibilitychange', onVisibility);
    }
    const observer = new IntersectionObserver(
      (entries) => {
        for (const entry of entries) {
          entry.target.setAttribute('data-offscreen', String(!entry.isIntersecting));
        }
      },
      { root: container }
    );
    container.querySelectorAll('.br-chat-row-ring').forEach((ring) => observer.observe(ring));
    return () => {
      document.removeEventListener('visibilitychange', onVisibility);
      observer.disconnect();
    };
  }, [groups, runningSessionIds]);

  const setScrollRef = (node: HTMLDivElement | null) => {
    scrollContainerRef.current = node;
    roving.containerRef.current = node;
  };

  const rowProps = (session: SessionSummary) => ({
    session,
    isActive: activeSessionId === session.id,
    isRunning: runningSessionIds.has(session.id),
    homeDir,
    tabIndex: roving.tabIndexFor(chatKey(session.id)),
    onRowFocus: roving.onRowFocus,
    onOpen,
    onStartRename: startRename,
    onRequestDelete: setPendingDelete,
    onDiverge: (sessionId: string) => void diverge(sessionId),
    editing: editing?.sessionId === session.id ? { draft: editing.draft } : null,
    onDraftChange: (draft: string) =>
      setEditing((current) => (current ? { ...current, draft } : current)),
    onCommitRename: (value: string) => commitRename(session.id, value),
    onCancelRename: () => finishRename(session.id),
  });

  const pendingTitle = pendingDelete ? pendingDelete.name.trim() || sidebarCopy.chats.untitled : '';
  const isEmpty = groups.length === 0;

  return (
    <div className="br-sidebar-chats" data-expanded={isExpanded} data-testid="recent-chats">
      <div className="br-sidebar-chats-header">
        <button
          type="button"
          className="br-sidebar-chats-toggle no-drag"
          data-testid="recents-disclosure"
          aria-expanded={isExpanded}
          aria-controls="recent-chat-scroll"
          onClick={toggleExpanded}
        >
          <span className="br-nav-row-label">{sidebarCopy.chats.header}</span>
          <ChevronRight className="br-nav-chevron" aria-hidden />
        </button>
        <div className="br-sidebar-chats-actions">
          <SidebarViewMenu view={view} onViewChange={onViewChange} />
          <Tooltip>
            <TooltipTrigger asChild>
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="xs"
                className="no-drag text-text-muted hover:text-text-default"
                data-testid="view-all-chat-history"
                aria-label={sidebarCopy.chats.allChats}
                onClick={onViewAll}
              >
                <History className="size-4" aria-hidden />
              </Button>
            </TooltipTrigger>
            <TooltipContent>{sidebarCopy.chats.allChats}</TooltipContent>
          </Tooltip>
        </div>
      </div>

      <div
        ref={setScrollRef}
        id="recent-chat-scroll"
        data-testid="recent-chat-scroll"
        hidden={!isExpanded}
        className="br-sidebar-chats-scroll"
        onScroll={handleScroll}
        onPointerEnter={() => {
          setPointerInside(true);
          captureHold();
        }}
        onPointerLeave={() => setPointerInside(false)}
        onKeyDown={roving.onKeyDown}
        onFocus={roving.onFocus}
        onBlur={roving.onBlur}
      >
        {isEmpty ? (
          <p
            className="br-sidebar-foot"
            role={isLoadingMore || isLoadingAll ? 'status' : undefined}
          >
            {isLoadingMore || isLoadingAll ? sidebarCopy.chats.loading : sidebarCopy.chats.empty}
          </p>
        ) : (
          <div
            key={viewGeneration}
            className="br-sidebar-chat-groups"
            data-view-changed={viewGeneration > 0 ? 'true' : undefined}
          >
            {shown.map(({ group, collapsed, rows, more }) => (
              <section
                key={group.key}
                className="br-sidebar-group"
                aria-label={group.label || sidebarCopy.chats.header}
              >
                {group.kind === 'date' ? <p className="br-sidebar-bucket">{group.label}</p> : null}
                {group.kind === 'folder' ? (
                  <FolderHeader
                    group={group}
                    collapsed={collapsed}
                    tabIndex={roving.tabIndexFor(folderKey(group))}
                    onRowFocus={roving.onRowFocus}
                    onToggle={() => toggleFolder(group.key)}
                  />
                ) : null}
                {rows.length > 0 ? (
                  <ul className="br-nav-list">
                    {rows.map((session) => (
                      <RecentChatRow key={session.id} {...rowProps(session)} />
                    ))}
                    {more ? (
                      <li data-row-item="">
                        <button
                          type="button"
                          className="br-nav-row br-sidebar-quiet-row no-drag"
                          data-row={moreKey(group)}
                          tabIndex={roving.tabIndexFor(moreKey(group))}
                          onFocus={() => roving.onRowFocus(moreKey(group))}
                          onClick={() =>
                            setExpandedFolders((current) => {
                              const next = new Set(current);
                              if (next.has(group.key)) next.delete(group.key);
                              else next.add(group.key);
                              return next;
                            })
                          }
                        >
                          <span className="br-nav-row-label">
                            {more === 'more'
                              ? sidebarCopy.chats.showMore
                              : sidebarCopy.chats.showLess}
                          </span>
                        </button>
                      </li>
                    ) : null}
                  </ul>
                ) : null}
              </section>
            ))}
          </div>
        )}
        {!isEmpty && (isLoadingAll || isLoadingMore) ? (
          <p role="status" data-testid="recent-chat-loading" className="br-sidebar-foot">
            {isLoadingAll ? sidebarCopy.chats.loading : sidebarCopy.chats.loadingMore}
          </p>
        ) : null}
      </div>

      <ConfirmationModal
        isOpen={pendingDelete !== null}
        title={chatRowCopy.deleteDialog.title}
        message={chatRowCopy.deleteDialog.message(pendingTitle)}
        confirmLabel={chatRowCopy.deleteDialog.confirm}
        isSubmitting={deleting}
        cancelLabel={chatRowCopy.deleteDialog.cancel}
        confirmVariant="destructive"
        onConfirm={() => void handleDelete()}
        onCancel={() => {
          if (!deleting) setPendingDelete(null);
        }}
      />
    </div>
  );
}
