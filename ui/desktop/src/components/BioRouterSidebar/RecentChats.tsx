import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  type CSSProperties,
  type FocusEvent as ReactFocusEvent,
  type KeyboardEvent,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type UIEvent,
} from 'react';
import { flushSync } from 'react-dom';

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
import { inMenuOpenedFrom, useRovingRows } from '../ui/useRovingRows';
import { sidebarCopy } from './copy';
import SidebarViewMenu from './SidebarViewMenu';
import {
  applyHeldArrangement,
  arrangeSidebarChatsCached,
  holdArrangement,
  linesToDrawRow,
  nextSidebarDrawRange,
  SIDEBAR_LINE_HEIGHT_PX,
  SIDEBAR_RENDER_WINDOW,
  SIDEBAR_RENDER_WINDOW_STEP,
  sidebarLinesInView,
  sidebarRenderWindow,
  sidebarRowTitle,
  sliceSidebarLines,
  windowSidebarRows,
  tildePath,
  type HeldArrangement,
  type SidebarChatGroup,
  type SidebarChatView,
  type SidebarDrawRange,
  type SidebarGroupBox,
} from './sidebarChatView';
import { useCollapsedFolders } from './useSidebarChatView';
import './sidebar.css';

const LOAD_MORE_THRESHOLD_PX = 64;
const RECENTS_EXPANDED_STORAGE_KEY = 'biorouter:sidebar-recents-expanded';
/**
 * A folder group shows this many chats (and the current chat, when it is
 * further down), then a "Show more" row.
 */
export const FOLDER_PREVIEW_COUNT = 5;

/**
 * The chats a folder's preview keeps past its first five. At rest, only the
 * current chat. While the list is held (principle 9), every chat that has been
 * current since the hold began, so a row drawn for the last one does not leave
 * from under the pointer. Returns `kept` itself when nothing changes.
 */
