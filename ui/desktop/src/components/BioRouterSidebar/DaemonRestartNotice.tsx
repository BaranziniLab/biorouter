import { useEffect, useState } from 'react';
import { AlertTriangle } from '../icons/app-icons';
import { Button } from '../ui/button';
import { InfoTip } from '../ui/info-tip';
import { Note } from '../ui/note';

type DaemonConnectionState = 'attached' | 'lost' | 'reconnecting';

export const daemonNoticeCopy = {
  failed: "Biorouter couldn't reconnect to its background service.",
  /**
   * What is true until a reconnect works, so the notice explains the failures
   * around it. It sits behind the InfoTip (F-15): the failure stays visible,
   * and the help is the glyph's description for a screen reader.
   */
  consequence: "Chats and Crew can't reach it until Biorouter reconnects.",
  /** What the InfoTip is about: its name is "About {about}". */
  about: 'the background service',
  retry: 'Try again',
  reconnecting: 'Reconnecting…',
  restart: 'Quit and reopen',
} as const;

/**
 * The standing notice for R-1. When the shared daemon this app verified is gone or was replaced,
 * the main process reconnects on its own, and nothing is shown while it does. This appears only
 * after those attempts failed, so every failure around it has an explanation and a way on. Try
 * again asks the main process for one more attempt (nothing secret passes through here), and
 * Quit and reopen is the existing `restartApp`.
 *
 * Renders nothing while attached or while the app reconnects by itself, and nothing on a surface
 * with no shared daemon (a browser, an external backend), which has no `getDaemonConnection`.
 */
export default function DaemonRestartNotice() {
  const [state, setState] = useState<DaemonConnectionState>('attached');
  // Shown from the first failure until the app is attached again, including while a Try again
  // runs; hidden during the automatic reconnect that comes before any failure.
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    const electron = window.electron;
    if (!electron?.getDaemonConnection) return;
    let cancelled = false;
    const apply = (next: DaemonConnectionState) => {
      if (cancelled) return;
      setState(next);
      if (next === 'lost') setFailed(true);
      if (next === 'attached') setFailed(false);
    };
    const dispose = electron.onDaemonConnection?.(apply);
    electron
      .getDaemonConnection()
      .then(apply)
      .catch(() => {
        // Live events stay the source of truth when the first read fails.
      });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  if (state === 'attached' || !failed) return null;
  const reconnecting = state === 'reconnecting';
  return (
    <Note tone="warning" role="status" icon={AlertTriangle} testId="daemon-restart-notice">
      <p>
        {daemonNoticeCopy.failed}{' '}
        <InfoTip
          label={daemonNoticeCopy.about}
          help={daemonNoticeCopy.consequence}
          data-testid="daemon-restart-notice-help"
        />
      </p>
      <div className="mt-2 flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={reconnecting}
          onClick={() => void window.electron?.reconnectDaemon?.()}
        >
          {reconnecting ? daemonNoticeCopy.reconnecting : daemonNoticeCopy.retry}
        </Button>
        <Button
          type="button"
          size="sm"
          variant="ghost"
          disabled={reconnecting}
          onClick={() => window.electron?.restartApp?.()}
        >
          {daemonNoticeCopy.restart}
        </Button>
      </div>
    </Note>
  );
}
