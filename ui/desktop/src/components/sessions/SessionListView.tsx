import React, { useEffect, useState, useRef, useCallback, useMemo, startTransition } from 'react';
import {
  MessageSquareText,
  Target,
  AlertCircle,
  Calendar,
  Folder,
  Edit2,
  Trash2,
  Download,
  Upload,
  Puzzle,
  GitBranch,
  MoreHorizontal,
  LoaderCircle,
} from '../icons/app-icons';
import { useNavigate } from 'react-router-dom';
import { toastError, toastSuccess } from '../../toasts';
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from '../ui/dropdown-menu';
import { Button } from '../ui/button';
import { Checkbox } from '../ui/Checkbox';
import { ScrollArea } from '../ui/scroll-area';
import { formatMessageTimestamp } from '../../utils/timeUtils';
import { billedSessionTokenEstimate, formatBilledTokenEstimate } from '../../utils/billedTokens';
import { SearchView } from '../conversation/SearchView';
import { SearchHighlighter } from '../../utils/searchHighlighter';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { groupSessionsByDate, type DateGroup } from '../../utils/dateUtils';
import { groupSessionsByParent, withoutSubagents } from './sessionGrouping';
import { ChatRowContextMenu } from '../chats/ChatRowContextMenu';
import { chatRowActions } from '../chats/chatRowActions';
import { Skeleton } from '../ui/skeleton';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { ImportSessionModal } from './ImportSessionModal';
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from '../ui/Tooltip';
import {
  deleteSession,
  exportSession,
  importSession,
  Session,
  ExtensionConfig,
  ExtensionData,
} from '../../api';
import { formatExtensionName } from '../settings/extensions/subcomponents/ExtensionList';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { ReadableContent } from '../Layout/ReadableContent';
import { PageHeader } from '../Layout/PageHeader';
import { Dialog, DialogContent, DialogFooter, DialogTitle } from '../ui/dialog';
import { MODAL_SIZE } from '../ModalShell';
import { Badge } from '../ui/badge';
import { EmptyState } from '../ui/empty-state';
import { ChatKindIcon } from '../chats/ChatKindIcon';
import { DeclassifySessionDialog } from './DeclassifySessionDialog';
import {
  getCachedSessionList,
  refreshSessionList,
  subscribeSessionList,
  updateCachedSessionList,
} from '../../utils/sessionListCache';
import { renameSession } from '../../utils/sessionNameSync';

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

interface EditSessionModalProps {
  session: Session | null;
  isOpen: boolean;
  onClose: () => void;
  onSave: (sessionId: string, newDescription: string) => Promise<void>;
  disabled?: boolean;
}

const EditSessionModal = React.memo<EditSessionModalProps>(
  ({ session, isOpen, onClose, onSave, disabled = false }) => {
    const [description, setDescription] = useState('');
    const [isUpdating, setIsUpdating] = useState(false);

    useEffect(() => {
      if (session && isOpen) {
        setDescription(session.name);
      } else if (!isOpen) {
        setDescription('');
        setIsUpdating(false);
      }
    }, [session, isOpen]);

    const handleSave = useCallback(async () => {
      if (!session || disabled || isUpdating) return;

      const trimmedDescription = description.trim();
      if (trimmedDescription === session.name) {
        onClose();
        return;
      }

      setIsUpdating(true);
      try {
        // Through the shared hub, not a raw PUT: renameSession persists AND
        // broadcasts on the name channel, so a rename made here in the history
        // panel reaches the chat tab, the title pill and the sidebar Recents
        // too. A raw updateSessionName updated only this view's local cache, so
        // the same session showed its new name here and its old name everywhere
        // else until those surfaces remounted.
        await renameSession(session.id, trimmedDescription, 'user');
        await onSave(session.id, trimmedDescription);
        onClose();
        toastSuccess({
          title: 'Chat updated',
          msg: 'Description saved.',
        });
      } catch (error) {
        const errorMessage = error instanceof Error ? error.message : 'Unknown error occurred';
        console.error('Failed to update session description:', errorMessage);
        toastError({
          title: 'Failed to update chat',
          msg: errorMessage,
        });
        setDescription(session.name);
      } finally {
        setIsUpdating(false);
      }
    }, [session, description, onSave, onClose, disabled, isUpdating]);

    const handleCancel = useCallback(() => {
      if (!isUpdating) {
        onClose();
      }
    }, [onClose, isUpdating]);

    const handleKeyDown = useCallback(
      (e: React.KeyboardEvent<HTMLInputElement>) => {
        if (e.key === 'Enter' && !isUpdating) {
          handleSave();
        }
      },
      [handleSave, isUpdating]
    );

    const handleInputChange = useCallback((e: React.ChangeEvent<HTMLInputElement>) => {
      setDescription(e.target.value);
    }, []);

    if (!isOpen || !session) return null;

    return (
      <Dialog
        open={isOpen}
        onOpenChange={(open) => !open && !isUpdating && !disabled && handleCancel()}
      >
        {/* V8 — `MODAL_SIZE.md`, the ladder's form rung, not the `w-[500px]
            max-w-[90vw] sm:max-w-[500px]` triple this carried. 500 is not a
            rung, and the `w-[500px]` half additionally forced the width rather
            than capping it, so the dialog could not narrow with the viewport
            the way every other one in the app does. */}
        <DialogContent
          aria-describedby={undefined}
          dismissible={!isUpdating && !disabled}
          className={MODAL_SIZE.md}
        >
          <DialogTitle>Edit chat description</DialogTitle>

          <div className="space-y-4">
            <div>
              <input
                id="session-description"
                type="text"
                value={description}
                onChange={handleInputChange}
                className="biorouter-modal-panel w-full p-3 rounded-element text-body text-text-default"
                placeholder="Enter chat description"
                autoFocus
                maxLength={200}
                onKeyDown={handleKeyDown}
                disabled={isUpdating || disabled}
              />
            </div>
          </div>

          {/* V8 again: the shared footer, not a hand-rolled `flex justify-end
              space-x-3`. V7's dialog row is an `outline` dismiss beside the
              `default` confirm — `ghost` reads as the quieter of two actions
              that are not ranked that way here. */}
          <DialogFooter>
            <Button onClick={handleCancel} variant="outline" disabled={isUpdating || disabled}>
              Cancel
            </Button>
            <Button
              onClick={handleSave}
              disabled={!description.trim() || isUpdating || disabled}
              variant="default"
            >
              {isUpdating ? 'Saving...' : 'Save'}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    );
  }
);

