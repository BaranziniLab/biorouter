import { useCallback, useEffect, useId, useRef, useState } from 'react';
import { Clock, Inbox, KeyRound, Server, Users } from '../../icons/app-icons';
import { Button } from '../../ui/button';
import { CopyField } from '../../ui/copy-field';
import { Disclosure } from '../../ui/disclosure';
import { Note } from '../../ui/note';
import { Progress } from '../../ui/progress';
import {
  claimJoin,
  groupDeviceCode,
  joinStatus as fetchJoinStatus,
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

function visible(): boolean {
  return typeof document === 'undefined' || document.visibilityState !== 'hidden';
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
  const [state, setState] = useState<CardState>({ kind: 'checking' });
  const [pollError, setPollError] = useState<string | null>(null);
  const [claim, setClaim] = useState<{ pending: boolean; error: string | null }>({
    pending: false,
    error: null,
  });
  const claimedFor = useRef<string | null>(null);
  const [pollNonce, setPollNonce] = useState(0);
  const waitingId = useId();

  const finishJoined = useCallback(() => {
    updateJoinContext(connectionId, { joining: false });
    setJoinStatus('joined');
    void refresh();
  }, [connectionId, refresh, setJoinStatus]);

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
  }, [connectionId, pollNonce, finishJoined, setJoinStatus]);

  // The host approved this computer's code: ask the daemon to finish the join, once per approval.
  const status = state.kind === 'status' ? state.status : null;
  useEffect(() => {
    if (status?.status !== 'approved' || !connectionId) return;
    const approval = `${connectionId}:${status.code ?? ''}`;
    if (claimedFor.current === approval) return;
    claimedFor.current = approval;
    setClaim({ pending: true, error: null });
    claimJoin(connectionId).then(
      () => {
        if (!mounted.current) return;
        setClaim({ pending: false, error: null });
        finishJoined();
      },
      (failure: unknown) => {
        if (!mounted.current) return;
        if (crewErrorCode(failure) === CREW_JOIN_CODE_MISMATCH) {
          // The server now expects a different code: read the status again at once.
          claimedFor.current = null;
          setClaim({ pending: false, error: null });
          setPollNonce((value) => value + 1);
          return;
        }
        setClaim({
          pending: false,
          error: failureMessage(failure, joinStateCopy.claimFailed),
        });
      }
    );
  }, [status, connectionId, finishJoined, mounted]);

  const retryClaim = () => {
    claimedFor.current = null;
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

  const otherWays = (
    <Disclosure label={joinStateCopy.other}>
      <LegacyJoinForm workspace={workspace} username={username || null} />
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
        title={joinStateCopy.invited(personSubject, workspace)}
      >
        {status.status === 'code_mismatch' ? (
          <Note tone="warning">{joinStateCopy.mismatchCode(first)}</Note>
        ) : (
          <p className="text-body text-text-default">{joinStateCopy.sendCode(first)}</p>
        )}
        {code ? (
          <CopyField
            size="code"
            value={code}
            display={groupDeviceCode(code)}
            label={joinStateCopy.codeLabel}
          />
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
      <SetupCard key="approved" icon={Users} title={joinStateCopy.approved(workspace)}>
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
      {pollError && !claim.error ? (
        <div className="crew-onboard-card">
          <Note tone="warning" role="status">
            {joinStateCopy.pollFailed} {pollError}
          </Note>
        </div>
      ) : null}
    </SetupScreen>
  );
}
