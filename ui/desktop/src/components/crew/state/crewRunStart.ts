import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import {
  crewHttp,
  CrewHttpError,
  type Channel,
  type CrewConnection,
  type ObservedRun,
  type Snapshot,
  type Team,
} from '../crewApi';
import { crewActionCopy } from './copy';
import type {
  ActionKey,
  ActOptions,
  ErrorSource,
  ObservedPrivacy,
  StartOwnedRunInput,
  SurfaceResetReason,
} from './types';

interface PendingRunAttempt {
  fingerprint: string;
  key: string;
  unknownDestination?: string;
}
// Inspection can navigate to another route; retain the uncertain attempt in memory, never on disk.
// Module scope makes the lock survive a remount of Crew (C10).
let unfinishedRunAttempt: PendingRunAttempt | null = null;

/**
 * How long a started task waits for a verified `state` frame that lists it before the observation
 * is started over (Q3-06). The daemon sends a state frame every 2 s while it observes.
 */
export const RUN_START_FRAME_WAIT_MS = 5000;

export interface CrewRunStartContext {
  connectionId: string;
  teamId: string;
  channelId: string;
  connection: CrewConnection | null;
  team: Team | null;
  channel: Channel | null;
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  /** The runs of the last verified `state` frame. */
  runs: readonly ObservedRun[];
  setBody: Dispatch<SetStateAction<string>>;
  /**
   * Observe again without clearing what is on screen: the verified view stays until the new
   * observation's first frame replaces it, or its end clears it.
   */
  restartObservation(): void;
  resetSurfaces(reason: SurfaceResetReason): void;
  act<T>(
    source: ErrorSource,
    key: ActionKey,
    fn: () => Promise<T>,
    options?: ActOptions
  ): Promise<T | undefined>;
}

export interface CrewRunStart {
  startOwnedRun(input: StartOwnedRunInput): Promise<boolean>;
  unknownRunDestination: string;
  inspectedPriorRun: boolean;
  setInspectedPriorRun: Dispatch<SetStateAction<boolean>>;
  pendingRun: MutableRefObject<PendingRunAttempt | null>;
}

/**
 * Start the person's own agent in the selected channel.
 *
 * The request carries the verified privacy epochs and one request id. A retry of an unchanged
 * payload reuses the id; a changed payload rotates it. When the daemon answers
 * `crew_start_outcome_unknown`, the attempt is locked (in module scope, so a remount cannot lose it):
 * nothing starts again until the person confirms they inspected the earlier task and chooses a
 * deliberate restart, which always rotates the id.
 *
 * A successful start never blanks the channel (live QA round 3, Q3-06). It used to end with a
 * refresh, which dropped the verified view and showed "Checking connection" and "Verifying
 * access…" for seconds right after the person's own action. The verified view, the messages and
 * the composer now stay, and the observer's next verified `state` frame brings the task. When no
 * verified frame lists it within `RUN_START_FRAME_WAIT_MS`, the observation is started over, which
 * keeps the view until its first frame. SECURITY-SENSITIVE (human review): only the viewer's own
 * successful `run.create` is bridged this way, and it cannot narrow what the viewer may read; every
 * observer refusal and privacy change still ends the view and clears what it protected.
 */
