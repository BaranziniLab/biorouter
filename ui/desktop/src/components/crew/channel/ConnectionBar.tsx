import { useEffect, useRef, useState, type MouseEvent } from 'react';
import { AlertTriangle, KeyRound, LoaderCircle, X } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { Note } from '../../ui/note';
import { cn } from '../../../utils';
import { parseRefusal, refusalText, type RefusalViewer } from '../dialogs/refusals';
import {
  CREW_DEVICE_KEY_MISSING,
  CREW_DEVICE_NOT_RECOGNIZED,
  CREW_RECONNECTING,
  CREW_REJOIN_CODES,
} from '../api/errors';
import { acknowledgeNewDevices, rejoinConnection, type CrewRejoined } from '../account/api';
import { useBuildBrokerVersion } from '../account/brokerVersion';
import { ChatSetupJobNotes } from '../account/ChatSetupJobs';
import { addedViaText, newDevicesCopy, rejoinCopy, versionCopy } from '../account/copy';
import { clearNeedsRejoin, useNeedsRejoin } from '../account/rejoinMarks';
import { buildPeopleDirectory, personLabel } from '../identity';
import { useJoinContext } from '../onboarding/joinContext';
import { sshUsername } from '../onboarding/joinText';
import { versionOlder } from '../onboarding/setup';
import { serverLabel } from '../sidebar/sidebarView';
import {
  connectFailureHost,
  isMembershipEnded,
  isNotSetUpFailure,
  isTrustFailure,
  workspaceKnownDeleted,
  type ConnectFailureKind,
} from '../state/connectFailure';
import { crewObservationCopy } from '../state/copy';
import { useCrew } from '../state/CrewControllerContext';
import { reconnectDaemon, useDaemonConnection } from '../state/daemonConnection';
import {
  DIALOG_FOCUS_FALLBACKS,
  focusIsLost,
  restoreFocus,
  restoreFocusSoon,
} from '../state/focusReturn';
import { CHANNEL_LOST_ERROR_CODE } from '../state/useCrewObservation';
import { useCrewChanged } from '../state/useCrewChanges';
import { connectionBarCopy } from './copy';
import { calendarDate, workspaceLabel } from './presentation';
import { useDevicesChanged, useNewDeviceNotice } from './useNewDeviceNotice';
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
export function actionErrorText(message: string, host = '', viewer: RefusalViewer = {}): string {
  // The transport's own record of a dropped link never reaches a person (R-4): it says the outcome
  // is unknown, so the sentence says so too.
  if (MACHINE_TEXT.test(message) && /Crew SSH failure|child_before_cleanup/.test(message))
    return connectionBarCopy.linkLost(host);
  // The host reads what the host can do, not "ask the host" (MSG2-N6).
  const words = refusalText(message, viewer);
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
export interface ConnectErrorContext {
  /** The hop OpenSSH named, when it is not the server itself (a jump host's key, W2-DMN-5). */
  failureHost?: string | null;
  /** The account the saved login signs in as, for a refused key. */
  user?: string | null;
  /** Whether this person hosts the workspace; `null` when this computer cannot tell. */
  hosts?: boolean | null;
  /** The workspace's host, as a sentence names them, for a member told whom to ask. */
  hostName?: string | null;
}

export function connectErrorText(
  kind: ConnectFailureKind | undefined,
  message: string,
  host: string,
  context: ConnectErrorContext = {}
): string {
  if (kind === 'unreachable') return connectionBarCopy.unreachable(host);
  if (kind === 'auth_required') return connectionBarCopy.signInNeeded(host);
  // A publickey-only refusal is not a password matter (F5): the login or its key is.
  if (kind === 'ssh_key_refused') return connectionBarCopy.keyRefused(host, context.user ?? null);
  // The hop whose key could not be verified, which may be a jump host (W2-DMN-5).
  if (isTrustFailure(kind)) return connectionBarCopy.cantVerify(context.failureHost || host);
  if (isNotSetUpFailure(kind)) return connectionBarCopy.notRunning(host);
  // A stopped workspace server (R-7): its host starts it; a member asks the host.
  if (kind === 'broker_not_running')
    return context.hosts === true
      ? connectionBarCopy.brokerStoppedHost(host)
      : context.hosts === false
        ? connectionBarCopy.brokerStoppedMember(context.hostName ?? null)
        : connectionBarCopy.brokerStopped(host);
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

/** How long a note's button keeps trying to hand focus on once its note has gone. */
const FOCUS_PATIENCE_MS = 1000;
const FOCUS_RETRY_MS = 50;

/**
 * Put focus back once a bar note took its focused button away, and keep trying for a moment:
 * Acknowledge reloads the connections, and the channel heading or composer it falls back to can
 * be drawn again a little later than the note goes (live QA: focus stayed on `<body>`). It only
 * ever rescues lost focus, so a control the person chose meanwhile keeps it.
 */
function restoreFocusPatiently(origin: HTMLElement | null): () => void {
  let timer: number | null = null;
  const started = Date.now();
  const attempt = () => {
    timer = null;
    if (!focusIsLost() || restoreFocus(origin, BAR_FOCUS_FALLBACKS)) return;
    if (Date.now() - started < FOCUS_PATIENCE_MS)
      timer = window.setTimeout(attempt, FOCUS_RETRY_MS);
  };
  attempt();
  return () => {
    if (timer !== null) window.clearTimeout(timer);
  };
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
 *    A workspace its host deleted (OW-5: the record's `workspace_deleted`, a connect answered
 *    `crew_workspace_deleted`, or an observation ended `workspace_deleted`) says "The host deleted
 *    …" with the one thing left to do, Remove from this computer…, and no Retry.
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
 * Account ownership (docs/crew/design/account-ownership.md, "Desktop") adds, each said once:
 * **Add this computer again** first, on `crew_device_key_missing`, `crew_device_not_recognized` or
 * a device the person removed in Keys, in place of the error that said it; the older-Crew notice
 * once a verified server runs an older Crew than this build carries (its `server_version` below
 * this build's broker's, or, from a broker that says no version, a `hello` without
 * `account_identity_v1`), with Update Crew for the host and whom to ask for everyone else; setups a
 * chat or a terminal started, with Sign in; and the daemon's new devices
 * (`new_devices`) with Review and Acknowledge, which replace this computer's own record when the
 * daemon sends them.
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
  // A computer added to or removed from the account shows in the next verified view, and the
  // daemon noted it from that same view: read the saved connections again so `new_devices` below
  // follows while the Crew view stays open (AO-10).
  useDevicesChanged(
    connectionId,
    verified ? (verified.actor.devices ?? null) : null,
    crew.reloadConnections
  );

  // ── Account ownership (docs/crew/design/account-ownership.md, "Desktop") ──────────────────
  // Add this computer again (AO-9): the person removed this computer in Keys, or the daemon said
  // its key is missing or the workspace no longer knows it while the account is a member.
  const markedForRejoin = useNeedsRejoin(connectionId);
  // This computer was added again somewhere else (a chat's `crew__change rejoin`, another window,
  // `biorouter crew rejoin`): the mark this window set when Keys removed it no longer holds
  // (WI-15). Only a rejoin that succeeded is announced. One that names no connection is taken as
  // this bar's: the mark is display only, and a refusal brings the note back.
  useCrewChanged((delta) => {
    for (const change of delta.changes)
      if (change.kind === 'rejoin') clearNeedsRejoin(change.connection_id || connectionId);
  });
  const rejoinCode =
    [
      connection?.last_error_code ?? null,
      crew.refreshErrorCode ?? null,
      error?.code ?? null,
      failure?.code ?? null,
    ].find((code): code is string => code !== null && CREW_REJOIN_CODES.includes(code)) ?? null;
  const [rejoin, setRejoin] = useState<{
    connectionId: string;
    pending: boolean;
    error: string | null;
    done: CrewRejoined | null;
  } | null>(null);
  const rejoinHere = rejoin?.connectionId === connectionId ? rejoin : null;
  // The daemon's new devices on the person's account (AO-10), until they are acknowledged. A
  // daemon from before AO-10 sends none, and the notice falls back to this computer's own record.
  const daemonDevices = Array.isArray(connection?.new_devices) ? connection.new_devices : null;
  const [acknowledged, setAcknowledged] = useState<{ connectionId: string; seen: string[] }>({
    connectionId: '',
    seen: [],
  });
  const unseenDevices = (daemonDevices ?? []).filter(
    (device) =>
      !(
        acknowledged.connectionId === connectionId && acknowledged.seen.includes(device.fingerprint)
      )
  );
  // Tagged with its connection, as `rejoin` is: a failure on one workspace is never said under
  // another's notice (finding 41).
  const [ackError, setAckError] = useState<{ connectionId: string; message: string } | null>(null);
  const ackErrorHere = ackError?.connectionId === connectionId ? ackError.message : null;
  // Acknowledge takes its note, and so its own focused button, away: once that has rendered,
  // focus goes back to the channel, the composer or the sidebar (`restoreFocusPatiently`).
  const ackOrigin = useRef<HTMLElement | null>(null);
  const [ackDone, setAckDone] = useState(0);
  useEffect(() => {
    if (ackDone === 0) return;
    return restoreFocusPatiently(ackOrigin.current);
  }, [ackDone]);
  // The workspace's server runs an older Crew than the one this build carries (AO-4): the version
  // its verified `hello` said (`server_version`) is below this build's broker's. A broker from
  // before AO-4 says no version; then a verified `hello` without `account_identity_v1` is what says
  // it is older. Said once the view is verified, until dismissed.
  const [versionDismissed, setVersionDismissed] = useState<string | null>(null);
  const serverVersion =
    typeof connection?.server_version === 'string' ? connection.server_version : null;
  const buildBrokerVersion = useBuildBrokerVersion(
    serverVersion !== null && connection?.status === 'connected'
  );

  const barError = error && (errorSlotFor('global') || errorSlotFor('observer')) ? error : null;
  const connectError = barError?.source === 'connect' ? barError : null;
  const actionError = barError && barError.source !== 'connect' ? barError : null;
  // The server by the person's own name for it, as the main area and the menu name it (DW-03).
  const host = serverLabel(connection) || connection?.name || '';
  const joinContext = useJoinContext(connectionId);
  const errorContext: ConnectErrorContext = {
    failureHost: connectFailureHost(failure, connection),
    user: sshUsername(connection?.ssh_target) ?? joinContext.username ?? null,
    hosts: crew.isHost ? true : (joinContext.hosts ?? null),
    hostName: joinContext.hostDisplayName ?? joinContext.hostUsername ?? null,
  };
  const unreachable = failure?.kind === 'unreachable';
  const workspace = workspaceLabel(crew, crew.snapshot ?? crew.lastVerified?.snapshot ?? null);
  // The app is not attached to its background service (RES2-N7): nothing here can reach a
  // workspace until it is, so the bar says that, with Reconnect, and offers no Retry that could
  // only fail again.
  const daemon = useDaemonConnection();
  const daemonAway = daemon !== 'attached';
  // The server has stopped saving changes (T3-BE-13): said while the daemon holds it connected,
  // so "Connected" is never all a person reads while every change is refused (RES2-N2).
  const serverStorage =
    !daemonAway && connection?.status === 'connected' ? (connection.server_storage ?? null) : null;
  // Whom a member asks: the workspace's host as the send failure names them, else the name the
  // invitation gave.
  const storageDir = buildPeopleDirectory(
    serverStorage ? (crew.snapshot ?? crew.lastVerified?.snapshot ?? null) : null
  );
  const storageHost =
    storageDir.host && !storageDir.host.isFormer
      ? personLabel(storageDir.host, 'authority', storageDir)
      : (errorContext.hostName ?? null);
  const notMember = crew.status === 'not-joined' || crew.screen === 'join';
  // The workspace ended this computer's or this person's membership: said once, here, with no
  // Retry — on the join screen its card says it instead.
  const membershipEnded = isMembershipEnded(connection) && !notMember;
  // The host deleted the workspace (OW-5): final, whatever the saved status says.
  const workspaceDeleted =
    Boolean(connection) &&
    workspaceKnownDeleted({
      saved: connection,
      lastConnectFailure: failure,
      observationCode: crew.refreshErrorCode,
    });
  // The main area says it, with Remove, once no verified view is left to show (L6): said once.
  const deletedSaidBelow = workspaceDeleted && crew.screen === 'workspace-deleted';
  const showObservationError =
    !daemonAway &&
    !deletedSaidBelow &&
    (workspaceDeleted ||
      membershipEnded ||
      (Boolean(refreshError) && !notMember && (!connection || connection.status === 'connected')));
  const observationText = workspaceDeleted
    ? crewObservationCopy.workspaceDeleted(workspace)
    : membershipEnded
      ? crewObservationCopy.noLongerMember(workspace)
      : refreshError;
  // The offline screen under the bar carries "Connect to …": the same action as Try again (Q3-07).
  const offlineCardShown = crew.screen === 'offline' && connection?.status === 'disconnected';
  // A closed channel's note describes the workspace view it was closed in: on a connection
  // problem screen, or while reconnecting, it is stale and not shown (Q2-19). The selection that
  // moves on dismisses it.
  // The daemon's "Reconnecting to …; try again in a moment" answer, while the offline screen under
  // the bar says the same with its time and Connect (RES2-N5): one state, not two that disagree.
  const redialShownBelow =
    offlineCardShown && actionError?.code === CREW_RECONNECTING && crew.redialSince != null;
  const staleChannelLoss =
    actionError?.source === 'observer' &&
    actionError.code === CHANNEL_LOST_ERROR_CODE &&
    (!WORKSPACE_SCREENS.includes(crew.screen) || crew.reconnecting === true);

  // Add this computer again: never for a deleted workspace, and never while the app cannot reach
  // its background service.
  const showRejoin =
    !daemonAway &&
    Boolean(connectionId) &&
    !workspaceDeleted &&
    (markedForRejoin || rejoinCode !== null || Boolean(rejoinHere?.pending || rejoinHere?.error));
  const rejoinText =
    rejoinCode === CREW_DEVICE_KEY_MISSING
      ? rejoinCopy.keyMissing(workspace)
      : rejoinCode === CREW_DEVICE_NOT_RECOGNIZED
        ? rejoinCopy.notRecognized(workspace)
        : rejoinCopy.removedHere(workspace);
  // The rejoin note says what the refusal said, with its action: the same words are not repeated
  // as an error or an observation note.
  const errorSaidByRejoin =
    showRejoin && actionError?.code !== undefined && CREW_REJOIN_CODES.includes(actionError.code);
  const observationSaidByRejoin =
    showRejoin &&
    crew.refreshErrorCode != null &&
    CREW_REJOIN_CODES.includes(crew.refreshErrorCode);
  const addThisComputerAgain = async () => {
    if (!connectionId || rejoinHere?.pending) return;
    const id = connectionId;
    setRejoin({ connectionId: id, pending: true, error: null, done: null });
    try {
      // The daemon adds a computer again over a live connection (it never dials under the
      // connection's guard): connect first, as the person.
      if (connection?.status !== 'connected') await crew.connect({ userInitiated: true });
      const done = await rejoinConnection(id);
      clearNeedsRejoin(id);
      if (error?.code && CREW_REJOIN_CODES.includes(error.code)) crew.dismissError();
      setRejoin({ connectionId: id, pending: false, error: null, done });
      await crew.refresh().catch(() => undefined);
    } catch (thrown) {
      setRejoin({
        connectionId: id,
        pending: false,
        error: actionErrorText(
          thrown instanceof Error && thrown.message ? thrown.message : rejoinCopy.failed,
          host,
          { isHost: crew.isHost }
        ),
        done: null,
      });
    }
  };

  // Only the devices this notice showed are acknowledged (AO-10): one the daemon lists after it
  // was drawn stays new until the person sees it.
  const acknowledgeDevices = async (origin: HTMLElement) => {
    if (!connectionId || unseenDevices.length === 0) return;
    const id = connectionId;
    const seen = unseenDevices.map((device) => device.fingerprint);
    ackOrigin.current = origin;
    setAckError(null);
    try {
      await acknowledgeNewDevices(id, seen);
      setAcknowledged({ connectionId: id, seen });
      setAckDone((count) => count + 1);
      crew.reloadConnections?.();
    } catch {
      setAckError({ connectionId: id, message: newDevicesCopy.acknowledgeFailed });
    }
  };
  const newestDevice = unseenDevices.reduce<(typeof unseenDevices)[number] | null>(
    (newest, device) =>
      newest === null || (device.added_at ?? 0) > (newest.added_at ?? 0) ? device : newest,
    null
  );

  const olderCrew =
    serverVersion !== null
      ? versionOlder(serverVersion, buildBrokerVersion)
      : connection?.account_identity === false;
  const olderServer =
    !daemonAway &&
    connection?.status === 'connected' &&
    olderCrew &&
    verified !== null &&
    versionDismissed !== connectionId;
  const versionHost = (() => {
    if (!olderServer || crew.isHost) return null;
    const dir = buildPeopleDirectory(verified);
    return dir.host && !dir.host.isFormer ? `@${dir.host.username}` : null;
  })();

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
      {daemonAway && (
        <Note
          tone="warning"
          role="status"
          icon={AlertTriangle}
          testId="crew-daemon-away"
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              disabled={daemon === 'reconnecting'}
              onClick={reconnectDaemon}
            >
              {daemon === 'reconnecting'
                ? connectionBarCopy.daemonReconnecting
                : connectionBarCopy.daemonReconnect}
            </Button>
          }
        >
          <p>{connectionBarCopy.daemonAway}</p>
        </Note>
      )}
      {serverStorage && (
        <Note tone="warning" role="status" icon={AlertTriangle} testId="crew-server-storage">
          <p>
            {connectionBarCopy.serverStorage}{' '}
            {errorContext.hosts === true
              ? connectionBarCopy.serverStorageHost(serverStorage.code)
              : connectionBarCopy.serverStorageMember(serverStorage.code, storageHost)}
          </p>
        </Note>
      )}
      {showRejoin && (
        <Note
          tone={rejoinHere?.error ? 'danger' : 'warning'}
          role="alert"
          icon={KeyRound}
          testId="crew-rejoin"
          action={
            <Button
              type="button"
              variant="secondary"
              size="sm"
              aria-disabled={rejoinHere?.pending || undefined}
              onClick={(event) => keepingFocus(event, addThisComputerAgain)}
            >
              {rejoinHere?.pending ? rejoinCopy.pending : rejoinCopy.action}
            </Button>
          }
        >
          <p>{rejoinHere?.error ?? rejoinText}</p>
        </Note>
      )}
      {rejoinHere?.done && !showRejoin && (
        <Note
          tone="info"
          role="status"
          icon={KeyRound}
          testId="crew-rejoined"
          action={
            <Button
              type="button"
              variant="ghost"
              shape="round"
              size="xs"
              aria-label={connectionBarCopy.dismiss}
              className="size-5"
              onClick={(event) => keepingFocus(event, () => setRejoin(null))}
            >
              <X aria-hidden="true" />
            </Button>
          }
        >
          <p>
            {rejoinCopy.done(workspace)}
            {rejoinHere.done.evictedFingerprint
              ? ` ${rejoinCopy.evicted(rejoinHere.done.evictedFingerprint)}`
              : null}
          </p>
        </Note>
      )}
      {showObservationError && !observationSaidByRejoin && (
        <Note
          tone="warning"
          role="alert"
          icon={AlertTriangle}
          action={
            // Retrying can only help a connection the daemon calls connected, and never a person
            // removed from the workspace (Q2-18). With no saved connection at all, the screen
            // under the bar has its own Try again. Retry reads the saved connection first and,
            // when the daemon has since called it disconnected, connects at once (Q2-01). A
            // deleted workspace offers removal instead: nothing is left to retry.
            workspaceDeleted && connectionId ? (
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={() =>
                  crew.openDialog({
                    kind: 'confirm',
                    confirm: { action: 'remove-connection', connectionId },
                  })
                }
              >
                {crewObservationCopy.removeFromComputer}
              </Button>
            ) : !membershipEnded &&
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

      {actionError && !staleChannelLoss && !redialShownBelow && !errorSaidByRejoin && (
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
          <p>{actionErrorText(actionError.message, host, { isHost: crew.isHost })}</p>
        </Note>
      )}

      {/* A deleted workspace's connect failure is said once, above, with Remove. */}
      {connectError &&
        !workspaceDeleted &&
        (unreachable ? (
          unreachableNote('alert')
        ) : (
          <Note tone="danger" role="alert" icon={AlertTriangle} action={connectAction}>
            <p>{connectErrorText(failure?.kind, connectError.message, host, errorContext)}</p>
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

      {olderServer && (
        <Note
          tone="neutral"
          role="status"
          icon={AlertTriangle}
          testId="crew-older-server"
          action={
            <div className="flex items-center gap-1">
              {crew.isHost && connection ? (
                <Button
                  type="button"
                  variant="secondary"
                  size="sm"
                  onClick={() =>
                    crew.openDialog({
                      kind: 'open-workspace',
                      route: {
                        ssh_target: connection.ssh_target,
                        port: connection.port ?? null,
                        identity_file: connection.identity_file ?? null,
                        proxy_jump: connection.proxy_jump ?? null,
                      },
                      check: true,
                    })
                  }
                >
                  {versionCopy.update}
                </Button>
              ) : null}
              <Button
                type="button"
                variant="ghost"
                shape="round"
                size="xs"
                aria-label={versionCopy.dismiss}
                className="size-5"
                onClick={(event) => keepingFocus(event, () => setVersionDismissed(connectionId))}
              >
                <X aria-hidden="true" />
              </Button>
            </div>
          }
        >
          <p>
            {versionCopy.older(workspace)}
            {crew.isHost ? null : ` ${versionCopy.askHost(versionHost)}`}
          </p>
        </Note>
      )}

      <ChatSetupJobNotes />

      {daemonDevices !== null && unseenDevices.length > 0 && (
        <Note
          tone="info"
          role="status"
          icon={KeyRound}
          testId="crew-new-devices"
          action={
            <div className="flex items-center gap-1">
              <Button
                type="button"
                variant="secondary"
                size="sm"
                aria-label={newDevicesCopy.reviewName}
                onClick={() => crew.openDialog({ kind: 'keys' })}
              >
                {newDevicesCopy.review}
              </Button>
              <Button
                type="button"
                variant="secondary"
                size="sm"
                onClick={(event) => {
                  const origin = event.currentTarget;
                  keepingFocus(event, () => acknowledgeDevices(origin));
                }}
              >
                {newDevicesCopy.acknowledge}
              </Button>
            </div>
          }
        >
          <p>
            {/* Which one: its fingerprint, as Keys lists it and marks it New, and how it was
                added, as `biorouter crew status` says it. */}
            {unseenDevices.length === 1 && newestDevice
              ? newDevicesCopy.one(
                  newestDevice.fingerprint,
                  typeof newestDevice.added_at === 'number'
                    ? calendarDate(newestDevice.added_at)
                    : null,
                  addedViaText(newestDevice.added_via)
                )
              : newDevicesCopy.many(unseenDevices.length)}
          </p>
          {ackErrorHere ? <p className="text-text-danger">{ackErrorHere}</p> : null}
        </Note>
      )}

      {daemonDevices === null && devices.notice && (
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