EditSessionModal.displayName = 'EditSessionModal';

// Debounce hook for search
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

interface SearchContainerElement extends HTMLDivElement {
  _searchHighlighter: SearchHighlighter | null;
}

interface SessionListViewProps {
  onSelectSession: (sessionId: string) => void;
}

const HISTORY_LOADING_GROUPS = [5, 4];
const HISTORY_LOADING_TITLE_WIDTHS = ['w-3/4', 'w-2/3', 'w-4/5', 'w-1/2'];
const INITIAL_VISIBLE_SESSIONS = 16;
const VISIBLE_SESSION_BATCH = 20;

function HistoryLoading() {
  let rowIndex = 0;

  return (
    <div role="status" aria-label="Loading chat history" className="space-y-8">
      <span className="sr-only">Loading chat history</span>
      {HISTORY_LOADING_GROUPS.map((rowCount, groupIndex) => (
        <div key={groupIndex} className="space-y-4" aria-hidden="true">
          <Skeleton
            className="biorouter-history-loading-cell h-3 w-16 rounded-inner bg-background-medium"
            style={{ animationDelay: `${-groupIndex * 180}ms` }}
          />
          <div className="session-grid biorouter-list-shell overflow-hidden">
            {Array.from({ length: rowCount }, (_, index) => {
              const currentRow = rowIndex++;
              const delay = -((currentRow * 95) % 1100);

              return (
                <div
                  key={index}
                  data-testid="history-loading-row"
                  className="flex min-h-row items-center gap-3 border-b border-border-subtle px-4 py-2 last:border-b-0"
                >
                  <div className="min-w-0 flex-1">
                    <Skeleton
                      className={`biorouter-history-loading-cell mb-1.5 h-4 ${HISTORY_LOADING_TITLE_WIDTHS[currentRow % HISTORY_LOADING_TITLE_WIDTHS.length]} rounded-inner bg-background-medium`}
                      style={{ animationDelay: `${delay}ms` }}
                    />
                    <div className="flex items-center gap-3">
                      <Skeleton
                        className="biorouter-history-loading-cell h-3 w-20 rounded-inner bg-background-medium"
                        style={{ animationDelay: `${delay - 70}ms` }}
                      />
                      <Skeleton
                        className="biorouter-history-loading-cell h-3 w-32 rounded-inner bg-background-medium"
                        style={{ animationDelay: `${delay - 140}ms` }}
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
 * One history row.
 *
 * ⚠ **Module scope, and it has to stay there.** This lived inside
 * `SessionListView`'s body, which made it a NEW component type on every render
 * of the list. React compares element types by identity, so every state change
 * in the parent — a refetch, the skeleton→content cross-fade, a search
 * keystroke — unmounted every row and mounted a replacement, discarding each
 * row's DOM node, its focus, its hover state and any Radix menu it had open.
 * That is why an overflow menu could close by itself mid-interaction, and why
 * tests had to retry a click that landed on a node the list had already
 * detached. `React.memo` could not help: memo compares props, and a fresh type
 * never reaches that comparison.
 *
 * Everything it used to close over — `onSelectSession`, the diverged-from name
 * lookup — is a prop now, so the type is stable for the life of the module.
 */
const SessionItem = React.memo(function SessionItem({
  session,
  onSelectSession,
  sessionNameById,
  onEditClick,
  onDeleteClick,
  onExportClick,
  onDeclassifyClick,
}: {
  session: Session;
  onSelectSession: (sessionId: string) => void;
  /** id → name, so a diverged row can name its lineage parent. */
  sessionNameById: Map<string, string>;
  onEditClick: (session: Session) => void;
  onDeleteClick: (session: Session) => void;
  onExportClick: (session: Session, e: React.MouseEvent) => void;
  onDeclassifyClick: (session: Session) => void;
}) {
  const handleEditClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation(); // Prevent card click
      onEditClick(session);
    },
    [onEditClick, session]
  );

  const handleDeleteClick = useCallback(
    (e: React.MouseEvent) => {
      e.stopPropagation(); // Prevent card click
      onDeleteClick(session);
    },
    [onDeleteClick, session]
  );

  const handleCardClick = useCallback(() => {
    onSelectSession(session.id);
  }, [onSelectSession, session.id]);

  // The menu's "Open in new tab" is the same call the row's own click makes.
  // It exists because that behaviour was undiscoverable, not because it was
  // missing — so it must stay the SAME path, not a second one that can drift.
  const handleOpenInNewTabClick = useCallback(() => {
    onSelectSession(session.id);
  }, [onSelectSession, session.id]);

  /**
   * The three actions this row offers, from the one shared list (#114).
   *
   * Both of this row's menus render THIS array: the right-click menu below and
   * the `⋯` overflow, which already had the first two items. Mapping the same
   * descriptors into both is what keeps the keyboard path (Tab to `⋯`, Enter)
   * and the pointer path (right-click) showing the same menu — and it is why
   * `openInNewTab` is the row's own `onSelectSession` rather than a second
   * opener the two menus could disagree about.
   */
  const rowActions = useMemo(
    () => ({
      sessionId: session.id,
      workingDir: session.working_dir,
      openInNewTab: handleOpenInNewTabClick,
    }),
    [session.id, session.working_dir, handleOpenInNewTabClick]
  );

  const handleExportClick = useCallback(
    (e: React.MouseEvent) => {
      onExportClick(session, e);
    },
    [onExportClick, session]
  );

  // Get extension names for this session
  const extensionNames = useMemo(
    () => getSessionExtensionNames(session.extension_data),
    [session.extension_data]
  );
  const billedTokenEstimate = billedSessionTokenEstimate(session);

  return (
    /* #114: right-click the row for the same three actions the `⋯` overflow
       carries. The trigger is `asChild` on the row itself, so the row keeps its
       own class list, its ref registration and its layout — a wrapper element
       here would sit between `.biorouter-list-shell` and its rows and take the
       separators with it. Keyboard users reach this with the Menu key or
       Shift+F10, which dispatch `contextmenu` on the focused element; the `⋯`
       button is the other keyboard path and shows the identical list. */
    <ChatRowContextMenu target={rowActions}>
      <div className="biorouter-list-row session-item flex items-center gap-3 py-2 px-4 relative group">
        {/* BR-71: the badge lives INSIDE the row, and is derived from the row
          itself rather than from where it was rendered — so a subagent run
          whose parent is missing from the list is still labelled instead of
          reading as an unexplained bare conversation. */}
        {session.session_type === 'sub_agent' && (
          // V8 — the `Badge` primitive, not a fourth hand-rolled chip recipe.
          // `badge` is the 20px STATUS tier (§3.4): "sub" is a read-only fact
          // about this row, not a category you can act on, so it takes the
          // smaller of the two and gets out of the way.
          <Badge data-testid="subagent-badge" title="Subagent run">
            sub
          </Badge>
        )}

        {/* Title + metadata */}
        <button
          type="button"
          onClick={handleCardClick}
          className="flex-1 min-w-0 cursor-pointer rounded-inner text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-border-focus"
          aria-label={`Open chat ${session.name}`}
        >
          {/* This row's SessionItem SHADOWS the exported
            components/sessions/SessionItem.tsx — same name, different
            component, both reachable from History. They draw the SAME glyph
            from the SAME resolver, or the marking would depend on which of the
            two a surface happened to mount. */}
          <div className="flex min-w-0 items-center gap-1.5">
            <ChatKindIcon session={session} tier={session.privacy_tier} className="h-4 w-4" />
            <h3 className="text-label truncate">{session.name}</h3>
          </div>
          {session.diverged_from && (
            <div className="flex items-center gap-2 mt-0.5 text-text-muted text-supporting min-w-0">
              <GitBranch className="w-3 h-3 flex-shrink-0" />
              <span className="truncate max-w-[320px]">
                branched from {sessionNameById.get(session.diverged_from) ?? session.diverged_from}
              </span>
            </div>
          )}
          <div className="flex items-center gap-3 mt-0.5 text-text-muted text-supporting">
            {/* A timestamp is one value, so it breaks as one or not at all. It had
              neither `shrink-0` nor `nowrap`, and its min-content is the longest
              WORD — so at 640px the rows broke "3:18 / AM" and "08/06/2026 10:14
              / PM" across two lines, and a list whose rows are then two
              different heights loses its rhythm exactly where it is most
              cramped. The working directory beside it already truncates, which
              is the right thing for that value to do under pressure. */}
            <div className="flex flex-shrink-0 items-center gap-2 whitespace-nowrap">
              <Calendar className="w-3 h-3 flex-shrink-0" />
              <span>{formatMessageTimestamp(Date.parse(session.updated_at) / 1000)}</span>
            </div>
            <div className="flex items-center gap-2 min-w-0">
              <Folder className="w-3 h-3 flex-shrink-0" />
              {/* Monospace, because this is a PATH. Clicking this row opens
                  SessionHistoryView, whose header draws the same value beside
                  the same Folder glyph at the same size and colour — in
                  `font-mono`. Body font here made a path change face on
                  navigation. See D-31 in styles/main.css: mono earns paths. */}
              <span className="truncate max-w-[240px] font-mono">{session.working_dir}</span>
            </div>
          </div>
        </button>

        {/* Right-side stats + hover actions.
          §3.10 "one optical axis per row": each right-hand cluster gets its
          own 20px-high centred box rather than trusting a 32px button and a
          12px glyph to agree on a baseline. The figures are `tabular-nums`
          in `min-w-*` boxes so the counts form real columns down the list
          instead of drifting with each value's width.

          MIN-width, not width. These were fixed `w-*` boxes, and a real
          29,988,671-token session measures 72px against a 48px box, so the
          last digits painted straight over the puzzle icon that follows —
          illegible on every row carrying an estimate. `w-8` and `w-4` were the
          same bug waiting for a five-digit message count and a three-digit
          extension count. A bigger fixed number only moves the cliff; a floor
          keeps the column and lets the rare long value push the cluster out.

          These spans deliberately keep flex's default `min-width: auto`, which
          is what refuses to shrink them below their content. That is the exact
          property this repo normally has to defeat with `min-w-0` — here it is
          the mechanism, so do not "tidy" one in. */}
        <div className="flex h-5 items-center gap-3 flex-shrink-0">
          <div className="flex h-5 items-center gap-3 text-supporting text-text-muted font-mono tabular-nums">
            <div className="flex items-center gap-2">
              <MessageSquareText className="w-3 h-3" />
              <span className="min-w-8 text-right whitespace-nowrap">{session.message_count}</span>
            </div>
            {billedTokenEstimate && (
              <div
                className="flex items-center gap-2"
                title={
                  billedTokenEstimate.lowerBound
                    ? 'At least this many tokens; only last-turn usage is available for this older chat'
                    : 'Billed tokens across every turn, including recorded cache usage'
                }
              >
                <Target className="w-3 h-3" />
                <span className="sr-only">Billed tokens: </span>
                <span className="min-w-12 text-right whitespace-nowrap">
                  {formatBilledTokenEstimate(billedTokenEstimate)}
                </span>
              </div>
            )}
            {extensionNames.length > 0 && (
              <TooltipProvider>
                <Tooltip>
                  <TooltipTrigger asChild>
                    <div className="flex items-center gap-2" onClick={(e) => e.stopPropagation()}>
                      <Puzzle className="w-3 h-3" />
                      <span className="min-w-4 text-right whitespace-nowrap">
                        {extensionNames.length}
                      </span>
                    </div>
                  </TooltipTrigger>
                  <TooltipContent side="top" className="max-w-xs">
                    <div className="text-supporting">
                      <div className="text-label mb-1">Extensions:</div>
                      <ul className="list-disc list-inside">
                        {extensionNames.map((name) => (
                          <li key={name}>{name}</li>
                        ))}
                      </ul>
                    </div>
                  </TooltipContent>
                </Tooltip>
              </TooltipProvider>
            )}
          </div>
          {/* §3.10: at most three visible actions, all 32px with 16px icons —
            this is where the 28-vs-32px fork between the outlined trio and
            the delete button ended. Everything else, and every DESTRUCTIVE
            action, lives behind the one `⋯` overflow, so a hover-revealed
            cluster can never put Delete under a stray click. */}
          <div className="flex gap-1 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  onClick={handleEditClick}
                  variant="outline"
                  shape="round"
                  aria-label={`Edit ${session.name}`}
                >
                  <Edit2 className="w-4 h-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="top">Edit chat name</TooltipContent>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger asChild>
                <Button
                  onClick={handleExportClick}
                  variant="outline"
                  shape="round"
                  aria-label={`Export ${session.name}`}
                >
                  <Download className="w-4 h-4" />
                </Button>
              </TooltipTrigger>
              <TooltipContent side="top">Export chat</TooltipContent>
            </Tooltip>
            <DropdownMenu>
              <Tooltip>
                <TooltipTrigger asChild>
                  <DropdownMenuTrigger asChild>
                    <Button
                      onClick={(e) => e.stopPropagation()}
                      variant="outline"
                      shape="round"
                      aria-label={`More actions for ${session.name}`}
                    >
                      <MoreHorizontal className="w-4 h-4" />
                    </Button>
                  </DropdownMenuTrigger>
                </TooltipTrigger>
                <TooltipContent side="top">More actions</TooltipContent>
              </Tooltip>
              <DropdownMenuContent align="end" onClick={(e) => e.stopPropagation()}>
                {/* Clicking the row already opens the session in a tab — the
                  first item is discoverability, not new behaviour, and it
                  calls the same `onSelectSession` the row's own click does.
                  The glyph is the one the tab strip uses for a tab.

                  ⚠ These come from `chatRowActions`, the same array the
                  right-click menu renders (#114). Spelling them out here
                  again is how the two menus on ONE row start disagreeing
                  about what "Open in new tab" does — which is exactly the
                  drift the note above was already guarding against, now with
                  a second menu able to cause it. */}
                {chatRowActions(rowActions).map(({ key, label, icon: Icon, run }) => (
                  <DropdownMenuItem key={key} onSelect={run}>
                    <Icon className="w-4 h-4" />
                    {label}
                  </DropdownMenuItem>
                ))}
                {/* Issue #56 §12.1. The row's own overflow menu, and the ONLY
                  place declassification is offered besides the session
                  page's action bar — deliberately not `SessionNamePill`'s
                  title menu, which is one careless click from the chat
                  title.

                  Private rows only: a public chat has nothing to
                  declassify. Before §3.10 folded every row action behind
                  this one `⋯`, the tier gated the whole BUTTON, because a
                  "More actions" trigger that opened a one-item menu — and
                  an empty one on every public row — was worse than no
                  trigger at all. The menu now always carries the open and
                  delete items, so only the ITEM is gated. */}
                {session.privacy_tier === 'private' && (
                  <DropdownMenuItem onSelect={() => onDeclassifyClick(session)}>
                    Make this chat public
                  </DropdownMenuItem>
                )}
                <DropdownMenuSeparator />
                <DropdownMenuItem
                  variant="destructive"
                  onClick={handleDeleteClick}
                  aria-label={`Delete ${session.name}`}
                >
                  <Trash2 className="w-4 h-4" />
                  Delete chat
                </DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </div>
    </ChatRowContextMenu>
  );
});

const SessionListView: React.FC<SessionListViewProps> = React.memo(({ onSelectSession }) => {
  const initialSessions = useRef(getCachedSessionList()).current;
  const navigate = useNavigate();
  const [sessions, setSessions] = useState<Session[]>(initialSessions ?? []);
  // The toggle below starts off, so the warm cache — which a sibling pane may
  // have filled with subagent rows — is filtered before it reaches the first
  // paint.
  const [filteredSessions, setFilteredSessions] = useState<Session[]>(() =>
    withoutSubagents(initialSessions ?? [])
  );
  const [dateGroups, setDateGroups] = useState<DateGroup[]>(() =>
    groupSessionsByDate(withoutSubagents(initialSessions ?? []))
  );
  const [isLoading, setIsLoading] = useState(initialSessions === null);
  const [showSkeleton, setShowSkeleton] = useState(initialSessions === null);
  const [showContent, setShowContent] = useState(initialSessions !== null);
  const [error, setError] = useState<string | null>(null);
  const [searchResults, setSearchResults] = useState<{
    count: number;
    currentIndex: number;
  } | null>(null);

  const [visibleSessionCount, setVisibleSessionCount] = useState(INITIAL_VISIBLE_SESSIONS);

  // BR-71: subagent transcripts are hidden by default — they are machinery,
  // not conversations the user started. Turning this on refetches with
  // `include_subagents` and nests each run under the session that spawned it.
  const [showSubagents, setShowSubagents] = useState(false);

  // `showSubagents` is this pane's state, but the session cache behind it is
  // module-global — a second History pane, or Home, can publish subagent rows
  // into this one at any time (and an orphaned request can leave the cache
  // holding the other identity for a moment). The toggle decides what is
  // FETCHED; this decides what this pane will SHOW, so the two can never
  // disagree on screen.
  const visibleSessions = useMemo(
    () => (showSubagents ? sessions : withoutSubagents(sessions)),
    [sessions, showSubagents]
  );

  // Parent grouping runs BEFORE date bucketing. `groupSessionsByDate` buckets
  // on `updated_at` and a parent's advances every time the conversation is
  // resumed, so a subagent that ran on an earlier day sits in a different
  // bucket — grouping within each bucket would drop it back to top level,
  // which is exactly the orphaned, unexplained row this feature removes.
  const parentGroups = useMemo(() => groupSessionsByParent(filteredSessions), [filteredSessions]);
  // Only top-level rows are dated and paginated; children ride with their
  // parent, so `visibleSessionCount` counts rendered parents, not raw rows.
  const topLevelSessions = useMemo(() => parentGroups.map((g) => g.session), [parentGroups]);
  const childrenByParentId = useMemo(() => {
    const map = new Map<string, Session[]>();
    for (const { session, children } of parentGroups) {
      if (children.length > 0) map.set(session.id, children);
    }
    return map;
  }, [parentGroups]);

  // Edit modal state
  const [showEditModal, setShowEditModal] = useState(false);
  const [editingSession, setEditingSession] = useState<Session | null>(null);

  // Delete confirmation modal state
  const [showDeleteConfirmation, setShowDeleteConfirmation] = useState(false);
  const [sessionToDelete, setSessionToDelete] = useState<Session | null>(null);

  // Import modal state
  const [showImportModal, setShowImportModal] = useState(false);

  // Issue #56 §12.1: declassification is attached to the History ROW, not to
  // the chat title menu. The dialog is mounted from here (rather than inside
  // `SessionItem`) because `SessionItem` is declared in this component's body,
  // so every re-render is a fresh component type and a dialog owned by a row
  // would be torn down and remounted mid-interaction.
  const [declassifyTarget, setDeclassifyTarget] = useState<Session | null>(null);

  // Search state for debouncing
  const [searchTerm, setSearchTerm] = useState('');
  const [caseSensitive, setCaseSensitive] = useState(false);
  const debouncedSearchTerm = useDebounce(searchTerm, 300); // 300ms debounce

  const containerRef = useRef<HTMLDivElement>(null);

  const visibleDateGroups = useMemo(() => {
    let remainingSessions = visibleSessionCount;

    return dateGroups.flatMap((group) => {
      if (remainingSessions <= 0) return [];
      const sessions = group.sessions.slice(0, remainingSessions);
      remainingSessions -= sessions.length;
      return [{ ...group, sessions }];
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
        visibleSessionCount < topLevelSessions.length
      ) {
        setVisibleSessionCount((previousCount) =>
          Math.min(previousCount + VISIBLE_SESSION_BATCH, topLevelSessions.length)
        );
      }
    },
    [visibleSessionCount, topLevelSessions.length]
  );

  useEffect(() => {
    if (debouncedSearchTerm) {
      setVisibleSessionCount(topLevelSessions.length);
    } else {
      setVisibleSessionCount(INITIAL_VISIBLE_SESSIONS);
    }
  }, [debouncedSearchTerm, topLevelSessions.length]);

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
      // Use startTransition to make state updates non-blocking
      startTransition(() => {
        setSessions(refreshedSessions);
        setFilteredSessions(
          showSubagents ? refreshedSessions : withoutSubagents(refreshedSessions)
        );
        setError(null);
      });
    } catch (err) {
      console.error('Failed to load sessions:', err);
      if (!hasCachedSessions) {
        setError('Could not load your chats. Try again in a moment.');
        setSessions([]);
        setFilteredSessions([]);
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
  // list channel, in this window or a sibling — so See-all reflects a branch
  // or a rename without a remount. The search effect re-derives
  // `filteredSessions` from `sessions`, so refreshing the source keeps any
  // active filter consistent.
  useEffect(() => {
    return subscribeSessionList(() => {
      const cached = getCachedSessionList();
      if (cached) startTransition(() => setSessions(cached));
    });
  }, []);

  // Timing logic to prevent flicker between skeleton and content on initial
  // load. `showContent` — not `showSkeleton` — is the "already revealed"
  // guard, and that is load-bearing: this effect WRITES `showSkeleton`, so
  // keeping it in the deps re-runs the effect on the very next render and the
  // cleanup below would cancel the reveal it had just armed. The two guards
  // admit the same states, because the only writer that re-arms a cold load
  // (`loadSessions`, on a cache miss) sets `showSkeleton` and `showContent`
  // together; this one just leaves the deps stable across the 10ms window.
  useEffect(() => {
    if (isLoading || showContent) return undefined;
    setShowSkeleton(false);
    // No `startTransition`: it used to wrap the `setTimeout` call, which
    // marked nothing (the callback returns before either setState runs), and
    // the reveal is one opacity class on a layer that is already mounted —
    // `renderActualContent()` renders under the skeleton too. Deferring the
    // frame the delay exists to schedule is the opposite of what it is for.
    const revealTimer = setTimeout(() => setShowContent(true), 10);
    // Unmounting inside that 10ms window (navigating away, or a test file
    // finishing) must disarm it: the callback would otherwise setState on an
    // unmounted tree, and on CI it fired after jsdom was gone and took the
    // whole run down with `window is not defined`.
    return () => clearTimeout(revealTimer);
    // `isInitialLoad` used to sit here and in the callback above. It was
    // write-only: the only thing that ever READ it was the guard around its
    // own setter, so the whole cycle — the `useState`, the branch, the
    // dependency — decided nothing. As a dependency it also re-ran this
    // effect one extra time on the very tick the reveal landed, which is the
    // opposite of the stable deps the comment above depends on.
  }, [isLoading, showContent]);

  // Memoize date groups calculation to prevent unnecessary recalculations
  const memoizedDateGroups = useMemo(() => {
    if (topLevelSessions.length > 0) {
      return groupSessionsByDate(topLevelSessions);
    }
    return [];
  }, [topLevelSessions]);

  // Update date groups when filtered sessions change
  useEffect(() => {
    startTransition(() => {
      setDateGroups(memoizedDateGroups);
    });
  }, [memoizedDateGroups]);

  // Debounced search effect - performs actual filtering
  useEffect(() => {
    if (!debouncedSearchTerm) {
      startTransition(() => {
        setFilteredSessions(visibleSessions);
        setSearchResults(null);
      });
      return;
    }

    // Use startTransition to make search non-blocking
    startTransition(() => {
      const searchTerm = caseSensitive ? debouncedSearchTerm : debouncedSearchTerm.toLowerCase();
      const filtered = visibleSessions.filter((session) => {
        const description = session.name;
        const workingDir = session.working_dir;
        const sessionId = session.id;

        if (caseSensitive) {
          return (
            description.includes(searchTerm) ||
            sessionId.includes(searchTerm) ||
            workingDir.includes(searchTerm)
          );
        } else {
          return (
            description.toLowerCase().includes(searchTerm) ||
            sessionId.toLowerCase().includes(searchTerm) ||
            workingDir.toLowerCase().includes(searchTerm)
          );
        }
      });

      setFilteredSessions(filtered);
      setSearchResults(filtered.length > 0 ? { count: filtered.length, currentIndex: 1 } : null);
    });
  }, [debouncedSearchTerm, caseSensitive, visibleSessions]);

  // Handle immediate search input (updates search term for debouncing)
  const handleSearch = useCallback((term: string, caseSensitive: boolean) => {
    setSearchTerm(term);
    setCaseSensitive(caseSensitive);
  }, []);

  // Handle search result navigation
  const handleSearchNavigation = (direction: 'next' | 'prev') => {
    if (!searchResults || filteredSessions.length === 0) return;

    let newIndex: number;
    if (direction === 'next') {
      newIndex = (searchResults.currentIndex % filteredSessions.length) + 1;
    } else {
      newIndex =
        searchResults.currentIndex === 1 ? filteredSessions.length : searchResults.currentIndex - 1;
    }

    setSearchResults({ ...searchResults, currentIndex: newIndex });

    // Find the SearchView's container element
    const searchContainer =
      containerRef.current?.querySelector<SearchContainerElement>('.search-container');
    if (searchContainer?._searchHighlighter) {
      // Update the current match in the highlighter
      searchContainer._searchHighlighter.setCurrentMatch(newIndex - 1, true);
    }
  };

  // Handle modal close
  const handleModalClose = useCallback(() => {
    setShowEditModal(false);
    setEditingSession(null);
  }, []);

  const handleModalSave = useCallback(async (sessionId: string, newDescription: string) => {
    // Update state immediately for optimistic UI
    const updateName = (currentSessions: Session[]) =>
      currentSessions.map((session) =>
        session.id === sessionId ? { ...session, name: newDescription } : session
      );
    updateCachedSessionList(updateName);
    setSessions(updateName);
  }, []);

  const handleEditSession = useCallback((session: Session) => {
    setEditingSession(session);
    setShowEditModal(true);
  }, []);

  const handleDeleteSession = useCallback((session: Session) => {
    setSessionToDelete(session);
    setShowDeleteConfirmation(true);
  }, []);

  const handleConfirmDelete = useCallback(async () => {
    if (!sessionToDelete) return;

    setShowDeleteConfirmation(false);
    const sessionToDeleteId = sessionToDelete.id;
    const sessionName = sessionToDelete.name;
    setSessionToDelete(null);

    try {
      await deleteSession({
        path: { session_id: sessionToDeleteId },
        throwOnError: true,
      });
      const removeDeletedSession = (currentSessions: Session[]) =>
        currentSessions.filter((session) => session.id !== sessionToDeleteId);
      updateCachedSessionList(removeDeletedSession);
      setSessions(removeDeletedSession);
      toastSuccess({
        title: 'Chat deleted',
        msg: `"${sessionName}" was removed from chat history.`,
      });
    } catch (error) {
      console.error('Error deleting session:', error);
      const errorMessage = error instanceof Error ? error.message : 'Unknown error';
      toastError({
        title: 'Failed to delete chat',
        msg: `Could not delete "${sessionName}": ${errorMessage}`,
      });
    }
    await loadSessions();
  }, [sessionToDelete, loadSessions]);

  const handleCancelDelete = useCallback(() => {
    setShowDeleteConfirmation(false);
    setSessionToDelete(null);
  }, []);

  const handleExportSession = useCallback(async (session: Session, e: React.MouseEvent) => {
    e.stopPropagation();

    const response = await exportSession({
      path: { session_id: session.id },
      throwOnError: true,
    });

    const json = response.data;
    const blob = new Blob([json], { type: 'application/json' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `${session.name}.json`;
    document.body.appendChild(a);
    a.click();
    document.body.removeChild(a);
    URL.revokeObjectURL(url);
    toastSuccess({
      title: 'Chat exported',
      msg: `"${session.name}" was downloaded.`,
    });
  }, []);

  const handleImportClick = useCallback(() => {
    setShowImportModal(true);
  }, []);

  const handleImportSession = useCallback(
    async (json: string) => {
      await importSession({ body: { json }, throwOnError: true });
      toastSuccess({
        title: 'Chat imported',
        msg: 'The imported chat is now available in chat history.',
      });
      await loadSessions();
    },
    [loadSessions]
  );

  const handleDeclassifyClick = useCallback((session: Session) => {
    setDeclassifyTarget(session);
  }, []);

  // The row is stale the moment the daemon answers: its badge and its
  // overflow menu both key on `privacy_tier`. Patch the shared cache as well
  // as this pane's list, exactly as the delete path does, so the sidebar's
  // Recents and any second History pane stop badging it too.
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

  const renderActualContent = () => {
    if (error) {
      // The error state is the empty state with a different cause: same icon
      // plate, same title/description/action stack, same quiet register. It
      // used to be a hand-rolled column with its own icon size, its own type
      // ramp and a Title Case sentence — one of the four error dialects §4.5
      // exists to close.
      return (
        <EmptyState
          icon={AlertCircle}
          title="Couldn't load your chat history"
          description={error}
          actions={
            <Button onClick={loadSessions} variant="outline">
              Try again
            </Button>
          }
        />
      );
    }

    if (sessions.length === 0) {
      return (
        <EmptyState
          icon={MessageSquareText}
          title="No chats yet"
          description="Past chats will appear here after you start chatting. You can also import an existing chat."
          actions={
            <>
              <Button onClick={() => navigate('/pair')}>Start a chat</Button>
              <Button onClick={handleImportClick} variant="outline">
                <Upload className="h-4 w-4" />
                Import chat
              </Button>
            </>
          }
        />
      );
    }

    if (dateGroups.length === 0 && searchResults !== null) {
      return (
        <EmptyState
          icon={MessageSquareText}
          title="No matching chats"
          description="Try a different name, folder, or session ID."
          compact
        />
      );
    }

    return (
      <div className="space-y-8">
        {visibleDateGroups.map((group) => (
          <div key={group.label} className="space-y-4">
            <div className="sticky top-0 z-10 bg-background-canvas pt-2 pb-2">
              {/* `text-caps` IS the caps style — it carries the transform as
                    well as the 11/500/+0.08em metrics, so `uppercase` and
                    `tracking-wider` beside it were spelling out what the role
                    already says. */}
              <h2 className="text-caps text-text-muted">{group.label}</h2>
            </div>
            <div className="session-grid biorouter-list-shell">
              {group.sessions.map((session) => {
                const children = childrenByParentId.get(session.id);
                return (
                  <React.Fragment key={session.id}>
                    <SessionItem
                      session={session}
                      onSelectSession={onSelectSession}
                      sessionNameById={sessionNameById}
                      onEditClick={handleEditSession}
                      onDeleteClick={handleDeleteSession}
                      onExportClick={handleExportSession}
                      onDeclassifyClick={handleDeclassifyClick}
                    />
                    {/* One indented block for all of a parent's children, not
                          one wrapper each: a lone row inside its own wrapper is
                          `:last-child` and loses the separator every other row
                          in the list has. */}
                    {children && (
                      <div className="ml-6 flex flex-col border-l border-border-subtle pl-2">
                        {children.map((child) => (
                          <SessionItem
                            key={child.id}
                            session={child}
                            onSelectSession={onSelectSession}
                            sessionNameById={sessionNameById}
                            onEditClick={handleEditSession}
                            onDeleteClick={handleDeleteSession}
                            onExportClick={handleExportSession}
                            onDeclassifyClick={handleDeclassifyClick}
                          />
                        ))}
                      </div>
                    )}
                  </React.Fragment>
                );
              })}
            </div>
          </div>
        ))}

        {visibleSessionCount < topLevelSessions.length && (
          <div className="flex justify-center py-8">
            {/* V6 — a status line is `text-supporting`, the same role the
                  metadata under every row above it uses. `text-secondary` is
                  the dense-CONTROL exception, and this is not a control. */}
            <div className="flex items-center gap-2 text-supporting text-text-muted">
              <LoaderCircle className="h-4 w-4 animate-spin" aria-hidden="true" />
              <span>Loading more chats...</span>
            </div>
          </div>
        )}
      </div>
    );
  };

  return (
    <>
      <MainPanelLayout>
        <div className="flex-1 flex flex-col min-h-0">
          {/* ⚠ The header's column and the body's below it are BOTH
                `size="chat"`, and they move together or not at all — the
                header's hairline is full-bleed (`PageHeader` owns the wrapper),
                so a size on one box and not the other is a visible step in the
                left edge they share rather than a mistake inside one component.

                Chat history reads the CHAT measure for the same reason
                Settings does (operator decision, 2026-09-07): a row here is a
                title on the left and a stats cluster on the right, so the
                extra width a wide window handed the page measure landed
                BETWEEN them — at 1440 the message/token/extension counts sat
                roughly 700px from the chat they count. There is a second
                argument this view has and Settings does not: clicking a row
                RESUMES the chat, which is already on the chat measure, so
                the list and the thing it opens now share one left edge.
                `measures.test.ts` asserts at the source that no
                `<ReadableContent` here is left on the default size.

                ⚠ **Import chat moved OFF the title row** (operator decision,
                2026-09-07). This view was built to §4.2's original recipe —
                actions right-aligned on the title row — and §4.2 has since been
                amended: the actions go on their own line under the description,
                the shape Workflows / Extensions / Skills / Built apps use. One
                consequence is worth naming, because it deletes the reason for a
                construction that used to be load-bearing here: the title no
                longer shares its row with a button, so it no longer needs
                `min-w-0 truncate` to give way to one, and a page title is no
                longer a thing that can be clipped by its own controls. */}
          <PageHeader
            title="Chat history"
            description={
              <>
                View and search your past chats with Biorouter. {getSearchShortcutText()} to search.
              </>
            }
            actions={
              /* V7 — no `className`. It carried `flex flex-shrink-0
                   items-center gap-2`, and every token of that is already in
                   `buttonVariants`' base (`inline-flex items-center
                   justify-center gap-2 … shrink-0`). The bare `flex` was not
                   merely redundant: tailwind-merge lets it FLIP the base's
                   `inline-flex`, which is how a row action elsewhere became a
                   full-width bar. */
              <Button onClick={handleImportClick} variant="outline">
                <Upload className="w-4 h-4" />
                Import chat
              </Button>
            }
          >
            {/* §3.3's checkbox, not the OS one. A bare `<input
                  type="checkbox">` here rendered as macOS system blue in light
                  mode and a bare white square in dark — the only unstyled
                  control in the app, directly under a page title. `-ml-px`
                  pulls back the 1px by which the 24px hit target overhangs its
                  22px visible box, so the box's edge lines up with the title
                  and description above it rather than the target's.

                  It is `PageHeader`'s `children`, NOT another `actions` entry:
                  a filter that changes what the list shows is not an action the
                  page offers, and putting it in the strip would give it the
                  same standing as Import chat. */}
            <label className="mt-3 flex cursor-pointer items-center gap-2 text-supporting text-text-muted">
              <Checkbox
                className="-ml-px"
                checked={showSubagents}
                onChange={(e) => setShowSubagents(e.target.checked)}
              />
              Show subagent runs
            </label>
          </PageHeader>

          <ReadableContent size="chat" className="flex-1 min-h-0 relative px-6 pt-6 pb-8">
            <ScrollArea
              handleScroll={handleScroll}
              className="h-full"
              paddingX={1}
              data-search-scroll-area
            >
              <div ref={containerRef} className="h-full relative">
                <SearchView
                  onSearch={handleSearch}
                  onNavigate={handleSearchNavigation}
                  searchResults={searchResults}
                  className="relative"
                  placeholder="Search history..."
                >
                  {/* Loading layer - shaped like the rows that will replace it. */}
                  <div
                    className={`absolute inset-0 transition-opacity duration-[var(--motion-fast)] ease-[var(--ease-in)] ${isLoading || showSkeleton ? 'opacity-100 z-10' : 'opacity-0 z-0 pointer-events-none'}`}
                  >
                    {(isLoading || showSkeleton) && <HistoryLoading />}
                  </div>

                  {/* Content layer - always rendered but conditionally visible */}
                  <div
                    className={`relative transition-opacity duration-[var(--motion-base)] ease-[var(--ease-out)] ${showContent ? 'opacity-100 z-10' : 'opacity-0 z-0'}`}
                  >
                    {renderActualContent()}
                  </div>
                </SearchView>
              </div>
            </ScrollArea>
          </ReadableContent>
        </div>
      </MainPanelLayout>

      <EditSessionModal
        session={editingSession}
        isOpen={showEditModal}
        onClose={handleModalClose}
        onSave={handleModalSave}
      />

      <ImportSessionModal
        isOpen={showImportModal}
        onClose={() => setShowImportModal(false)}
        onImport={handleImportSession}
      />

      <ConfirmationModal
        isOpen={showDeleteConfirmation}
        title="Delete chat?"
        message={`Are you sure you want to delete the chat "${sessionToDelete?.name}"? This action cannot be undone.`}
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        onConfirm={handleConfirmDelete}
        onCancel={handleCancelDelete}
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
