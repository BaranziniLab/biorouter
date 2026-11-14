import type { ReactNode } from 'react';
import { Download } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Progress } from '../ui/progress';
import { SettingRow } from '../ui/setting-row';
import { SidebarMenuItem } from '../ui/sidebar';
import { sidebarCopy } from '../BioRouterSidebar/copy';
import { offersUpdate, type ServeUpdateStore, type ServeUpdateView } from '../../utils/serveUpdate';
import { serveUpdateCopy as copy } from './copy';
import { useServeUpdate } from './useServeUpdate';

/**
 * The lower-left "Update to …" row on a `biorouter serve` page (SD-19): the same
 * row, words and accessible name as the desktop's, opening the browser's update
 * dialog instead of the desktop one.
 */
export function ServeSidebarUpdateRow({ store: given }: { store?: ServeUpdateStore | null }) {
  const { store, view } = useServeUpdate(given);
  if (!store || !offersUpdate(view)) return null;
  const latest = view.status?.latest ?? null;
  const versionLabel = latest ? ` to ${latest}` : '';
  return (
    <SidebarMenuItem>
      <button
        type="button"
        data-testid="sidebar-update-button"
        data-tone="update"
        aria-label={`Update Biorouter${versionLabel}`}
        onClick={store.openDialog}
        className="br-nav-row no-drag"
      >
        <Download className="br-nav-row-icon" aria-hidden />
        <span className="br-nav-row-label">{sidebarCopy.update.row(latest)}</span>
      </button>
    </SidebarMenuItem>
  );
}

/** Settings › About › Version's status line on a `serve` page (pure, for tests). */
export function serveStatusLine(view: ServeUpdateView): ReactNode {
  const status = view.status;
  if (view.reconnecting) return copy.settings.restarting;
  // A refusal (a daemon that cannot update itself, a check that failed) is said
  // here, so the action never looks dead.
  if (view.actionError) return view.actionError;
  switch (status?.phase) {
    case 'checking':
      return copy.settings.checking;
    case 'downloading':
      return (
        <span className="flex flex-col gap-1">
          <span>
            {copy.settings.downloading(status.latest)}{' '}
            <span className="tabular-nums">{status.percent ?? 0}%</span>
          </span>
          <Progress
            size="thin"
            className="max-w-xs"
            label={copy.downloadingLabel(status.latest)}
            value={status.percent ?? 0}
            minVisiblePercent={4}
          />
        </span>
      );
    case 'ready':
      return copy.settings.ready(status.latest);
    case 'needs_admin':
      return copy.settings.needsAdmin(status.latest);
    case 'installed':
      return copy.settings.installed(status.latest);
    case 'manual':
      return status.manualReason === 'no_package'
        ? view.checkRequested
          ? copy.settings.upToDate
          : undefined
        : copy.settings.manual(status.latest);
    case 'installing':
    case 'restarting':
      return copy.settings.restarting;
    case 'up_to_date':
      return view.checkRequested ? copy.settings.upToDate : undefined;
    case 'error':
      return view.checkRequested || offersUpdate(view) ? copy.settings.failed : undefined;
    default:
      return undefined;
  }
}

/**
 * Settings › App › About › Version on a `serve` page: the daemon's own version
 * (the desktop's reads the app's, which a browser does not have) and one
 * action, checking or opening the update dialog.
 */
export function ServeUpdateSection({ store: given }: { store?: ServeUpdateStore | null }) {
  const { store, view } = useServeUpdate(given);
  if (!store) return null;
  const phase = view.status?.phase;
  const busy = phase === 'checking' || phase === 'downloading' || phase === 'installing';
  if (view.status && !view.status.available) {
    // A daemon `serve` did not start with the restart contract (an older
    // `serve`, or a daemon started by hand): it updates with its installation.
    return (
      <SettingRow
        label="Version"
        help={copy.settings.unavailableHelp}
        value={view.status.version ?? undefined}
        valueMono
        data-testid="settings-version-row"
      />
    );
  }
  const offered = offersUpdate(view) && !busy;
  const actionLabel =
    phase === 'ready' || phase === 'installed'
      ? copy.settings.restartToUpdate
      : offered
        ? copy.settings.update
        : copy.settings.checkForUpdates;
  return (
    <SettingRow
      label="Version"
      help={copy.settings.versionHelp}
      value={view.status?.current ?? view.status?.version ?? undefined}
      valueMono
      status={serveStatusLine(view)}
      data-testid="settings-version-row"
    >
      <Button
        variant="secondary"
        size="sm"
        className={busy ? 'br-action-busy' : undefined}
        aria-disabled={busy || undefined}
        aria-busy={busy || undefined}
        onClick={busy ? undefined : offered ? store.openDialog : () => void store.check()}
        aria-label={actionLabel}
      >
        {actionLabel}
      </Button>
    </SettingRow>
  );
}
