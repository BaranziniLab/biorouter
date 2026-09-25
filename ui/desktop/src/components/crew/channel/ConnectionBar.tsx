import { useEffect, useState, type MouseEvent } from 'react';
import { AlertTriangle, KeyRound, LoaderCircle, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { parseRefusal, refusalText } from '../dialogs/refusals';
import { connectionServer } from '../identity';
import {
  isMembershipEnded,
  isNotSetUpFailure,
  isTrustFailure,
  type ConnectFailureKind,
} from '../state/connectFailure';
import { crewObservationCopy } from '../state/copy';
import { useCrew } from '../state/CrewControllerContext';
import { DIALOG_FOCUS_FALLBACKS, restoreFocusSoon } from '../state/focusReturn';
import { CHANNEL_LOST_ERROR_CODE } from '../state/useCrewObservation';
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

/**
 * Machine text a person must never read (NEW-1): the transport's own record of an SSH failure
 * ("Crew SSH failure [ssh_eof; child_before_cleanup=exit_255]: … inspect history before
 * retrying"), a bracketed or `key=value` status, or a JSON envelope.
 */
const MACHINE_TEXT =
  /Crew SSH failure|child_before_cleanup|\[[^\]]*[a-z0-9]_[a-z0-9][^\]]*\]|\b[a-z]+_[a-z0-9_]+=|[{}]/;

/**
 * A failed Connect in words, by its kind (NEW-1). A classified SSH failure always reads as its
 * kind — the daemon's message for every one of them is the transport record above — and so does
 * any machine-shaped text. Only a failure the daemon answered in a person's words (a missing
 * approval, an outdated background service) keeps them. Never the raw text.
 */
export function connectErrorText(
  kind: ConnectFailureKind | undefined,
  message: string,
  host: string
): string {
  if (kind === 'unreachable') return connectionBarCopy.unreachable(host);
  if (kind === 'auth_required') return connectionBarCopy.signInNeeded(host);
  if (isTrustFailure(kind)) return connectionBarCopy.cantVerify(host);
  if (isNotSetUpFailure(kind)) return connectionBarCopy.notRunning(host);
  if ((kind === undefined || kind === 'unknown') && message.trim() && !MACHINE_TEXT.test(message))
    return actionErrorText(message);
  return connectionBarCopy.cantConnect(host);
}

/**
 * Where focus goes when a bar button leaves with its note (Q2-20): the channel heading, the
 * composer, then the Crew sidebar's first control.
 */
export const BAR_FOCUS_FALLBACKS: readonly string[] = [
  ...DIALOG_FOCUS_FALLBACKS,
  'nav[aria-label="Crew"] button',
];

/**
 * Run a bar button's action and keep keyboard focus off `<body>`: Retry, Dismiss and Try again
 * each remove their own note, so the button that had focus is gone. Checked once the click has
 * rendered and again when the action settles; focus that the person (or a surface) put somewhere
 * else is left alone.
 */
function keepingFocus(event: MouseEvent<HTMLElement>, action: () => Promise<unknown> | void): void {
  const origin = event.currentTarget;
  const settle = () => restoreFocusSoon(origin, BAR_FOCUS_FALLBACKS);
  const result = action();
  settle();
  if (result) void Promise.resolve(result).finally(settle);
}

/** The screens that draw a verified workspace, where a closed channel's note belongs. */
const WORKSPACE_SCREENS: readonly string[] = ['channel', 'no-channel', 'no-team'];

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
 *    T-09). The error stays in the controller, where the join probe reads its code. Retry reads
 *    the saved connection first and connects at once when the daemon has since called it
 *    disconnected (Q2-01); a person removed from the workspace gets no Retry at all (Q2-18). A
 *    saved connection whose membership the workspace ended (`crew_membership_ended`, Q3-12 and
 *    Q3-50) says so here, as "You're no longer a member of …", with no Retry, whatever its status.
 * 2. an observer or global action error — or a connect failure whose own surface is not on
 *    screen — with Dismiss (Try again for a connect failure). A closed channel's note shows only
 *    over a workspace view (Q2-19). A connect failure reads as its kind, never in the transport's
 *    words (`connectErrorText`, NEW-1), and goes as soon as the connection verifies again, however
 *    it came back;
 * 3. the one highest-priority need: the vault is locked (Unlock), the server can't be reached
 *    (Try again), or a reconnect has taken over a second (a spinner, no action);
 *
 * A connect failure's note offers no Try again while the main area is the offline screen of a
 * connection the daemon calls disconnected: that screen's "Connect to …" is the same action, and
 * two buttons for one action read as two different things (Q3-07). The reason and "Connection
 * settings…" stay.
 * 4. the new-device notice, until it is reviewed.
 *
 * Each message renders here exactly once: an action error reaches this bar only when the
 * controller's resolver says its own surface is not mounted, and a connect failure that is shown as
 * an error is not repeated as a need. Security state never animates here; notes appear in place.
 * Retry, Dismiss and Try again take their note away with them, so each puts keyboard focus back
 * on the channel, the composer or the sidebar rather than leave it on `<body>` (Q2-20).
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
  // The workspace ended this computer's or this person's membership: said once, here, with no
  // Retry — on the join screen its card says it instead.
  const membershipEnded = isMembershipEnded(connection) && !notMember;
  const showObservationError =
    membershipEnded ||
    (Boolean(refreshError) && !notMember && (!connection || connection.status === 'connected'));
  const observationText = membershipEnded
    ? crewObservationCopy.noLongerMember(workspace)
    : refreshError;
  // The offline screen under the bar carries "Connect to …": the same action as Try again (Q3-07).
  const offlineCardShown = crew.screen === 'offline' && connection?.status === 'disconnected';
  // A closed channel's note describes the workspace view it was closed in: on a connection
  // problem screen, or while reconnecting, it is stale and not shown (Q2-19). The selection that
  // moves on dismisses it.
  const staleChannelLoss =
    actionError?.source === 'observer' &&
    actionError.code === CHANNEL_LOST_ERROR_CODE &&
    (!WORKSPACE_SCREENS.includes(crew.screen) || crew.reconnecting === true);

  const tryAgain = (
    <Button
      type="button"
      variant="secondary"
      size="sm"
      disabled={connecting}
      onClick={(event) => keepingFocus(event, () => crew.connect({ userInitiated: true }))}
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
  const connectAction = offlineCardShown ? undefined : tryAgain;
  const unreachableNote = (role: 'alert' | 'status') => (
    <Note tone="warning" role={role} icon={AlertTriangle} action={connectAction}>
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
            // Retrying can only help a connection the daemon calls connected, and never a person
            // removed from the workspace (Q2-18). With no saved connection at all, the screen
            // under the bar has its own Try again. Retry reads the saved connection first and,
            // when the daemon has since called it disconnected, connects at once (Q2-01).
            !membershipEnded &&
            connection?.status === 'connected' &&
            crew.refreshErrorRetryable !== false ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                aria-label={connectionBarCopy.retryName}
                onClick={(event) =>
                  keepingFocus(event, () => (crew.retryUpdates ?? crew.refresh)())
                }
              >
                {connectionBarCopy.retry}
              </Button>
            ) : undefined
          }
        >
          <p>{observationText}</p>
        </Note>
      )}

      {actionError && !staleChannelLoss && (
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
              onClick={(event) => keepingFocus(event, crew.dismissError)}
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
          <Note tone="danger" role="alert" icon={AlertTriangle} action={connectAction}>
            <p>{connectErrorText(failure?.kind, connectError.message, host)}</p>
            {offlineCardShown && settingsLink}
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
