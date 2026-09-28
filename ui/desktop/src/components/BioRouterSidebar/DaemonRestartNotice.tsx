import { useEffect, useState } from 'react';
import { AlertTriangle } from '../icons/app-icons';
import { Button } from '../ui/button';
import { Note } from '../ui/note';

type DaemonConnectionState = 'attached' | 'lost' | 'reconnecting';

export const daemonNoticeCopy = {
  restarted: "Biorouter's background service restarted.",
  /** What is true until the person acts, so the notice explains the failures around it. */
  consequence: "Chats and Crew can't reach it until Biorouter reconnects.",
  reconnect: 'Reconnect',
  reconnecting: 'Reconnecting…',
  restart: 'Quit and reopen',
} as const;

/**
 * The standing notice for R-1: the shared daemon this app verified is gone or was replaced,
 * and the local proxy refuses to follow a new instance by itself. The main process asks once
 * in a native prompt; this is what stays after "Not Now", so every failure around it has an
 * explanation and a way on. Reconnect asks the main process to reattach (it asks for the
 * approval secret in its own native prompt; nothing secret passes through here), and Quit and
 * reopen is the existing `restartApp`.
 *
 * Renders nothing while attached, and nothing on a surface with no shared daemon (a browser,
 * an external backend), which has no `getDaemonConnection`.
 */
export default function DaemonRestartNotice() {
  const [state, setState] = useState<DaemonConnectionState>('attached');

  useEffect(() => {
    const electron = window.electron;
    if (!electron?.getDaemonConnection) return;
    let cancelled = false;
    const dispose = electron.onDaemonConnection?.((next) => {
      if (!cancelled) setState(next);
    });
    electron
      .getDaemonConnection()
      .then((current) => {
        if (!cancelled) setState(current);
      })
      .catch(() => {
        // Live events stay the source of truth when the first read fails.
      });
    return () => {
      cancelled = true;
      dispose?.();
    };
  }, []);

  if (state === 'attached') return null;
  const reconnecting = state === 'reconnecting';
  return (
    <Note tone="warning" role="status" icon={AlertTriangle} testId="daemon-restart-notice">
      <p>{daemonNoticeCopy.restarted}</p>
      <p>{daemonNoticeCopy.consequence}</p>
      <div className="mt-2 flex flex-wrap gap-2">
        <Button
          type="button"
          size="sm"
          variant="secondary"
          disabled={reconnecting}
          onClick={() => void window.electron?.reconnectDaemon?.()}
        >
          {reconnecting ? daemonNoticeCopy.reconnecting : daemonNoticeCopy.reconnect}
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