export function useCrewRunStart(context: CrewRunStartContext): CrewRunStart {
  const pendingRun = useRef<PendingRunAttempt | null>(unfinishedRunAttempt);
  const [unknownRunDestination, setUnknownRunDestination] = useState(
    unfinishedRunAttempt?.unknownDestination ?? ''
  );
  const [inspectedPriorRun, setInspectedPriorRun] = useState(false);
  const {
    connectionId,
    teamId,
    channelId,
    connection,
    team,
    channel,
    snapshot,
    observedPrivacy,
    runs,
    setBody,
    restartObservation,
    resetSurfaces,
    act,
  } = context;

  // The started task the view waits for, and the timer that observes again if it does not come.
  const awaited = useRef<{
    connectionId: string;
    runId: string | null;
    timer: ReturnType<typeof setTimeout>;
  } | null>(null);
  const restart = useRef(restartObservation);
  useEffect(() => {
    restart.current = restartObservation;
  }, [restartObservation]);
  const listed = useRef(runs);
  useEffect(() => {
    listed.current = runs;
  }, [runs]);
  const settleAwaited = useCallback(() => {
    if (awaited.current) clearTimeout(awaited.current.timer);
    awaited.current = null;
  }, []);
  const awaitStartedRun = (runId: string | null) => {
    settleAwaited();
    // A frame that listed it may already have arrived while the start was answered.
    if (runId !== null && listed.current.some((run) => run.run_id === runId)) return;
    const entry = {
      connectionId,
      runId,
      timer: setTimeout(() => {
        if (awaited.current !== entry) return;
        awaited.current = null;
        restart.current();
      }, RUN_START_FRAME_WAIT_MS),
    };
    awaited.current = entry;
  };
  // A verified frame listed the task: nothing to wait for. Another connection, or a view that
  // ended (an observer refusal, a privacy change, a Disconnect): not this wait's to observe again —
  // what ended the view decides what happens next, and a stopped view is the person's to retry.
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  useEffect(() => {
    const entry = awaited.current;
    if (!entry) return;
    if (
      !verified ||
      entry.connectionId !== connectionId ||
      (entry.runId !== null && runs.some((run) => run.run_id === entry.runId))
    )
      settleAwaited();
  }, [runs, connectionId, verified, settleAwaited]);
  useEffect(() => settleAwaited, [settleAwaited]);

  const submitOwnedRun = async ({
    prompt,
    provider,
    model,
    contextChannels,
    deliberateRestart = false,
    clearBody = false,
  }: StartOwnedRunInput) => {
    if (!snapshot || observedPrivacy?.connectionId !== connectionId)
      throw new Error(crewActionCopy.grantPrivacyUnverified);
    if (pendingRun.current?.unknownDestination && !deliberateRestart) {
      throw new Error(crewActionCopy.unknownOutcomeGate);
    }
    if (deliberateRestart && (!pendingRun.current?.unknownDestination || !inspectedPriorRun)) {
      throw new Error(crewActionCopy.restartNeedsInspection);
    }
    const payload = {
      expected_mode: observedPrivacy.mode,
      expected_policy_epoch: observedPrivacy.policyEpoch,
      expected_workspace_policy_epoch: snapshot.workspace.policy_epoch,
      channel_id: channelId,
      prompt,
      provider,
      model,
      context_channels: [channelId, ...contextChannels],
      posting_grant: true,
    };
    const fingerprint = JSON.stringify({ connectionId, ...payload });
    if (deliberateRestart || pendingRun.current?.fingerprint !== fingerprint) {
      pendingRun.current = { fingerprint, key: crypto.randomUUID() };
    }
    unfinishedRunAttempt = pendingRun.current;
    if (deliberateRestart) {
      setUnknownRunDestination('');
      setInspectedPriorRun(false);
    }
    let started: unknown;
    try {
      started = await crewHttp<unknown>(`/connections/${connectionId}/runs`, 'POST', {
        ...payload,
        request_id: pendingRun.current.key,
      });
    } catch (failure) {
      if (failure instanceof CrewHttpError && failure.code === 'crew_start_outcome_unknown') {
        const destination = `${connection?.name ?? connectionId} / ${team?.name ?? teamId} / #${channel?.name ?? channelId}`;
        pendingRun.current.unknownDestination = destination;
        unfinishedRunAttempt = pendingRun.current;
        setUnknownRunDestination(destination);
        setInspectedPriorRun(false);
      }
      throw failure;
    }
    pendingRun.current = null;
    unfinishedRunAttempt = null;
    setUnknownRunDestination('');
    setInspectedPriorRun(false);
    resetSurfaces('run-started');
    if (clearBody) setBody('');
    const runId =
      started !== null && typeof started === 'object'
        ? (started as { run_id?: unknown }).run_id
        : undefined;
    awaitStartedRun(typeof runId === 'string' && runId ? runId : null);
    return true;
  };

  const startOwnedRun = async (input: StartOwnedRunInput) =>
    (await act('pane:agent', 'run.start', () => submitOwnedRun(input))) === true;

  return {
    startOwnedRun,
    unknownRunDestination,
    inspectedPriorRun,
    setInspectedPriorRun,
    pendingRun,
  };
}
