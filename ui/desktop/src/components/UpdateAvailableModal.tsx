import { useEffect, useRef, useState } from 'react';
import { Button } from './ui/button';
import { Progress } from './ui/progress';
import { ModalShell } from './ModalShell';
import { Download, ExternalLink, AlertCircle, Rocket } from './icons/app-icons';
import { Spinner } from './ui/spinner';
import { UPDATES_ENABLED } from '../updates';
import {
  initialUpdaterState,
  reduceUpdaterEvent,
  shouldShowUpdateModal,
  stateFromSnapshot,
  normalizeVersion,
  hasKnownUpdate,
  needsAssistedDownload,
  UPDATE_PHASE_ORDER,
  type UpdaterState,
} from '../utils/updaterState';
import { OPEN_UPDATE_MODAL_EVENT } from '../utils/updateUiEvents';

const DOWNLOAD_WEBSITE_URL = 'https://biorouter.ucsf.edu/download';
const DISMISS_STORAGE_KEY = 'biorouter:update-modal-dismissed-version';

/**
 * One-click auto-update prompt.
 *
 * The Electron main process (`src/utils/autoUpdater.ts`) checks GitHub on
 * startup and every three hours. When a newer release is found, the sidebar
 * shows UPDATE while `electron-updater` downloads it in the background. This
 * modal opens when the user clicks that button and, once the update is fully
 * downloaded, offers a single **Restart & Update** action that calls
 * `installUpdate()` → `autoUpdater.quitAndInstall()`. The app quits, swaps in
 * the new version (all bundled binaries included), and relaunches — no manual
 * DMG download, drag-and-drop, or app restart required.
 *
 * User settings live in `~/.config/biorouter` and are untouched by the
 * in-place app replacement, so nothing is lost across the update.
 */
