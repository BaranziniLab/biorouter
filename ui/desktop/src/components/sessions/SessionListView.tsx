import { deleteConversation } from '../../utils/deleteConversation';
import React, { useEffect, useState, useRef, useCallback, useMemo, startTransition } from 'react';
import {
  MessageSquareText,
  Target,
  AlertCircle,
  Pencil,
  Upload,
  Puzzle,
  GitBranch,
  MoreHorizontal,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { useNavigate } from 'react-router-dom';
import { toastError, toastSuccess } from '../../toasts';
import { ContextMenu, ContextMenuTrigger } from '../ui/context-menu';
import { DropdownMenu, DropdownMenuTrigger } from '../ui/dropdown-menu';
import { IconAction } from '../ui/row-actions';
import { Button } from '../ui/button';
import { Input } from '../ui/input';
import { ScrollArea } from '../ui/scroll-area';
import { Spinner } from '../ui/spinner';
import { FilterInput } from '../ui/filter-input';
import { uiCopy } from '../ui/copy';
import { isContextMenuKey, openContextMenuFromKeyboard } from '../ui/keyboardContextMenu';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { groupSessionsByDate, type DateGroup } from '../../utils/dateUtils';
import { groupSessionsByParent, withoutSubagents } from './sessionGrouping';
import {
  ChatRowContextMenuContent,
  ChatRowDropdownMenuContent,
  type ChatRowMenuHandlers,
} from '../chats/ChatRowContextMenu';
import { chatRowCopy } from '../chats/copy';
import { Skeleton } from '../ui/skeleton';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { ImportSessionModal } from './ImportSessionModal';
import { Tooltip, TooltipContent, TooltipTrigger } from '../ui/Tooltip';
import { importSession, Session, ExtensionConfig, ExtensionData } from '../../api';
import { formatExtensionName } from '../settings/extensions/subcomponents/ExtensionList';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ModalShell } from '../ModalShell';
import { Badge } from '../ui/badge';
import { EmptyState } from '../ui/empty-state';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { DeclassifySessionDialog } from './DeclassifySessionDialog';
import { DECLASSIFY_NEEDS_HOST_REASON, declassifyBrowserReason } from './declassifyOnBrowser';
import {
  getCachedSessionList,
  notifySessionListChanged,
  refreshSessionList,
  subscribeSessionList,
  updateCachedSessionList,
} from '../../utils/sessionListCache';
import { renameSessionOptimistically, SESSION_NAME_MAX_LENGTH } from '../../utils/sessionNameSync';
import { exportConversation } from '../../utils/exportConversation';
import { useDiverge } from '../../hooks/useDiverge';
import { folderName } from './historyRow';
import { applyHeldHistory, holdHistory, type HeldHistory } from './historyHold';
import {
  BILLED_TOKENS_EXACT,
  BILLED_TOKENS_LOWER_BOUND,
  BILLED_TOKENS_STAT,
  CANCEL,
  CHAT_DELETED,
  CHAT_IMPORTED,
  EXTENSIONS_STAT,
  HISTORY_EMPTY,
  HISTORY_EMPTY_TITLE,
  HISTORY_INFO,
  HISTORY_LOAD_ERROR,
  HISTORY_LOAD_ERROR_TITLE,
  HISTORY_NO_MATCH,
  HISTORY_NO_MATCH_TITLE,
  HISTORY_TITLE,
  IMPORT_CHAT,
  LOADING_HISTORY,
  LOADING_MORE_CHATS,
  MAKE_CHAT_PUBLIC,
  MESSAGES_STAT,
  MORE_ACTIONS,
  RENAME,
  RENAME_PLACEHOLDER,
  RENAME_TITLE,
  SAVE,
  SHOW_SUBAGENT_RUNS,
  START_A_CHAT,
  SUBAGENT_BADGE,
  TRY_AGAIN,
  branchedFrom,
  historySearchPlaceholder,
  moreActionsLabel,
  openChatLabel,
  renameLabel,
} from './copy';
import './history.css';

function getSessionExtensionNames(extensionData: ExtensionData): string[] {
  try {
    const enabledExtensionData = extensionData?.['enabled_extensions.v0'] as
      | { extensions?: ExtensionConfig[] }
      | undefined;
    if (!enabledExtensionData?.extensions) return [];

    return enabledExtensionData.extensions.map((ext) => formatExtensionName(ext.name));
  } catch {
    return [];
  }
}

/**
 * Rename a chat from History. Saving goes through `renameSessionOptimistically`,
 * the one rename path every surface shares: the new name shows everywhere at
 * once (the cache this list reads patches itself on the name channel), a
 * refusal puts the old name back with the daemon's own sentence in a toast, and
 * there is no success toast, because the row is the confirmation.
 */