export function keepInFolderPreview(
  kept: readonly string[],
  activeSessionId: string | null,
  holding: boolean
): readonly string[] {
  if (holding) {
    return !activeSessionId || kept.includes(activeSessionId) ? kept : [...kept, activeSessionId];
  }
  if (!activeSessionId) return kept.length === 0 ? kept : [];
  return kept.length === 1 && kept[0] === activeSessionId ? kept : [activeSessionId];
}
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
 * What a row SHOWS, and what its accessible names and the delete sentence say:
 * the kind is said once, in the glyph (`sidebarRowTitle`, History's resolver),
 * so a scheduled run reads "daily-meditation" beside its clock (vision V1-R3-6).
 */
const shownTitle = (session: SessionSummary) =>
  sidebarRowTitle(session) || sidebarCopy.chats.untitled;

/**
 * The chat's own name, for what works ON the name rather than showing it: the
 * rename draft starts from it and an export is titled with it, as in History.
 */
const storedTitle = (session: SessionSummary) => session.name.trim() || sidebarCopy.chats.untitled;

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

interface LazyMenuButtonProps {
  /** The button's accessible name, and its tooltip. */
  label: string;
  className: string;
  testId?: string;
  tabIndex: 0 | -1;
  onFocus: () => void;
  /** The menu opened or closed (the row or header lights while it is open). */
  onOpenChange: (open: boolean) => void;
  /** The menu's content, built the first time the menu opens. */
  children: ReactNode;
}

/**
 * A ⋯ button whose menu root is mounted only once the button is used (R-28,
 * vision V1-R2-2). A mounted Radix menu root adds a capture `keydown` listener
 * to the document, and on every key press each of those adds two more, so a
 * list with a menu root in every drawn row paid for every row on every key,
 * and again when a view switch unmounted them all: with 4,233 rows drawn
 * after a scroll to the end, one switch took 0.56 s in a production build,
 * most of it in `addEventListener` and `removeEventListener`. Until it is
 * focused, pressed or opened from the keyboard the button is a plain one that
 * says it opens a menu; then the menu takes it over, already open when the
 * press asked for that, and keeps focus on it when focus was what armed it.
 * It lets go when it loses focus with the menu shut.
 */
function LazyMenuButton({
  label,
  className,
  testId,
  tabIndex,
  onFocus,
  onOpenChange,
  children,
}: LazyMenuButtonProps) {
  const [armed, setArmed] = useState(false);
  const [open, setOpen] = useState(false);
  const [built, buildOnOpen] = useBuiltOnOpen<'menu' | 'label'>();
  const buttonRef = useRef<HTMLButtonElement>(null);
  const focusWhenArmed = useRef(false);
  const changeOpen = (next: boolean) => {
    buildOnOpen('menu')(next);
    setOpen(next);
    onOpenChange(next);
  };
  const armOpen = () => {
    setArmed(true);
    changeOpen(true);
  };
  // Armed by focus: the menu's trigger replaces the plain button, so focus
  // moves to it in the same frame.
  useLayoutEffect(() => {
    if (!armed || !focusWhenArmed.current) return;
    focusWhenArmed.current = false;
    buttonRef.current?.focus({ preventScroll: true });
  }, [armed]);

  const button = {
    type: 'button' as const,
    variant: 'ghost' as const,
    shape: 'round' as const,
    size: 'xs' as const,
    className,
    'aria-label': label,
    'data-testid': testId,
    tabIndex,
  };
  const labelTooltip = built.has('label') ? <TooltipContent>{label}</TooltipContent> : null;

  if (!armed) {
    return (
      <Tooltip onOpenChange={buildOnOpen('label')}>
        <TooltipTrigger asChild>
          <Button
            {...button}
            aria-haspopup="menu"
            aria-expanded={false}
            data-state="closed"
            onFocus={() => {
              onFocus();
              focusWhenArmed.current = true;
              setArmed(true);
            }}
            // The press that opens the menu, as the menu's own trigger takes it:
            // the main button without Control, and no focus moved by it.
            onPointerDown={(event: ReactPointerEvent<HTMLButtonElement>) => {
              if (event.button !== 0 || event.ctrlKey) return;
              event.preventDefault();
              armOpen();
            }}
            onKeyDown={(event: KeyboardEvent<HTMLButtonElement>) => {
              if (event.key !== 'Enter' && event.key !== ' ' && event.key !== 'ArrowDown') return;
              event.preventDefault();
              armOpen();
            }}
          >
            <MoreHorizontal className="size-4" aria-hidden />
          </Button>
        </TooltipTrigger>
        {labelTooltip}
      </Tooltip>
    );
  }

  return (
    <DropdownMenu open={open} onOpenChange={changeOpen}>
      <Tooltip onOpenChange={buildOnOpen('label')}>
        <TooltipTrigger asChild>
          <DropdownMenuTrigger asChild>
            <Button
              {...button}
              ref={buttonRef}
              onFocus={onFocus}
              onBlur={() => {
                if (!open) setArmed(false);
              }}
            >
              <MoreHorizontal className="size-4" aria-hidden />
            </Button>
          </DropdownMenuTrigger>
        </TooltipTrigger>
        {labelTooltip}
      </Tooltip>
      {built.has('menu') ? children : null}
    </DropdownMenu>
  );
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
  /** The row's ⋯ menu opened or closed: the list keeps a row drawn while its menu is open. */
  onLineMenuChange: (key: string, open: boolean) => void;
  /**
   * `title` is the name this row is ALREADY rendering, handed to the opener so
   * the new tab is born with it. Omitted when the session is unnamed — then the
   * tab's own placeholder is the honest answer, not this row's "Untitled chat".
   */
  onOpen: (sessionId: string, title?: string, userSetName?: boolean) => void;
  onStartRename: (sessionId: string) => void;
  onRequestDelete: (session: SessionSummary) => void;
  onDiverge: (sessionId: string) => void;
  /**
   * True while the list's right-click menu is open on this row (one menu for
   * the list, R-28): the row keeps its lit ground under it, as it does for its
   * own ⋯ menu.
   */
  contextMenuOpen: boolean;
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
  onLineMenuChange,
  onOpen,
  onStartRename,
  onRequestDelete,
  onDiverge,
  contextMenuOpen,
  editingDraft,
  onDraftChange,
  onCommitRename,
  onCancelRename,
}: RecentChatRowProps) {
  const title = shownTitle(session);
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
  // Nor while the row's own menu is open: the card would draw beside the menu,
  // and the first Escape would close the card instead of the menu (V1-R5-2).
  const rowMenuShown = menuOpen || contextMenuOpen;
  useEffect(() => {
    if (!listShown || rowMenuShown) setCardOpen(false);
  }, [listShown, rowMenuShown]);
  // The hover card is built when first shown; the ⋯ builds its own label and menu.
  const [built, buildOnOpen] = useBuiltOnOpen<'card'>();
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
    onExport: () => void exportConversation(session.id, storedTitle(session), { scope: 'app' }),
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
      // The editor's own right-click is the app's text menu (cut, copy, paste,
      // spelling), not the list's chat menu, which a right-click anywhere else
      // in the list reaches by bubbling.
      <li
        className="br-chat-row-item"
        data-row-item=""
        onContextMenu={(event) => event.stopPropagation()}
      >
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
    <li
      className="br-chat-row-item"
      data-row-item=""
      data-menu-open={menuOpen || contextMenuOpen || undefined}
    >
      <Tooltip
        delayDuration={CHAT_HOVER_CARD_DELAY_MS}
        open={cardOpen && listShown && !rowMenuShown}
        onOpenChange={(open) => {
          if (open && rowMenuShown) return;
          if (open && labelRef.current) {
            setTitleIsCut(labelRef.current.scrollWidth > labelRef.current.clientWidth);
          }
          buildOnOpen('card')(open);
          setCardOpen(open);
        }}
      >
        {/* The right-click menu is the list's (one for every row, R-28): a
            right-click on this button, or Shift+F10 on it, bubbles to the
            list, which reads the row from `data-row`. */}
        <TooltipTrigger asChild>
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
      <LazyMenuButton
        label={sidebarCopy.chats.moreActions(title)}
        className="br-chat-row-more no-drag text-text-muted hover:text-text-default"
        testId={`recent-chat-more-${session.id}`}
        tabIndex={tabIndex}
        onFocus={() => onRowFocus(key)}
        onOpenChange={(open) => {
          setMenuOpen(open);
          onLineMenuChange(key, open);
        }}
      >
        <ChatRowDropdownMenuContent
          target={target}
          {...handlers}
          onCloseAutoFocus={onMenuCloseAutoFocus}
        />
      </LazyMenuButton>
    </li>
  );
});

