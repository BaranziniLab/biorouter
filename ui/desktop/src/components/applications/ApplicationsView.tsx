import { useCallback, useEffect, useRef, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Play, RefreshCw, Trash2 } from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Badge } from '../ui/badge';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { FilterInput } from '../ui/filter-input';
import { InfoTip } from '../ui/info-tip';
import { Note } from '../ui/note';
import { ScrollArea } from '../ui/scroll-area';
import { Skeleton } from '../ui/skeleton';
import { IconAction, RowActions, RowContextMenu, type RowActionItem } from '../ui/row-actions';
import { toastSuccess, toastError, toastWarning } from '../../toasts';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { fromUnixSeconds, rowTime, rowTimeTitle } from '../workflows/shared/rowTime';
import {
  configuredBaseUrl,
  deleteAgentDrafterApp,
  exportedMessage,
  exportNotesMessage,
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
  buildChatNote,
  buildChatRefusal,
  buildChatTarget,
  resolveBuildChat,
  resolveBuildChats,
  type BuildChatState,
} from './buildChat';
import { appsCopy as copy } from './copy';

/** Loading is rows that are the shape of rows, not a line of prose in dead space. */
function ApplicationItemSkeleton() {
  return (
    <div className="biorouter-list-row flex items-center gap-3">
      <div className="min-w-0 flex-1">
        <Skeleton className="h-4 w-48" />
        <Skeleton className="mt-1 h-3 w-64" />
      </div>
    </div>
  );
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
  // Where each app's chat item leads, keyed by app id (`buildChat.ts`). A row
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
  // say this: leaving the page advances no ticket, and react-router's
  // `navigate` keeps working after its component unmounts, so a slow answer
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
      // Not awaited: the rows show at once and each chat item settles as its
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
        // The item led to the parent of the subagent run that built the app.
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
      // browser opened); `failureSentence` takes off Electron's IPC preamble.
      toastError({ title: app.title, msg: failureSentence(err, copy.couldNotOpen) });
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
            : undefined) ?? copy.buildChat.stillChecking,
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
      msg: buildChatRefusal(state) ?? copy.buildChat.couldNotOpen,
    });
  };

  // Export runs where the files can be written: the main process picks the folder FIRST, has
  // this window's daemon build the export and unpacks it, launchers executable
  // (`utils/appExport.ts`); a `biorouter serve` tab downloads the daemon's zip instead
  // (`exportAppInThisBrowser`). The renderer neither picks a folder nor writes a file. The
  // dialog stays open until this answers, so it can show the harness's findings and offer to
  // export anyway.
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
        // skill that is not installed, an older background service. Never folded into success,
        // and never clamped: a note cut at three lines would hide which item was left out.
        const notes = exportNotesMessage(result.summary.notes);
        if (notes) {
          toastWarning({ title: app.title, msg: notes, clampMessage: false });
        }
      } else if (result.status === 'failed') {
        toastError({ title: app.title, msg: result.message });
      }
      return result;
    } catch (err) {
      console.error('Failed to export app:', err);
      const message = failureSentence(err, copy.couldNotExport);
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
      toastSuccess({ title: copy.deleted(app.title) });
    } catch (err) {
      console.error('Failed to delete app:', err);
      toastError({
        title: app.title,
        msg: copy.couldNotDelete(err instanceof Error ? err.message : undefined),
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
    <MainPanelLayout removeTopPadding>
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {/* The band (spec 3.10) is fixed; only the body scrolls. Its actions
            are the filter (⌘F focuses it) and Refresh as a ghost round icon,
            the same glyph as the Scheduler's. There is no primary: an app is
            built from a chat, so this view has nothing to create. */}
        <PageHeader
          title={copy.title}
          info={copy.info}
          actions={
            <>
              <FilterInput
                value={searchTerm}
                onValueChange={setSearchTerm}
                label={copy.filterLabel}
              />
              <PageHeaderAction icon={RefreshCw} label={copy.refresh} onClick={load} />
            </>
          }
        />

        <ReadableContent size="chat" className="relative min-h-0 flex-1 px-6 pt-2">
          <ScrollArea className="h-full">
            <div className="relative pb-6">
              {/* ⚠ The error is rendered whether or not there are apps to show.
                  It used to be an `apps.length === 0` branch, so a refresh that
                  failed while rows were already on screen said nothing at all and
                  the list silently went stale. Retry rides the note. */}
              {error && (
                <Note
                  tone="danger"
                  role="alert"
                  className="mb-4"
                  action={
                    <Button variant="secondary" size="sm" onClick={load}>
                      {copy.retry}
                    </Button>
                  }
                >
                  {copy.loadFailed(error)}
                </Note>
              )}

              {loading && apps.length === 0 && (
                <div className="biorouter-list-shell br-rows-flat" aria-hidden>
                  <ApplicationItemSkeleton />
                  <ApplicationItemSkeleton />
                  <ApplicationItemSkeleton />
                </div>
              )}

              {/* Two states, decided by what there is to filter, as Workflows decides them
                  (V3-R7-2). With no apps at all the empty state stands whatever is typed in the
                  filter: a filter over nothing has nothing to say "no match" about. With apps
                  and a filter that matches none, the no-match tile is compact, so it sits where
                  the sibling views' no-match tiles sit and does not jump 84 px between them. */}
              {!loading && !error && apps.length === 0 && (
                <EmptyState
                  icon={ENTITY_ICONS.application}
                  title={copy.emptyTitle}
                  description={copy.emptyDescription}
                />
              )}

              {!loading && !error && apps.length > 0 && filtered.length === 0 && (
                <EmptyState
                  icon={ENTITY_ICONS.application}
                  title={copy.noMatchTitle}
                  description={copy.noMatchDescription}
                  compact
                />
              )}

              {/* ⚠ Deliberately NOT `!loading && filtered.length > 0`. `load()`
                  sets `loading` on every Refresh as well as on first mount, so
                  gating the list on it would UNMOUNT the rows for the length of
                  every request. The three branches stay mutually exclusive
                  without it. */}
              {filtered.length > 0 && (
                <div className="biorouter-list-shell br-rows-flat">
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
            </div>
          </ScrollArea>
        </ReadableContent>
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
        title={copy.deleteTitle(appToDelete?.title ?? '')}
        message={copy.deleteMessage}
        confirmLabel={copy.deleteConfirm}
        cancelLabel={copy.cancel}
        confirmVariant="destructive"
        isSubmitting={isDeleting}
        onConfirm={confirmDelete}
        onCancel={() => setAppToDelete(null)}
      />
    </MainPanelLayout>
  );
}

// ---------------------------------------------------------------------------
// One built app, as a two-line row
// ---------------------------------------------------------------------------
interface ApplicationItemProps {
  app: AppManifest;
  onLaunch: () => void;
  onOpenConversation: () => void;
  onExport: () => void;
  onDelete: () => void;
  /**
   * Where the chat item leads (`buildChat.ts`). Absent means still being
   * checked, which renders like an open chat: the click re-checks anyway.
   */
  buildChat?: BuildChatState;
  isOpeningChat?: boolean;
  isLaunching?: boolean;
  isExporting?: boolean;
}

/**
 * One app (spec 3.10): the title and its kind badge, then ONE meta line, "{model} · {kb}", in
 * the sans face (the description when the app names neither). At rest the row shows only its
 * relative "Updated" time; on hover or focus that gives way to Launch and `⋯` (Open the chat it
 * was built in, Export…, Delete). Right-click and Shift+F10 open the same menu.
 *
 * The chat item is checked against the daemon (`buildChat.ts`). When the chat is gone, the item
 * is unavailable and says why in its own words ("Build chat deleted"), and the meta line says the
 * same at rest, so nobody has to open the menu to learn it. Refresh checks again.
 *
 * Delete lives HERE and nowhere else: deleting an Agent Drafter app removes its files from disk,
 * and the in-chat artifact card deliberately has no delete control (CLAUDE.md).
 *
 * The kind badge carries the app's declared surface ("2 actions · 1 signal") as its help, in
 * Crew's ClassificationBadge pattern: the badge itself is the focusable trigger.
 *
 * ⚠ Every box down the text column carries `min-w-0`: a flex item's `min-width` is `auto`,
 * which resolves to its content's minimum, so an unbroken model or KB name would push the
 * column past the reading measure.
 */
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
  const chatEnabled = buildChatEnabled(chat);
  const chatNote = buildChatNote(chat);
  const model = app.agent?.model?.model;
  const kb = app.agent?.knowledge_base;
  const surface = copy.surface(
    app.surface?.actions?.length ?? 0,
    app.surface?.signals?.length ?? 0
  );
  const meta = [model, kb].filter(Boolean).join(' · ') || app.description;
  const updated = fromUnixSeconds(app.updated_at);
  const created = fromUnixSeconds(app.created_at);
  const updatedText = rowTime(updated);
  const timeTitle = [
    updated ? copy.updated(rowTimeTitle(updated) ?? '') : null,
    created ? copy.created(rowTimeTitle(created) ?? '') : null,
  ]
    .filter(Boolean)
    .join('\n');

  // An unavailable chat names its reason instead of the action; a run that only a subagent
  // left behind says so.
  const chatItem: RowActionItem = chatEnabled
    ? {
        label:
          chat.kind === 'available' && chat.via === 'subagent'
            ? copy.openSubagentRun
            : copy.openChat,
        onSelect: onOpenConversation,
        disabled: isOpeningChat,
      }
    : { label: chatNote ?? copy.openChat, onSelect: onOpenConversation, disabled: true };

  const menu: RowActionItem[] = [
    chatItem,
    { label: copy.exportItem, onSelect: onExport, disabled: isExporting },
    { kind: 'separator' },
    { label: copy.delete, onSelect: onDelete, destructive: true },
  ];

  return (
    <RowContextMenu items={menu}>
      <div
        className="biorouter-list-row flex items-center gap-3"
        aria-busy={isLaunching || isExporting || isOpeningChat}
        data-testid="app-row"
      >
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h3 className="min-w-0 truncate text-label text-text-default">{app.title}</h3>
            {surface ? (
              <InfoTip label={app.kind} help={surface} asChild>
                <Badge>{app.kind}</Badge>
              </InfoTip>
            ) : (
              <Badge>{app.kind}</Badge>
            )}
          </div>
          {(meta || chatNote) && (
            <p className="min-w-0 truncate text-supporting text-text-muted">
              {meta}
              {meta && chatNote ? ' · ' : null}
              {/* Visible at rest: the menu is out of sight until the row is hovered, so an
                  unavailable item alone would not tell anyone the chat is gone. */}
              {chatNote ? <span data-testid="app-build-chat-note">{chatNote}</span> : null}
            </p>
          )}
        </div>
        <RowActions
          primary={
            // ⚠ Not `disabled` while the launch is out: a focused button that turns `disabled`
            // hands focus to <body> in Chromium. `launch` refuses a second press for the same
            // app; the row's `aria-busy` says it is working and `.br-action-busy` dims the action.
            <IconAction
              icon={Play}
              label={copy.launchNamed(app.title)}
              tooltip={copy.launch}
              onSelect={onLaunch}
              className={isLaunching ? 'br-action-busy' : undefined}
            />
          }
          menu={menu}
          menuLabel={copy.moreActionsNamed(app.title)}
          meta={
            updatedText ? (
              <time
                className="tabular-nums"
                dateTime={updated?.toISOString()}
                title={timeTitle || undefined}
              >
                {updatedText}
              </time>
            ) : undefined
          }
        />
      </div>
    </RowContextMenu>
  );
}

/**
 * A row for an app whose manifest the daemon could not read. Launch, the chat and Export all
 * need the manifest, so they are left out rather than offered and failing; Delete needs only the
 * id, and is the one way out, so it is the row's one action. The daemon used to drop such an app
 * from the list, which left it on disk with no way to see or remove it from the app.
 *
 * Two lines, like every row (principle 4): the title with a danger "Can’t be read" badge, then
 * the parser's own reason. What that means for the app (it can be neither launched nor exported)
 * is the badge's help, the way a kind badge carries its surface: the state is said at rest by
 * the badge and the reason, and the row's one action already shows what is left.
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
  const menu: RowActionItem[] = [{ label: copy.delete, onSelect: onDelete, destructive: true }];
  return (
    <RowContextMenu items={menu}>
      <div className="biorouter-list-row flex items-center gap-3" data-testid="app-row">
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-2">
            <h3 className="min-w-0 truncate text-label text-text-default">{app.title}</h3>
            <InfoTip label={copy.broken.badge} help={copy.broken.message} asChild>
              <Badge tone="danger">{copy.broken.badge}</Badge>
            </InfoTip>
          </div>
          {/* `break-all`: the reason is usually a path or a parser message with no break
              opportunity, which would otherwise push past the row. Mono: it is the parser's
              machine string, not a sentence written for the row. */}
          {reason && (
            <p
              className="min-w-0 break-all font-mono text-supporting text-text-muted"
              data-testid="app-broken-reason"
            >
              {reason}
            </p>
          )}
        </div>
        <RowActions
          primary={
            <IconAction
              icon={Trash2}
              label={copy.broken.deleteNamed(app.title)}
              tooltip={copy.delete}
              onSelect={onDelete}
            />
          }
        />
      </div>
    </RowContextMenu>
  );
}
