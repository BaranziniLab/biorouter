import { useCallback, useEffect, useRef, useState } from 'react';
import { useSearchParams } from 'react-router-dom';
import { crewHttp, crewRequest, type Snapshot } from '../crewApi';
import { crewActionCopy } from './copy';
import { useCrewActions } from './crewActions';
import { createSend, useCrewDraft } from './crewSend';
import { useCrewRunStart } from './crewRunStart';
import { deriveConnectionStatus, deriveCrewScreen } from './crewStatus';
import { useCrewSurfaces } from './crewSurfaces';
import { failureMessage } from './observationFailure';
import {
  createConnectionLifecycle,
  useCrewConnectFailures,
  useCrewConnections,
} from './useCrewConnections';
import { useCrewObservation } from './useCrewObservation';
import type { CrewController, CrewControllerOptions, CrewJoinStatus } from './types';

export type * from './types';

/** `host_principal_id` (S1) when the broker sends it, else the host's UID. */
export function isWorkspaceHost(snapshot: Snapshot | null): boolean {
  if (!snapshot) return false;
  const hostPrincipal = (snapshot.workspace as { host_principal_id?: unknown }).host_principal_id;
  return typeof hostPrincipal === 'string' && hostPrincipal
    ? snapshot.actor.id === hostPrincipal
    : snapshot.actor.uid === snapshot.workspace.host_uid;
}

/** A join status other than `joined` (or a broker without joins) means not a member yet. */
function isNotJoined(status: CrewJoinStatus | null): boolean {
  return status !== null && status !== 'joined' && status !== 'unsupported';
}

/**
 * Crew's state and actions, shared by every layout through `CrewControllerContext`.
 *
 * Extracted from `CrewView` with its behavior unchanged — refresh order, draft rules, idempotency,
 * single flight, the unknown-outcome lock — plus seams the legacy layout ignores: controller
 * options, the last verified view, the classified connect failure, dialog and pane intents,
 * `markRead` without a refresh, `clearBodyIfEquals`, per-action pending keys and error sources,
 * and the pure derived `status` and `screen`. React authorizes nothing here: every action is a
 * request the daemon and broker decide.
 */