// ── A folder group's header ───────────────────────────────────────────────

interface FolderHeaderProps {
  group: SidebarChatGroup;
  collapsed: boolean;
  tabIndex: 0 | -1;
  onRowFocus: (key: string) => void;
  onLineMenuChange: (key: string, open: boolean) => void;
  onToggle: () => void;
}

function FolderHeader({
  group,
  collapsed,
  tabIndex,
  onRowFocus,
  onLineMenuChange,
  onToggle,
}: FolderHeaderProps) {
  const key = folderKey(group);
  const [menuOpen, setMenuOpen] = useState(false);
  // The path tooltip and the + label are built when first shown; the ⋯ builds its own.
  const [built, buildOnOpen] = useBuiltOnOpen<'path' | 'newChatLabel'>();
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
          <LazyMenuButton
            label={sidebarCopy.folder.moreActions(group.label)}
            className="no-drag text-text-muted hover:text-text-default"
            tabIndex={tabIndex}
            onFocus={() => onRowFocus(key)}
            onOpenChange={(open) => {
              setMenuOpen(open);
              onLineMenuChange(key, open);
            }}
          >
            <DropdownMenuContent
              align="end"
              className="w-56"
              onCloseAutoFocus={keepFocusOffAfterPointerClose}
            >
              <DropdownMenuItem onSelect={() => void copyPath()}>
                {sidebarCopy.folder.copyPath}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </LazyMenuButton>
        ) : null}
      </div>
    </div>
  );
}

/**
 * The list's line pitch, read off a drawn row: its height and the gap its
 * list puts below it. Both scale with the Text size preference (30px at the
 * standard size, 34.5px under Larger text), so reading the lines in view at a
 * fixed 30px put them further down the list the further it was scrolled, until
 * the view showed only placeholder. The standard pitch when no row is drawn.
 */
export function measuredLinePitch(container: HTMLElement): number {
  const item = container.querySelector<HTMLElement>(
    'section[data-rows-line] > ul > li[data-row-item]'
  );
  const list = item?.parentElement;
  if (!item || !list) return SIDEBAR_LINE_HEIGHT_PX;
  const pitch =
    item.getBoundingClientRect().height + (parseFloat(getComputedStyle(list).rowGap) || 0);
  return pitch > 0 ? pitch : SIDEBAR_LINE_HEIGHT_PX;
}

/**
 * Scroll the list just enough to show `row`, and its date bucket or folder
 * header too when it is the first chat under one, so a chat that moved to the
 * top reads under its "Today". Measured against the list itself, never with
 * `scrollIntoView`, which also scrolls every clipped box around it. The list's
 * `scroll-padding` (sidebar.css) keeps the row clear of its edge fades, as it
 * does for the browser's own focus scroll.
 */
export function scrollRowIntoView(container: HTMLElement, row: HTMLElement): void {
  const section = row.closest('section');
  // The group's first LINE, not its first drawn row: rows above it may be
  // folded into a gap (R-28), and showing that row's header would scroll the
  // list up past all of them.
  const item = row.closest('li');
  const firstLine = section?.querySelector(':scope > ul > li');
  const firstInSection =
    item && firstLine ? firstLine === item : section?.querySelector('[data-row^="chat:"]') === row;
  const top = (firstInSection && section ? section : row).getBoundingClientRect().top;
  const { bottom } = row.getBoundingClientRect();
  const box = container.getBoundingClientRect();
  const style = getComputedStyle(container);
  const from = box.top + (parseFloat(style.scrollPaddingTop) || 0);
  const to = box.bottom - (parseFloat(style.scrollPaddingBottom) || 0);
  if (top < from) container.scrollTop -= from - top;
  else if (bottom > to) container.scrollTop += bottom - to;
}

/**
 * Whether the list is scrolled off its top, as `data-scrolled` (vision
 * V1-R4-2): it fades the list's top edge (sidebar.css), so a date label or a
 * row cut by the Chats header fades out rather than being sliced through. Set
 * on the element itself from the scroll handler, so a scroll re-renders
 * nothing; React does not own the attribute.
 */
export function markScrolledOffTop(container: HTMLElement): void {
  const scrolled = container.scrollTop > 0;
  if (container.hasAttribute('data-scrolled') === scrolled) return;
  if (scrolled) container.setAttribute('data-scrolled', 'true');
  else container.removeAttribute('data-scrolled');
}

// ── The list ──────────────────────────────────────────────────────────────

interface RecentChatsProps {
  sessions: SessionSummary[];
  activeSessionId?: string | null;
  runningSessionIds: ReadonlySet<string>;
  hasMore: boolean;
  isLoadingMore: boolean;
  /**
   * True while a non-default view reads the whole list for the first time. The
   * view is not drawn until it lands (vision V1-R3-3): see `shownView`.
   */
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

