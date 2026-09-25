import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Clock, Inbox, KeyRound, Server, Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { Progress } from '../../ui/progress';
import {
  claimJoin,
  CREW_NOT_CONNECTED,
  groupDeviceCode,
  joinStatus as fetchJoinStatus,
  type CrewJoinClaim,
  type CrewJoinStatus,
} from '../api/join';
import { CREW_JOIN_CODE_MISMATCH, crewErrorCode, isStaleDaemon } from '../api/errors';
import {
  connectionNames,
  personFromProjection,
  personLabel,
  sanitizeDisplayText,
  type CrewPerson,
} from '../identity';
import { useCrew } from '../state/CrewControllerContext';
import { failureMessage } from '../state/observationFailure';
import { joinStateCopy, legacyJoinCopy } from './copy';
import { useMounted } from './fields';
import { forgetJoinClaim, readJoinClaim, updateJoinClaim, useJoinClaim } from './joinClaimState';
import { updateJoinContext, useJoinContext } from './joinContext';
import { firstName, sshUsername } from './joinText';
import { LegacyJoinForm } from './LegacyJoinForm';
import { SetupCard, SetupScreen, Spinner } from './parts';

/** How often the join screen asks where this computer's join stands, while it is visible. */
export const JOIN_POLL_INTERVAL_MS = 5000;

/**
 * The status this renderer reports for a workspace that cannot join by invitation code while this
 * computer is not one of its devices: the invitation-token path. The daemon never sends it.
 */
export const LEGACY_JOIN_STATUS = 'legacy';

type CardState =
  | { kind: 'checking' }
  | { kind: 'legacy' }
  | { kind: 'status'; status: CrewJoinStatus };

/**
 * Whether the join route can reach the workspace. `reconnecting`: it answered
 * `crew_not_connected` and the card is reconnecting once by itself; `lost`: that did not help, so
 * the person reconnects (a user-initiated connect opens Sign in when the server asks).
 */
type Link = 'ok' | 'reconnecting' | 'lost';

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
}

/** One host approval of this computer's code, as the claim path counts it. */
function approvalOf(connectionId: string, status: CrewJoinStatus): string {
  return `${connectionId}:${status.code ?? ''}`;
}

/**
 * The join state machine in the channel column (naming slice S3a).
 *
 * It asks `GET …/join` every five seconds while the page is visible and shows one card per state.
 * The device code it shows is the one THIS computer's daemon computed from its own saved key —
 * `joinStatus` accepts nothing else — so nothing the workspace's server returns can change what
 * the person reads out to their host. When the host approves, the daemon is asked to finish the
 * join; pressing Join was the consent.
 */
