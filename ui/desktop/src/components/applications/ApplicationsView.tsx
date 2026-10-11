import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import {
  Play,
  Trash2,
  RefreshCw,
  Download,
  MessageSquare,
  Calendar,
  Clock,
  Loader2,
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Badge } from '../ui/badge';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { SearchView } from '../conversation/SearchView';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { toastSuccess, toastError, toastWarning } from '../../toasts';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import {
  configuredBaseUrl,
  deleteAgentDrafterApp,
  exportedMessage,
  requireOk,
  secretHeader,
} from './appManagement';
import type { AppManifest } from './appManagement';
import ExportAppDialog from './ExportAppDialog';
import type { AppExportRequest, AppExportResult } from '../../utils/appExportRequest';
import { failureSentence } from '../../utils/ipcError';
import { isBrowserSurface } from '../../utils/surface';
import { userActionHeaders } from '../../utils/userAction';
import { subscribeSessionRemoved } from '../../utils/sessionListCache';
import {
  buildChatEnabled,
  buildChatLabel,
  buildChatNote,
  buildChatRefusal,
  buildChatTarget,
  resolveBuildChat,
  resolveBuildChats,
  type BuildChatState,
} from './buildChat';

/** Loading is rows that are the shape of rows, not a line of prose in dead space. */
function ApplicationItemSkeleton() {
  return (
    <div className="biorouter-list-row flex items-start gap-3 px-3 py-3">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="mt-2 h-3 w-64" />
        <Skeleton className="mt-2 h-3 w-40" />
      </div>
    </div>
  );
}

/** Format a Unix-seconds timestamp as a short, readable date (e.g. "Jun 24, 2026"). */
function formatDate(secs?: number | null): string {
  if (!secs) return 'unknown';
  return new Date(secs * 1000).toLocaleDateString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
  });
}