  // The view the list DRAWS (vision V1-R3-3). A view that sorts or groups the
  // whole list is not drawn from part of it: until its first full read lands,
  // the list keeps drawing the view it drew before (the default, the only one
  // a part of the list can be drawn in), or, on a first load with that view
  // stored, no rows and only "Loading chats…". Drawn from its first page, a
  // name sort put the wrong 50 chats in name order and then reordered them all
  // when the read landed, 0.2 to 0.4 s later, under a resting pointer. The view
  // menu shows the choice at once; the rows change once, to the complete list.
  const viewReady = !isLoadingAll;
  const [lastReadyView, setLastReadyView] = useState<SidebarChatView | null>(
    viewReady ? view : null
  );
  if (
    viewReady &&
    (lastReadyView?.groupBy !== view.groupBy || lastReadyView?.sortBy !== view.sortBy)
  ) {
    setLastReadyView(view);
  }
  const shownView = viewReady ? view : lastReadyView;

  // Kept per view while this list is held: switching back is a lookup (R-28).
  const arranged = useMemo(
    () =>
      shownView ? arrangeSidebarChatsCached(sessions, shownView, Date.now(), { homeDir }) : [],
    [sessions, shownView, homeDir]
  );

  // Rename in place. The draft lives here, keyed by session id, so a head
  // refresh that moves or re-renders the row keeps what the person typed.
  const [editing, setEditing] = useState<{ sessionId: string; draft: string } | null>(null);
  const focusAfterEdit = useRef<string | null>(null);

  // The list's right-click menu (one for the list, R-28) and the row or folder
  // header whose ⋯ menu is open. A ⋯ opened with the pointer never takes focus
  // (Radix prevents it), and a programmatic scroll can still move the list under
  // a menu: the line stays drawn while its menu is open, or the menu would vanish.
  const [menuRowId, setMenuRowId] = useState<string | null>(null);
  const [rowMenuOpen, setRowMenuOpen] = useState(false);
  const [lineMenuKey, setLineMenuKey] = useState<string | null>(null);
  const onLineMenuChange = useCallback(
    (key: string, open: boolean) =>
      setLineMenuKey((current) => (open ? key : current === key ? null : current)),
    []
  );
  // A row menu is open: the right-click menu, or a row's or folder header's ⋯.
  // Neither menu is modal (a modal one restyles the whole window, R-28), so
  // the list makes itself modal instead, and only itself (vision V1-R5-2):
  // - it does not scroll (`useEffect` below), so the menu stays beside its row
  //   and Delete, Rename and Export act on the chat on screen;
  // - its rows take no pointer (`data-row-menu-open`, sidebar.css): no other
  //   row lights or opens its hover card over the menu, and the press that
  //   dismisses the menu lands on the list, not on a row, so it only closes
  //   the menu, as it does in History;
  // - its rows hold still, wherever the pointer is (`holding`, below).
  const listMenuOpen = rowMenuOpen || lineMenuKey !== null;
  // The wheel and a touch drag scroll nothing while a row menu is open. Not
  // `overflow: hidden`: with scroll bars always shown, that takes the bar away
  // and widens every row under the menu.
  useEffect(() => {
    const list = scrollContainerRef.current;
    if (!listMenuOpen || !list) return;
    const stay = (event: Event) => event.preventDefault();
    list.addEventListener('wheel', stay, { passive: false });
    list.addEventListener('touchmove', stay, { passive: false });
    return () => {
      list.removeEventListener('wheel', stay);
      list.removeEventListener('touchmove', stay);
    };
  }, [listMenuOpen]);