function RenameChatDialog({ session, onClose }: { session: Session; onClose: () => void }) {
  const [name, setName] = useState(session.name);

  const save = () => {
    const previous = { name: session.name, userSetName: Boolean(session.user_set_name) };
    onClose();
    void renameSessionOptimistically(session.id, name, previous);
  };

  return (
    <ModalShell
      open
      onOpenChange={(open) => !open && onClose()}
      size="md"
      purpose="form"
      title={RENAME_TITLE}
      footer={
        <>
          <Button variant="secondary" onClick={onClose}>
            {CANCEL}
          </Button>
          <Button onClick={save} disabled={!name.trim()}>
            {SAVE}
          </Button>
        </>
      }
    >
      <Input
        aria-label={RENAME_PLACEHOLDER}
        placeholder={RENAME_PLACEHOLDER}
        value={name}
        maxLength={SESSION_NAME_MAX_LENGTH}
        autoFocus
        onFocus={(event) => event.currentTarget.select()}
        onChange={(event) => setName(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === 'Enter' && !event.nativeEvent.isComposing && name.trim()) save();
        }}
      />
    </ModalShell>
  );
}

// Debounce hook for the filter
function useDebounce<T>(value: T, delay: number): T {
  const [debouncedValue, setDebouncedValue] = useState<T>(value);

  useEffect(() => {
    const handler = setTimeout(() => {
      setDebouncedValue(value);
    }, delay);

    return () => {
      window.clearTimeout(handler);
    };
  }, [value, delay]);

  return debouncedValue;
}

interface SessionListViewProps {
  onSelectSession: (sessionId: string) => void;
}

const HISTORY_LOADING_GROUPS = [5, 4];
const HISTORY_LOADING_TITLE_WIDTHS = ['w-3/4', 'w-2/3', 'w-4/5', 'w-1/2'];
const INITIAL_VISIBLE_SESSIONS = 16;
const VISIBLE_SESSION_BATCH = 20;
/** Filtering is local and cheap; the delay only keeps a fast typist's list from flickering. */
const FILTER_DEBOUNCE_MS = 150;

function HistoryLoading() {
  let rowIndex = 0;

  return (
    <div role="status" aria-label={LOADING_HISTORY} className="flex flex-col">
      <span className="sr-only">{LOADING_HISTORY}</span>
      {HISTORY_LOADING_GROUPS.map((rowCount, groupIndex) => (
        <div key={groupIndex} aria-hidden="true">
          <div className="br-history-bucket">
            <Skeleton
              className="biorouter-history-loading-cell h-3 w-16 rounded-inner bg-background-medium"
              style={{ animationDelay: `${-groupIndex * 180}ms` }}
            />
          </div>
          <div className="session-grid biorouter-list-shell overflow-hidden">
            {Array.from({ length: rowCount }, (_, index) => {
              const currentRow = rowIndex++;
              const delay = -((currentRow * 95) % 1100);

              return (
                <div
                  key={index}
                  data-testid="history-loading-row"
                  className="biorouter-list-row br-history-row pointer-events-none"
                >
                  {/* The row's own shape: the 16px glyph slot, a title line and a
                      meta line at the real lines' heights, then the stats. The
                      skeleton replaces the rows without moving them. */}
                  <Skeleton
                    className="biorouter-history-loading-cell h-4 w-4 flex-none rounded-inner bg-background-medium"
                    style={{ animationDelay: `${delay}ms` }}
                  />
                  <div className="flex min-w-0 flex-1 flex-col gap-0.5">
                    <div className="flex h-5 items-center">
                      <Skeleton
                        className={`biorouter-history-loading-cell h-3.5 ${HISTORY_LOADING_TITLE_WIDTHS[currentRow % HISTORY_LOADING_TITLE_WIDTHS.length]} rounded-inner bg-background-medium`}
                        style={{ animationDelay: `${delay}ms` }}
                      />
                    </div>
                    <div className="flex h-4 items-center">
                      <Skeleton
                        className="biorouter-history-loading-cell h-3 w-32 rounded-inner bg-background-medium"
                        style={{ animationDelay: `${delay - 70}ms` }}
                      />
                    </div>
                  </div>
                  <Skeleton
                    className="biorouter-history-loading-cell h-3 w-14 rounded-inner bg-background-medium"
                    style={{ animationDelay: `${delay - 210}ms` }}
                  />
                </div>
              );
            })}
          </div>
        </div>
      ))}
    </div>
  );
}