export default function UpdateAvailableModal() {
  const [state, setState] = useState<UpdaterState>(initialUpdaterState);
  const [open, setOpen] = useState(false);
  const releaseUrlRef = useRef<string>(DOWNLOAD_WEBSITE_URL);
  const currentVersion = normalizeVersion(window.electron?.getVersion?.() || '');

  useEffect(() => {
    if (!UPDATES_ENABLED) return;
    if (!window.electron?.onUpdaterEvent) return;

    let cancelled = false;

    // Subscribe to live updater events from the main process.
    const dispose = window.electron.onUpdaterEvent((payload) => {
      setState((prev) => reduceUpdaterEvent(prev, payload));
    });

    // Recover any state that was already established before this mounted (the
    // background check fires ~5s after launch and auto-downloads).
    window.electron
      .getUpdateState?.()
      ?.then((snapshot) => {
        if (cancelled || !snapshot) return;
        const recovered = stateFromSnapshot(snapshot);
        setState((prev) =>
          // Only adopt the snapshot if it's "further along" than current.
          UPDATE_PHASE_ORDER.indexOf(recovered.phase) > UPDATE_PHASE_ORDER.indexOf(prev.phase)
            ? recovered
            : prev
        );
      })
      .catch(() => {
        /* no snapshot available — rely on live events */
      });

    const handleOpenRequest = (event: Event) => {
      const requestedState = (event as CustomEvent<UpdaterState>).detail;
      if (!requestedState || !hasKnownUpdate(requestedState)) return;
      setState(requestedState);
      setOpen(true);
    };

    window.addEventListener(OPEN_UPDATE_MODAL_EVENT, handleOpenRequest);

    return () => {
      cancelled = true;
      dispose?.();
      window.removeEventListener(OPEN_UPDATE_MODAL_EVENT, handleOpenRequest);
    };
  }, []);

  const dismiss = () => {
    if (state.latestVersion) {
      localStorage.setItem(DISMISS_STORAGE_KEY, state.latestVersion);
    }
    setOpen(false);
  };

  // ⚠ Nothing in the renderer called `downloadUpdate` before this. The IPC
  // existed on both ends -- `preload.ts` invokes it, `autoUpdater.ts` handles
  // it -- and the assisted GitHub path deliberately waits for it rather than
  // writing hundreds of megabytes into Downloads on a background timer. With no
  // caller, a Windows user was told an update was "downloading in the
  // background" and watched a progress bar sit at 0% forever.
  const [downloadRequested, setDownloadRequested] = useState(false);
  const [downloadError, setDownloadError] = useState<string | null>(null);

  const handleDownload = async () => {
    setDownloadRequested(true);
    setDownloadError(null);
    try {
      const result = await window.electron?.downloadUpdate?.();
      if (result && result.success === false && result.error) {
        setDownloadError(result.error);
        setDownloadRequested(false);
      }
    } catch (err) {
      setDownloadError(err instanceof Error ? err.message : String(err));
      setDownloadRequested(false);
    }
  };

  // ⚠ Awaited, not fire-and-forget. The install used to be fired and
  // forgotten, so when the main process refused it ("Update file path not
  // found") the button simply did nothing and nobody was told. The main process
  // also reports install failures as `install-failed` events; this catches the
  // ones that are only a rejected call.
  const handleRestartAndUpdate = async () => {
    setDownloadError(null);
    try {
      const result = await window.electron?.installUpdate?.();
      if (result && result.ok === false) {
        setState((prev) =>
          reduceUpdaterEvent(prev, {
            event: 'install-failed',
            data: { message: result.error || 'The update could not be installed.' },
          })
        );
      }
    } catch (err) {
      setState((prev) =>
        reduceUpdaterEvent(prev, {
          event: 'install-failed',
          data: { message: err instanceof Error ? err.message : String(err) },
        })
      );
    }
  };

  // A failed download or install used to leave only "Download from Biorouter":
  // the next attempt was the three-hourly background check. Try again starts
  // one now. Assisted mode downloads again; otherwise a fresh check finds the
  // update and downloads it, resuming from anything already on disk.
  const handleRetry = async () => {
    setDownloadError(null);
    if (state.usingFallback) {
      await handleDownload();
      return;
    }
    setState((prev) => reduceUpdaterEvent(prev, { event: 'checking-for-update' }));
    try {
      const result = await window.electron?.checkForUpdates?.();
      if (result?.error) {
        setState((prev) => reduceUpdaterEvent(prev, { event: 'error', data: result.error }));
      }
    } catch (err) {
      setState((prev) =>
        reduceUpdaterEvent(prev, {
          event: 'error',
          data: err instanceof Error ? err.message : String(err),
        })
      );
    }
  };

  const openReleasePage = () => {
    const url = releaseUrlRef.current;
    if (window.electron?.openExternal) {
      window.electron.openExternal(url);
    } else {
      window.open(url, '_blank');
    }
  };

  if (!shouldShowUpdateModal(state)) return null;

  const downloaded = state.phase === 'downloaded';
  const preparing = state.phase === 'preparing';
  const installing = state.phase === 'installing';
  const isError = state.phase === 'error';
  // A failed CHECK keeps no version, and retrying one is the Settings page's
  // Check for updates; a failure with an update in hand gets Try again here.
  const canRetry = isError && !!state.latestVersion;
  // Assisted mode cannot restart itself: its install opens the downloaded
  // installer, so the button must not promise an automatic restart.
  const assisted = state.usingFallback;
  // Assisted mode, nothing downloading yet: the update is FOUND, not arriving.
  const awaitingDownload = needsAssistedDownload(state) && !downloadRequested;

  return (
    <ModalShell
      open={open}
      onOpenChange={(o) => (!o ? dismiss() : setOpen(true))}
      size="md"
      purpose="info"
      title={
        <span className="flex items-center gap-2">
          {downloaded || installing ? (
            <Rocket className="w-5 h-5 flex-none text-background-accent" />
          ) : isError ? (
            <AlertCircle className="w-5 h-5 flex-none text-text-danger" />
          ) : (
            <Download className="w-5 h-5 flex-none text-background-accent" />
          )}
          {downloaded
            ? 'Update ready to install'
            : installing
              ? 'Restarting to update…'
              : preparing
                ? 'Preparing the update…'
                : isError
                  ? 'Update failed'
                  : awaitingDownload
                    ? 'Update available'
                    : 'Downloading update…'}
        </span>
      }
      footer={
        <>
          <Button variant="ghost" size="sm" onClick={dismiss}>
            {downloaded || preparing || installing ? 'Later' : 'Not now'}
          </Button>
          <Button
            variant="secondary"
            size="sm"
            onClick={openReleasePage}
            className="flex items-center gap-2"
          >
            <ExternalLink className="w-4 h-4" />
            Download from Biorouter
          </Button>
          {downloaded ? (
            <Button
              variant="default"
              size="sm"
              onClick={handleRestartAndUpdate}
              className="flex items-center gap-2"
            >
              <Rocket className="w-4 h-4" />
              {assisted ? 'Install update' : <>Restart &amp; update</>}
            </Button>
          ) : canRetry ? (
            <Button
              variant="default"
              size="sm"
              onClick={handleRetry}
              className="flex items-center gap-2"
            >
              <Download className="w-4 h-4" />
              Try again
            </Button>
          ) : installing ? (
            <Button variant="default" size="sm" disabled className="flex items-center gap-2">
              <Spinner />
              Restarting…
            </Button>
          ) : awaitingDownload ? (
            <Button
              variant="default"
              size="sm"
              onClick={handleDownload}
              className="flex items-center gap-2"
            >
              <Download className="w-4 h-4" />
              Download update
            </Button>
          ) : !isError ? (
            <Button variant="default" size="sm" disabled className="flex items-center gap-2">
              <Spinner />
              Preparing…
            </Button>
          ) : null}
        </>
      }
    >
      <div className="py-3 space-y-3">
        {(state.latestVersion || currentVersion) && (
          <div className="bg-background-muted rounded-element px-3 py-2 space-y-1">
            <div className="flex items-center justify-between">
              <span className="text-supporting text-text-muted">Current</span>
              <span className="text-code font-mono text-text-default">{currentVersion}</span>
            </div>
            {state.latestVersion && (
              <div className="flex items-center justify-between">
                <span className="text-supporting text-text-muted">New version</span>
                <span className="text-code font-mono font-semibold text-text-default">
                  {state.latestVersion}
                </span>
              </div>
            )}
          </div>
        )}

        {downloaded && !assisted && (
          <p className="text-body text-text-default">
            The update has been downloaded. Click{' '}
            <strong className="font-semibold">Restart &amp; update</strong> and Biorouter will
            reopen on the new version automatically. Your settings, chats, and extensions are
            preserved.
          </p>
        )}

        {downloaded && assisted && (
          <p className="text-body text-text-default">
            The update has been downloaded. Click{' '}
            <strong className="font-semibold">Install update</strong> to see the installer and the
            steps to finish. Your settings, chats, and extensions are preserved.
          </p>
        )}

        {preparing && (
          <p className="text-body text-text-default">
            The update has been downloaded and macOS is checking it before it can be installed. This
            usually takes a few seconds.
          </p>
        )}

        {installing && (
          <p className="text-body text-text-default">
            Biorouter is closing to install the update and will reopen on the new version.
          </p>
        )}

        {awaitingDownload && (
          <div className="space-y-2">
            <p className="text-body text-text-default">
              A new version is ready to download. Biorouter will fetch it and then show you the file
              to finish installing: it can&apos;t replace itself while it&apos;s running on this
              platform.
            </p>
            {downloadError && (
              <p className="text-supporting font-mono text-text-danger bg-background-muted rounded-inner px-2 py-1">
                {downloadError}
              </p>
            )}
          </div>
        )}

        {state.phase === 'available' && !awaitingDownload && (
          <div className="space-y-2">
            <p className="text-body text-text-default">
              A new version is downloading in the background. You can keep working. We&apos;ll let
              you know the moment it&apos;s ready to install.
            </p>
            <Progress
              label={`Downloading update${state.latestVersion ? ` ${state.latestVersion}` : ''}`}
              value={state.percent}
              // A download at 0% is still a download. Without a floor the first
              // seconds read as a dead control.
              minVisiblePercent={4}
            />
            <p className="text-supporting text-text-muted text-right tabular-nums">
              {state.percent}%
            </p>
          </div>
        )}

        {isError && (
          <p className="text-supporting font-mono text-text-muted bg-background-muted rounded-inner px-2 py-1">
            {state.error || 'Something went wrong while downloading the update.'}
          </p>
        )}
      </div>
    </ModalShell>
  );
}
