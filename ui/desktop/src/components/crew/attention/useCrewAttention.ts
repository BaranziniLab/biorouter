import { useEffect, useRef, useState } from 'react';
import { crewHttp, crewRequest, type CrewConnection, type CrewMessageResult } from '../crewApi';
import { rememberLastChannel } from '../state/draftStash';
import { isBrowserSurface } from '../../../utils/surface';
import { CrewAttentionWatcher, type AttentionNotification } from './crewAttention';

/** The desktop bridge the attention signals need; absent on a browser surface. */
interface AttentionBridge {
  setCrewAttentionBadge(count: number): void;
  notifyCrewAttention(notification: AttentionNotification): void;
  onCrewAttentionOpen?(
    callback: (target: { connectionId: string; channelId: string }) => void
  ): () => void;
}

function attentionBridge(): AttentionBridge | null {
  if (isBrowserSurface()) return null;
  const electron = (window as { electron?: Partial<AttentionBridge> }).electron;
  return typeof electron?.setCrewAttentionBadge === 'function' &&
    typeof electron.notifyCrewAttention === 'function'
    ? (electron as AttentionBridge)
    : null;
}

/** The longest one attention read may take; a daemon that hangs must not freeze the badge. */
export const ATTENTION_READ_TIMEOUT_MS = 15_000;

/** `signal`, also aborted after {@link ATTENTION_READ_TIMEOUT_MS}. */
function bounded(signal: AbortSignal): AbortSignal {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ATTENTION_READ_TIMEOUT_MS);
  const abort = () => {
    clearTimeout(timer);
    controller.abort();
  };
  if (signal.aborted) abort();
  else signal.addEventListener('abort', abort, { once: true });
  return controller.signal;
}

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve) => {
    if (signal.aborted) return resolve();
    const timer = setTimeout(resolve, ms);
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timer);
        resolve();
      },
      { once: true }
    );
  });

export interface CrewAttentionOptions {
  /** This window shows Crew now. */
  onCrewRoute: boolean;
  /** A notification for a channel was clicked: show Crew. The channel is remembered first. */
  onOpenChannel(connectionId: string, channelId: string): void;
}

/**
 * Crew's attention signals outside Crew (M2), for the app shell: the number of unread messages
 * across every connected workspace, for the Crew item; the same number on the dock (the main
 * process shows the largest any window reports); and a system notification when a channel's
 * count rises while the window is not showing Crew in front ("Alice Chen mentioned you in
 * #general", or "3 new messages in chen-lab"). The main process shows at most one per channel a
 * minute, and none while another window of the app is in front.
 *
 * Reads go through the daemon with the person's proof, like every Crew read; reading a
 * channel's newest messages to word a notification marks nothing read. Only on the desktop app.
 */
export function useCrewAttention({ onCrewRoute, onOpenChannel }: CrewAttentionOptions): number {
  const [total, setTotal] = useState(0);
  const onCrew = useRef(onCrewRoute);
  const open = useRef(onOpenChannel);
  useEffect(() => {
    onCrew.current = onCrewRoute;
    open.current = onOpenChannel;
  }, [onCrewRoute, onOpenChannel]);

  useEffect(() => {
    const bridge = attentionBridge();
    if (!bridge) return;
    const watcher = new CrewAttentionWatcher({
      listConnections: async (signal) =>
        (
          await crewHttp<{ connections: CrewConnection[] }>(
            '/connections',
            'GET',
            undefined,
            bounded(signal)
          )
        ).connections ?? [],
      readSnapshot: (connectionId, signal) =>
        crewRequest(connectionId, 'workspace.snapshot', {}, false, bounded(signal)),
      readLatest: (connectionId, channelId, limit, signal) =>
        crewRequest<CrewMessageResult>(
          connectionId,
          'messages.history',
          { channel_id: channelId, limit, latest: true },
          false,
          bounded(signal)
        ),
      notify: (notification) => bridge.notifyCrewAttention(notification),
      onTotal: (count) => {
        setTotal(count);
        bridge.setCrewAttentionBadge(count);
      },
      attended: () =>
        onCrew.current && document.visibilityState !== 'hidden' && document.hasFocus(),
      sleep,
      now: () => Date.now(),
    });
    watcher.start();
    const refresh = () => {
      if (document.visibilityState !== 'hidden') watcher.refresh();
    };
    window.addEventListener('focus', refresh);
    document.addEventListener('visibilitychange', refresh);
    const disposeOpen = bridge.onCrewAttentionOpen?.(({ connectionId, channelId }) => {
      rememberLastChannel(connectionId, channelId);
      open.current(connectionId, channelId);
    });
    return () => {
      watcher.stop();
      window.removeEventListener('focus', refresh);
      document.removeEventListener('visibilitychange', refresh);
      disposeOpen?.();
      bridge.setCrewAttentionBadge(0);
    };
  }, []);

  return total;
}
