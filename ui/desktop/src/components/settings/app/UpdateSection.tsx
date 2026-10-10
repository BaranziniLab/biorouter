import { useState, useEffect, type ReactNode } from 'react';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { Progress } from '../../ui/progress';
import { SettingRow } from '../../ui/setting-row';
import {
  initialUpdaterState,
  reduceUpdaterEvent,
  type UpdaterState,
} from '../../../utils/updaterState';
import { aboutCopy } from './copy';

// Always point users at the official Biorouter download website (the same place
// the startup update flow directs to) rather than the raw GitHub releases page.
const DOWNLOAD_WEBSITE_URL = 'https://biorouter.ucsf.edu/download';

/**
 * The Version row's status line. Whatever keeps the action busy is always said, whoever started
 * it: a ready update, a check under way and a download in progress (the startup check finds
 * updates before this panel opens, and a disabled button with no reason beside it reads as
 * broken). The verdicts, "up to date" and a failure, answer a check made HERE, so they show
 * only after one.
 */
function statusLine(state: UpdaterState, checkRequested: boolean): ReactNode {
  switch (state.phase) {
    case 'downloaded':
      return aboutCopy.ready(state.latestVersion);
    case 'checking':
      return aboutCopy.checking;
    case 'available':
      return (
        <span className="flex flex-col gap-1">
          <span>
            {aboutCopy.downloading(state.latestVersion)}{' '}
            <span className="tabular-nums">{state.percent}%</span>
          </span>
          <Progress
            className="max-w-xs"
            label={aboutCopy.downloadingLabel(state.latestVersion)}
            value={state.percent}
            minVisiblePercent={4}
          />
        </span>
      );
    case 'up-to-date':
      return checkRequested ? aboutCopy.upToDate : undefined;
    case 'error':
      return checkRequested ? (
        <span className="text-text-danger">{aboutCopy.failed}</span>
      ) : undefined;
    default:
      return undefined;
  }
}

/**
 * Settings > App > About > Version: the version as the row's value, one action at the
 * trailing edge, and the update's progress as the row's status line (spec §3.13).
 *
 * Drives the same `electron-updater` pipeline as the startup modal: "Check for updates" asks
 * the main process to check GitHub; a newer release downloads in the background while the
 * status line counts it, then "Restart to update" replaces the button. No manual DMG.
 *
 * Returns a fragment: the row (and, after a failure, the error note under it) belong directly
 * to the About section's list.
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

  const status = statusLine(state, checkRequested);

  // Each action names itself: the visible words are the accessible name (label in name).
  const action =
    phase === 'downloaded' ? (
      <Button
        variant="secondary"
        size="sm"
        onClick={handleRestartAndUpdate}
        aria-label={aboutCopy.restartToUpdate}
      >
        {aboutCopy.restartToUpdate}
      </Button>
    ) : (
      <Button
        variant="secondary"
        size="sm"
        onClick={checkForUpdates}
        disabled={busy}
        aria-label={aboutCopy.checkForUpdates}
      >
        {aboutCopy.checkForUpdates}
      </Button>
    );

  return (
    <>
      <SettingRow
        label={aboutCopy.version}
        help={aboutCopy.versionHelp}
        value={currentVersion || aboutCopy.development}
        valueMono
        status={status}
        data-testid="settings-version-row"
      >
        {action}
      </SettingRow>

      {checkRequested && phase === 'error' && state.error && (
        // The failure stays visible (principle 2), with the one way forward beside it.
        <Note
          tone="danger"
          role="alert"
          className="mt-2"
          action={
            <Button
              variant="secondary"
              size="sm"
              onClick={() => window.open(DOWNLOAD_WEBSITE_URL, '_blank')}
            >
              {aboutCopy.downloadFromSite}
            </Button>
          }
        >
          <span className="font-mono">{state.error}</span>
        </Note>
      )}
    </>
  );
}