export function JoinStatusCard() {
  const crew = useCrew();
  const { connectionId, connection, setJoinStatus, refresh, openDialog } = crew;
  const context = useJoinContext(connectionId);
  const mounted = useMounted();
  // The probe may already know this is the token path; say so at once rather than "Checking".
  const [state, setState] = useState<CardState>(() =>
    crew.joinStatus === LEGACY_JOIN_STATUS ? { kind: 'legacy' } : { kind: 'checking' }
  );
  const [pollError, setPollError] = useState<string | null>(null);
  const [claim, setClaim] = useState<{ pending: boolean; error: string | null }>({
    pending: false,
    error: null,
  });
  // The claim and reconnect counters live in `joinClaimState`, per connection, not in this card:
  // the card's own reconnect unmounts it (the screen is `connecting` while a connect runs), so a
  // counter held here restarted from zero on every attempt. Callbacks read the store at the moment
  // they run (`readJoinClaim`); render reads it through `useJoinClaim`. What they count:
  // - `claimedFor`: the approval a claim was sent for; nothing claims it again by itself.
  // - `claimReconnectFor`: the approval whose claim has used its one automatic reconnect. Counted
  //   apart from `reconnectTried`, which every answered poll resets: a claim only ever runs after
  //   a poll answered, so that counter alone let a link that drops on each claim reconnect and
  //   re-claim, signed with the device key, without end.
  // - `claimLost`: the approval whose claim still found no connection after that reconnect. It
  //   stays claimed (`claimedFor`), so nothing claims it again by itself, and the card offers
  //   Reconnect for as long as the status still says that approval, however many polls answer.
  // - `reconnectTried`: the poll path's automatic reconnect, one per loss of the connection,
  //   reset once the route answers again.
  // - `connecting`: a connect the card started is running. A second not-connected answer (the poll
  //   and a claim can both give one, and so can a new mount of this card) waits for its outcome
  //   instead of counting as a failed attempt.
  const claimState = useJoinClaim(connectionId);
  const [pollNonce, setPollNonce] = useState(0);
  const waitingId = useId();
  const [link, setLink] = useState<Link>('ok');
  // This mount met a not-connected answer while a connect was running, possibly one an earlier
  // mount started (that connect is what unmounted it, so its own "ask again" lands on no mount):
  // ask again once the store says the connect settled.
  const awaitingConnect = useRef(false);
  // The approval whose claim failed with an error this mount shows beside Retry. The error is this
  // mount's display state, so when the mount goes the approval is released and the next mount
  // claims it once, rather than spinning on a claim nobody is making.
  const claimErrorFor = useRef<string | null>(null);
  // This mount finished the join. The store forgets the connection then, so this flag is what stops
  // a poll that still answers `approved` from claiming again before the screen moves on.
  const joined = useRef(false);
  // The controller's `connect` is bound to the render it came from; read the newest one.
  const connectRef = useRef(crew.connect);
  useEffect(() => {
    connectRef.current = crew.connect;
  });

  const finishJoined = useCallback(
    (claimed?: CrewJoinClaim) => {
      // The join is done: nothing is left to count, and a later join of this connection starts
      // clean.
      joined.current = true;
      forgetJoinClaim(connectionId);
      // The workspace named who invited this computer: remember it for the screens that follow.
      const inviter = claimed?.inviter;
      updateJoinContext(connectionId, {
        joining: false,
        ...(inviter
          ? { hostUsername: inviter.username, hostDisplayName: inviter.display_name ?? null }
          : {}),
        ...(claimed?.workspace_name ? { workspaceName: claimed.workspace_name } : {}),
      });
      setJoinStatus('joined');
      void refresh();
    },
    [connectionId, refresh, setJoinStatus]
  );

  /**
   * The join route answered `crew_not_connected`: reconnect once by itself and ask again. If the
   * route still can't reach the workspace, stop asking and offer Reconnect instead of repeating a
   * poll error that can never clear by itself.
   *
   * Two counters in the store decide "once", and survive the remount that connect causes. A
   * poll's reconnect is `reconnectTried`, reset by every answered poll. A claim's is
   * `claimReconnectFor`, one per approval: the claim path clears `reconnectTried` before calling
   * here so its one reconnect really runs, and does not call here a second time for the same
   * approval.
   */
  const recoverConnection = useCallback(() => {
    setPollError(null);
    const counted = readJoinClaim(connectionId);
    if (counted.connecting) {
      awaitingConnect.current = true;
      setLink('reconnecting');
      return;
    }
    if (counted.reconnectTried) {
      setLink('lost');
      return;
    }
    updateJoinClaim(connectionId, { reconnectTried: true, connecting: true });
    setLink('reconnecting');
    // `connect` records its own failure; the next answer says whether it helped. The store is
    // written whether or not this mount survived the connect; only this mount's state is gated.
    const askAgain = () => {
      updateJoinClaim(connectionId, { connecting: false });
      if (!mounted.current) return;
      awaitingConnect.current = false;
      setPollNonce((value) => value + 1);
    };
    connectRef.current().then(askAgain, askAgain);
  }, [connectionId, mounted]);

  const reconnect = () => {
    if (readJoinClaim(connectionId).connecting) return;
    updateJoinClaim(connectionId, { connecting: true });
    setLink('reconnecting');
    const askAgain = () => {
      // After a blocked claim, the person's connect stands in for that approval's reconnect: it
      // gets one fresh claim, and a claim that still finds no connection comes back to Reconnect
      // rather than reconnecting by itself (`claimReconnectFor` still names the approval). This
      // runs before the mount check: the connect usually unmounted the card that was pressed, and
      // the mount that replaced it must see the approval released, or Reconnect stays forever.
      updateJoinClaim(connectionId, { connecting: false, claimedFor: null, claimLost: null });
      if (!mounted.current) return;
      awaitingConnect.current = false;
      setClaim({ pending: false, error: null });
      setPollNonce((value) => value + 1);
    };
    crew.connect({ userInitiated: true }).then(askAgain, askAgain);
  };

  // A connect this mount did not start (or that unmounted the mount that did) settled: ask again.
  // Keyed on the whole entry, not `connecting` alone, so a flip this mount never rendered in
  // between still counts.
  useEffect(() => {
    if (claimState.connecting || !awaitingConnect.current) return;
    awaitingConnect.current = false;
    setPollNonce((value) => value + 1);
  }, [claimState]);

  // The error beside Retry goes with this mount; release its approval so the next mount claims it.
  useEffect(
    () => () => {
      const approval = claimErrorFor.current;
      if (approval && readJoinClaim(connectionId).claimedFor === approval) {
        updateJoinClaim(connectionId, { claimedFor: null });
      }
    },
    [connectionId]
  );

  // Poll while visible; stop once the answer can no longer change by itself.
  useEffect(() => {
    if (!connectionId) return;
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | null = null;
    let inFlight = false;
    let stopped = false;
    const schedule = () => {
      if (stopped || controller.signal.aborted) return;
      if (timer) clearTimeout(timer);
      timer = setTimeout(() => void tick(), JOIN_POLL_INTERVAL_MS);
    };
    const tick = async () => {
      if (controller.signal.aborted || inFlight) return;
      if (!visible()) {
        schedule();
        return;
      }
      inFlight = true;
      try {
        const result = await fetchJoinStatus(connectionId, controller.signal);
        if (controller.signal.aborted) return;
        setPollError(null);
        updateJoinClaim(connectionId, { reconnectTried: false });
        setLink('ok');
        if (result.status === 'unsupported') {
          stopped = true;
          setState({ kind: 'legacy' });
          setJoinStatus(LEGACY_JOIN_STATUS);
          return;
        }
        setState({ kind: 'status', status: result });
        if (result.status === 'joined') {
          stopped = true;
          finishJoined();
          return;
        }
        setJoinStatus(result.status);
      } catch (failure) {
        if (controller.signal.aborted) return;
        if (isStaleDaemon(failure)) {
          stopped = true;
          setState({ kind: 'legacy' });
          setJoinStatus(LEGACY_JOIN_STATUS);
          return;
        }
        if (crewErrorCode(failure) === CREW_NOT_CONNECTED) {
          // Asking again cannot help until the connection is back.
          stopped = true;
          recoverConnection();
          return;
        }
        setPollError(failureMessage(failure, joinStateCopy.pollFailed));
      } finally {
        inFlight = false;
        schedule();
      }
    };
    const onVisibility = () => {
      if (!visible() || stopped) return;
      if (timer) clearTimeout(timer);
      void tick();
    };
    void tick();
    document.addEventListener('visibilitychange', onVisibility);
    return () => {
      controller.abort();
      if (timer) clearTimeout(timer);
      document.removeEventListener('visibilitychange', onVisibility);
    };
  }, [connectionId, pollNonce, finishJoined, setJoinStatus, recoverConnection]);

  // The host approved this computer's code: ask the daemon to finish the join, once per approval.
  const status = state.kind === 'status' ? state.status : null;
  useEffect(() => {
    if (status?.status !== 'approved' || !connectionId) return;
    if (joined.current) return;
    const approval = approvalOf(connectionId, status);
    if (readJoinClaim(connectionId).claimedFor === approval) return;
    updateJoinClaim(connectionId, { claimedFor: approval });
    setClaim({ pending: true, error: null });
    claimJoin(connectionId).then(
      (claimed) => {
        // Unmounted: leave the store as it is. The approval stays claimed, so no mount sends a
        // second claim for a join that succeeded; the next mount's poll reads `joined` and
        // finishes. (`finishJoined` is not called from here: it reports to the controller, whose
        // selected connection may no longer be this one.)
        if (!mounted.current) return;
        setClaim({ pending: false, error: null });
        finishJoined(claimed);
      },
      (failure: unknown) => {
        const counted = readJoinClaim(connectionId);
        if (crewErrorCode(failure) === CREW_NOT_CONNECTED) {
          if (counted.claimReconnectFor !== approval) {
            // Reconnect once by itself for this approval, and claim again once the connection is
            // back and the status still says approved. The reconnect is charged to the approval
            // even when this mount is gone (no connect is started from an unmounted card): the
            // next mount's claim is the "again", and it cannot reconnect by itself a second time.
            updateJoinClaim(connectionId, {
              claimReconnectFor: approval,
              claimedFor: null,
              reconnectTried: false,
            });
            if (!mounted.current) return;
            setClaim({ pending: false, error: null });
            recoverConnection();
            return;
          }
          // That reconnect did not help. Keep the approval claimed so no poll claims it again by
          // itself, and wait for the person to press Reconnect.
          updateJoinClaim(connectionId, { claimLost: approval });
          if (mounted.current) setClaim({ pending: false, error: null });
          return;
        }
        if (!mounted.current) {
          // The outcome lands on no mount: release the approval, or the next mount would spin on
          // a claim nobody is making.
          if (counted.claimedFor === approval) updateJoinClaim(connectionId, { claimedFor: null });
          return;
        }
        if (crewErrorCode(failure) === CREW_JOIN_CODE_MISMATCH) {
          // The server now expects a different code: read the status again at once.
          updateJoinClaim(connectionId, { claimedFor: null });
          setClaim({ pending: false, error: null });
          setPollNonce((value) => value + 1);
          return;
        }
        claimErrorFor.current = approval;
        setClaim({
          pending: false,
          error: failureMessage(failure, joinStateCopy.claimFailed),
        });
      }
    );
  }, [status, connectionId, finishJoined, mounted, recoverConnection]);

  const retryClaim = () => {
    // A person's press: one fresh claim, with its own automatic reconnect if the link dropped.
    claimErrorFor.current = null;
    updateJoinClaim(connectionId, { claimReconnectFor: null, claimedFor: null, claimLost: null });
    setClaim({ pending: false, error: null });
    setPollNonce((value) => value + 1);
  };

  // Who to ask: the inviter the workspace named, else the host the invitation named.
  const inviter: CrewPerson | null =
    personFromProjection(status?.inviter) ??
    (context.hostUsername
      ? personFromProjection({
          username: context.hostUsername,
          display_name: context.hostDisplayName,
        })
      : null);
  const person = inviter ? personLabel(inviter, 'inline') : joinStateCopy.yourHost;
  const personSubject = inviter ? person : joinStateCopy.yourHostSubject;
  const first = firstName(inviter) ?? joinStateCopy.yourHost;
  const workspace =
    sanitizeDisplayText(status?.workspace_name) ||
    sanitizeDisplayText(context.workspaceName) ||
    (connection ? (connectionNames([connection]).get(connection.id) ?? '') : '') ||
    joinStateCopy.theWorkspace;
  const username = sshUsername(connection?.ssh_target) ?? sanitizeDisplayText(context.username);
  const code = status?.code ?? null;
  // The invitation adds this computer to the person's existing account.
  const addDevice = status?.add_device === true;
  // The claim for the approval on screen found no connection twice: offer Reconnect.
  const claimBlocked =
    status?.status === 'approved' && claimState.claimLost === approvalOf(connectionId, status);
  // A connect the card started is running, whichever mount started it.
  const reconnecting = link === 'reconnecting' || claimState.connecting;

  // The token path, folded under "Having trouble joining?". It opens on its condition ("If @alice
  // asks for it, send this instead:") with the device key folded again, so it never reads as a
  // second thing to send after the code (T-35, Q2-35).
  const otherWays = (
    <Disclosure label={joinStateCopy.other}>
      <LegacyJoinForm
        workspace={workspace}
        username={username || null}
        host={inviter ? `@${inviter.username}` : joinStateCopy.yourHost}
      />
    </Disclosure>
  );

  let card;
  if (context.hostSetup) {
    // This computer saved the workspace from Host but never created it: finish that instead.
    card = (
      <SetupCard key="host" icon={Server} title={joinStateCopy.hostPendingTitle(workspace)}>
        <p className="text-body text-text-muted">{joinStateCopy.hostPendingBody}</p>
        <div className="crew-onboard-actions">
          <Button type="button" onClick={() => openDialog({ kind: 'host' })}>
            {joinStateCopy.hostPendingAction}
          </Button>
        </div>
      </SetupCard>
    );
  } else if (state.kind === 'legacy') {
    card = (
      <SetupCard key="legacy" icon={KeyRound} title={legacyJoinCopy.title}>
        <LegacyJoinForm workspace={workspace} username={username || null} />
      </SetupCard>
    );
  } else if (state.kind === 'checking') {
    card = (
      <SetupCard key="checking" title={joinStateCopy.checking}>
        <Spinner />
      </SetupCard>
    );
  } else if (status?.status === 'invited' || status?.status === 'code_mismatch') {
    card = (
      <SetupCard
        key={status.status}
        icon={Inbox}
        title={
          addDevice
            ? joinStateCopy.invitedDevice(personSubject, workspace)
            : joinStateCopy.invited(personSubject, workspace)
        }
      >
        {status.status === 'code_mismatch' ? (
          <Note tone="warning">{joinStateCopy.mismatchCode(first)}</Note>
        ) : (
          <p className="text-body text-text-default">{joinStateCopy.sendCode(first)}</p>
        )}
        {code ? (
          // Copy what is shown, dashes and all (T-36): the host's code field takes it either way.
          <CopyField size="code" value={groupDeviceCode(code)} label={joinStateCopy.codeLabel} />
        ) : null}
        <Progress indeterminate aria-labelledby={waitingId} />
        <p id={waitingId} className="text-supporting text-text-muted">
          {joinStateCopy.waiting(first)}
        </p>
        {otherWays}
      </SetupCard>
    );
  } else if (status?.status === 'approved' || status?.status === 'joined') {
    card = (
      <SetupCard
        key="approved"
        icon={Users}
        title={
          addDevice ? joinStateCopy.approvedDevice(workspace) : joinStateCopy.approved(workspace)
        }
      >
        {claim.error ? (
          <Note
            tone="danger"
            role="alert"
            action={
              <Button type="button" size="sm" variant="outline" onClick={retryClaim}>
                {joinStateCopy.retry}
              </Button>
            }
          >
            {claim.error}
          </Note>
        ) : (
          <Spinner />
        )}
      </SetupCard>
    );
  } else if (status?.status === 'expired') {
    card = <SetupCard key="expired" icon={Clock} title={joinStateCopy.expired(person)} />;
  } else {
    card = (
      <SetupCard key="not-invited" icon={Users} title={joinStateCopy.notInvitedTitle(workspace)}>
        <p className="text-body text-text-default">
          {joinStateCopy.notInvitedBody(person, username || null)}
        </p>
        <CopyField
          multiline
          label={joinStateCopy.notInvitedMessageLabel}
          value={joinStateCopy.notInvitedMessage(
            inviter ? first : null,
            username || null,
            workspace
          )}
        />
        {otherWays}
      </SetupCard>
    );
  }

  return (
    <SetupScreen>
      {card}
      {reconnecting ? (
        <div className="crew-onboard-card">
          <Note tone="neutral" role="status" testId="crew-join-reconnecting">
            {joinStateCopy.reconnecting(workspace)}
          </Note>
        </div>
      ) : link === 'lost' || claimBlocked ? (
        <div className="crew-onboard-card">
          <Note
            tone="warning"
            role="status"
            testId="crew-join-not-connected"
            action={
              <Button type="button" size="sm" variant="outline" onClick={reconnect}>
                {joinStateCopy.reconnect}
              </Button>
            }
          >
            {joinStateCopy.notConnected(workspace)}
          </Note>
        </div>
      ) : pollError && !claim.error ? (
        <div className="crew-onboard-card">
          <Note tone="warning" role="status">
            {joinStateCopy.pollFailed} {pollError}
          </Note>
        </div>
      ) : null}
    </SetupScreen>
  );
}