  // Principle 9: while the pointer is over the list (or a row is being renamed,
  // or a row's menu is open) the rows hold still; the live order applies when
  // it leaves.
  //
  // A hold is of the view DRAWN. A view that waits for its full read is not
  // drawn until the read lands (`shownView`), so what is drawn is always right
  // for its view and a hold taken from it can stand (V1-R2-3 had to let go of a
  // hold taken from a first page, which was the wrong 50 rows in name order).
  // When the read lands the drawn view changes, the old hold stops applying,
  // and the list is held again from the complete arrangement. Nothing is held
  // while the list is empty either, so a first load's first page is held as
  // drawn rather than as "every row arrives below nothing", which held nothing.
  const [pointerInside, setPointerInside] = useState(false);
  const [held, setHeld] = useState<HeldArrangement | null>(null);
  const holding = pointerInside || editing !== null || listMenuOpen;
  const heldApplies =
    holding &&
    held !== null &&
    shownView !== null &&
    held.view.groupBy === shownView.groupBy &&
    held.view.sortBy === shownView.sortBy;
  const groups = useMemo(
    () => (heldApplies && held ? applyHeldArrangement(held, arranged) : arranged),
    [heldApplies, held, arranged]
  );
  // Take the hold from what is drawn: whenever the list is held and no hold
  // applies (none yet, or another view's), the rows on screen ARE the live
  // arrangement.
  useEffect(() => {
    if (!holding) {
      setHeld(null);
      return;
    }
    if (heldApplies || !shownView || arranged.length === 0) return;
    setHeld(holdArrangement(arranged, shownView));
  }, [holding, heldApplies, arranged, shownView]);

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
  //
  // The first view a list draws is not a change: a first load whose stored
  // view waited for its full read (`shownView` null until then) draws at rest.
  //
  // `range` is the lines drawn for real (vision V1-R2-2, round 4): the rows
  // near the viewport. `budget` is how far down the list reaches, and every
  // line of it outside `range` is a placeholder of its height, so rows drawn
  // never pile up as the list is scrolled. Null until the list is measured,
  // and after a view change, which starts at the top: every line is drawn.
  const viewKey = shownView ? `${shownView.groupBy}/${shownView.sortBy}` : null;
  const [drawn, setDrawn] = useState<{
    viewKey: string | null;
    generation: number;
    budget: number;
    range: SidebarDrawRange | null;
  }>({ viewKey, generation: 0, budget: SIDEBAR_RENDER_WINDOW, range: null });
  if (drawn.viewKey !== viewKey && viewKey !== null) {
    setDrawn({
      viewKey,
      generation: drawn.viewKey === null ? drawn.generation : drawn.generation + 1,
      budget: drawn.viewKey === null ? drawn.budget : sidebarRenderWindow(listHeight),
      range: null,
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

  // The chats a folder's preview draws past its first five: the current chat,
  // so it is never hidden behind Show more (vision V1-R7-2: the folder used to
  // open in full for it, and its Show less, which could not close it, did
  // nothing). Held like the order (principle 9): while the list is held, a chat
  // that stops being current keeps its line until the hold ends, so opening
  // another chat moves no row under the pointer.
  const [previewChats, setPreviewChats] = useState<readonly string[]>(() =>
    activeSessionId ? [activeSessionId] : []
  );
  const nextPreviewChats = keepInFolderPreview(previewChats, activeSessionId ?? null, holding);
  if (nextPreviewChats !== previewChats) setPreviewChats(nextPreviewChats);

  // What each group actually shows: a collapsed folder shows no rows (unless it
  // holds the current chat, which opens it without touching the stored choice),
  // and an open folder shows five rows (and the current chat, when it is further
  // down) before "Show more".
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
            firstRevealed: null as SessionSummary | null,
          };
        }
        const activeIndex = group.sessions.findIndex((session) => session.id === activeSessionId);
        const closedByPerson =
          closedWithActive.sessionId === activeSessionId &&
          closedWithActive.keys.includes(group.key);
        const collapsed = (collapsedFolders.has(group.key) && activeIndex < 0) || closedByPerson;
        if (collapsed) {
          return {
            group,
            collapsed,
            holdsActive: activeIndex >= 0,
            rows: [],
            more: null,
            firstRevealed: null,
          };
        }
        const preview =
          group.sessions.length > FOLDER_PREVIEW_COUNT
            ? group.sessions.filter(
                (session, index) =>
                  index < FOLDER_PREVIEW_COUNT || previewChats.includes(session.id)
              )
            : group.sessions;
        // Show more is drawn only when it has rows to show: a folder of six whose
        // sixth is the current chat has none.
        const foldable = preview.length < group.sessions.length;
        const expanded = expandedFolders.has(group.key);
        return {
          group,
          collapsed,
          holdsActive: activeIndex >= 0,
          rows: foldable && !expanded ? preview : group.sessions,
          more: !foldable ? null : expanded ? ('less' as const) : ('more' as const),
          // Where keyboard focus goes after Show more: the first chat it shows.
          firstRevealed: foldable
            ? (group.sessions.find(
                (session, index) =>
                  index >= FOLDER_PREVIEW_COUNT && !previewChats.includes(session.id)
              ) ?? null)
            : null,
        };
      }),
    [groups, collapsedFolders, expandedFolders, closedWithActive, activeSessionId, previewChats]
  );

  // A rename the person made: declared here because the row it names is always
  // drawn (below).
  const [reveal, setReveal] = useState<{ sessionId: string } | null>(null);
  // The row that last had focus in the list, kept drawn so focus is never
  // unmounted from under the person when the list scrolls away from it.
  const [focusedRowKey, setFocusedRowKey] = useState<string | null>(null);
  // Where focus goes after a folder's Show more or Show less pressed from the
  // keyboard (vision V1-R5-3): the first chat it showed, or the toggle itself.
  // Kept drawn until it has focus.
  const [focusAfterToggle, setFocusAfterToggle] = useState<string | null>(null);

  // R-28: the list reaches `renderBudget` lines down (the rest wait until it is
  // scrolled near its end), and of those it draws for real only the lines in
  // `range` plus the lines that must stay: the focused line (a row, a folder
  // header or a Show more), the current chat (the list's single Tab stop), a
  // row being renamed or just renamed, and a line whose right-click or ⋯ menu
  // is open. The whole list is still held and arranged. A current chat past
  // `renderBudget` has no row, and Tab lands on the first row drawn, as before.
  const extent = useMemo(
    () => windowSidebarRows(shown, renderBudget, headerLines),
    [shown, renderBudget]
  );
  const keptLines = useMemo(() => {
    const keys = new Set<string>();
    if (focusedRowKey) keys.add(focusedRowKey);
    if (focusAfterToggle) keys.add(focusAfterToggle);
    if (lineMenuKey) keys.add(lineMenuKey);
    if (activeSessionId) keys.add(chatKey(activeSessionId));
    if (editing) keys.add(chatKey(editing.sessionId));
    if (reveal) keys.add(chatKey(reveal.sessionId));
    if (rowMenuOpen && menuRowId) keys.add(chatKey(menuRowId));
    return keys;
  }, [
    focusedRowKey,
    focusAfterToggle,
    lineMenuKey,
    activeSessionId,
    editing,
    reveal,
    rowMenuOpen,
    menuRowId,
  ]);
  const drawRange = drawn.range;
  const windowed = useMemo(
    () => ({
      cut: extent.cut,
      entries: sliceSidebarLines(
        extent.entries,
        drawRange,
        headerLines,
        (session) => keptLines.has(chatKey(session.id)),
        ({ group }, line) => keptLines.has(line === 'header' ? folderKey(group) : moreKey(group))
      ),
    }),
    [extent, drawRange, keptLines]
  );

  // One tab stop for the list (spec 3.4, F-18): ↑/↓/Home/End move between rows.
  const rowKeys = useMemo(
    () =>
      windowed.entries.flatMap(({ group, rows, more, headerDrawn }) => [
        ...(group.kind === 'folder' && headerDrawn ? [folderKey(group)] : []),
        ...rows.map((session) => chatKey(session.id)),
        ...(more ? [moreKey(group)] : []),
      ]),
    [windowed]
  );
  const roving = useRovingRows(rowKeys, activeSessionId ? chatKey(activeSessionId) : null);
  const setRovingRow = roving.onRowFocus;
  const onRowFocus = useCallback(
    (key: string) => {
      setFocusedRowKey(key);
      setRovingRow(key);
    },
    [setRovingRow]
  );

  // Which lines the list shows, read from the drawn groups' boxes (a group's
  // rows and gaps sit on one pitch below its first row, the pitch read off a
  // drawn row, so no row or header height is assumed), and the range to draw
  // for them. From a scroll, the new
  // rows are committed before the frame paints (`flushSync`), so a scroll
  // never shows the empty placeholder where a row is about to be. A list that
  // cannot be measured (hidden, or no layout, as in jsdom) keeps what it drew.
  const syncDrawRange = useCallback((container: HTMLElement, beforePaint: boolean) => {
    if (container.clientHeight === 0 || container.getBoundingClientRect().height === 0) return;
    const sections = container.querySelectorAll<HTMLElement>('section[data-rows-line]');
    const origin = container.getBoundingClientRect().top - container.scrollTop;
    const boxes = new Map<number, SidebarGroupBox>();
    const boxAt = (index: number): SidebarGroupBox => {
      const known = boxes.get(index);
      if (known) return known;
      const section = sections[index];
      const box = section.getBoundingClientRect();
      const list = section.querySelector(':scope > ul');
      const measured = {
        rowsLine: Number(section.dataset.rowsLine),
        headerLines: Number(section.dataset.headerLines),
        top: box.top - origin,
        rowsTop: (list ? list.getBoundingClientRect().top : box.bottom) - origin,
      };
      boxes.set(index, measured);
      return measured;
    };
    const inView = sidebarLinesInView(
      sections.length,
      boxAt,
      container.scrollTop,
      container.scrollTop + container.clientHeight,
      measuredLinePitch(container)
    );
    const update = () =>
      setDrawn((current) => {
        const range = nextSidebarDrawRange(current.range, inView);
        return range === current.range ? current : { ...current, range };
      });
    if (beforePaint) flushSync(update);
    else update();
  }, []);

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
      // The draft starts from the STORED name, not the shown one: a scheduled
      // run's editor reads "Scheduled job: <id>" (V1-R3-6, display only).
      setEditing({ sessionId, draft: storedTitle(session) });
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

  // The right-click menu: ONE for the list, opened on the row the event came
  // from (R-28, vision V1-R2-2). A Radix menu root in every drawn row put a
  // document `keydown` listener in every row; see `LazyMenuButton`. The row is
  // read from the target's `data-row` when the event arrives, and a
  // right-click (or a long press) that is not on a chat row opens nothing, as
  // before: it is stopped before the menu sees it.
  const renameAfterRowMenu = useRef<string | null>(null);
  const pickMenuRow = (event: ReactMouseEvent | ReactPointerEvent) => {
    const row = (event.target as Element | null)?.closest?.('[data-row^="chat:"]');
    const id = row?.getAttribute('data-row')?.slice('chat:'.length) ?? null;
    if (!id) {
      event.preventDefault();
      return;
    }
    setMenuRowId(id);
  };
  const menuRow = menuRowId ? sessions.find((session) => session.id === menuRowId) : undefined;
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
      markScrolledOffTop(container);
      syncDrawRange(container, true);
      const remainingScroll = container.scrollHeight - container.scrollTop - container.clientHeight;
      if (remainingScroll > LOAD_MORE_THRESHOLD_PX) return;
      if (windowCut) setRenderBudget((budget) => budget + SIDEBAR_RENDER_WINDOW_STEP);
      else if (hasMore && !isLoadingMore && !isLoadingAll) onLoadMore();
    },
    [windowCut, hasMore, isLoadingMore, isLoadingAll, onLoadMore, setRenderBudget, syncDrawRange]
  );

  // The roving keys move focus to lines that may be folded into gaps (R-28).
  // Home and End scroll the list to that end and draw it first, as focusing
  // its row would have scrolled the list there anyway. ArrowUp and ArrowDown on
  // a line the list was scrolled away from (it stays drawn while it holds
  // focus) bring the list back to it first, so the key reaches the line next to
  // it rather than the first row drawn far down the list. A key typed in the
  // rename field is the field's.
  const onListKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const target = event.target as HTMLElement | null;
    const item = target?.closest<HTMLElement>('[data-row-item]');
    if (
      ['Home', 'End', 'ArrowUp', 'ArrowDown'].includes(event.key) &&
      !event.altKey &&
      !event.ctrlKey &&
      !event.metaKey &&
      !event.shiftKey &&
      drawn.range !== null &&
      item &&
      !target?.matches('input, textarea')
    ) {
      const container = event.currentTarget;
      if (event.key === 'Home' || event.key === 'End') {
        container.scrollTop = event.key === 'Home' ? 0 : container.scrollHeight;
        syncDrawRange(container, true);
      } else {
        const row = item.matches('[data-row]')
          ? item
          : (item.querySelector<HTMLElement>('[data-row]') ?? item);
        const at = row.getBoundingClientRect();
        const box = container.getBoundingClientRect();
        if (at.bottom <= box.top || at.top >= box.bottom) {
          scrollRowIntoView(container, row);
          syncDrawRange(container, true);
        }
      }
    }
    roving.onKeyDown(event);
  };
  // Focus that leaves the list (not into a menu one of its rows opened) lets go
  // of the row it was on, so that row is no longer kept drawn.
  const onListBlur = (event: ReactFocusEvent<HTMLDivElement>) => {
    roving.onBlur(event);
    const container = event.currentTarget;
    const next = event.relatedTarget;
    if (next instanceof Node && container.contains(next)) return;
    if (next instanceof Element && inMenuOpenedFrom(next, container)) return;
    setFocusedRowKey(null);
  };

  // A folder's Show more or Show less pressed from the keyboard (vision
  // V1-R5-3). The toggle moves when it acts: below every chat it showed, often
  // past the lines drawn so far (it unmounted, and focus fell to the page) or
  // far below the view; or back up under the folder's first five, above the
  // view. So focus goes where the person is looking: to the first chat Show
  // more showed, on the line the toggle held, and ArrowDown goes on from
  // there; after Show less it stays on the toggle, brought into view. Both are
  // drawn (`keptLines`) in the commit that moved them, so this runs before
  // that frame paints.
  useLayoutEffect(() => {
    if (!focusAfterToggle) return;
    setFocusAfterToggle(null);
    const container = scrollContainerRef.current;
    const target = container?.querySelector<HTMLElement>(
      `[data-row="${CSS.escape(focusAfterToggle)}"]`
    );
    if (!container || !target) return;
    if (document.activeElement !== target) target.focus({ preventScroll: true });
    scrollRowIntoView(container, target);
    syncDrawRange(container, false);
  }, [focusAfterToggle, syncDrawRange]);

  // Anything that moves lines without a scroll (a view drawn, a folder opened
  // or closed, rows arriving, the list's box growing) is measured again before
  // the frame paints, so no placeholder is ever left where a row should be.
  // The top fade is read again too: a list hidden and shown again (Chats
  // closed and opened) is back at its top with no scroll event to say so.
  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    markScrolledOffTop(container);
    syncDrawRange(container, false);
  }, [windowed, listHeight, isExpanded, syncDrawRange]);

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

  const rowMenu = menuRow ? (
    <ChatRowContextMenuContent
      target={{
        sessionId: menuRow.id,
        workingDir: menuRow.working_dir,
        openInNewTab: () =>
          openChat(menuRow.id, menuRow.name.trim() || undefined, menuRow.user_set_name),
      }}
      onRename={() => {
        renameAfterRowMenu.current = menuRow.id;
      }}
      onDiverge={canDivergeChat(menuRow) ? () => divergeRow(menuRow.id) : undefined}
      onExport={() => void exportConversation(menuRow.id, storedTitle(menuRow), { scope: 'app' })}
      onDelete={() => setPendingDelete(menuRow)}
      onCloseAutoFocus={(event: Event) => {
        const renameId = renameAfterRowMenu.current;
        if (!renameId) {
          // A choice made with the pointer leaves focus alone: handing it
          // back to the row would ring a button the person clicked with
          // a mouse and pin the row's hover-revealed ⋯ open.
          keepFocusOffAfterPointerClose(event);
          return;
        }
        // Rename was chosen: start editing once the menu has let go of
        // focus, or it would hand focus back to the row over the input.
        renameAfterRowMenu.current = null;
        event.preventDefault();
        startRenameRow(renameId);
      }}
    />
  ) : null;

  const rowProps = (session: SessionSummary) => ({
    session,
    isActive: activeSessionId === session.id,
    isRunning: runningSessionIds.has(session.id),
    homeDir,
    listShown: isExpanded && sidebarOpen,
    tabIndex: roving.tabIndexFor(chatKey(session.id)),
    onRowFocus,
    onLineMenuChange,
    onOpen: openChat,
    onStartRename: startRenameRow,
    onRequestDelete: setPendingDelete,
    onDiverge: divergeRow,
    contextMenuOpen: rowMenuOpen && menuRowId === session.id,
    editingDraft: editing?.sessionId === session.id ? editing.draft : null,
    onDraftChange: changeDraft,
    onCommitRename: commitRenameRow,
    onCancelRename: finishRename,
  });

  // The title the row showed, so the sentence names what the person clicked (V1-R3-6).
  const pendingTitle = pendingDelete ? shownTitle(pendingDelete) : '';
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
        data-row-menu-open={listMenuOpen ? 'true' : undefined}
        onScroll={handleScroll}
        onPointerEnter={() => setPointerInside(true)}
        onPointerLeave={() => setPointerInside(false)}
        onKeyDown={onListKeyDown}
        onFocus={roving.onFocus}
        onBlur={onListBlur}
      >
        {isEmpty ? (
          <p
            className="br-sidebar-foot"
            role={isLoadingMore || isLoadingAll ? 'status' : undefined}
          >
            {isLoadingMore || isLoadingAll ? sidebarCopy.chats.loading : sidebarCopy.chats.empty}
          </p>
        ) : (
          // Not modal (R-28, vision V1-R2-2): a modal menu sets `pointer-events`
          // on <body>, an inherited property, so opening and closing it restyled
          // every element in the window (about 50 ms each way with 2,100 rows
          // drawn, more with a long chat open), and it mounted a scroll lock that
          // read the body's computed style. The row's ⋯ menu, the same menu,
          // was never modal: both now behave alike. The LIST is modal while
          // either is open (`listMenuOpen`, vision V1-R5-2): it holds still and
          // a press on it only closes the menu; a click outside the list closes
          // the menu and lands where it was aimed.
          <ContextMenu modal={false} onOpenChange={setRowMenuOpen}>
            <ContextMenuTrigger
              asChild
              onContextMenu={pickMenuRow}
              // A long press opens the menu on touch and pen; a mouse press is
              // not one, and picks nothing.
              onPointerDown={(event) => {
                if (event.pointerType !== 'mouse') pickMenuRow(event);
              }}
            >
              <div
                key={viewGeneration}
                className="br-sidebar-chat-groups"
                data-view-changed={viewGeneration > 0 ? 'true' : undefined}
              >
                {windowed.entries.map(
                  ({
                    group,
                    collapsed,
                    holdsActive,
                    lines,
                    more,
                    firstRevealed,
                    headerDrawn,
                    rowsLine,
                  }) => (
                    <section
                      key={group.key}
                      className="br-sidebar-group"
                      aria-label={group.label || sidebarCopy.chats.header}
                      // Where this group's rows start, for the line math (`syncDrawRange`).
                      data-rows-line={rowsLine}
                      data-header-lines={headerLines({ group })}
                    >
                      {group.kind === 'date' ? (
                        <p className="br-sidebar-bucket">{group.label}</p>
                      ) : null}
                      {group.kind === 'folder' && !headerDrawn ? (
                        // A folder header far from the viewport: its box alone, so
                        // the lines below it keep their place.
                        <div className="br-sidebar-folder" aria-hidden="true" />
                      ) : null}
                      {group.kind === 'folder' && headerDrawn ? (
                        <FolderHeader
                          group={group}
                          collapsed={collapsed}
                          tabIndex={roving.tabIndexFor(folderKey(group))}
                          onRowFocus={onRowFocus}
                          onLineMenuChange={onLineMenuChange}
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
                      {lines.length > 0 ? (
                        <ul className="br-nav-list">
                          {lines.map((line) => {
                            if (line.kind === 'row') {
                              return <RecentChatRow key={line.row.id} {...rowProps(line.row)} />;
                            }
                            if (line.kind === 'gap') {
                              // Rows held but far from the viewport: one box their height.
                              return (
                                <li
                                  key={line.key}
                                  className="br-chat-row-gap"
                                  aria-hidden="true"
                                  style={{ '--br-gap-lines': line.lines } as CSSProperties}
                                />
                              );
                            }
                            return (
                              <li key="more" data-row-item="">
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
                                  onFocus={() => onRowFocus(moreKey(group))}
                                  onClick={(event) => {
                                    // Enter or Space: a click no pointer counted. Focus
                                    // follows what the toggle did (above); a pointer's click
                                    // moves no focus, or a row would show its ⋯.
                                    if (event.detail === 0) {
                                      setFocusAfterToggle(
                                        more === 'more' && firstRevealed
                                          ? chatKey(firstRevealed.id)
                                          : moreKey(group)
                                      );
                                    }
                                    setExpandedFolders((current) => {
                                      const next = new Set(current);
                                      if (next.has(group.key)) next.delete(group.key);
                                      else next.add(group.key);
                                      return next;
                                    });
                                  }}
                                >
                                  <span className="br-nav-row-label">
                                    {more === 'more'
                                      ? sidebarCopy.chats.showMore
                                      : sidebarCopy.chats.showLess}
                                  </span>
                                </button>
                              </li>
                            );
                          })}
                        </ul>
                      ) : null}
                    </section>
                  )
                )}
              </div>
            </ContextMenuTrigger>
            {rowMenu}
          </ContextMenu>
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