/**
 * One cell of a History row's stats: a 14px glyph at the left, the figure at the
 * right, and for the screen reader "Messages: 6". The cell's width is the
 * column's (`history.css`), so this never sizes itself to its content.
 */
function HistoryStat({
  icon: Icon,
  label,
  value,
  tooltip,
  tooltipClassName,
}: {
  icon: React.ComponentType<{ className?: string; 'aria-hidden'?: boolean | 'true' }>;
  label: string;
  value: string | number;
  tooltip?: string;
  tooltipClassName?: string;
}) {
  const cell = (
    <span className="br-history-stat">
      <Icon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
      <span className="sr-only">{label}: </span>
      <span className="whitespace-nowrap">{value}</span>
    </span>
  );
  if (!tooltip) return cell;
  return (
    <Tooltip>
      <TooltipTrigger asChild>{cell}</TooltipTrigger>
      <TooltipContent side="top" className={tooltipClassName}>
        {tooltip}
      </TooltipContent>
    </Tooltip>
  );
}

interface HistoryRowProps {
  session: Session;
  onSelectSession: (sessionId: string) => void;
  /** id → name, so a diverged row can name its lineage parent. */
  sessionNameById: Map<string, string>;
  onRename: (session: Session) => void;
  onDelete: (session: Session) => void;
  onExport: (session: Session) => void;
  onDiverge: (session: Session) => void;
  onDeclassify: (session: Session) => void;
}

/**
 * One History row: the kind glyph and title, one muted line under it, the stats
 * as fixed columns, then Rename and `⋯` revealed on hover or focus.
 *
 * ⚠ **Module scope, and it has to stay there.** Declared inside the list's body
 * it was a new component type on every render, so every refetch, cross-fade or
 * keystroke unmounted every row: its DOM node, its focus and any menu it had
 * open. Everything it needs is a prop, so the type is stable for the module's
 * life.
 */
