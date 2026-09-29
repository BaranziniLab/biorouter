import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import type * as Api from '../../../api/types.gen';
import { isTransportFailure } from '../api/errors';
import { wireOf } from '../api/parse';
import { crewHttp, CrewHttpError, type Channel, type ObservedRun, type Snapshot } from '../crewApi';
import { channelName, teamName } from '../identity/objectNames';
import { crewActionCopy } from './copy';
import { postDestination } from './crewSend';
import { resetBetweenTests } from './draftStash';
import {
  draftScope,
  draftScopeChanged,
  failureCode,
  failureMessage,
  type DraftScope,
  type ScopeFrame,
} from './observationFailure';
import type {
  ActionKey,
  ActOptions,
  ErrorDetails,
  ErrorSource,
  ObservedPrivacy,
  StartOwnedRunInput,
  SurfaceResetReason,
  UnknownRunDestination,
} from './types';

interface PendingRunAttempt {
  fingerprint: string;
  key: string;
  unknownDestination?: UnknownRunDestination;
}
// Inspection can navigate to another route; retain the uncertain attempt in memory, never on disk.
// Module scope makes the lock survive a remount of Crew (C10).
let unfinishedRunAttempt: PendingRunAttempt | null = null;

/**
 * How long a started task waits for a verified `state` frame that lists it before the observation
 * is started over (Q3-06). The daemon sends a state frame every 2 s while it observes.
 */
export const RUN_START_FRAME_WAIT_MS = 5000;

// ---------------------------------------------------------------------------------------------
// A task whose start failed (MSG2-N10)
// ---------------------------------------------------------------------------------------------
//
// SECURITY-SENSITIVE (human review). The task's words, kept for the channel it was meant for, so
// Ask my agent there offers them again: a start that failed after the person had moved on lost
// them with the pane that held them. Kept like an unsent draft (`draftStash`): memory only, the
// words only, with the scope of the verified view the start was made under, and handed back only
// while nothing in that scope moved (`draftScopeChanged`).

interface FailedTask {
  prompt: string;
  scope: DraftScope;
}

/** The most failed tasks kept at once; the oldest goes first. */
export const FAILED_TASK_MAX = 20;
const failedTasks = new Map<string, FailedTask>();

/** The scope frame a verified view of `connectionId` makes, or null while none is verified. */
export function viewScopeFrame(
  connectionId: string,
  snapshot: Snapshot | null,
  observedPrivacy: ObservedPrivacy | null
): ScopeFrame | null {
  if (!snapshot || !connectionId || observedPrivacy?.connectionId !== connectionId) return null;
  return {
    connection_id: connectionId,
    connection_mode: observedPrivacy.mode,
    connection_policy_epoch: observedPrivacy.policyEpoch,
    connection_institution_id: observedPrivacy.institutionId,
    snapshot,
  };
}

function keepFailedTask(
  connectionId: string,
  channelId: string,
  prompt: string,
  frame: ScopeFrame
) {
  if (!prompt.trim() || !channelId) return;
  const key = postDestination(connectionId, channelId);
  failedTasks.delete(key);
  failedTasks.set(key, { prompt, scope: draftScope(frame, channelId, []) });
  while (failedTasks.size > FAILED_TASK_MAX) {
    const oldest = failedTasks.keys().next().value;
    if (oldest === undefined) break;
    failedTasks.delete(oldest);
  }
}

/**
 * The words of the task whose start failed in `channelId` on `connectionId`, when the verified view
 * now (`frame`) still offers that channel under the scope the start was made under; else null. A
 * kept task whose scope moved is forgotten. It stays kept until `forgetFailedTask`, so a pane that
 * opens and closes again offers it again.
 */
export function keptFailedTask(
  connectionId: string,
  channelId: string,
  frame: ScopeFrame | null
): string | null {
  const key = postDestination(connectionId, channelId);
  const kept = failedTasks.get(key);
  if (!kept || !frame || frame.connection_id !== connectionId) return null;
  if (
    kept.scope.connectionId !== connectionId ||
    !frame.snapshot.channels.some((item) => item.id === channelId) ||
    draftScopeChanged(kept.scope, frame, channelId, [])
  ) {
    failedTasks.delete(key);
    return null;
  }
  return kept.prompt;
}

/** Forget the failed task kept for a channel: it started, or its words were taken. */
export function forgetFailedTask(connectionId: string, channelId: string): void {
  failedTasks.delete(postDestination(connectionId, channelId));
}

