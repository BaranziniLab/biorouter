import { useState, useEffect } from 'react';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Progress } from '../../ui/progress';
import {
  ExternalLink,
  CheckCircle,
  Download,
  AlertCircle,
  Loader2,
  Rocket,
} from '../../icons/app-icons';
import {
  initialUpdaterState,
  reduceUpdaterEvent,
  type UpdaterState,
} from '../../../utils/updaterState';

// Always point users at the official Biorouter download website (the same place
// the startup update flow directs to) rather than the raw GitHub releases page.
const DOWNLOAD_WEBSITE_URL = 'https://biorouter.ucsf.edu/download';

/**
 * Settings → "Check for Updates".
 *
 * Drives the same `electron-updater` pipeline as the startup modal: pressing
 * "Check for Updates" asks the main process to check GitHub; if a newer release
 * exists it downloads in the background and this panel shows progress, then a
 * one-click **Restart & Update** button. No manual DMG/drag-and-drop.
 */
export default function UpdateSection() {
  const [currentVersion, setCurrentVersion] = useState('');
  const [state, setState] = useState<UpdaterState>(initialUpdaterState);
  const [checkRequested, setCheckRequested] = useState(false);

  useEffect(() => {
    setCurrentVersion(window.electron.getVersion());

    if (!window.electron?.onUpdaterEvent) return;
    const dispose = window.electron.onUpdaterEvent((payload) => {
      setState((prev) => reduceUpdaterEvent(prev, payload));
    });

    // Recover any in-flight/ready update established before this panel opened.
    window.electron
      .getUpdateState?.()
      ?.then((snapshot) => {
        if (!snapshot) return;
        if (snapshot.status === 'downloaded' || snapshot.updateAvailable) {
          setState((prev) =>
            reduceUpdaterEvent(prev, {
              event: snapshot.status === 'downloaded' ? 'update-downloaded' : 'update-available',
              data: { version: snapshot.latestVersion, percent: snapshot.percent },
            })
          );
        }
      })
      .catch(() => {});

    return () => dispose?.();
  }, []);

  const checkForUpdates = async () => {
    setCheckRequested(true);
    setState((prev) => reduceUpdaterEvent(prev, { event: 'checking-for-update' }));
    try {
      const res = await window.electron.checkForUpdates();
      if (res?.error) {
        setState((prev) => reduceUpdaterEvent(prev, { event: 'error', data: res.error }));
      }
      // Success path: the main process emits update-available / -not-available
      // (and download-progress / -downloaded) through onUpdaterEvent.
    } catch (err) {
      setState((prev) =>
        reduceUpdaterEvent(prev, {
          event: 'error',
          data: err instanceof Error ? err.message : 'Unknown error',
        })
      );
    }
  };

  const handleRestartAndUpdate = () => window.electron?.installUpdate?.();

  const { phase } = state;
  const busy = phase === 'checking' || phase === 'available';

  return (
    <div>
      {/* The wrapper's own `text-sm text-text-muted` was dead — both children
          set their own type — and the 16px gap under it belonged to a
          `.biorouter-settings-control-strip` that no longer wraps this panel. */}
      <div className="mb-2">
        <div className="flex flex-col">
          <div className="text-text-default text-display font-mono">
            {currentVersion || 'Loading...'}
          </div>
          <div className="text-supporting text-text-muted">Current version</div>
        </div>
      </div>

      {/* The control strip, and none of the three Buttons carries geometry any
          more: `flex items-center gap-2` was flipping the cva base's
          `inline-flex` through tailwind-merge while restating what the base
          already emits. */}
      <div className="biorouter-settings-control-strip">
        {phase === 'downloaded' ? (
          <Button onClick={handleRestartAndUpdate} variant="default">
            <Rocket className="w-4 h-4" />
            Restart &amp; Update{state.latestVersion ? ` to ${state.latestVersion}` : ''}
          </Button>
        ) : (
          <Button onClick={checkForUpdates} disabled={busy} variant="secondary">
            {busy ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <ExternalLink className="w-4 h-4" />
            )}
            Check for Updates
          </Button>
        )}
        <p className="text-supporting text-text-muted">
          Biorouter installs updates automatically. Restart to use the new version.
        </p>
      </div>

      {/* Status line */}
      {checkRequested && (
        <div className="mt-3 text-supporting">
          {phase === 'checking' && (
            <div className="flex items-center gap-2 text-text-muted">
              <Loader2 className="w-4 h-4 animate-spin" />
              Checking for updates…
            </div>
          )}

          {phase === 'up-to-date' && (
            <div className="flex items-center gap-2 text-text-success">
              <CheckCircle className="w-4 h-4" />
              Biorouter is up to date.
            </div>
          )}

          {phase === 'available' && (
            <div className="space-y-2 max-w-sm">
              <div className="flex items-center gap-2 text-text-default">
                <Download className="w-4 h-4 text-background-accent" />
                Downloading {state.latestVersion ?? 'update'}…
              </div>
              <Progress
                label={`Downloading ${state.latestVersion ?? 'update'}`}
                value={state.percent}
                minVisiblePercent={4}
              />
              <p className="text-right font-mono text-supporting text-text-muted">
                {state.percent}%
              </p>
            </div>
          )}

          {phase === 'downloaded' && (
            <div className="flex items-center gap-2 text-text-default">
              <CheckCircle className="w-4 h-4 text-text-success" />
              Version {state.latestVersion} is ready. Click Restart &amp; Update.
            </div>
          )}

          {phase === 'error' && (
            <div className="space-y-2">
              <div className="flex items-center gap-2 text-text-danger">
                <AlertCircle className="w-4 h-4" />
                Could not complete the update.
              </div>
              {state.error && (
                <Note tone="danger" className="font-mono">
                  {state.error}
                </Note>
              )}
              <Button
                variant="secondary"
                onClick={() => window.open(DOWNLOAD_WEBSITE_URL, '_blank')}
              >
                <ExternalLink className="w-4 h-4" />
                Download from Biorouter
              </Button>
            </div>
          )}
        </div>
      )}
    </div>
  );
}