const HistoryRow = React.memo(function HistoryRow({
  session,
  onSelectSession,
  sessionNameById,
  onRename,
  onDelete,
  onExport,
  onDiverge,
  onDeclassify,
}: HistoryRowProps) {
  /**
   * Is this page served to a browser, where declassification cannot work (SD-8)?
   * Read from the DOM marker `renderer.tsx` stamps, not held as state: the
   * surface cannot change while the renderer runs, and a state hook would add a
   * render in which the item is still offered.
   */
  const declassifyOnHost = declassifyBrowserReason();
  const [menuOpen, setMenuOpen] = useState(false);

  const open = useCallback(() => onSelectSession(session.id), [onSelectSession, session.id]);

  /**
   * The row's menu: the shared items (Rename · open in a tab or window · Diverge ·
   * Export… · Copy chat ID · Delete chat…), drawn by the same two components the
   * sidebar and the tab strip draw (`chats/ChatRowContextMenu`), plus "Make this
   * chat public" on a private row only (issue #56 §12.1: History's row and the
   * session page are the only two places it is offered). "Open in new tab" is the
   * row's own click, so the two can never disagree about what it does. A subagent
   * run is machinery, not a chat to branch, and an empty chat has nothing to
   * branch from, so neither offers Diverge.
   *
   * On a browser-served page that item can never work (SD-8), so it carries its
   * refusal as a `note` the menu draws above it: visible, never hover-only, and
   * not a `title=` (the global tooltip enhancer would make the sentence the
   * item's accessible NAME).
   */
  const target = useMemo(
    () => ({ sessionId: session.id, workingDir: session.working_dir, openInNewTab: open }),
    [session.id, session.working_dir, open]
  );
  const handlers = useMemo<ChatRowMenuHandlers>(() => {
    const canDiverge = session.session_type !== 'sub_agent' && session.message_count > 0;
    return {
      onRename: () => onRename(session),
      onDiverge: canDiverge ? () => onDiverge(session) : undefined,
      onExport: () => onExport(session),
      onDelete: () => onDelete(session),
      extraItems:
        session.privacy_tier === 'private'
          ? [
              {
                key: 'declassify',
                label: MAKE_CHAT_PUBLIC,
                disabled: declassifyOnHost !== null,
                note: declassifyOnHost !== null ? DECLASSIFY_NEEDS_HOST_REASON : undefined,
                noteTestId: 'declassify-browser-note',
                onSelect: () => {
                  if (declassifyOnHost === null) onDeclassify(session);
                },
              },
            ]
          : [],
    };
  }, [session, onRename, onDiverge, onExport, onDelete, onDeclassify, declassifyOnHost]);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    const target = event.target;
    // A key pressed inside an open menu (portalled, but still a React child) is the menu's.
    if (target instanceof Element && target.closest('[data-radix-menu-content]')) return;
    if (isContextMenuKey(event)) {
      // macOS sends no `contextmenu` for Shift+F10 or the Menu key, so the row
      // dispatches the one a right-click would (ui/keyboardContextMenu.ts).
      event.preventDefault();
      openContextMenuFromKeyboard(
        event.currentTarget,
        target instanceof Element ? target : undefined
      );
      return;
    }
    if (event.key === 'F2' && !event.altKey && !event.ctrlKey && !event.metaKey) {
      event.preventDefault();
      onRename(session);
    }
  };

  const extensionNames = useMemo(
    () => getSessionExtensionNames(session.extension_data),
    [session.extension_data]
  );
  const billedTokenEstimate = billedSessionTokenEstimate(session);
  const folder = folderName(session.working_dir);
  const parentName = session.diverged_from
    ? (sessionNameById.get(session.diverged_from) ?? session.diverged_from)
    : null;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          className="biorouter-list-row br-history-row session-item"
          data-session-id={session.id}
          onKeyDown={handleKeyDown}
        >
          {/* The glyph says what the chat is and whether it is private; the
              same resolver every chat row in the app draws from. */}
          <ChatKindIcon session={session} tier={session.privacy_tier} className="h-4 w-4" />

          <button
            type="button"
            onClick={open}
            className="br-history-row-open"
            aria-label={openChatLabel(session.name)}
          >
            <span className="flex min-w-0 items-center gap-2">
              <span className="text-label truncate">{session.name}</span>
              {session.session_type === 'sub_agent' && (
                <Badge data-testid="subagent-badge">{SUBAGENT_BADGE}</Badge>
              )}
            </span>
            {/* One muted line: when, then where (by name; the path is in the
                tooltip and in the filter, never on the row). A diverged chat
                says what it branched from instead of where it ran. */}
            <span className="br-history-row-meta text-supporting text-text-muted">
              <span className="shrink-0 whitespace-nowrap tabular-nums">
                {formatMessageTimestamp(Date.parse(session.updated_at) / 1000)}
              </span>
              {parentName ? (
                <span className="flex min-w-0 items-center gap-1">
                  <span aria-hidden="true">·</span>
                  <GitBranch className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                  <span className="truncate">{branchedFrom(parentName)}</span>
                </span>
              ) : folder ? (
                <span className="flex min-w-0 items-center gap-1">
                  <span aria-hidden="true">·</span>
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <span className="truncate" data-working-dir={session.working_dir}>
                        {folder}
                      </span>
                    </TooltipTrigger>
                    {/* The path is a machine string: mono, and only in a tooltip. */}
                    <TooltipContent side="bottom" align="start">
                      <span className="font-mono">{session.working_dir}</span>
                    </TooltipContent>
                  </Tooltip>
                </span>
              ) : null}
            </span>
          </button>

          {/* The stats are three fixed columns, the same in every row: sans and
              tabular, the glyph at each cell's left edge and the figure at its
              right. A row with no token figure or no extensions leaves its cell
              empty rather than letting the others slide, so the columns read
              straight down the list. */}
          <div className="br-history-row-stats text-supporting text-text-muted tabular-nums">
            <HistoryStat
              icon={MessageSquareText}
              label={MESSAGES_STAT}
              value={session.message_count}
            />
            {billedTokenEstimate ? (
              <HistoryStat
                icon={Target}
                label={BILLED_TOKENS_STAT}
                value={formatBilledTokenEstimate(billedTokenEstimate)}
                tooltip={
                  billedTokenEstimate.lowerBound ? BILLED_TOKENS_LOWER_BOUND : BILLED_TOKENS_EXACT
                }
              />
            ) : (
              <span aria-hidden="true" />
            )}
            {extensionNames.length > 0 ? (
              <HistoryStat
                icon={Puzzle}
                label={EXTENSIONS_STAT}
                value={extensionNames.length}
                tooltip={extensionNames.join(', ')}
                tooltipClassName="max-w-xs"
              />
            ) : (
              <span aria-hidden="true" />
            )}
          </div>

          {/* One action and `⋯`, the shared row-actions cluster: revealed on hover,
              on focus within the row and while the menu is open, and always shown
              on a touch screen. Only the opacity changes, so both stay in the tab
              order. Everything else, Delete included, is behind `⋯`. */}
          <div className="br-row-actions-slot" data-slot="row-actions-slot">
            <div
              className="br-row-actions"
              data-slot="row-actions"
              data-state={menuOpen ? 'open' : 'closed'}
            >
              <IconAction
                icon={Pencil}
                label={renameLabel(session.name)}
                tooltip={RENAME}
                onSelect={() => onRename(session)}
              />
              <DropdownMenu open={menuOpen} onOpenChange={setMenuOpen}>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <DropdownMenuTrigger asChild>
                      <Button
                        type="button"
                        variant="ghost"
                        shape="round"
                        aria-label={moreActionsLabel(session.name)}
                        data-row-action=""
                        onClick={(event) => event.stopPropagation()}
                      >
                        <MoreHorizontal aria-hidden="true" />
                      </Button>
                    </DropdownMenuTrigger>
                  </TooltipTrigger>
                  <TooltipContent side="top">{MORE_ACTIONS}</TooltipContent>
                </Tooltip>
                <ChatRowDropdownMenuContent target={target} {...handlers} />
              </DropdownMenu>
            </div>
          </div>
        </div>
      </ContextMenuTrigger>
      <ChatRowContextMenuContent target={target} {...handlers} />
    </ContextMenu>
  );
});

