import { useCallback, useEffect, useState } from 'react';
import { useNavigate } from 'react-router-dom';
import { MainPanelLayout } from '../Layout/MainPanelLayout';
import { Button } from '../ui/button';
import { Play, RefreshCw } from '../icons/app-icons';
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
import { toastSuccess, toastError } from '../../toasts';
import { PageHeader, PageHeaderAction } from '../Layout/PageHeader';
import { ReadableContent } from '../Layout/ReadableContent';
import { fromUnixSeconds, rowTime, rowTimeTitle } from '../workflows/shared/rowTime';
import {
  buildExportUrl,
  configuredBaseUrl,
  deleteAgentDrafterApp,
  requireOk,
  secretHeader,
} from './appManagement';
import type { AppManifest, ExportOptions } from './appManagement';
import ExportAppDialog from './ExportAppDialog';
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
      toastError({ title: app.title, msg: reason || copy.couldNotOpen });
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
        toastSuccess({ title: copy.savedTo(targetDir) });
      } else {
        toastError({ title: app.title, msg: copy.nothingExported });
      }
    } catch (err) {
      console.error('Failed to export app:', err);
      toastError({ title: app.title, msg: copy.couldNotExport });
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
                <div className="biorouter-list-shell" aria-hidden>
                  <ApplicationItemSkeleton />
                  <ApplicationItemSkeleton />
                  <ApplicationItemSkeleton />
                </div>
              )}

              {!loading && !error && filtered.length === 0 && (
                <EmptyState
                  icon={ENTITY_ICONS.application}
                  title={searchTerm ? copy.noMatchTitle : copy.emptyTitle}
                  description={searchTerm ? copy.noMatchDescription : copy.emptyDescription}
                />
              )}

              {/* ⚠ Deliberately NOT `!loading && filtered.length > 0`. `load()`
                  sets `loading` on every Refresh as well as on first mount, so
                  gating the list on it would UNMOUNT the rows for the length of
                  every request. The three branches stay mutually exclusive
                  without it. */}
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
            </div>
          </ScrollArea>
        </ReadableContent>
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
  isLaunching?: boolean;
  isExporting?: boolean;
}

/**
 * One app (spec 3.10): the title and its kind badge, then ONE meta line, "{model} · {kb}", in
 * the sans face (the description when the app names neither). At rest the row shows only its
 * relative "Updated" time; on hover or focus that gives way to Launch and `⋯` (Open the chat it
 * was built in, Export…, Delete). Right-click and Shift+F10 open the same menu.
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
  isLaunching = false,
  isExporting = false,
}: ApplicationItemProps) {
  const model = app.agent?.model?.model;
  const kb = app.agent?.knowledge_base;
  const surface = copy.surface(
    app.surface?.actions?.length ?? 0,
    app.surface?.signals?.length ?? 0
  );
  const meta = [model, kb].filter(Boolean).join(' · ');
  const updated = fromUnixSeconds(app.updated_at);
  const created = fromUnixSeconds(app.created_at);
  const updatedText = rowTime(updated);
  const timeTitle = [
    updated ? copy.updated(rowTimeTitle(updated) ?? '') : null,
    created ? copy.created(rowTimeTitle(created) ?? '') : null,
  ]
    .filter(Boolean)
    .join('\n');

  const menu: RowActionItem[] = [
    ...(app.session_id ? [{ label: copy.openChat, onSelect: onOpenConversation }] : []),
    { label: copy.exportItem, onSelect: onExport, disabled: isExporting },
    { kind: 'separator' },
    { label: copy.delete, onSelect: onDelete, destructive: true },
  ];

  return (
    <RowContextMenu items={menu}>
      <div
        className="biorouter-list-row flex items-center gap-3"
        aria-busy={isLaunching || isExporting}
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
          {(meta || app.description) && (
            <p className="min-w-0 truncate text-supporting text-text-muted">
              {meta || app.description}
            </p>
          )}
        </div>
        <RowActions
          primary={
            <IconAction
              icon={Play}
              label={copy.launchNamed(app.title)}
              onSelect={onLaunch}
              disabled={isLaunching}
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
