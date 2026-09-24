import { useEffect, useState } from 'react';
import { AlertTriangle, KeyRound, LoaderCircle, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { parseRefusal, refusalText } from '../dialogs/refusals';
import { connectionServer } from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { connectionBarCopy } from './copy';
import { calendarDate, workspaceLabel } from './presentation';
import { useNewDeviceNotice } from './useNewDeviceNotice';
import { useVaultStatus } from './useVaultStatus';
import './channel.css';

/** True once `active` has held continuously for `delayMs`; false again as soon as it stops. */
function useHeldFor(active: boolean, delayMs: number): boolean {
  const [held, setHeld] = useState(false);
  useEffect(() => {
    if (!active) {
      setHeld(false);
      return;
    }
    const timer = window.setTimeout(() => setHeld(true), delayMs);
    return () => window.clearTimeout(timer);
  }, [active, delayMs]);
  return active && held;
}

/**
 * An action error in words: the copy deck's or the broker's own sentence for a refusal it knows,
 * and never a bare `code: ` prefix (a `name_taken: …` reached this bar verbatim once the dialog that
 * caused it had closed, T-08).
 */
export function actionErrorText(message: string): string {
  const words = refusalText(message);
  const refusal = parseRefusal(words);
  if (!refusal.code) return words;
  const sentence = refusal.sentence.trim();
  return sentence ? sentence.charAt(0).toUpperCase() + sentence.slice(1) : words;
}

export interface ConnectionBarProps {
  /** Layout only. */
  className?: string;
}

/**
 * The connection bar: the top of the channel column, under the header (ui-redesign-spec, "Where
 * errors render: exactly once"). At most one note of each kind, in this order:
 *
 * 1. the observation error, with **Retry** ("Retry Crew updates") — but only for a member on a
 *    connection the daemon calls connected. Before a person is let in (not joined yet, or the join
 *    screen) the join card says what is happening, and on an offline connection the screen offers
 *    Connect; a note there would repeat it with a Retry that can only fail the same way (T-06,
 *    T-09). The error stays in the controller, where the join probe reads its code.
 * 2. an observer or global action error — or a connect failure whose own surface is not on
 *    screen — with Dismiss (Try again for a connect failure);
 * 3. the one highest-priority need: the vault is locked (Unlock), the server can't be reached
 *    (Try again), or a reconnect has taken over a second (a spinner, no action);
 * 4. the new-device notice, until it is reviewed.
 *
 * Each message renders here exactly once: an action error reaches this bar only when the
 * controller's resolver says its own surface is not mounted, and a connect failure that is shown as
 * an error is not repeated as a need. Security state never animates here; notes appear in place.
 */
export function ConnectionBar({ className }: ConnectionBarProps) {
  const crew = useCrew();
  const {
    error,
    refreshError,
    lastConnectFailure: failure,
    connection,
    connectionId,
    errorSlotFor,
    isPending,
  } = crew;
  const vault = useVaultStatus(connectionId, error?.message ?? null);
  const connecting = isPending('connect');
  const slowConnect = useHeldFor(connecting, 1000);
  const verified =
    crew.snapshot && crew.observedPrivacy?.connectionId === connectionId ? crew.snapshot : null;
  const devices = useNewDeviceNotice(
    connectionId,
    verified?.actor.id ?? null,
    verified ? (verified.actor.devices ?? null) : null
  );

  const barError = error && (errorSlotFor('global') || errorSlotFor('observer')) ? error : null;
  const connectError = barError?.source === 'connect' ? barError : null;
  const actionError = barError && barError.source !== 'connect' ? barError : null;
  const host = connectionServer(connection) || connection?.name || '';
  const unreachable = failure?.kind === 'unreachable';
  const workspace = workspaceLabel(crew, crew.snapshot ?? crew.lastVerified?.snapshot ?? null);
  const notMember = crew.status === 'not-joined' || crew.screen === 'join';
  const showObservationError =
    Boolean(refreshError) && !notMember && (!connection || connection.status === 'connected');

  const tryAgain = (
    <Button
      type="button"
      variant="secondary"
      size="sm"
      disabled={connecting}
      onClick={() => void crew.connect({ userInitiated: true })}
    >
      {connectionBarCopy.tryAgain}
    </Button>
  );
  const settingsLink = connectionId ? (
    <Button
      type="button"
      variant="link"
      className="h-auto p-0 text-supporting"
      onClick={() => crew.openDialog({ kind: 'connection-settings', connectionId })}
    >
      {connectionBarCopy.connectionSettings}
    </Button>
  ) : null;
  const unreachableNote = (role: 'alert' | 'status') => (
    <Note tone="warning" role={role} icon={AlertTriangle} action={tryAgain}>
      <p>{connectionBarCopy.unreachable(host)}</p>
      {settingsLink}
    </Note>
  );

  const need = vault.locked
    ? 'vault'
    : unreachable && !connectError && !connecting
      ? 'unreachable'
      : slowConnect && crew.screen !== 'connecting'
        ? 'reconnecting'
        : null;

  return (
    <div className={cn('crew-connection-bar', className)} data-testid="crew-connection-bar">
      {showObservationError && (
        <Note
          tone="warning"
          role="alert"
          icon={AlertTriangle}
          action={
            // Retrying can only help a connection the daemon calls connected. With no saved
            // connection at all, the screen under the bar has its own Try again.
            connection?.status === 'connected' ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                aria-label={connectionBarCopy.retryName}
                onClick={() => void crew.refresh()}
              >
                {connectionBarCopy.retry}
              </Button>
            ) : undefined
          }
        >
          <p>{refreshError}</p>
        </Note>
      )}

      {actionError && (
        <Note
          tone="danger"
          role="alert"
          icon={AlertTriangle}
          action={
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="xs"
              aria-label={connectionBarCopy.dismiss}
              className="size-5"
              onClick={crew.dismissError}
            >
              <X aria-hidden="true" />
            </Button>
          }
        >
          <p>{actionErrorText(actionError.message)}</p>
        </Note>
      )}

      {connectError &&
        (unreachable ? (
          unreachableNote('alert')
        ) : (
          <Note tone="danger" role="alert" icon={AlertTriangle} action={tryAgain}>
            <p>{connectError.message}</p>
          </Note>
        ))}

      {need === 'vault' && (
        <Note
          tone="warning"
          role="status"
          icon={KeyRound}
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={vault.unlocking}
              onClick={() =>
                void vault
                  .unlock()
                  .then((unlocked) => {
                    if (unlocked && connection?.status === 'connected') void crew.refresh();
                  })
                  .catch((failure: unknown) =>
                    crew.reportError(
                      failure instanceof Error && failure.message
                        ? failure.message
                        : connectionBarCopy.unlockFailed,
                      'global'
                    )
                  )
              }
            >
              {connectionBarCopy.unlock}
            </Button>
          }
        >
          <p>{connectionBarCopy.vaultLocked}</p>
        </Note>
      )}
      {need === 'unreachable' && unreachableNote('status')}
      {need === 'reconnecting' && (
        <Note tone="neutral" role="status">
          <p className="flex items-center gap-2">
            <LoaderCircle aria-hidden="true" className="h-4 w-4 shrink-0 animate-spin" />
            {connectionBarCopy.reconnecting(workspace)}
          </p>
        </Note>
      )}

      {devices.notice && (
        <Note
          tone="info"
          role="status"
          icon={KeyRound}
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              aria-label={connectionBarCopy.reviewName}
              onClick={() => {
                devices.acknowledge();
                crew.openDialog({ kind: 'keys' });
              }}
            >
              {connectionBarCopy.review}
            </Button>
          }
        >
          <p>
            {devices.notice.addedAt !== null
              ? connectionBarCopy.newDevice(calendarDate(devices.notice.addedAt))
              : connectionBarCopy.newDeviceUndated}
          </p>
        </Note>
      )}
    </div>
  );
}
