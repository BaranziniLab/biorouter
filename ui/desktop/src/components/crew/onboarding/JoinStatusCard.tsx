import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
} from 'react';
import { Clock, Inbox, KeyRound, Server, Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { SecretInput } from '../../ui/secret-input';
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
import { crewActionCopy } from '../state/copy';
import { failureMessage } from '../state/observationFailure';
import { connectionVerifiedThisSession } from '../state/useCrewConnections';
import { joinStateCopy, legacyJoinCopy } from './copy';
import { useMounted } from './fields';
import { forgetJoinClaim, readJoinClaim, updateJoinClaim, useJoinClaim } from './joinClaimState';
import { readJoinContext, updateJoinContext, useJoinContext } from './joinContext';
import { firstName, invitationExpiry, membershipEnded, sshUsername } from './joinText';
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
 * Record what the workspace just said about who invited this computer (and what it calls itself),
 * the moment it says it (Q3-46). The rail's "Waiting for …" and "Having trouble joining?" read the
 * join context, so recording it only when the join finished left them naming `@alice` beside a card
 * that said "Alice Chen (@alice)". Nothing is written when nothing changed: a poll every five
 * seconds must not re-render every reader.
 */
function rememberInviter(
  connectionId: string,
  inviter: { username: string; display_name?: string | null } | null | undefined,
  workspaceName: string | null | undefined
): void {
  if (!connectionId) return;
  const known = readJoinContext(connectionId);
  const patch: { hostUsername?: string; hostDisplayName?: string | null; workspaceName?: string } =
    {};
  if (inviter?.username) {
    const displayName = inviter.display_name ?? null;
    if (
      known.hostUsername !== inviter.username ||
      (known.hostDisplayName ?? null) !== displayName
    ) {
      patch.hostUsername = inviter.username;
      patch.hostDisplayName = displayName;
    }
  }
  if (workspaceName && known.workspaceName !== workspaceName) patch.workspaceName = workspaceName;
  if (Object.keys(patch).length > 0) updateJoinContext(connectionId, patch);
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
      rememberInviter(connectionId, claimed?.inviter, claimed?.workspace_name);
      updateJoinContext(connectionId, { joining: false });
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
        // Who invited this computer is true whichever mount hears it (Q3-46).
        rememberInviter(connectionId, claimed?.inviter, claimed?.workspace_name);
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

  // A poll that names the inviter names them for the rail and "Having trouble joining?" too, at
  // once rather than when the join finishes (Q3-46).
  const inviterUsername = status?.inviter?.username ?? null;
  const inviterDisplayName = status?.inviter?.display_name ?? null;
  const statusWorkspace = status?.workspace_name ?? null;
  useEffect(() => {
    rememberInviter(
      connectionId,
      inviterUsername ? { username: inviterUsername, display_name: inviterDisplayName } : null,
      statusWorkspace
    );
  }, [connectionId, inviterUsername, inviterDisplayName, statusWorkspace]);

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

  // "Having trouble joining?", folded (Q4-43): while a code is out it first says that waiting is
  // normal; then the join request, on its condition and behind Show; then the token path behind
  // its own quiet link. It never reads as a second thing to send after the code (T-35, Q2-35), and
  // names the host as the card's sentences do (Q3-46).
  const waitingForHost = status?.status === 'invited' || status?.status === 'code_mismatch';
  const otherWays = (
    <Disclosure label={joinStateCopy.other}>
      <TroubleJoining
        workspace={workspace}
        username={username || null}
        host={first}
        reassure={waitingForHost}
      />
    </Disclosure>
  );
  const expiry = invitationExpiry(status?.expires_at);
  // This computer was a member and the workspace no longer admits it: removed, not "not yet"
  // invited (Q3-50). Seen verified this session, or the daemon recorded the membership's end.
  const removed = connectionVerifiedThisSession(connectionId) || membershipEnded(connection);

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
        {/* A wait on a person, possibly for hours: a still clock, never a bar that sweeps as if
            the computer were working on it (Q4-46). */}
        <p
          id={waitingId}
          className="crew-onboard-wait text-supporting text-text-muted"
          data-testid="crew-join-waiting"
        >
          <Clock aria-hidden className="crew-onboard-wait-icon" />
          <span>{joinStateCopy.waiting(first)}</span>
        </p>
        <p className="text-supporting text-text-muted" data-testid="crew-join-wait-note">
          {expiry
            ? `${expiry.expired ? joinStateCopy.expiredNow : joinStateCopy.expires(expiry.when)} `
            : null}
          {joinStateCopy.closeNote(first, workspace)}
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
  } else if (removed) {
    card = (
      <SetupCard key="removed" icon={Users} title={joinStateCopy.removedTitle(workspace)}>
        <p className="text-body text-text-default">
          {joinStateCopy.removedBody(workspace, person)}
        </p>
      </SetupCard>
    );
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
    // Anchored to the top of the column, so a section that opens ("Having trouble joining?") grows
    // the card downward instead of moving the code under the pointer (Q3-48).
    <SetupScreen anchor="top">
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

/**
 * What "Having trouble joining?" holds (Q4-43), one mechanism at a time:
 *
 * 1. While a code is out (`reassure`): waiting is normal, and how the host lets them in.
 * 2. "If Alice asks for a join request:" with the request — the person's username and this
 *    computer's public device key, never a secret — in a `CopyField` behind **Show the join
 *    request**.
 * 3. **Alice sent me a token instead**, a quiet link that reveals the token field and **Join with a
 *    token**. The field stays masked: a token is a credential. The broker decides (`auth.enroll`),
 *    exactly as the token-only path (`LegacyJoinForm`) sends it.
 */
function TroubleJoining({
  workspace,
  username,
  host,
  reassure,
}: {
  workspace: string;
  username: string | null;
  /** The host, as the card's sentences name them ("Alice", `@alice`, or "your host"). */
  host: string;
  reassure: boolean;
}) {
  const crew = useCrew();
  const mounted = useMounted();
  const publicKey = crew.connection?.public_key ?? '';
  const connectionId = crew.connectionId;
  const [requestShown, setRequestShown] = useState(false);
  const [tokenShown, setTokenShown] = useState(false);
  const [token, setToken] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestId = useId();
  const tokenFieldRef = useRef<HTMLInputElement>(null);

  // The link leaves as the field arrives: put focus in the field rather than on the page.
  const [focusToken, setFocusToken] = useState(false);
  useLayoutEffect(() => {
    if (!focusToken || !tokenShown) return;
    setFocusToken(false);
    tokenFieldRef.current?.focus();
  }, [focusToken, tokenShown]);

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    if (pending || !token.trim()) return;
    setPending(true);
    setError(null);
    try {
      await crew.request(
        'auth.enroll',
        { invitation: token.trim(), public_key: publicKey },
        { mutation: true }
      );
      if (!mounted.current) return;
      setToken('');
      updateJoinContext(connectionId, { joining: false });
      crew.setJoinStatus('joined');
      await crew.refresh();
    } catch (failure) {
      if (mounted.current) setError(failureMessage(failure, crewActionCopy.actionFallback));
    } finally {
      if (mounted.current) setPending(false);
    }
  };

  return (
    <div className="crew-onboard-stack" data-testid="crew-join-trouble">
      {reassure ? (
        <p className="text-body text-text-default">{joinStateCopy.troubleWaiting(host)}</p>
      ) : null}
      {publicKey ? (
        <>
          <p className="text-body text-text-default">{joinStateCopy.otherBody(host)}</p>
          <div className="crew-onboard-row">
            <Button
              type="button"
              variant="link"
              className="h-auto p-0"
              aria-expanded={requestShown}
              aria-controls={requestShown ? requestId : undefined}
              onClick={() => setRequestShown((shown) => !shown)}
            >
              {requestShown ? legacyJoinCopy.hideRequest : legacyJoinCopy.showJoinRequest}
            </Button>
          </div>
          {requestShown ? (
            <div id={requestId}>
              <CopyField
                multiline
                label={legacyJoinCopy.requestLabel}
                value={legacyJoinCopy.request(workspace, username, publicKey)}
              />
            </div>
          ) : null}
        </>
      ) : null}
      {tokenShown ? (
        <form className="crew-onboard-stack" onSubmit={(event) => void submit(event)}>
          <SecretInput
            ref={tokenFieldRef}
            aria-label={legacyJoinCopy.tokenName}
            revealLabel={legacyJoinCopy.tokenPlaceholder.toLowerCase()}
            placeholder={legacyJoinCopy.tokenPlaceholder}
            required
            disabled={pending}
            value={token}
            onChange={(event) => setToken(event.target.value)}
          />
          <div className="crew-onboard-actions">
            <Button type="submit" disabled={pending || !token.trim()}>
              {pending ? legacyJoinCopy.submitting : legacyJoinCopy.submitToken}
            </Button>
          </div>
        </form>
      ) : (
        <div className="crew-onboard-row">
          <Button
            type="button"
            variant="link"
            className="h-auto p-0 text-text-muted"
            onClick={() => {
              setTokenShown(true);
              setFocusToken(true);
            }}
          >
            {joinStateCopy.tokenInstead(host)}
          </Button>
        </div>
      )}
      {error ? (
        <Note tone="danger" role="alert">
          {error}
        </Note>
      ) : null}
    </div>
  );
}