export default function ApplicationsView() {
  const navigate = useNavigate();
  const [apps, setApps] = useState<AppManifest[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [searchTerm, setSearchTerm] = useState('');
  const [appToDelete, setAppToDelete] = useState<AppManifest | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);
  const [launchingAppId, setLaunchingAppId] = useState<string | null>(null);
  const [exportingAppId, setExportingAppId] = useState<string | null>(null);
  const [appToExport, setAppToExport] = useState<AppManifest | null>(null);
  // Where each app's chat button leads, keyed by app id (`buildChat.ts`). A row
  // with no entry yet is still being checked and stays enabled.
  const [chatStates, setChatStates] = useState<Record<string, BuildChatState>>({});
  const [openingChatAppId, setOpeningChatAppId] = useState<string | null>(null);
  // ⚠ Answers land out of order: a load's resolution can be overtaken by a click,
  // a Refresh or a deletion broadcast. Every resolution takes a ticket per app and
  // only the app's newest ticket may write its row or open its chat, so a read
  // issued before a chat was deleted can never flip the row back to "open", or
  // open it.
  const chatTicketRef = useRef(0);
  const chatLatestRef = useRef(new Map<string, number>());
  // ⚠ A second guard, on the click rather than the row: only the newest click,
  // on a view that is still mounted, may navigate or toast. The ticket cannot
  // say this — leaving the page advances no ticket, and react-router's
  // `navigate` keeps working after its component unmounts — so a slow answer
  // used to pull a user who had moved on to Settings back into /pair and save
  // a tab there, and two quick clicks on different rows opened two tabs.
  const chatClickRef = useRef(0);
  useEffect(() => {
    const clicks = chatClickRef;
    return () => {
      clicks.current += 1;
    };
  }, []);
  const appsRef = useRef<AppManifest[]>([]);
  const chatStatesRef = useRef<Record<string, BuildChatState>>({});
  useEffect(() => {
    appsRef.current = apps;
    chatStatesRef.current = chatStates;
  });

  const takeChatTicket = useCallback((appId: string) => {
    const ticket = ++chatTicketRef.current;
    chatLatestRef.current.set(appId, ticket);
    return ticket;
  }, []);

  const resolveChats = useCallback(
    async (list: AppManifest[]) => {
      // A row whose manifest could not be read has no chat to look for.
      const readable = list.filter((app) => typeof app.broken !== 'string');
      if (readable.length === 0) return;
      const tickets = new Map(readable.map((app) => [app.id, takeChatTicket(app.id)]));
      const states = await resolveBuildChats(readable).catch(
        () => ({}) as Record<string, BuildChatState>
      );
      const fresh = Object.entries(states).filter(
        ([appId]) => chatLatestRef.current.get(appId) === tickets.get(appId)
      );
      if (fresh.length > 0) {
        setChatStates((prev) => ({ ...prev, ...Object.fromEntries(fresh) }));
      }
    },
    [takeChatTicket]
  );

  const load = useCallback(async () => {
    setLoading(true);
    try {
      // The proof rides along although the route does not ask for it yet: the
      // list names the chat each app was built in, and a listing that names a
      // chat is gated like every other one (`routes::session_reach`). Sending it
      // now keeps private-built apps' chats from silently reading "not recorded"
      // the day the route starts filtering them.
      const res = await fetch(`${configuredBaseUrl()}/apps`, {
        headers: { ...(await secretHeader()), ...(await userActionHeaders()) },
      });
      await requireOk(res);
      const data: AppManifest[] = await res.json();
      // Most recently updated first.
      data.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
      setApps(data);
      setError(null);
      // Not awaited: the rows show at once and each chat button settles as its
      // check lands. Refresh re-checks too, keeping the last answer meanwhile.
      void resolveChats(data);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load apps');
    } finally {
      setLoading(false);
    }
  }, [resolveChats]);

  useEffect(() => {
    load();
  }, [load]);

  // A chat deleted in ANY window (the broadcast crosses windows) flips its rows
  // at once, without waiting for a Refresh. Deletions made by the CLI or an
  // agent are not announced; the click-time check below covers those.
  useEffect(
    () =>
      subscribeSessionRemoved((removedId) => {
        const builtThere: string[] = [];
        const ledThere: AppManifest[] = [];
        for (const app of appsRef.current) {
          if (app.session_id === removedId) builtThere.push(app.id);
          else if (buildChatTarget(chatStatesRef.current[app.id]) === removedId) ledThere.push(app);
        }
        if (builtThere.length > 0) {
          const deleted: BuildChatState = { kind: 'deleted' };
          for (const appId of builtThere) takeChatTicket(appId);
          setChatStates((prev) => ({
            ...prev,
            ...Object.fromEntries(builtThere.map((appId) => [appId, deleted] as const)),
          }));
        }
        // The button led to the parent of the subagent run that built the app.
        // The run itself may still exist, so ask again rather than assume.
        if (ledThere.length > 0) void resolveChats(ledThere);
      }),
    [resolveChats, takeChatTicket]
  );

  // Launch opens the app's own served URL in the real browser — archetype-
  // agnostic by construction (nothing here assumes a chat-shaped app), so the
  // v2 non-chat starters need no special affordance (plan Phase 5 item 5).
  // The main process mints the app's one-time launch link and hands it to the
  // browser without putting it on a command line (W2-HRD-1), so this names the
  // app and nothing else.
  const launch = async (app: AppManifest) => {
    if (launchingAppId === app.id) return;
    setLaunchingAppId(app.id);
    try {
      await window.electron.openAppInBrowser(app.id);
    } catch (err) {
      console.error('Failed to open app:', err);
      // The main process says why (the app is gone, the daemon is down, no
      // browser opened); Electron wraps its sentence in an IPC preamble.
      const reason =
        err instanceof Error
          ? err.message.replace(/^Error invoking remote method '[^']*': (?:\w*Error: )?/, '')
          : '';
      toastError({
        title: app.title,
        msg: reason || 'Could not open the app in your browser.',
      });
    } finally {
      setLaunchingAppId((current) => (current === app.id ? null : current));
    }
  };

  // Reopen the chat this app was built in so the user can keep iterating.
  // ⚠ Checked again at the click, and navigation happens ONLY on a chat the
  // daemon just said exists. Navigating is what opens a tab, and a tab is saved
  // per window: a dead id used to leave a "Could not load this chat" tab that
  // came back on every reload. Every other answer is a toast and no tab.
  const openConversation = async (app: AppManifest) => {
    if (openingChatAppId === app.id) return;
    setOpeningChatAppId(app.id);
    const ticket = takeChatTicket(app.id);
    const click = ++chatClickRef.current;
    let state: BuildChatState;
    try {
      state = await resolveBuildChat(app);
    } catch {
      state = { kind: 'unknown' };
    } finally {
      setOpeningChatAppId((current) => (current === app.id ? null : current));
    }
    // ⚠ The ticket guards the navigation as well as the row. A deletion
    // broadcast or a Refresh that arrived while this read was in flight is newer
    // than this answer, so a 200 read just before another window deleted the
    // chat must not open (and save) a tab for it.
    const current = chatLatestRef.current.get(app.id) === ticket;
    if (current) {
      // A read nobody answered does not overwrite an answer someone gave: a
      // daemon blip must not re-enable a row that was known to be deleted.
      setChatStates((prev) =>
        state.kind === 'unknown' && prev[app.id] && prev[app.id].kind !== 'checking'
          ? prev
          : { ...prev, [app.id]: state }
      );
    }
    // The row keeps the answer either way; a click the user has moved on from
    // (another row's click, or another page) says nothing and opens nothing.
    if (chatClickRef.current !== click) return;
    if (!current) {
      // The newer answer speaks for the row. When it has not landed yet (a
      // Refresh still reading), say so rather than guess.
      const newer = chatStatesRef.current[app.id];
      toastError({
        title: app.title,
        msg:
          (newer && newer.kind !== 'available' && newer.kind !== 'checking'
            ? buildChatRefusal(newer)
            : undefined) ?? 'Biorouter was still checking that chat. Try again.',
      });
      return;
    }
    if (state.kind === 'available') {
      // The title rides in route state so the tab opens already named, the way
      // the sidebar opens a chat; BaseChat's load still renames it for certain.
      navigate(`/pair?resumeSessionId=${encodeURIComponent(state.sessionId)}`, {
        state: {
          resumeSessionId: state.sessionId,
          title: state.title,
          userSetName: state.userSetName,
        },
      });
      return;
    }
    toastError({
      title: app.title,
      msg: buildChatRefusal(state) ?? "Couldn't open the chat where this app was built.",
    });
  };

  // Export runs where the files can be written: the main process picks the folder FIRST, has
  // this window's daemon build the export and unpacks it, launchers executable
  // (`utils/appExport.ts`); a `biorouter serve` tab downloads the daemon's zip instead
  // (`exportAppInThisBrowser`). The dialog stays open until this answers, so it can show the
  // harness's findings and offer to export anyway.
  const exportApp = async (
    app: AppManifest,
    request: AppExportRequest
  ): Promise<AppExportResult> => {
    setExportingAppId(app.id);
    try {
      const result = await window.electron.exportApp(app.id, request);
      if (result.status === 'exported') {
        toastSuccess({ title: app.title, msg: exportedMessage(result) });
        // What was left out, and why: a private knowledge base this caller could not open, a
        // skill that is not installed, an older background service. Never folded into success.
        if (result.summary.notes.length > 0) {
          toastWarning({
            title: app.title,
            msg: result.summary.notes.join(' '),
            clampMessage: false,
          });
        }
      } else if (result.status === 'failed') {
        toastError({ title: app.title, msg: result.message });
      }
      return result;
    } catch (err) {
      console.error('Failed to export app:', err);
      const message = failureSentence(err, 'Could not export the app.');
      toastError({ title: app.title, msg: message });
      return { status: 'failed', message };
    } finally {
      setExportingAppId((current) => (current === app.id ? null : current));
    }
  };

  const confirmDelete = async () => {
    if (!appToDelete) return;
    const app = appToDelete;
    setIsDeleting(true);
    try {
      await deleteAgentDrafterApp(app.id);
      setApps((prev) => prev.filter((a) => a.id !== app.id));
      toastSuccess({ title: app.title, msg: 'App deleted' });
    } catch (err) {
      console.error('Failed to delete app:', err);
      toastError({
        title: app.title,
        msg:
          err instanceof Error
            ? `Could not delete the app: ${err.message}`
            : 'Could not delete the app.',
      });
    } finally {
      setIsDeleting(false);
      setAppToDelete(null);
    }
  };

  const filtered = apps.filter((app) => {
    if (!searchTerm) return true;
    const q = searchTerm.toLowerCase();
    return app.title.toLowerCase().includes(q) || (app.description ?? '').toLowerCase().includes(q);
  });

  return (
    <MainPanelLayout>
      <div
        className="flex flex-col min-w-0 flex-1 overflow-y-auto relative"
        data-search-scroll-area
      >
        {/* The one page header (`Layout/PageHeader`), which owns the full-bleed
            hairline, the chat measure and the button strip under the
            description. Refresh keeps its LABEL where the Scheduler's is a bare
            glyph: the Scheduler's sits beside "New schedule", which anchors the
            cluster, and this strip has no labelled sibling for a lone circular
            glyph to borrow meaning from. `variant="outline"` is the family
            spelling for a non-committing header action (Extensions' Browse,
            Skills' two) — a re-fetch is not the view's committing action, so it
            does not take the solid `default` fill. */}
        <PageHeader
          title="Built apps"
          description={
            <>
              Apps you built with Agent Drafter. Each one runs a full Biorouter agent with its own
              model, extensions, skills, and knowledge, and opens in your browser.{' '}
              {getSearchShortcutText()} to search.
            </>
          }
          actions={
            <Button variant="outline" onClick={load}>
              <RefreshCw />
              Refresh
            </Button>
          }
        />

        {/* List */}
        <SearchView
          onSearch={(term, _caseSensitive) => setSearchTerm(term)}
          placeholder="Search built apps..."
        >
          <ReadableContent size="chat" className="px-6 py-4">
            {/* ⚠ The error is rendered whether or not there are apps to show.
                It used to be an `apps.length === 0` branch, so a refresh that
                failed while rows were already on screen said nothing at all and
                the list silently went stale. Retry rides the note (`Note`'s
                `action` slot names exactly this control) rather than sitting in
                a hand-rolled centred block of its own. */}
            {error && (
              <Note
                tone="danger"
                role="alert"
                className="mb-4"
                action={
                  <Button variant="outline" size="sm" onClick={load}>
                    Retry
                  </Button>
                }
              >
                Could not load apps: {error}
              </Note>
            )}

            {loading && apps.length === 0 && (
              <div className="biorouter-list-shell" aria-hidden>
                <ApplicationItemSkeleton />
                <ApplicationItemSkeleton />
                <ApplicationItemSkeleton />
              </div>
            )}

            {!loading && !error && filtered.length === 0 && (
              <EmptyState
                icon={ENTITY_ICONS.application}
                title={searchTerm ? 'No matching apps' : 'No apps built yet'}
                description={
                  searchTerm
                    ? 'No apps match your search.'
                    : 'Ask Biorouter to build one, for example "use Agent Drafter to build a dashboard app". It will appear here.'
                }
              />
            )}

            {/* ⚠ Deliberately NOT `!loading && filtered.length > 0`. `load()`
                sets `loading` on every Refresh as well as on first mount, so
                gating the list on it would UNMOUNT the rows for the length of
                every request — the defect `SchedulesView` records for its own
                fifteen-second poll. The three branches stay mutually exclusive
                without it: the skeletons require an empty list, and the empty
                state requires the load to have finished and to have succeeded. */}
            {filtered.length > 0 && (
              <div className="biorouter-list-shell">
                {filtered.map((app) => (
                  <ApplicationItem
                    key={app.id}
                    app={app}
                    onLaunch={() => launch(app)}
                    onOpenConversation={() => void openConversation(app)}
                    onExport={() => setAppToExport(app)}
                    onDelete={() => setAppToDelete(app)}
                    buildChat={chatStates[app.id]}
                    isOpeningChat={openingChatAppId === app.id}
                    isLaunching={launchingAppId === app.id}
                    isExporting={exportingAppId === app.id}
                  />
                ))}
              </div>
            )}
          </ReadableContent>
        </SearchView>
      </div>

      {appToExport && (
        <ExportAppDialog
          key={appToExport.id}
          app={appToExport}
          browser={isBrowserSurface()}
          onClose={() => setAppToExport(null)}
          onExport={(request) => exportApp(appToExport, request)}
        />
      )}

      <ConfirmationModal
        isOpen={appToDelete !== null}
        title={`Delete "${appToDelete?.title}"?`}
        message="This permanently removes the app and its files from disk. This action cannot be undone."
        confirmLabel="Delete"
        cancelLabel="Cancel"
        confirmVariant="destructive"
        isSubmitting={isDeleting}
        onConfirm={confirmDelete}
        onCancel={() => setAppToDelete(null)}
      />
    </MainPanelLayout>
  );
}

