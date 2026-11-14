import { useEffect } from 'react';
import { Button } from '../ui/button';
import { CopyField } from '../ui/copy-field';
import { Disclosure } from '../ui/disclosure';
import { InfoTip } from '../ui/info-tip';
import { Progress } from '../ui/progress';
import { Spinner } from '../ui/spinner';
import { ModalShell } from '../ModalShell';
import { AlertCircle, Download, ExternalLink, Rocket } from '../icons/app-icons';
import { toastSuccess } from '../../toasts';
import {
  UPDATE_REQUESTED_KEY,
  type ServeUpdateStore,
  type ServeUpdateView,
} from '../../utils/serveUpdate';
import { serveUpdateCopy as copy, DOWNLOAD_WEBSITE_URL } from './copy';
import { useServeUpdate } from './useServeUpdate';

/** What the dialog is showing, decided from the view alone (pure, for tests). */
export type DialogMode =
  | 'result'
  | 'restarting'
  | 'installing'
  | 'checking'
  | 'downloading'
  | 'ready'
  | 'needs_admin'
  | 'installed'
  | 'manual'
  | 'error'
  | null;

export function dialogMode(view: ServeUpdateView): DialogMode {
  if (view.result) return 'result';
  if (view.reconnecting) return 'restarting';
  switch (view.status?.phase) {
    case 'restarting':
      return 'restarting';
    case 'installing':
      return 'installing';
    case 'checking':
      return 'checking';
    case 'downloading':
      return 'downloading';
    case 'ready':
    case 'needs_admin':
    case 'installed':
    case 'manual':
    case 'error':
      return view.status.phase;
    default:
      return null;
  }
}

function sayUpdatedOnce(view: ServeUpdateView) {
  const result = view.status?.lastResult;
  if (!result || result.outcome !== 'updated') return;
  let requested = false;
  try {
    requested = window.sessionStorage.getItem(UPDATE_REQUESTED_KEY) === '1';
    if (requested) window.sessionStorage.removeItem(UPDATE_REQUESTED_KEY);
  } catch {
    // No session storage: nothing to say, and nothing lost.
  }
  if (requested) toastSuccess({ title: copy.updatedToast(result.to) });
}

/**
 * The update dialog of a `biorouter serve` page (SD-19): the browser's
 * counterpart of the desktop's `UpdateAvailableModal`, opened from the sidebar
 * row, from Settings, or on its own while Biorouter restarts. Renders nothing
 * outside a browser page.
 */
