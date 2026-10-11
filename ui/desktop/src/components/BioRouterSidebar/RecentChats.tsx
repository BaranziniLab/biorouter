import {
  memo,
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
import { normalizeSessionName, renameSessionOptimistically } from '../../utils/sessionNameSync';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { chatKindOf } from '../chats/chatKind';
import {
  ChatRowContextMenuContent,
  ChatRowDropdownMenuContent,
  type ChatRowMenuHandlers,
} from '../chats/ChatRowContextMenu';
import { ChatRowRenameInput } from '../chats/ChatRowRenameInput';
import { chatRowCopy } from '../chats/copy';
import { keepFocusOffAfterPointerClose } from '../chats/menuFocusReturn';
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
import { useSidebar } from '../ui/sidebar';
import { useRovingRows } from '../ui/useRovingRows';
import { sidebarCopy } from './copy';
import SidebarViewMenu from './SidebarViewMenu';
import {
  applyHeldArrangement,
  arrangeSidebarChatsCached,
  holdArrangement,
  linesToDrawRow,
  SIDEBAR_RENDER_WINDOW,
  SIDEBAR_RENDER_WINDOW_STEP,
  sidebarRenderWindow,
  windowSidebarRows,
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
/**
 * How far a row's hover card (and a folder's path tooltip) sits from the end of
 * the row it describes. A row ends 8px inside the sidebar (the list's inset), so
 * the app's 8px offset put the card's edge exactly ON the pane divider, where it
 * read as stuck to the column; 16 lets it float 8px clear of the divider, at
 * every sidebar width.
 */
export const CHAT_HOVER_CARD_OFFSET = 16;

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

/** Lines a group's header draws: a date or folder header is one, the ungrouped list none. */
const headerLines = ({ group }: { group: SidebarChatGroup }) => (group.kind === 'all' ? 0 : 1);

/**
 * What a row's menu offers, decided from the chat itself, by History's rule
 * (spec 3.4: History's menu is the reference, and the two share one menu): a
 * subagent run is machinery, not a chat to branch, and an empty chat has
 * nothing to branch from. Everything else offers Diverge, and the daemon
 * answers for the rest.
 */
export function canDivergeChat(session: SessionSummary): boolean {
  return chatKindOf(session) !== 'subagent' && session.message_count > 0;
}

/**
 * A function that keeps one identity for the component's whole life and always
 * calls the latest `handler`, so a memoized row is not re-rendered just because
 * its parent made a new closure. Only for event handlers: it reads the handler
 * the last commit installed.
 */
function useStableHandler<Args extends unknown[], Result>(
  handler: (...args: Args) => Result
): (...args: Args) => Result {
  const latest = useRef(handler);
  useLayoutEffect(() => {
    latest.current = handler;
  });
  return useCallback((...args: Args) => latest.current(...args), []);
}

/**
 * What a row or folder header shows only on request (a hover card, a button's
 * label, a menu) is built the first time it opens, then kept, so it still
 * closes with its own exit animation (R-28, vision V1-R2-2). Radix mounts
 * nothing for a closed tooltip or menu, but its elements were still created on
 * every render of every line drawn: a chat row's two menus of ten items each
 * are more elements than the row, and a view switch draws 60 lines.
 */
function useBuiltOnOpen<Part extends string>(): [
  ReadonlySet<Part>,
  (part: Part) => (open: boolean) => void,
] {
  const [built, setBuilt] = useState<ReadonlySet<Part>>(() => new Set());
  const buildOnOpen = useCallback(
    (part: Part) => (open: boolean) => {
      if (!open) return;
      setBuilt((current) => (current.has(part) ? current : new Set(current).add(part)));
    },
    []
  );
  return [built, buildOnOpen];
}

// ── One chat row ──────────────────────────────────────────────────────────

interface RecentChatRowProps {
  session: SessionSummary;
  isActive: boolean;
  isRunning: boolean;
  homeDir?: string | null;
  /**
   * False while the row cannot be seen: the Chats list is collapsed or the
   * sidebar is closed. The row is still mounted then (a `hidden` list, a panel
   * translated off screen), so its hover card would stay open with no anchor
   * and float to the window's top-left over the titlebar (vision V1-M9).
   */
  listShown: boolean;
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
  editingDraft: string | null;
  onDraftChange: (draft: string) => void;
  onCommitRename: (sessionId: string, value: string) => void;
  onCancelRename: (sessionId: string) => void;
}

/*
 * Memoized (R-28, vision V1-R2-2): every handler the list passes is stable and
 * every other prop is the row's own data, so a list re-render (the pointer
 * entering it, a keystroke in another row's rename, a chat starting to run)
 * re-renders only the rows whose own props changed, not every drawn row.
 */
const RecentChatRow = memo(function RecentChatRow({
  session,
  isActive,
  isRunning,
  homeDir,
  listShown,
  tabIndex,
  onRowFocus,
  onOpen,
  onStartRename,
  onRequestDelete,
  onDiverge,
  editingDraft,
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
  // The hover card is held here so it can be shut when the row stops being
  // seen: no pointer event tells Radix the anchor went away.
  const [cardOpen, setCardOpen] = useState(false);
  useEffect(() => {
    if (!listShown) setCardOpen(false);
  }, [listShown]);
  // The hover card, the ⋯ label and the two menus are built when first shown.
  const [built, buildOnOpen] = useBuiltOnOpen<'card' | 'moreLabel' | 'contextMenu' | 'dropdown'>();
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
    onDiverge: canDivergeChat(session) ? () => onDiverge(session.id) : undefined,
    onExport: () => void exportConversation(session.id, title, { scope: 'app' }),
    onDelete: () => onRequestDelete(session),
  };
  const onMenuCloseAutoFocus = (event: Event) => {
    if (!renameAfterMenu.current) {
      // A choice made with the pointer leaves focus alone: handing it back to
      // the row would ring a button the person clicked with a mouse and pin the
      // row's hover-revealed ⋯ open.
      keepFocusOffAfterPointerClose(event);
      return;
    }
    renameAfterMenu.current = false;
    event.preventDefault();
    onStartRename(session.id);
  };

  if (editingDraft !== null) {
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
            value={editingDraft}
            onChange={onDraftChange}
            onCommit={(value) => onCommitRename(session.id, value)}
            onCancel={() => onCancelRename(session.id)}
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
      <ContextMenu
        onOpenChange={(open) => {
          buildOnOpen('contextMenu')(open);
          setMenuOpen(open);
        }}
      >
        <Tooltip
          delayDuration={CHAT_HOVER_CARD_DELAY_MS}
          open={cardOpen && listShown}
          onOpenChange={(open) => {
            if (open && labelRef.current) {
              setTitleIsCut(labelRef.current.scrollWidth > labelRef.current.clientWidth);
            }
            buildOnOpen('card')(open);
            setCardOpen(open);
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
          {/* Sized to its text, up to the tooltip's 320px, so `~/path · 3h ago ·
              12 messages` stays one line (V1-M6); a long title clamps to two.
              `hideWhenDetached`: a row scrolled out of the list takes its card
              with it rather than leaving it pinned to the list's edge. */}
          {built.has('card') ? (
            <TooltipContent
              side="right"
              align="start"
              sideOffset={CHAT_HOVER_CARD_OFFSET}
              hideWhenDetached
              className="px-2 py-1.5 font-normal"
            >
              <div data-testid={`recent-chat-summary-${session.id}`}>
                {titleIsCut ? <p className="line-clamp-2">{title}</p> : null}
                <p>{chatHoverDetail(session, homeDir)}</p>
              </div>
            </TooltipContent>
          ) : null}
        </Tooltip>
        {built.has('contextMenu') ? (
          <ChatRowContextMenuContent
            target={target}
            {...handlers}
            onCloseAutoFocus={onMenuCloseAutoFocus}
          />
        ) : null}
      </ContextMenu>
      <DropdownMenu
        onOpenChange={(open) => {
          buildOnOpen('dropdown')(open);
          setMenuOpen(open);
        }}
      >
        <Tooltip onOpenChange={buildOnOpen('moreLabel')}>
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
          {built.has('moreLabel') ? (
            <TooltipContent>{sidebarCopy.chats.moreActions(title)}</TooltipContent>
          ) : null}
        </Tooltip>
        {built.has('dropdown') ? (
          <ChatRowDropdownMenuContent
            target={target}
            {...handlers}
            onCloseAutoFocus={onMenuCloseAutoFocus}
          />
        ) : null}
      </DropdownMenu>
    </li>
  );
});

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
  // The path tooltip, the two button labels and the menu are built when first shown.
  const [built, buildOnOpen] = useBuiltOnOpen<'path' | 'newChatLabel' | 'moreLabel' | 'menu'>();
  const canStartChat =
    !group.isCrew && Boolean(group.workingDir) && Boolean(window.electron?.createChatWindow);
  // The path tooltip opens beside the header's toggle, whose right edge is where
  // the hover-revealed + and ⋯ begin, so at the default offset it covers the very
  // buttons the pointer is heading for. Clear them: 24px each, 2px between and 2px
  // of end padding (`.br-sidebar-folder`, `.br-sidebar-actions`), then the row
  // card's own offset, so the two land on the same x past the divider.
  const actionCount = (canStartChat ? 1 : 0) + (group.workingDir ? 1 : 0);
  const pathTooltipOffset =
    CHAT_HOVER_CARD_OFFSET + 2 + actionCount * 24 + Math.max(0, actionCount - 1) * 2;

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
      <Tooltip delayDuration={CHAT_HOVER_CARD_DELAY_MS} onOpenChange={buildOnOpen('path')}>
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
        {group.path && built.has('path') ? (
          <TooltipContent side="right" sideOffset={pathTooltipOffset}>
            {group.path}
          </TooltipContent>
        ) : null}
      </Tooltip>
      <div className="br-sidebar-actions">
        {canStartChat ? (
          <Tooltip onOpenChange={buildOnOpen('newChatLabel')}>
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
            {built.has('newChatLabel') ? (
              <TooltipContent>{sidebarCopy.folder.newChat(group.label)}</TooltipContent>
            ) : null}
          </Tooltip>
        ) : null}
        {group.workingDir ? (
          <DropdownMenu
            onOpenChange={(open) => {
              buildOnOpen('menu')(open);
              setMenuOpen(open);
            }}
          >
            <Tooltip onOpenChange={buildOnOpen('moreLabel')}>
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
              {built.has('moreLabel') ? (
                <TooltipContent>{sidebarCopy.folder.moreActions(group.label)}</TooltipContent>
              ) : null}
            </Tooltip>
            {built.has('menu') ? (
              <DropdownMenuContent
                align="end"
                className="w-56"
                onCloseAutoFocus={keepFocusOffAfterPointerClose}
              >
                <DropdownMenuItem onSelect={() => void copyPath()}>
                  {sidebarCopy.folder.copyPath}
                </DropdownMenuItem>
              </DropdownMenuContent>
            ) : null}
          </DropdownMenu>
        ) : null}
      </div>
    </div>
  );
}

/**
 * Scroll the list just enough to show `row`, and its date bucket or folder
 * header too when it is the first chat under one, so a chat that moved to the
 * top reads under its "Today". Measured against the list itself, never with
 * `scrollIntoView`, which also scrolls every clipped box around it.
 */
export function scrollRowIntoView(container: HTMLElement, row: HTMLElement): void {
  const section = row.closest('section');
  const firstInSection = section?.querySelector('[data-row^="chat:"]') === row;
  const top = (firstInSection && section ? section : row).getBoundingClientRect().top;
  const { bottom } = row.getBoundingClientRect();
  const box = container.getBoundingClientRect();
  if (top < box.top) container.scrollTop -= box.top - top;
  else if (bottom > box.bottom) container.scrollTop += bottom - box.bottom;
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
  const { open: sidebarOpen, setOpen: setSidebarOpen } = useSidebar();
  const scrollContainerRef = useRef<HTMLDivElement | null>(null);
  const { diverge } = useDiverge();

  // Kept per view while this list is held: switching back is a lookup (R-28).
  const arranged = useMemo(
    () => arrangeSidebarChatsCached(sessions, view, Date.now(), { homeDir }),
    [sessions, view, homeDir]
  );

  // Rename in place. The draft lives here, keyed by session id, so a head
  // refresh that moves or re-renders the row keeps what the person typed.
  const [editing, setEditing] = useState<{ sessionId: string; draft: string } | null>(null);
  const focusAfterEdit = useRef<string | null>(null);

  // Principle 9: while the pointer is over the list (or a row is being renamed)
  // the rows hold still; the live order applies when it leaves.
  //
  // ⚠ Only a COMPLETE list is held past its read (vision V1-R2-3). A non-default
  // view shows its first page while the rest is read. A hold taken from that
  // page keeps its rows still during the read (V1-H2), and gives way to the
  // complete arrangement the moment the read lands: one move, at the end. Kept
  // as it was, it held the first page's order (50 rows sorted by name, the
  // wrong 50) for as long as the pointer rested on the list. Nothing is held
  // while the list is empty either, so a first load's first page is held as
  // drawn rather than as "every row arrives below nothing", which held nothing.
  const listComplete = !isLoadingAll;
  const [pointerInside, setPointerInside] = useState(false);
  const [held, setHeld] = useState<{ arrangement: HeldArrangement; complete: boolean } | null>(
    null
  );
  const holding = pointerInside || editing !== null;
  const heldApplies =
    holding &&
    held !== null &&
    held.arrangement.view.groupBy === view.groupBy &&
    held.arrangement.view.sortBy === view.sortBy &&
    (held.complete || !listComplete);
  const groups = useMemo(
    () => (heldApplies && held ? applyHeldArrangement(held.arrangement, arranged) : arranged),
    [heldApplies, held, arranged]
  );
  // Take the hold from what is drawn: whenever the list is held and no hold
  // applies (none yet, another view's, or a partial one the full read has
  // replaced), the rows on screen ARE the live arrangement.
  useEffect(() => {
    if (!holding) {
      setHeld(null);
      return;
    }
    if (heldApplies || arranged.length === 0) return;
    setHeld({ arrangement: holdArrangement(arranged, view), complete: listComplete });
  }, [holding, heldApplies, arranged, view, listComplete]);

  // The list's own height (a ResizeObserver below keeps it): how many lines a
  // view draws at first, and when the fill check runs again.
  const [listHeight, setListHeight] = useState(0);

  // A view change re-renders the list at rest and fades it in from 0.6, and
  // draws again from the first lines (R-28: `renderBudget` is how many lines
  // the list draws; it grows as the list is scrolled), only as many as the
  // list shows plus an overscan. Both are decided while the new view RENDERS
  // (vision V1-R2-2). Set from an effect, they landed one commit late: the new
  // order was first drawn under the old key and the old budget, then every row
  // was mounted a second time under the new key, so a switch paid for its rows
  // twice (and once more for every line scrolled into view before it).
  const viewKey = `${view.groupBy}/${view.sortBy}`;
  const [drawn, setDrawn] = useState({ viewKey, generation: 0, budget: SIDEBAR_RENDER_WINDOW });
  if (drawn.viewKey !== viewKey) {
    setDrawn({
      viewKey,
      generation: drawn.generation + 1,
      budget: sidebarRenderWindow(listHeight),
    });
  }
  const viewGeneration = drawn.generation;
  const renderBudget = drawn.budget;
  const setRenderBudget = useCallback(
    (next: number | ((budget: number) => number)) =>
      setDrawn((current) => {
        const budget = typeof next === 'function' ? next(current.budget) : next;
        return budget === current.budget ? current : { ...current, budget };
      }),
    []
  );
  // A different arrangement starts from its top, not from wherever the last
  // one had been scrolled to.
  useLayoutEffect(() => {
    if (viewGeneration > 0 && scrollContainerRef.current) {
      scrollContainerRef.current.scrollTop = 0;
    }
  }, [viewGeneration]);

  const [collapsedFolders, , setFolderCollapsed] = useCollapsedFolders();
  const [expandedFolders, setExpandedFolders] = useState<ReadonlySet<string>>(new Set());
  // A folder that holds the current chat is drawn open whatever is stored, so a
  // click to close it needs somewhere to live: the keys the person closed while
  // that chat was current. It expires when another chat becomes current, so the
  // folder opens itself again for the next chat that lives in it.
  const [closedWithActive, setClosedWithActive] = useState<{
    sessionId: string | null;
    keys: readonly string[];
  }>({ sessionId: null, keys: [] });

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
            holdsActive: false,
            rows: group.sessions,
            more: null as null | 'more' | 'less',
          };
        }
        const activeIndex = group.sessions.findIndex((session) => session.id === activeSessionId);
        const closedByPerson =
          closedWithActive.sessionId === activeSessionId &&
          closedWithActive.keys.includes(group.key);
        const collapsed = (collapsedFolders.has(group.key) && activeIndex < 0) || closedByPerson;
        if (collapsed) {
          return { group, collapsed, holdsActive: activeIndex >= 0, rows: [], more: null };
        }
        const long = group.sessions.length > FOLDER_PREVIEW_COUNT;
        const expanded = expandedFolders.has(group.key) || activeIndex >= FOLDER_PREVIEW_COUNT;
        return {
          group,
          collapsed,
          holdsActive: activeIndex >= 0,
          rows: long && !expanded ? group.sessions.slice(0, FOLDER_PREVIEW_COUNT) : group.sessions,
          more: !long ? null : expanded ? ('less' as const) : ('more' as const),
        };
      }),
    [groups, collapsedFolders, expandedFolders, closedWithActive, activeSessionId]
  );

  // R-28: only the first lines are drawn; the rest wait until the list is
  // scrolled near its end. The whole list is still held and arranged.
  const windowed = useMemo(
    () => windowSidebarRows(shown, renderBudget, headerLines),
    [shown, renderBudget]
  );

  // One tab stop for the list (spec 3.4, F-18): ↑/↓/Home/End move between rows.
  const rowKeys = useMemo(
    () =>
      windowed.entries.flatMap(({ group, rows, more }) => [
        ...(group.kind === 'folder' ? [folderKey(group)] : []),
        ...rows.map((session) => chatKey(session.id)),
        ...(more ? [moreKey(group)] : []),
      ]),
    [windowed]
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
      // Whatever started the edit (F2, ⌘⌥R, the menu), the rows hold still
      // while the person types, as they do under the pointer (`holding`).
      // The editor mounts in the chat's row. ⌘⌥R names the chat you are in, and
      // in a long list that row may lie past the lines drawn so far (R-28): draw
      // up to it, or the list would hold still around an editor nobody can see.
      const needed = linesToDrawRow(shown, sessionId, headerLines);
      if (needed !== null) setRenderBudget((budget) => Math.max(budget, needed));
      setEditing({ sessionId, draft: session.name.trim() || sidebarCopy.chats.untitled });
    },
    [sessions, shown, setRenderBudget]
  );

  // A chat deleted (here or in another window) while it is being renamed leaves
  // no row to hold the editor, and nothing else would end the hold it carries.
  useEffect(() => {
    if (editing && !sessions.some((session) => session.id === editing.sessionId)) {
      setEditing(null);
    }
  }, [editing, sessions]);

  const finishRename = useCallback((sessionId: string) => {
    focusAfterEdit.current = sessionId;
    setEditing(null);
  }, []);

  // Ruling 16 (R-30): a person's own rename moves its chat AT ONCE, with the
  // pointer still over the list (vision V1-R2-4). The hold is there so a row
  // never moves under a click the person is about to make; the move a rename
  // makes is the one they just asked for, and the usual way to rename (row
  // menu, type, Enter) leaves the pointer on the list, so held, the move almost
  // never showed. A committed rename lets go of the hold twice: when the new
  // name is announced (a name sort moves the row then), and when the daemon has
  // saved it (`activityAt` raises the row under "Last activity") or refused it
  // (the old name comes back). Each time the list is held again from the new
  // order, and the row is kept in view.
  const [reveal, setReveal] = useState<{ sessionId: string } | null>(null);
  const revealed = useRef<{ sessionId: string } | null>(null);
  const showOwnRename = useCallback((sessionId: string) => {
    setHeld(null);
    setReveal({ sessionId });
  }, []);

  const commitRename = (sessionId: string, value: string) => {
    const session = sessions.find((candidate) => candidate.id === sessionId);
    finishRename(sessionId);
    if (!session) return;
    const name = normalizeSessionName(value);
    // A chat with no name is drawn "Untitled chat", and that is the text the
    // editor opens with: Enter on it, unchanged, must not name the chat that.
    if (!session.name.trim() && name === sidebarCopy.chats.untitled) return;
    // No call for an empty or unchanged name (the helper's own rule), so no move.
    if (!name || name === session.name) return;
    showOwnRename(sessionId);
    void renameSessionOptimistically(sessionId, value, {
      name: session.name,
      userSetName: session.user_set_name ?? false,
    }).then(() => showOwnRename(sessionId));
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

  // The renamed row, where it has moved to: drawn (a name sort can put it past
  // the lines drawn so far, R-28), scrolled into view, and focused again when
  // the move took focus with it (a row that changes date bucket remounts, and
  // focus falls to the page). Focus the person put somewhere else stays there.
  useLayoutEffect(() => {
    if (!reveal || revealed.current === reveal) return;
    const container = scrollContainerRef.current;
    if (!container) return;
    const row = container.querySelector<HTMLElement>(
      `[data-row="${CSS.escape(chatKey(reveal.sessionId))}"]`
    );
    if (!row) {
      const needed = linesToDrawRow(shown, reveal.sessionId, headerLines);
      if (needed !== null && needed > renderBudget) {
        setRenderBudget(needed);
        return;
      }
      revealed.current = reveal;
      return;
    }
    revealed.current = reveal;
    const focused = document.activeElement;
    if (!focused || focused === document.body) row.focus({ preventScroll: true });
    scrollRowIntoView(container, row);
  }, [reveal, windowed, shown, renderBudget, setRenderBudget]);

  // ⌘⌥R (Ctrl+Alt+R) renames the chat you are in, when its row is listed.
  useEffect(() => {
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      const mod = isMacPlatform() ? event.metaKey : event.ctrlKey;
      if (!mod || !event.altKey || event.shiftKey || event.code !== 'KeyR') return;
      if (!activeSessionId || !sessions.some((session) => session.id === activeSessionId)) return;
      event.preventDefault();
      // A closed sidebar is inert and off screen: an editor mounted in it could
      // not take focus and would sit there unseen. Open it first.
      if (!sidebarOpen) setSidebarOpen(true);
      setIsExpanded(true);
      startRename(activeSessionId);
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [activeSessionId, sessions, startRename, sidebarOpen, setSidebarOpen]);

  // Delete, behind one confirmation for the list.
  const [pendingDelete, setPendingDelete] = useState<SessionSummary | null>(null);
  const [deleting, setDeleting] = useState(false);
  const deletePending = useRef(false);
  const handleDelete = async () => {
    const session = pendingDelete;
    if (!session || deletePending.current) return;
    deletePending.current = true;
    setDeleting(true);
    try {
      await deleteConversation(session.id);
      setPendingDelete(null);
      // History's toast, word for word: the title alone, no sentence under it.
      toastSuccess({ title: chatRowCopy.deleteDialog.deleted });
    } catch (error) {
      // A refusal stays readable: the daemon's own sentence is the message.
      toastError({
        title: chatRowCopy.deleteDialog.failed,
        msg:
          typeof error === 'string' && error.trim()
            ? error
            : error instanceof Error && error.message.trim()
              ? error.message
              : chatRowCopy.deleteDialog.failedFallback,
      });
    } finally {
      deletePending.current = false;
      setDeleting(false);
    }
  };

  // Near the end of the list: draw more of what is held before asking the
  // daemon for another page (R-28).
  const windowCut = windowed.cut;
  const handleScroll = useCallback(
    (event: UIEvent<HTMLDivElement>) => {
      const container = event.currentTarget;
      const remainingScroll = container.scrollHeight - container.scrollTop - container.clientHeight;
      if (remainingScroll > LOAD_MORE_THRESHOLD_PX) return;
      if (windowCut) setRenderBudget((budget) => budget + SIDEBAR_RENDER_WINDOW_STEP);
      else if (hasMore && !isLoadingMore && !isLoadingAll) onLoadMore();
    },
    [windowCut, hasMore, isLoadingMore, isLoadingAll, onLoadMore, setRenderBudget]
  );

  // The list's own height, so the fill check below runs again when its box
  // changes (vision V1-R2-1). A list that ten rows filled in the short overlay
  // sidebar has room to spare once the window widens into the docked one, or
  // Components collapses, or Chats opens: nothing then scrolls, so no scroll
  // event would ever ask for the next page, and every chat past the first ten
  // was out of reach of the sidebar.
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => setListHeight(container.clientHeight));
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || container.clientHeight === 0) return;
    if (container.scrollHeight > container.clientHeight) return;
    if (windowCut) setRenderBudget((budget) => budget + SIDEBAR_RENDER_WINDOW_STEP);
    else if (hasMore && !isLoadingMore && !isLoadingAll) onLoadMore();
    // `renderBudget` is a dependency so a very tall list that 60 lines do not
    // fill keeps drawing until it overflows (or everything held is drawn), and
    // `listHeight` so a list whose box grew fills the room it gained.
  }, [
    windowCut,
    renderBudget,
    hasMore,
    isLoadingMore,
    isLoadingAll,
    onLoadMore,
    sessions.length,
    listHeight,
    setRenderBudget,
  ]);

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
  }, [windowed, runningSessionIds]);

  const setScrollRef = (node: HTMLDivElement | null) => {
    scrollContainerRef.current = node;
    roving.containerRef.current = node;
  };

  // The rows are memoized, so what the list hands them must keep its identity
  // across renders (R-28): `onOpen` arrives as a new closure on every render of
  // the sidebar, and the rename handlers read the latest list.
  const openChat = useStableHandler(onOpen);
  const startRenameRow = useStableHandler(startRename);
  const commitRenameRow = useStableHandler(commitRename);
  const divergeRow = useCallback((sessionId: string) => void diverge(sessionId), [diverge]);
  const changeDraft = useCallback(
    (draft: string) => setEditing((current) => (current ? { ...current, draft } : current)),
    []
  );

  const rowProps = (session: SessionSummary) => ({
    session,
    isActive: activeSessionId === session.id,
    isRunning: runningSessionIds.has(session.id),
    homeDir,
    listShown: isExpanded && sidebarOpen,
    tabIndex: roving.tabIndexFor(chatKey(session.id)),
    onRowFocus: roving.onRowFocus,
    onOpen: openChat,
    onStartRename: startRenameRow,
    onRequestDelete: setPendingDelete,
    onDiverge: divergeRow,
    editingDraft: editing?.sessionId === session.id ? editing.draft : null,
    onDraftChange: changeDraft,
    onCommitRename: commitRenameRow,
    onCancelRename: finishRename,
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
        onPointerEnter={() => setPointerInside(true)}
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
            {windowed.entries.map(({ group, collapsed, holdsActive, rows, more }) => (
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
                    onToggle={() => {
                      // What a click does follows what is DRAWN: an open folder
                      // closes and a closed one opens, even when the open one is
                      // only open because it holds the current chat.
                      if (collapsed) {
                        setFolderCollapsed(group.key, false);
                        setClosedWithActive((current) => ({
                          ...current,
                          keys: current.keys.filter((key) => key !== group.key),
                        }));
                        return;
                      }
                      setFolderCollapsed(group.key, true);
                      if (holdsActive) {
                        setClosedWithActive((current) => ({
                          sessionId: activeSessionId ?? null,
                          keys: [
                            ...(current.sessionId === (activeSessionId ?? null)
                              ? current.keys
                              : []),
                            group.key,
                          ],
                        }));
                      }
                    }}
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
                          data-testid={`sidebar-folder-more-${group.key}`}
                          // A disclosure of the folder's other rows. `aria-expanded`
                          // also tells the overlay sidebar this is not a choice, so
                          // showing more rows does not close the panel.
                          aria-expanded={more === 'less'}
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