// ---------------------------------------------------------------------------
// Inline app row component
// ---------------------------------------------------------------------------
interface ApplicationItemProps {
  app: AppManifest;
  onLaunch: () => void;
  onOpenConversation: () => void;
  onExport: () => void;
  onDelete: () => void;
  /**
   * Where the chat button leads (`buildChat.ts`). Absent means still being
   * checked, which renders like an open chat: the click re-checks anyway.
   */
  buildChat?: BuildChatState;
  isOpeningChat?: boolean;
  isLaunching?: boolean;
  isExporting?: boolean;
}

export function ApplicationItem({
  app,
  onLaunch,
  onOpenConversation,
  onExport,
  onDelete,
  buildChat,
  isOpeningChat = false,
  isLaunching = false,
  isExporting = false,
}: ApplicationItemProps) {
  if (typeof app.broken === 'string') {
    return <BrokenApplicationItem app={app} reason={app.broken} onDelete={onDelete} />;
  }
  const chat: BuildChatState = !app.session_id
    ? { kind: 'none' }
    : (buildChat ?? { kind: 'checking' });
  const chatLabel = buildChatLabel(chat, app.title);
  const chatEnabled = buildChatEnabled(chat);
  const chatNote = buildChatNote(chat);
  const model = app.agent?.model?.model;
  const kb = app.agent?.knowledge_base;
  const actionCount = app.surface?.actions?.length ?? 0;
  const signalCount = app.surface?.signals?.length ?? 0;
  const plural = (n: number, noun: string) => `${n} ${noun}${n === 1 ? '' : 's'}`;
  const surfaceSummary = [
    ...(actionCount > 0 ? [plural(actionCount, 'action')] : []),
    ...(signalCount > 0 ? [plural(signalCount, 'signal')] : []),
  ].join(' · ');
  return (
    <div
      className="biorouter-list-row flex items-start py-3 px-3 group gap-3"
      aria-busy={isLaunching || isExporting || isOpeningChat}
    >
      {/* ⚠ Every box down this column carries `min-w-0`. A flex item's
          `min-width` is `auto`, which resolves to its CONTENT's minimum — so an
          app id, a model name or a KB name with no break opportunity pushes the
          column wider than the row and bleeds past the reading measure.
          `break-words` does not help: it changes where a line MAY break, not
          the min-content width the flex algorithm reads. The title, the model
          and the KB additionally `truncate` (which also zeroes that automatic
          minimum, because the overflow is no longer `visible`) and carry a
          `title` so the whole value is still reachable. */}
      <div className="flex-1 min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="min-w-0 truncate text-label text-text-default" title={app.title}>
            {app.title}
          </p>
          <Badge>{app.kind}</Badge>
          {surfaceSummary && (
            <Badge title="Declared app surface: verbs the agent can call and signals it can subscribe to">
              {surfaceSummary}
            </Badge>
          )}
        </div>
        {app.description && (
          <p className="mt-0.5 line-clamp-1 text-supporting text-text-muted">{app.description}</p>
        )}
        <div className="mt-1 flex min-w-0 flex-wrap items-center gap-3 text-supporting text-text-subtle">
          <span className="flex shrink-0 items-center whitespace-nowrap">
            <Calendar className="w-3 h-3 mr-1" />
            Created {formatDate(app.created_at)}
          </span>
          <span className="flex shrink-0 items-center whitespace-nowrap">
            <Clock className="w-3 h-3 mr-1" />
            Updated {formatDate(app.updated_at)}
          </span>
          {model && (
            <span className="min-w-0 truncate font-mono" title={model}>
              {model}
            </span>
          )}
          {kb && (
            <span className="min-w-0 truncate font-mono" title={kb}>
              KB: {kb}
            </span>
          )}
          {/* Visible without hovering: the action strip is hidden at `sm+`
              until the row is hovered or focused, so a grayed chat button alone
              would not tell anyone the chat is gone. */}
          {chatNote && (
            <span className="flex shrink-0 items-center whitespace-nowrap text-text-muted">
              <MessageSquare className="w-3 h-3 mr-1" />
              {chatNote}
            </span>
          )}
        </div>
      </div>
      {/* V7 — glyph-only row actions are `ghost` + `round` (the 32px rung the
          shape carries itself), never a `size="sm"` button re-geometried to
          28px with `h-7 w-7 p-0`, and never a `hover:bg-*`: `tint-interactive`
          inside the variant owns hover and press. Delete is the quiet
          destructive spelling — ghost with `text-text-danger` — because this is
          the ONE place an Agent Drafter app can be deleted at all (the in-chat
          artifact card deliberately has no delete control), so it must read as
          consequential without becoming the loudest thing in the row. */}
      <div className="flex items-center gap-1 flex-shrink-0 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
        <Button
          onClick={onLaunch}
          variant="ghost"
          shape="round"
          title="Launch in browser"
          aria-label={`Launch ${app.title} in browser`}
          disabled={isLaunching}
        >
          <Play />
        </Button>
        {/* Always rendered, so every row's strip has one shape and an absent
            chat is explained rather than missing. ⚠ `aria-disabled`, never
            `disabled`: a native disabled button gets `pointer-events: none`, so
            its tooltip never shows, and it leaves the tab order — the user
            would see a gray icon with no way to learn why. The click still
            reaches `onOpenConversation`, which re-checks and says why in a
            toast (or opens the chat, if it has come back). While the click's
            own check is out, the glyph spins: the answer can take a moment, and
            a control that shows nothing invites a second click or a walk away.
            The name stays put, so a screen reader is not re-announced a
            different control under focus. */}
        <Button
          onClick={onOpenConversation}
          variant="ghost"
          shape="round"
          title={chatLabel}
          aria-label={chatLabel}
          aria-disabled={chatEnabled ? undefined : true}
          aria-busy={isOpeningChat || undefined}
          className={
            isOpeningChat
              ? 'cursor-wait'
              : chatEnabled
                ? undefined
                : 'opacity-50 cursor-not-allowed'
          }
        >
          {isOpeningChat ? <Loader2 className="animate-spin" /> : <MessageSquare />}
        </Button>
        <Button
          onClick={onExport}
          variant="ghost"
          shape="round"
          title="Export to a folder"
          aria-label={`Export ${app.title} to a folder`}
          disabled={isExporting}
        >
          <Download />
        </Button>
        <Button
          onClick={onDelete}
          variant="ghost"
          shape="round"
          className="text-text-danger"
          title="Delete this app"
          aria-label={`Delete ${app.title}`}
        >
          <Trash2 />
        </Button>
      </div>
    </div>
  );
}

