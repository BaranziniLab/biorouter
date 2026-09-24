import { useRef, useState, type Dispatch, type MutableRefObject, type SetStateAction } from 'react';
import {
  crewHttp,
  CrewHttpError,
  type Channel,
  type CrewConnection,
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

export interface CrewRunStartContext {
  connectionId: string;
  teamId: string;
  channelId: string;
  connection: CrewConnection | null;
  team: Team | null;
  channel: Channel | null;
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  setBody: Dispatch<SetStateAction<string>>;
  refresh(): Promise<void>;
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
    setBody,
    refresh,
    resetSurfaces,
    act,
  } = context;

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
    try {
      await crewHttp(`/connections/${connectionId}/runs`, 'POST', {
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
    await refresh();
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