export default function ServeUpdateDialog({ store: given }: { store?: ServeUpdateStore | null }) {
  const { store, view } = useServeUpdate(given);

  useEffect(() => {
    sayUpdatedOnce(view);
  }, [view.status?.lastResult?.at]); // eslint-disable-line react-hooks/exhaustive-deps

  if (!store) return null;
  const mode = dialogMode(view);
  if (!mode) return null;
  const status = view.status;
  const latest = status?.latest ?? null;
  const runningChats = status?.runningTurns ?? 0;
  const runningJobs = status?.runningWork ?? 0;
  const running = runningChats + runningJobs;

  const restartAndUpdate = () => void store.install(running > 0);
  const openSite = () => window.open(DOWNLOAD_WEBSITE_URL, '_blank', 'noopener,noreferrer');

  const titles: Record<NonNullable<DialogMode>, string> = {
    result: copy.title.rolledBack,
    restarting: copy.title.restarting,
    installing: copy.title.installing,
    checking: copy.title.checking,
    downloading: copy.title.downloading,
    ready: copy.title.ready,
    needs_admin: copy.title.available,
    installed: copy.title.installed,
    manual: copy.title.available,
    error: copy.title.failed,
  };
  const title = titles[mode];

  const icon =
    mode === 'ready' || mode === 'installed' || mode === 'installing' || mode === 'restarting' ? (
      <Rocket className="w-5 h-5 flex-none text-background-accent" />
    ) : mode === 'error' || mode === 'result' ? (
      // Red is rare: a failed update is news, not danger.
      <AlertCircle className="w-5 h-5 flex-none text-text-muted" />
    ) : (
      <Download className="w-5 h-5 flex-none text-background-accent" />
    );

  const busyButton = (label: string) => (
    <Button variant="default" size="sm" disabled className="flex items-center gap-2">
      <Spinner />
      {label}
    </Button>
  );

  const footer = (() => {
    switch (mode) {
      case 'result':
        return (
          <Button variant="default" size="sm" onClick={store.closeDialog}>
            {copy.button.close}
          </Button>
        );
      case 'restarting':
        return busyButton(copy.title.restarting);
      case 'installing':
        return busyButton(copy.title.installing);
      case 'checking':
      case 'downloading':
        return (
          <Button variant="ghost" size="sm" onClick={store.closeDialog}>
            {copy.button.notNow}
          </Button>
        );
      case 'ready':
        return (
          <>
            <Button variant="ghost" size="sm" onClick={store.closeDialog}>
              {copy.button.later}
            </Button>
            <Button
              variant="default"
              size="sm"
              onClick={restartAndUpdate}
              className="flex items-center gap-2"
            >
              <Rocket className="w-4 h-4" />
              {copy.button.restartAndUpdate}
            </Button>
          </>
        );
      case 'installed':
        return (
          <>
            <Button variant="ghost" size="sm" onClick={store.closeDialog}>
              {copy.button.later}
            </Button>
            <Button variant="default" size="sm" onClick={restartAndUpdate}>
              {copy.button.restart}
            </Button>
          </>
        );
      case 'needs_admin':
        return (
          <>
            <Button variant="ghost" size="sm" onClick={store.closeDialog}>
              {copy.button.notNow}
            </Button>
            <Button variant="default" size="sm" onClick={() => void store.check()}>
              {copy.button.checkAgain}
            </Button>
          </>
        );
      case 'manual':
        return status?.manualReason === 'source' ? (
          <Button variant="default" size="sm" onClick={store.closeDialog}>
            {copy.button.close}
          </Button>
        ) : (
          <>
            <Button variant="ghost" size="sm" onClick={store.closeDialog}>
              {copy.button.notNow}
            </Button>
            <Button
              variant="default"
              size="sm"
              onClick={openSite}
              className="flex items-center gap-2"
            >
              <ExternalLink className="w-4 h-4" />
              {copy.button.downloadFromSite}
            </Button>
          </>
        );
      case 'error':
        return (
          <>
            <Button variant="ghost" size="sm" onClick={store.closeDialog}>
              {copy.button.notNow}
            </Button>
            <Button variant="default" size="sm" onClick={() => void store.check()}>
              {copy.button.tryAgain}
            </Button>
          </>
        );
    }
  })();

  const showVersions = mode !== 'result' && mode !== 'restarting' && (status?.current || latest);

  return (
    <ModalShell
      open={view.dialogOpen}
      onOpenChange={(open) => (open ? store.openDialog() : store.closeDialog())}
      size="md"
      purpose="info"
      title={
        <span className="flex items-center gap-2">
          {icon}
          {title}
        </span>
      }
      footer={footer}
    >
      <div className="py-3 space-y-3" data-testid="serve-update-dialog" data-mode={mode}>
        {showVersions ? (
          <div className="bg-background-muted rounded-element px-3 py-2 space-y-1">
            {status?.current ? (
              <div className="flex items-center justify-between">
                <span className="text-supporting text-text-muted">{copy.current}</span>
                <span className="text-code font-mono text-text-default">{status.current}</span>
              </div>
            ) : null}
            {latest ? (
              <div className="flex items-center justify-between">
                <span className="text-supporting text-text-muted">
                  {copy.newVersion}
                  {status?.feedOverridden ? ` · ${copy.testFeed}` : ''}
                </span>
                <span className="text-code font-mono font-semibold text-text-default">
                  {latest}
                </span>
              </div>
            ) : null}
          </div>
        ) : null}

        {mode === 'result' && view.result ? (
          <>
            <p className="text-body text-text-default">
              {view.result.outcome === 'rolled_back'
                ? copy.rolledBackLine(view.result.from, view.result.to)
                : copy.failedLine(view.result.from, view.result.to)}
            </p>
            {view.result.reason ? (
              <Disclosure label={copy.whatHappened}>
                <p className="text-supporting font-mono text-text-muted break-words">
                  {view.result.reason}
                </p>
              </Disclosure>
            ) : null}
          </>
        ) : null}

        {mode === 'restarting' ? (
          <p className="text-body text-text-default">
            {view.waitedSeconds >= 90 ? copy.slowLine : copy.restartingLine}
          </p>
        ) : null}

        {mode === 'installing' ? (
          <p className="text-body text-text-default">{copy.installingLine(latest)}</p>
        ) : null}

        {mode === 'downloading' ? (
          <div className="space-y-2">
            <p className="text-body text-text-default">{copy.downloadingLine}</p>
            <Progress
              label={copy.downloadingLabel(latest)}
              value={status?.percent ?? 0}
              minVisiblePercent={4}
            />
            <p className="text-supporting text-text-muted text-right tabular-nums">
              {status?.percent ?? 0}%
            </p>
          </div>
        ) : null}

        {mode === 'ready' ? (
          <>
            <p className="text-body text-text-default flex items-center gap-1">
              <span>{copy.readyLine(latest)}</span>
              <InfoTip label={copy.title.ready} help={copy.readyHelp} />
            </p>
            {running > 0 ? (
              <p className="text-body text-text-default" data-testid="serve-update-running">
                {copy.runningLine(runningChats, runningJobs)}
              </p>
            ) : null}
          </>
        ) : null}

        {mode === 'installed' ? (
          <>
            <p className="text-body text-text-default">{copy.installedLine(latest)}</p>
            {running > 0 ? (
              <p className="text-body text-text-default" data-testid="serve-update-running">
                {copy.runningLine(runningChats, runningJobs)}
              </p>
            ) : null}
          </>
        ) : null}

        {mode === 'needs_admin' ? (
          <>
            <p className="text-body text-text-default flex items-center gap-1">
              <span>{copy.adminLine}</span>
              <InfoTip label={copy.adminLine} help={copy.adminHelp} />
            </p>
            {status?.adminCommand ? (
              <CopyField
                value={status.adminCommand}
                label={copy.adminCommandLabel}
                truncate="end"
              />
            ) : null}
          </>
        ) : null}

        {mode === 'manual' ? (
          <p className="text-body text-text-default">
            {copy.manual[status?.manualReason ?? 'unrecognised'] ?? copy.manual.unrecognised}
          </p>
        ) : null}

        {mode === 'error' ? (
          // One line, in ink that is not red: a failed update is news, not danger.
          <p className="text-body text-text-muted break-words">
            {status?.error || copy.failedFallback}
          </p>
        ) : null}

        {view.actionError ? (
          <p className="text-supporting text-text-muted" role="status">
            {view.actionError}
          </p>
        ) : null}
      </div>
    </ModalShell>
  );
}
