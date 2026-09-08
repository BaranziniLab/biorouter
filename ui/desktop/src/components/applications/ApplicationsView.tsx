import { useCallback, useEffect, useState } from 'react';
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
} from '../icons/app-icons';
import { ENTITY_ICONS } from '../icons/entity-icons';
import { Badge } from '../ui/badge';
import { ConfirmationModal } from '../ui/ConfirmationModal';
import { EmptyState } from '../ui/empty-state';
import { Note } from '../ui/note';
import { Skeleton } from '../ui/skeleton';
import { SearchView } from '../conversation/SearchView';
import { getSearchShortcutText } from '../../utils/keyboardShortcuts';
import { toastSuccess, toastError } from '../../toasts';
import { PageHeader } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import {
  appUrl,
  buildExportUrl,
  configuredBaseUrl,
  deleteAgentDrafterApp,
  requireOk,
  secretHeader,
} from './appManagement';
import type { AppManifest, ExportOptions } from './appManagement';
import ExportAppDialog from './ExportAppDialog';

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

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const res = await fetch(`${configuredBaseUrl()}/apps`, { headers: await secretHeader() });
      await requireOk(res);
      const data: AppManifest[] = await res.json();
      // Most recently updated first.
      data.sort((a, b) => (b.updated_at ?? 0) - (a.updated_at ?? 0));
      setApps(data);
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load apps');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  // Launch opens the app's own served URL in the real browser — archetype-
  // agnostic by construction (nothing here assumes a chat-shaped app), so the
  // v2 non-chat starters need no special affordance (plan Phase 5 item 5).
  const launch = async (app: AppManifest) => {
    if (launchingAppId === app.id) return;
    const baseUrl = configuredBaseUrl();
    if (!baseUrl) {
      setError('Backend URL unavailable. Is biorouterd running?');
      return;
    }
    setLaunchingAppId(app.id);
    try {
      await window.electron.openExternal(appUrl(app.id, baseUrl));
    } catch (err) {
      console.error('Failed to open app:', err);
      toastError({ title: app.title, msg: 'Could not open the app in your browser.' });
    } finally {
      setLaunchingAppId((current) => (current === app.id ? null : current));
    }
  };

  const openConversation = (app: AppManifest) => {
    if (!app.session_id) return;
    // Reopen the chat this app was built in so the user can keep iterating.
    navigate(`/pair?resumeSessionId=${encodeURIComponent(app.session_id)}`, {
      state: { resumeSessionId: app.session_id },
    });
  };

  const exportApp = async (app: AppManifest, options: ExportOptions) => {
    if (exportingAppId === app.id) return;
    setExportingAppId(app.id);
    try {
      // Older daemons ignore the query params and return the same scaffold
      // map, so this degrades gracefully to a launcher export.
      const res = await fetch(buildExportUrl(configuredBaseUrl(), app.id, options), {
        headers: await secretHeader(),
      });
      await requireOk(res);
      const payload: { files?: Record<string, string> } = await res.json();
      const files = payload.files ?? {};
      // A real directory picker (openDirectory) so Export works the same on
      // macOS, Windows, and Linux — unlike selectFileOrDirectory, which only
      // offers directory selection on macOS.
      const picked = await window.electron.directoryChooser();
      if (picked.canceled || picked.filePaths.length === 0) return;
      const targetDir = `${picked.filePaths[0]}/${app.id}`;
      let written = 0;
      for (const [rel, content] of Object.entries(files)) {
        const ok = await window.electron.writeFile(`${targetDir}/${rel}`, content);
        if (ok) written += 1;
      }
      if (written > 0) {
        toastSuccess({ title: app.title, msg: `Exported ${written} files to ${targetDir}` });
      } else {
        toastError({ title: app.title, msg: 'Nothing was exported.' });
      }
    } catch (err) {
      console.error('Failed to export app:', err);
      toastError({ title: app.title, msg: 'Could not export the app.' });
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
                    onOpenConversation={() => openConversation(app)}
                    onExport={() => setAppToExport(app)}
                    onDelete={() => setAppToDelete(app)}
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
          onCancel={() => setAppToExport(null)}
          onConfirm={(options) => {
            const app = appToExport;
            setAppToExport(null);
            void exportApp(app, options);
          }}
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
  isLaunching?: boolean;
  isExporting?: boolean;
}

export function ApplicationItem({
  app,
  onLaunch,
  onOpenConversation,
  onExport,
  onDelete,
  isLaunching = false,
  isExporting = false,
}: ApplicationItemProps) {
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
      aria-busy={isLaunching || isExporting}
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
        {app.session_id && (
          <Button
            onClick={onOpenConversation}
            variant="ghost"
            shape="round"
            title="Open the chat where this app was built"
            aria-label={`Open the chat where ${app.title} was built`}
          >
            <MessageSquare />
          </Button>
        )}
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