/** Forget every failed task of a connection (it was removed). */
export function forgetConnectionFailedTasks(connectionId: string): void {
  const prefix = postDestination(connectionId, '');
  for (const key of [...failedTasks.keys()]) if (key.startsWith(prefix)) failedTasks.delete(key);
}

resetBetweenTests(() => failedTasks.clear());

export interface CrewRunStartContext {
  connectionId: string;
  teamId: string;
  channelId: string;
  channel: Channel | null;
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  /** The runs of the last verified `state` frame. */
  runs: readonly ObservedRun[];
  /**
   * The observer's generation. Every observation that starts, ends (a refusal, a privacy end, a
   * lost connection), is stopped (Disconnect) or is replaced (another connection, team or channel)
   * moves it, synchronously, before anything renders.
   */
  generation: MutableRefObject<number>;
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
  /**
   * Record a failed start's words where they belong: the pane's slot while the person is on the
   * channel the task was for, else the connection bar, naming that channel (MSG2-N10).
   */
  reportError(message: string, source?: ErrorSource, code?: string, details?: ErrorDetails): void;
}

export interface CrewRunStart {
  startOwnedRun(input: StartOwnedRunInput): Promise<boolean>;
  unknownRunDestination: UnknownRunDestination | null;
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
 *
 * The wait belongs to the one observation the Start was pressed on, and it never observes a view
 * that ended. It is armed only when, as the start is answered, that observation is still the
 * running one (the generation has not moved) and its view is still verified for the same
 * connection; and it observes again only when both still hold as it runs out. So an observer
 * refusal, a privacy end, a Disconnect or another selection — whether it lands while the start is
 * still being answered (provider creation and preflight can take seconds) or during the wait —
 * leaves the next step to whatever ended the view: a stopped view is the person's to retry.
 */
export function useCrewRunStart(context: CrewRunStartContext): CrewRunStart {
  const pendingRun = useRef<PendingRunAttempt | null>(unfinishedRunAttempt);
  const [unknownRunDestination, setUnknownRunDestination] = useState<UnknownRunDestination | null>(
    unfinishedRunAttempt?.unknownDestination ?? null
  );
  const [inspectedPriorRun, setInspectedPriorRun] = useState(false);
  const {
    connectionId,
    teamId,
    channelId,
    channel,
    snapshot,
    observedPrivacy,
    runs,
    generation,
    setBody,
    restartObservation,
    resetSurfaces,
    act,
    reportError,
  } = context;

  // The started task the view waits for, and the timer that observes again if it does not come.
  const awaited = useRef<{
    connectionId: string;
    observed: number;
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
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  // The selection and whether its view is verified, as of the latest commit: read by the start's
  // answer and by the timer, which both run after the render that armed them. Written as the
  // render commits, so neither ever reads a view older than the one on screen.
  const view = useRef({ connectionId, channelId, verified });
  useLayoutEffect(() => {
    view.current = { connectionId, channelId, verified };
  }, [connectionId, channelId, verified]);
  /**
   * SECURITY-SENSITIVE (human review): the observation `observed` (a generation) is still the
   * running one, and its view is still verified for `forConnection`. The generation moves
   * synchronously when an observation ends or is replaced, so this is false even for an end whose
   * render has not committed yet; the view catches anything that cleared it before `observed` was
   * read.
   */
  const stillObserving = (forConnection: string, observed: number) =>
    generation.current === observed &&
    view.current.verified &&
    view.current.connectionId === forConnection;
  const settleAwaited = useCallback(() => {
    if (awaited.current) clearTimeout(awaited.current.timer);
    awaited.current = null;
  }, []);
  const awaitStartedRun = (forConnection: string, observed: number, runId: string | null) => {
    settleAwaited();
    // A frame that listed it may already have arrived while the start was answered.
    if (runId !== null && listed.current.some((run) => run.run_id === runId)) return;
    // The view ended, or another took its place, while the start was being answered: whatever did
    // it decides what happens next. A new observation's first frame brings the task by itself.
    if (!stillObserving(forConnection, observed)) return;
    const entry = {
      connectionId: forConnection,
      observed,
      runId,
      timer: setTimeout(() => {
        if (awaited.current !== entry) return;
        awaited.current = null;
        if (!stillObserving(entry.connectionId, entry.observed)) return;
        restart.current();
      }, RUN_START_FRAME_WAIT_MS),
    };
    awaited.current = entry;
  };
  // A verified frame listed the task: nothing to wait for. Another connection, or a view that
  // ended (an observer refusal, a privacy change, a Disconnect): not this wait's to observe again —
  // what ended the view decides what happens next, and a stopped view is the person's to retry.
  // The timer checks the same again when it runs out, for an end this effect has not seen yet.
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
    // The observation this Start was pressed on: the only one its wait may ever observe again.
    const observed = generation.current;
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
    } satisfies Api.StartRunRequest;
    const fingerprint = JSON.stringify({ connectionId, ...payload });
    if (deliberateRestart || pendingRun.current?.fingerprint !== fingerprint) {
      pendingRun.current = { fingerprint, key: crypto.randomUUID() };
    }
    unfinishedRunAttempt = pendingRun.current;
    if (deliberateRestart) {
      setUnknownRunDestination(null);
      setInspectedPriorRun(false);
    }
    let started: unknown;
    try {
      started = await crewHttp<unknown>(
        `/connections/${encodeURIComponent(connectionId)}/runs`,
        'POST',
        {
          ...payload,
          request_id: pendingRun.current.key,
        }
      );
    } catch (failure) {
      if (failure instanceof CrewHttpError && failure.code === 'crew_start_outcome_unknown') {
        // Named as Crew names them everywhere else, from the channel's own team (RENDERER-5): the
        // gate shows this in every channel until it is cleared, never as stored names or an ID.
        const destinationTeamId = channel?.team_id ?? teamId;
        const destination: UnknownRunDestination = {
          connectionId,
          teamId: destinationTeamId,
          channelId,
          channel: channelName(channel),
          team: teamName(snapshot.teams.find((item) => item.id === destinationTeamId)),
        };
        pendingRun.current.unknownDestination = destination;
        unfinishedRunAttempt = pendingRun.current;
        setUnknownRunDestination(destination);
        setInspectedPriorRun(false);
      }
      throw failure;
    }
    pendingRun.current = null;
    unfinishedRunAttempt = null;
    forgetFailedTask(connectionId, channelId);
    setUnknownRunDestination(null);
    setInspectedPriorRun(false);
    resetSurfaces('run-started');
    if (clearBody) setBody('');
    const runId = wireOf<Api.RunView>(started)?.run_id;
    awaitStartedRun(connectionId, observed, typeof runId === 'string' && runId ? runId : null);
    return true;
  };

