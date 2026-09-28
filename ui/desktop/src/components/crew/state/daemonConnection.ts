import { useEffect, useState } from 'react';

/** Mirrors `DaemonConnectionState` in the preload: whether this app is attached to its daemon. */
export type DaemonConnectionState = 'attached' | 'lost' | 'reconnecting';

/**
 * Whether this app is attached to its background service (RES2-N7), from the preload's
 * `getDaemonConnection` and `onDaemonConnection`, which `DaemonRestartNotice` in the app's sidebar
 * reads too. After the service restarted, every Crew request fails until Biorouter reconnects, so
 * Crew's own Retry only led from "Live updates stopped" to "Crew couldn't load your saved
 * workspaces"; the connection bar says what is wrong instead. `attached` on a surface with no
 * shared service (a browser, an external backend), which has no `getDaemonConnection`.
 */
export function useDaemonConnection(): DaemonConnectionState {
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
  return state;
}

/** Ask the app to attach to its background service again: the sidebar notice's Reconnect. */
export function reconnectDaemon(): void {
  void window.electron?.reconnectDaemon?.();
}