/**
 * A row for an app whose manifest the daemon could not read. Launch, the chat
 * and Export all need the manifest, so they are left out rather than offered and
 * failing; Delete needs only the id, and is the one way out. The daemon used to
 * drop such an app from the list, which left it on disk with no way to see or
 * remove it from the app.
 */
function BrokenApplicationItem({
  app,
  reason,
  onDelete,
}: {
  app: AppManifest;
  reason: string;
  onDelete: () => void;
}) {
  return (
    <div className="biorouter-list-row flex items-start py-3 px-3 group gap-3">
      <div className="flex-1 min-w-0">
        <div className="flex min-w-0 items-center gap-1.5">
          <p className="min-w-0 truncate text-label text-text-default" title={app.title}>
            {app.title}
          </p>
          <Badge tone="danger">Can't be read</Badge>
        </div>
        <p className="mt-0.5 min-w-0 break-words text-supporting text-text-danger">
          Biorouter can't read this app's manifest, so it can't be launched or exported. Delete it
          to remove its files.
          {/* `break-all`: the reason is usually a path or a parser message with
              no break opportunity, which would otherwise push past the row. */}
          {reason && <span className="block break-all font-mono text-text-muted">{reason}</span>}
        </p>
      </div>
      <div className="flex items-center gap-1 flex-shrink-0 opacity-100 transition-opacity sm:opacity-0 sm:group-hover:opacity-100 sm:group-focus-within:opacity-100">
        <Button
          onClick={onDelete}
          variant="ghost"
          shape="round"
          className="text-text-danger"
          title="Delete this app"
          aria-label={`Delete ${app.title}`}
        >
          <Trash2 />
        </Button>
      </div>
    </div>
  );
}