export function useCrewController(options: CrewControllerOptions = {}): CrewController {
  const { autoOpenSignIn = false, keepLastVerifiedView = false } = options;
  const [searchParams] = useSearchParams();
  const grantSessionId = searchParams.get('sessionId');

  const actions = useCrewActions();
  const { act, reportError, dismissError, isPending } = actions;
  const generation = useRef(0);
  const {
    connections,
    setConnections,
    connectionId,
    setConnectionId,
    connectionsState,
    markConnectionsFailed,
    loadConnections,
    saveConnection,
    updateConnection,
    removeConnection,
    prepareHostingDevice,
  } = useCrewConnections(generation);
  const [teamId, setTeamId] = useState('');
  const [channelId, setChannelId] = useState('');
  const draft = useCrewDraft();
  const surfaces = useCrewSurfaces();
  const { resetSurfaces, openSignIn, closeSignIn } = surfaces;
  const [joinStatus, setJoinStatus] = useState<CrewJoinStatus | null>(null);
  const connectFailures = useCrewConnectFailures();

  useEffect(() => {
    void loadConnections().catch((failure: unknown) => {
      markConnectionsFailed();
      reportError(failureMessage(failure, crewActionCopy.actionFallback), 'global');
    });
  }, [loadConnections, markConnectionsFailed, reportError]);

  const observation = useCrewObservation({
    connectionId,
    channelId,
    teamId,
    setTeamId,
    setChannelId,
    generation,
    loadConnections,
    setConnections,
    draft,
    reportError,
    dismissError,
    closeSignIn,
    setJoinStatus,
    resetSurfaces,
    onVerifiedFrame: connectFailures.clear,
    keepLastVerifiedView,
  });
  const {
    snapshot,
    observedPrivacy,
    runs,
    messages,
    setMessages,
    messagesLoaded,
    setMessagesLoaded,
    historyBefore,
    setHistoryBefore,
    historyPage,
    labels,
    backlogComplete,
    pageSize,
    people,
    capabilities,
    refreshError,
    lastVerified,
    setSnapshot,
    refresh,
    stopObserving,
    restartObservation,
  } = observation;

  const channels = snapshot?.channels.filter((item) => item.team_id === teamId) ?? [];
  useEffect(() => {
    if (!snapshot) return;
    setChannelId((old) =>
      channels.some((item) => item.id === old)
        ? old
        : (channels.find((item) => !item.archived)?.id ?? '')
    );
  }, [teamId, snapshot]); // eslint-disable-line react-hooks/exhaustive-deps

  const savedConnection = connections.find((item) => item.id === connectionId);
  const connection =
    (savedConnection && observedPrivacy?.connectionId === connectionId
      ? {
          ...savedConnection,
          mode: observedPrivacy.mode,
          institution_id: observedPrivacy.institutionId,
          policy_epoch: observedPrivacy.policyEpoch,
        }
      : savedConnection) ?? null;
  const verified = Boolean(snapshot && observedPrivacy?.connectionId === connectionId);
  const channel = snapshot?.channels.find((item) => item.id === channelId) ?? null;
  const team = snapshot?.teams.find((item) => item.id === teamId) ?? null;
  const connectFailure =
    connectFailures.failure?.connectionId === connectionId
      ? (({ connectionId: _connection, ...failure }) => failure)(connectFailures.failure)
      : null;

  const request = useCallback(
    <T>(
      method: string,
      params: Record<string, unknown> = {},
      opts?: { mutation?: boolean; signal?: AbortSignal }
    ) =>
      opts?.signal
        ? crewRequest<T>(connectionId, method, params, opts.mutation ?? false, opts.signal)
        : crewRequest<T>(connectionId, method, params, opts?.mutation ?? false),
    [connectionId]
  );
  const mutate = async <T>(
    method: string,
    params: Record<string, unknown>,
    opts?: { refresh?: boolean }
  ) => {
    const result = await request<T>(method, params, { mutation: true });
    if (opts?.refresh !== false) await refresh();
    resetSurfaces('mutated');
    return result;
  };
  const markRead = async (readChannel: string, sequence: string) => {
    await request('channel.read', { channel_id: readChannel, sequence }, { mutation: true });
  };

  const selectConnection = (id: string) => {
    generation.current += 1;
    setSnapshot(null);
    setMessages([]);
    setMessagesLoaded(false);
    setConnectionId(id);
  };
  const selectTeam = (id: string) => {
    generation.current += 1;
    draft.setReferences([]);
    setMessages([]);
    setMessagesLoaded(false);
    draft.setBody('');
    draft.setAttachments([]);
    draft.setContextChannels([]);
    setTeamId(id);
  };
  const selectChannel = (id: string) => {
    if (id === channelId) return;
    generation.current += 1;
    draft.setReferences([]);
    setMessages([]);
    setMessagesLoaded(false);
    draft.setAttachments([]);
    draft.setBody('');
    setChannelId(id);
    draft.setContextChannels([]);
  };

  const { connect, disconnect } = createConnectionLifecycle({
    connectionId,
    failures: connectFailures,
    autoOpenSignIn,
    openSignIn,
    loadConnections,
    refresh,
    stopObserving,
    act,
  });
  const onSignedIn = () => {
    closeSignIn();
    connectFailures.clear(connectionId);
    void act('global', 'sign-in', async () => {
      await loadConnections();
      await refresh();
    });
  };
  const cancelRun = async (runId: string) => {
    await act('global', 'run.cancel', async () => {
      await crewHttp(`/connections/${connectionId}/runs/${runId}/cancel`, 'POST', {});
      await refresh();
    });
  };
  const grantSession = async ({
    contextChannels,
    sessionId = grantSessionId ?? undefined,
  }: {
    contextChannels: string[];
    sessionId?: string;
  }) => {
    if (!sessionId) return;
    if (!snapshot || observedPrivacy?.connectionId !== connectionId)
      throw new Error(crewActionCopy.grantPrivacyUnverified);
    await crewHttp(
      `/connections/${connectionId}/sessions/${encodeURIComponent(sessionId)}/grant`,
      'POST',
      {
        expected_mode: observedPrivacy.mode,
        expected_policy_epoch: observedPrivacy.policyEpoch,
        expected_workspace_policy_epoch: snapshot.workspace.policy_epoch,
        channel_id: channelId,
        context_channels: [channelId, ...contextChannels],
      }
    );
  };

  const runStart = useCrewRunStart({
    connectionId,
    teamId,
    channelId,
    connection,
    team,
    channel,
    snapshot,
    observedPrivacy,
    setBody: draft.setBody,
    refresh,
    resetSurfaces,
    act,
  });
  const send = createSend({
    draft,
    busy: actions.busy,
    connectionId,
    channelId,
    channel,
    snapshot,
    observedPrivacy,
    generation,
    historyPage,
    setHistoryBefore,
    restartObservation,
    request,
    act,
    reportError,
  });

  const notJoined = isNotJoined(joinStatus);
  const inFlight = isPending('connect') || isPending('sign-in') || surfaces.signIn.open;
  const view = verified
    ? snapshot
    : lastVerified?.connectionId === connectionId
      ? lastVerified.snapshot
      : null;

  return {
    connections,
    connectionId,
    connection,
    connectionsState,
    selectConnection,
    saveConnection,
    updateConnection,
    removeConnection,
    prepareHostingDevice,
    connect,
    disconnect,
    lastConnectFailure: connectFailure,
    reportConnectFailure: (failure: unknown) => {
      connectFailures.record(connectionId, failure);
    },

    snapshot,
    lastVerified: lastVerified?.connectionId === connectionId ? lastVerified : null,
    observedPrivacy,
    runs,
    messages,
    messagesLoaded,
    historyBefore,
    labels,
    backlogComplete,
    pageSize,
    people,
    capabilities,
    refreshError: refreshError || null,
    refresh,
    loadOlder: () => {
      historyPage.current = messages[0]?.sequence ?? null;
      setHistoryBefore(historyPage.current);
    },
    jumpToLatest: () => {
      historyPage.current = null;
      setHistoryBefore(null);
      void refresh();
    },

    teamId,
    channelId,
    team,
    channel,
    selectTeam,
    selectChannel,

    act,
    error: actions.error,
    errorSlotFor: actions.errorSlotFor,
    registerErrorSlot: actions.registerErrorSlot,
    reportError,
    dismissError,
    isPending,
    busy: actions.busy,
    request,
    mutate,
    markRead,

    draft: { body: draft.body, attachments: draft.attachments, references: draft.references },
    setBody: draft.setBody,
    addAttachment: draft.addAttachment,
    removeAttachment: draft.removeAttachment,
    addReference: draft.addReference,
    removeReference: draft.removeReference,
    contextChannels: draft.contextChannels,
    setContextChannels: draft.setContextChannels,
    send,
    clearBodyIfEquals: draft.clearBodyIfEquals,

    startOwnedRun: runStart.startOwnedRun,
    unknownRunDestination: runStart.unknownRunDestination || null,
    inspectedPriorRun: runStart.inspectedPriorRun,
    setInspectedPriorRun: runStart.setInspectedPriorRun,
    cancelRun,

    grantSessionId,
    grantSession,

    signIn: surfaces.signIn,
    openSignIn: () => openSignIn('user'),
    closeSignIn,
    onSignedIn,

    ui: surfaces.ui,
    openDialog: surfaces.openDialog,
    closeDialog: surfaces.closeDialog,
    openPane: surfaces.openPane,
    closePane: surfaces.closePane,
    subscribeSurfaceReset: surfaces.subscribeSurfaceReset,

    joinStatus,
    setJoinStatus,

    status: deriveConnectionStatus({
      connection,
      lastConnectFailure: connectFailure,
      inFlight,
      verified,
      observationError: Boolean(refreshError),
      notJoined,
    }),
    screen: deriveCrewScreen({
      connectionsState,
      connectionCount: connections.length,
      connection,
      lastConnectFailure: connectFailure,
      inFlight,
      signInOpen: surfaces.signIn.open,
      view,
      channelId,
      observationError: Boolean(refreshError),
      notJoined,
    }),
    effectivePrivacy:
      verified && snapshot && observedPrivacy
        ? observedPrivacy.mode === 'private' || snapshot.workspace.mode === 'private'
          ? 'private'
          : 'public'
        : null,
    isHost: isWorkspaceHost(snapshot),
  };
}