  /**
   * A failed start, told where the person is (MSG2-N10): in the pane's slot while they are still
   * on the channel the task was for, else in the connection bar, naming that channel. A failure of
   * the link is marked so, so the connection verifying again takes it away instead of leaving it
   * red under "Connected". Unless the task may have started (the gate above handles that), its
   * words are kept for that channel's Ask my agent, which offers them again.
   */
  const tellStartFailure = (
    failure: unknown,
    input: StartOwnedRunInput,
    forConnection: string,
    forChannel: string,
    channelLabel: string,
    frame: ScopeFrame | null
  ) => {
    const outcomeUnknown =
      failure instanceof CrewHttpError && failure.code === 'crew_start_outcome_unknown';
    if (!outcomeUnknown && frame) keepFailedTask(forConnection, forChannel, input.prompt, frame);
    const message = failureMessage(failure, crewActionCopy.actionFallback);
    const code = failureCode(failure);
    const details = isTransportFailure(failure) ? { transport: true } : undefined;
    const now = view.current;
    if (now.connectionId === forConnection && now.channelId === forChannel) {
      reportError(message, 'pane:agent', code, details);
      return;
    }
    reportError(
      outcomeUnknown ? message : crewActionCopy.startFailedIn(channelLabel, message),
      'global',
      code,
      details
    );
  };

  const startOwnedRun = async (input: StartOwnedRunInput) => {
    // What the start is for, as Start was pressed: a failure is told against it, wherever the
    // person is by then.
    const forConnection = connectionId;
    const forChannel = channelId;
    const channelLabel = channelName(channel);
    const frame = viewScopeFrame(connectionId, snapshot, observedPrivacy);
    return (
      (await act('pane:agent', 'run.start', async () => {
        try {
          return await submitOwnedRun(input);
        } catch (failure) {
          tellStartFailure(failure, input, forConnection, forChannel, channelLabel, frame);
          return false;
        }
      })) === true
    );
  };

  return {
    startOwnedRun,
    unknownRunDestination,
    inspectedPriorRun,
    setInspectedPriorRun,
    pendingRun,
  };
}