function matchesFilter(session: Session, needle: string): boolean {
  return (
    session.name.toLowerCase().includes(needle) ||
    session.id.toLowerCase().includes(needle) ||
    session.working_dir.toLowerCase().includes(needle)
  );
}

const SessionListView: React.FC<SessionListViewProps> = React.memo(({ onSelectSession }) => {
  const initialSessions = useRef(getCachedSessionList()).current;
  const navigate = useNavigate();
  const { diverge } = useDiverge();
  const [sessions, setSessions] = useState<Session[]>(initialSessions ?? []);
  const [isLoading, setIsLoading] = useState(initialSessions === null);
  const [showSkeleton, setShowSkeleton] = useState(initialSessions === null);
  const [showContent, setShowContent] = useState(initialSessions !== null);
  const [error, setError] = useState<string | null>(null);

  const [visibleSessionCount, setVisibleSessionCount] = useState(INITIAL_VISIBLE_SESSIONS);

  // BR-71: subagent transcripts are hidden by default — they are machinery,
  // not chats the person started. Turning this on refetches with
  // `include_subagents` and nests each run under the chat that spawned it.
  const [showSubagents, setShowSubagents] = useState(false);

  const [filter, setFilter] = useState('');
  const debouncedFilter = useDebounce(filter.trim().toLowerCase(), FILTER_DEBOUNCE_MS);

  // `showSubagents` is this pane's state, but the session cache behind it is
  // module-global: a second History pane, or Home, can publish subagent rows
  // into it at any time. The toggle decides what is FETCHED; this decides what
  // this pane SHOWS, so the two never disagree on screen.
  const visibleSessions = useMemo(
    () => (showSubagents ? sessions : withoutSubagents(sessions)),
    [sessions, showSubagents]
  );

  const filteredSessions = useMemo(
    () =>
      debouncedFilter
        ? visibleSessions.filter((session) => matchesFilter(session, debouncedFilter))
        : visibleSessions,
    [visibleSessions, debouncedFilter]
  );

  // Parent grouping runs BEFORE date bucketing. Buckets key on `updated_at`,
  // and a parent's advances every time it is resumed, so a subagent that ran
  // on an earlier day sits in another bucket; grouping within each bucket
  // would drop it back to top level, the orphaned row this feature removes.
  const parentGroups = useMemo(() => groupSessionsByParent(filteredSessions), [filteredSessions]);
  // Only top-level rows are dated and paginated; children ride with their
  // parent, so `visibleSessionCount` counts rendered parents, not raw rows.
  const liveTopLevelSessions = useMemo(() => parentGroups.map((g) => g.session), [parentGroups]);
  const liveChildrenByParentId = useMemo(() => {
    const map = new Map<string, Session[]>();
    for (const { session, children } of parentGroups) {
      if (children.length > 0) map.set(session.id, children);
    }
    return map;
  }, [parentGroups]);
  const liveDateGroups = useMemo<DateGroup[]>(
    () => groupSessionsByDate(liveTopLevelSessions),
    [liveTopLevelSessions]
  );

  // Principle 9: the list is live, and while the pointer is over it the rows
  // keep the order they had when it came in (`historyHold.ts`). A new filter or
  // the subagent toggle is the person's own change, not a live one, so it
  // starts a new hold rather than being withheld.
  const holdScope = `${debouncedFilter}\u0000${showSubagents}`;
  const [pointerInside, setPointerInside] = useState(false);
  const [held, setHeld] = useState<{ scope: string; order: HeldHistory } | null>(null);
  const captureHold = useCallback(
    () => setHeld({ scope: holdScope, order: holdHistory(liveDateGroups, liveChildrenByParentId) }),
    [holdScope, liveDateGroups, liveChildrenByParentId]
  );
  useEffect(() => {
    if (pointerInside && held && held.scope !== holdScope) captureHold();
  }, [pointerInside, held, holdScope, captureHold]);
  const arranged = useMemo(
    () =>
      pointerInside && held && held.scope === holdScope
        ? applyHeldHistory(held.order, liveDateGroups, liveChildrenByParentId)
        : { groups: liveDateGroups, children: liveChildrenByParentId },
    [pointerInside, held, holdScope, liveDateGroups, liveChildrenByParentId]
  );
  const dateGroups = arranged.groups;
  const childrenByParentId = arranged.children;
  const topLevelCount = useMemo(
    () => dateGroups.reduce((count, group) => count + group.sessions.length, 0),
    [dateGroups]
  );

  const [renameTarget, setRenameTarget] = useState<Session | null>(null);
  const [deleteTarget, setDeleteTarget] = useState<Session | null>(null);
  const [showImportModal, setShowImportModal] = useState(false);
  // Issue #56 §12.1: declassification is attached to the History ROW. The
  // dialog is mounted here rather than inside a row so a row re-render can
  // never tear it down mid-interaction.
  const [declassifyTarget, setDeclassifyTarget] = useState<Session | null>(null);

  const visibleDateGroups = useMemo(() => {
    let remainingSessions = visibleSessionCount;

    return dateGroups.flatMap((group) => {
      if (remainingSessions <= 0) return [];
      const groupSessions = group.sessions.slice(0, remainingSessions);
      remainingSessions -= groupSessions.length;
      return [{ ...group, sessions: groupSessions }];
    });
  }, [dateGroups, visibleSessionCount]);

  // id → name lookup so a diverged session can show its lineage parent's name.
  const sessionNameById = useMemo(() => {
    const map = new Map<string, string>();
    for (const s of sessions) map.set(s.id, s.name);
    return map;
  }, [sessions]);

  const handleScroll = useCallback(
    (target: HTMLDivElement) => {
      const { scrollTop, scrollHeight, clientHeight } = target;
      const threshold = 200;

      if (
        scrollHeight - scrollTop - clientHeight < threshold &&
        visibleSessionCount < topLevelCount
      ) {
        setVisibleSessionCount((previousCount) =>
          Math.min(previousCount + VISIBLE_SESSION_BATCH, topLevelCount)
        );
      }
    },
    [visibleSessionCount, topLevelCount]
  );

  // A filter shows every match at once; clearing it returns to the paged list.
  useEffect(() => {
    if (debouncedFilter) {
      setVisibleSessionCount(topLevelCount);
    } else {
      setVisibleSessionCount(INITIAL_VISIBLE_SESSIONS);
    }
  }, [debouncedFilter, topLevelCount]);

  const loadSessions = useCallback(async () => {
    const hasCachedSessions = getCachedSessionList() !== null;
    if (!hasCachedSessions) {
      setIsLoading(true);
      setShowSkeleton(true);
      setShowContent(false);
      setError(null);
    }
    try {
      const refreshedSessions = await refreshSessionList(showSubagents);
      startTransition(() => {
        setSessions(refreshedSessions);
        setError(null);
      });
    } catch (err) {
      console.error('Failed to load sessions:', err);
      if (!hasCachedSessions) {
        setError(HISTORY_LOAD_ERROR);
        setSessions([]);
      }
    } finally {
      if (!hasCachedSessions) setIsLoading(false);
    }
  }, [showSubagents]);

  useEffect(() => {
    loadSessions();
  }, [loadSessions]);

  // Stay live while open. The shared cache is mutated (and emits here) by a
  // rename on the name channel and by any create / diverge / delete on the
  // list channel, in this window or a sibling, so the list reflects a branch
  // or a rename without a remount.
  useEffect(() => {
    return subscribeSessionList(() => {
      const cached = getCachedSessionList();
      if (cached) startTransition(() => setSessions(cached));
    });
  }, []);

  // The skeleton-to-content reveal on a cold load. `showContent`, not
  // `showSkeleton`, is the "already revealed" guard, and that is load-bearing:
  // this effect WRITES `showSkeleton`, so keeping it in the deps would re-run
  // the effect on the next render and the cleanup would cancel the reveal it
  // had just armed.
  useEffect(() => {
    if (isLoading || showContent) return undefined;
    setShowSkeleton(false);
    const revealTimer = setTimeout(() => setShowContent(true), 10);
    // Unmounting inside that 10ms window must disarm it, or the callback
    // setStates an unmounted tree (on CI it fired after jsdom was gone).
    return () => clearTimeout(revealTimer);
  }, [isLoading, showContent]);

  const handleRename = useCallback((session: Session) => setRenameTarget(session), []);
  const handleDelete = useCallback((session: Session) => setDeleteTarget(session), []);
  const handleDeclassify = useCallback((session: Session) => setDeclassifyTarget(session), []);
  const handleDiverge = useCallback((session: Session) => void diverge(session.id), [diverge]);
  // About THIS screen's row, so a refusal toast goes when the person leaves
  // the screen (`scope: 'screen'`, T3-SH-11) rather than following them into the
  // chat they open next.
  const handleExport = useCallback(
    (session: Session) => void exportConversation(session.id, session.name, { scope: 'screen' }),
    []
  );

  const handleConfirmDelete = useCallback(async () => {
    if (!deleteTarget) return;
    const target = deleteTarget;
    setDeleteTarget(null);

    try {
      await deleteConversation(target.id);
      setSessions((current) => current.filter((session) => session.id !== target.id));
      toastSuccess({ title: CHAT_DELETED });
    } catch (err) {
      console.error('Error deleting session:', err);
      toastError({
        title: chatRowCopy.deleteDialog.failed,
        msg: err instanceof Error ? err.message : String(err),
      });
    }
    await loadSessions();
  }, [deleteTarget, loadSessions]);

  const handleImportSession = useCallback(
    async (json: string) => {
      await importSession({ body: { json }, throwOnError: true });
      toastSuccess({ title: CHAT_IMPORTED });
      // ⚠ Announce BEFORE the local reload, not instead of it and not after. An
      // import is a membership change, and the sidebar, Home and any History
      // pane in ANOTHER window learn of it here, over the list channel. First
      // also means `loadSessions` dedupes onto the request this starts.
      //
      // An imported chat carries its transcript, so unlike a freshly created
      // one it is listable the moment it lands (`GET /sessions` INNER JOINs
      // `messages`).
      notifySessionListChanged();
      await loadSessions();
    },
    [loadSessions]
  );

  // The row is stale the moment the daemon answers: its glyph and its menu key
  // on `privacy_tier`. Patch the shared cache as well as this pane's list, so
  // the sidebar and any second History pane stop marking it too.
  const handleDeclassified = useCallback((sessionId: string) => {
    const markPublic = (currentSessions: Session[]) =>
      currentSessions.map((session) =>
        session.id === sessionId
          ? {
              ...session,
              privacy_tier: 'public' as const,
              privacy_reason: 'declassified_by_user',
            }
          : session
      );
    updateCachedSessionList(markPublic);
    setSessions(markPublic);
    setDeclassifyTarget(null);
  }, []);

  const renderRow = (session: Session) => (
    <HistoryRow
      key={session.id}
      session={session}
      onSelectSession={onSelectSession}
      sessionNameById={sessionNameById}
      onRename={handleRename}
      onDelete={handleDelete}
      onExport={handleExport}
      onDiverge={handleDiverge}
      onDeclassify={handleDeclassify}
    />
  );

  const renderActualContent = () => {
    if (error) {
      return (
        <EmptyState
          icon={AlertCircle}
          title={HISTORY_LOAD_ERROR_TITLE}
          description={error}
          actions={
            <Button onClick={loadSessions} variant="secondary">
              {TRY_AGAIN}
            </Button>
          }
        />
      );
    }

    if (sessions.length === 0) {
      return (
        <EmptyState
          icon={ENTITY_ICONS.chat}
          title={HISTORY_EMPTY_TITLE}
          description={HISTORY_EMPTY}
          actions={
            <Button variant="link" onClick={() => navigate('/pair')}>
              {START_A_CHAT}
            </Button>
          }
        />
      );
    }

    if (dateGroups.length === 0 && debouncedFilter) {
      return (
        <EmptyState
          icon={ENTITY_ICONS.chat}
          title={HISTORY_NO_MATCH_TITLE}
          description={HISTORY_NO_MATCH}
          compact
        />
      );
    }

    return (
      <div className="flex flex-col">
        {visibleDateGroups.map((group) => (
          <section key={group.key} aria-label={group.label}>
            {/* Bucket labels are sentence case at 12px, the sidebar's, not caps:
                caps are for page section labels only. */}
            <h2 className="br-history-bucket text-supporting text-text-subtle">{group.label}</h2>
            <div className="session-grid biorouter-list-shell">
              {group.sessions.map((session) => {
                const children = childrenByParentId.get(session.id);
                return (
                  <React.Fragment key={session.id}>
                    {renderRow(session)}
                    {/* One indented block for all of a parent's children, not
                        one wrapper each: a lone row inside its own wrapper is
                        `:last-child` and loses its separator. */}
                    {children && (
                      <div className="ml-6 flex flex-col border-l border-border-subtle pl-2">
                        {children.map(renderRow)}
                      </div>
                    )}
                  </React.Fragment>
                );
              })}
            </div>
          </section>
        ))}

        {visibleSessionCount < topLevelCount && (
          <div className="flex items-center justify-center gap-2 py-6 text-supporting text-text-muted">
            <Spinner size={14} />
            <span>{LOADING_MORE_CHATS}</span>
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      <MainPanelLayout removeTopPadding>
        <div className="flex-1 flex flex-col min-h-0">
          {/* The 44px band: the title with its help in an InfoTip, then the
              filter and two icon actions. The filter's placeholder carries the
              shortcut, so no sentence under the title has to. */}
          <PageHeader
            title={HISTORY_TITLE}
            info={HISTORY_INFO}
            actions={
              <>
                <FilterInput
                  value={filter}
                  onValueChange={setFilter}
                  label={uiCopy.searchHistory}
                  placeholder={historySearchPlaceholder(uiCopy.findShortcut())}
                  className="br-history-filter"
                />
                {/* A view filter, not an action: a toggle that says whether it
                    is on (`aria-pressed`). Named by what it does, whatever its
                    state. */}
                <PageHeaderAction
                  icon={ENTITY_ICONS.agent}
                  label={SHOW_SUBAGENT_RUNS}
                  aria-pressed={showSubagents}
                  className="br-history-toggle"
                  onClick={() => setShowSubagents((value) => !value)}
                />
                <PageHeaderAction
                  icon={Upload}
                  label={IMPORT_CHAT}
                  onClick={() => setShowImportModal(true)}
                />
              </>
            }
          />

          {/* Chat history reads the CHAT measure, like Settings (operator
              decision, 2026-09-07): a row is a title on the left and stats on
              the right, so a wider column only puts air between them, and a
              row opens the live chat, which sits on this measure too.
              `measures.test.ts` asserts no `<ReadableContent` here is left on the
              default size. The scroll area is the pane's, so its bar sits at
              the window edge rather than at the column's. */}
          <ScrollArea handleScroll={handleScroll} className="flex-1 min-h-0">
            <ReadableContent size="chat" className="relative px-6 pt-2 pb-6">
              {/* Loading layer, shaped like the rows that replace it. */}
              <div
                data-history-layer="loading"
                className={`br-history-layer absolute inset-x-6 top-2 ${isLoading || showSkeleton ? 'opacity-100 z-10' : 'opacity-0 z-0 pointer-events-none'}`}
              >
                {(isLoading || showSkeleton) && <HistoryLoading />}
              </div>

              {/* Content layer: always rendered, revealed once. The pointer's
                  presence holds the order (principle 9, `historyHold.ts`); a
                  menu or tooltip portalled out of a row still counts as inside,
                  because React's enter and leave follow the component tree. */}
              <div
                data-history-layer="content"
                className={`br-history-layer relative ${showContent ? 'opacity-100 z-10' : 'opacity-0 z-0'}`}
                onPointerEnter={() => {
                  captureHold();
                  setPointerInside(true);
                }}
                onPointerLeave={() => {
                  setPointerInside(false);
                  setHeld(null);
                }}
              >
                {renderActualContent()}
              </div>
            </ReadableContent>
          </ScrollArea>
        </div>
      </MainPanelLayout>

      {/* Keyed by chat, so the field starts from that chat's name every time. */}
      {renameTarget && (
        <RenameChatDialog
          key={renameTarget.id}
          session={renameTarget}
          onClose={() => setRenameTarget(null)}
        />
      )}

      <ImportSessionModal
        isOpen={showImportModal}
        onClose={() => setShowImportModal(false)}
        onImport={handleImportSession}
      />

      <ConfirmationModal
        isOpen={deleteTarget !== null}
        title={chatRowCopy.deleteDialog.title}
        message={chatRowCopy.deleteDialog.message(deleteTarget?.name ?? '')}
        confirmLabel={chatRowCopy.deleteDialog.confirm}
        cancelLabel={chatRowCopy.deleteDialog.cancel}
        confirmVariant="destructive"
        onConfirm={handleConfirmDelete}
        onCancel={() => setDeleteTarget(null)}
      />

      {/* Mounted only while a row is targeted, so a closed dialog holds no
          session and cannot arrive pre-satisfied by a previous row's phrase. */}
      {declassifyTarget && (
        <DeclassifySessionDialog
          session={declassifyTarget}
          onClose={() => setDeclassifyTarget(null)}
          onDeclassified={handleDeclassified}
        />
      )}
    </>
  );
});

SessionListView.displayName = 'SessionListView';

export default SessionListView;
