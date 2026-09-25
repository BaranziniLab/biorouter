import { useCallback, useEffect, useRef, useState } from 'react';
import { useLocation, useSearchParams } from 'react-router-dom';
import { crewHttp, crewRequest, type CrewConnection, type Snapshot } from '../crewApi';
import { crewActionCopy } from './copy';
import { useCrewActions } from './crewActions';
import { createSend, useCrewDraft } from './crewSend';
import { useCrewRunStart } from './crewRunStart';
import { arrivalConnectDecision, isMembershipEnded } from './connectFailure';
import { deriveConnectionStatus, deriveCrewScreen } from './crewStatus';
import { useCrewSurfaces } from './crewSurfaces';
import { rememberedLastChannel, rememberLastChannel } from './draftStash';
import { failureMessage, isFinalObservationEnd } from './observationFailure';
import {
  arrivalConnectConsumed,
  arrivalConnectIntent,
  connectionVerifiedThisSession,
  consumeArrivalConnect,
  createConnectionLifecycle,
  DAEMON_REDIAL_FOLLOW_MS,
  takeQuietReobserve,
  useCrewConnectFailures,
  useCrewConnections,
} from './useCrewConnections';
import {
  CHANNEL_LOST_ERROR_CODE,
  useCrewObservation,
  type ObservationEnd,
} from './useCrewObservation';
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

/**
 * The channel to show for `teamId`: `current` while the team still has it, else `preferred` (the
 * channel the person last chose, Q2-21) while the team has it open, else the team's first channel
 * that is not archived, else none.
 */
export function channelForTeam(
  snapshot: Pick<Snapshot, 'channels'>,
  teamId: string,
  current: string,
  preferred: string | null = null
): string {
  const channels = snapshot.channels.filter((item) => item.team_id === teamId);
  if (channels.some((item) => item.id === current)) return current;
  if (preferred && channels.some((item) => item.id === preferred && !item.archived))
    return preferred;
  return channels.find((item) => !item.archived)?.id ?? '';
}

/** A join status other than `joined` (or a broker without joins) means not a member yet. */
function isNotJoined(status: CrewJoinStatus | null): boolean {
  return status !== null && status !== 'joined' && status !== 'unsupported';
}

/**
 * Crew's state and actions, shared by every Crew area through `CrewControllerContext`.
 *
 * Extracted from `CrewView` with its behavior unchanged — refresh order, draft rules, idempotency,
 * single flight, the unknown-outcome lock — plus the seams the redesigned layout added: controller
 * options, the last verified view, the classified connect failure, dialog and pane intents,
 * `markRead` without a refresh, `clearBodyIfEquals`, per-action pending keys and error sources,
 * and the pure derived `status` and `screen`. React authorizes nothing here: every action is a
 * request the daemon and broker decide.
 */
export function useCrewController(options: CrewControllerOptions = {}): CrewController {
  const { autoOpenSignIn = false, keepLastVerifiedView = false } = options;
  const [searchParams] = useSearchParams();
  const grantSessionId = searchParams.get('sessionId');
  // A chat's "Connect in Crew" (Q3-08): the connection it asks to connect, and its intent id.
  const location = useLocation();
  const arrival = arrivalConnectIntent(location.state);

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
  // The connection whose observation ended as a dropped connection would (Q2-01): its saved record
  // is being read again, or it is being observed again quietly. Never connected by the renderer.
  const [reconnecting, setReconnecting] = useState<string | null>(null);
  const lossHandler = useRef<(id: string, end: ObservationEnd) => void>(() => undefined);
  const onConnectionLost = useCallback(
    (id: string, end: ObservationEnd) => lossHandler.current(id, end),
    []
  );
  const { clear: clearConnectFailure } = connectFailures;
  const onVerifiedFrame = useCallback(
    (id: string) => {
      clearConnectFailure(id);
      setReconnecting((current) => (current === id ? null : current));
    },
    [clearConnectFailure]
  );

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
    connections,
    loadConnections,
    setConnections,
    draft,
    reportError,
    dismissError,
    closeSignIn,
    setJoinStatus,
    resetSurfaces,
    onVerifiedFrame,
    keepLastVerifiedView,
    joinStatus,
    onConnectionLost,
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
    refreshErrorCode,
    reverifying,
    lastVerified,
    setSnapshot,
    refresh,
    stopObserving,
    restartObservation,
    observationFailure,
    stashDraft,
  } = observation;

  useEffect(() => {
    if (!snapshot) return;
    setChannelId((old) =>
      channelForTeam(snapshot, teamId, old, rememberedLastChannel(connectionId))
    );
  }, [teamId, snapshot]); // eslint-disable-line react-hooks/exhaustive-deps -- the remembered channel is read when the team or view changes, as before

  // A reconnect belongs to the connection it started on, and ends with any error on show.
  useEffect(() => {
    setReconnecting(null);
  }, [connectionId]);
  useEffect(() => {
    if (refreshError) setReconnecting(null);
  }, [refreshError]);

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

  // Each selection retires the running observer (the generation moves), so each must also make
  // sure a new one starts. A different connection or channel restarts it through the observer's
  // own dependencies; the same connection, or a team whose channel does not change (a team just
  // created, absent from the verified view, leaves the channel empty), would otherwise leave no
  // observer at all, and every later frame dropped until Crew is left (T-08). A team whose channel
  // does change moves the channel here, in the same update, rather than restarting first and
  // letting the effect above move it — that started an observer for the old channel on the way.
  const selectConnection = (id: string) => {
    generation.current += 1;
    setSnapshot(null);
    setMessages([]);
    setMessagesLoaded(false);
    setConnectionId(id);
    if (id === connectionId) restartObservation();
  };
  //
  // A deliberate selection also puts the unsent body aside for the channel it was written in
  // (Q2-07), remembers the chosen channel for next time (Q2-21), and dismisses a "channel was
  // closed" note that no longer describes what is on screen (Q2-19).
  const leaveChannel = () => {
    stashDraft();
    if (actions.error?.source === 'observer' && actions.error.code === CHANNEL_LOST_ERROR_CODE)
      dismissError();
  };
  const selectTeam = (id: string) => {
    leaveChannel();
    generation.current += 1;
    draft.setReferences([]);
    setMessages([]);
    setMessagesLoaded(false);
    draft.setBody('');
    draft.setAttachments([]);
    draft.setContextChannels([]);
    setTeamId(id);
    const next = snapshot
      ? channelForTeam(snapshot, id, channelId, rememberedLastChannel(connectionId))
      : channelId;
    if (next) rememberLastChannel(connectionId, next);
    if (next !== channelId) setChannelId(next);
    else restartObservation();
  };
  const selectChannel = (id: string) => {
    if (id === channelId) return;
    leaveChannel();
    generation.current += 1;
    draft.setReferences([]);
    setMessages([]);
    setMessagesLoaded(false);
    draft.setAttachments([]);
    draft.setBody('');
    // The channel's own team, so a channel in another team is not undone by the team effect
    // above (Q2-10): a chat's grant opens its channel wherever it is.
    const view =
      snapshot ?? (lastVerified?.connectionId === connectionId ? lastVerified.snapshot : null);
    const target = view?.channels.find((item) => item.id === id);
    if (target && target.team_id !== teamId) setTeamId(target.team_id);
    rememberLastChannel(connectionId, id);
    setChannelId(id);
    draft.setContextChannels([]);
  };

  // Moves on every connection change, unmount, Disconnect, and connect or Retry the person made:
  // a loss handled before it is over. Also ends the reads that follow the daemon's re-dial.
  const lossToken = useRef(0);
  const followTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const settleLoss = useCallback(() => {
    lossToken.current += 1;
    if (followTimer.current !== undefined) clearTimeout(followTimer.current);
    followTimer.current = undefined;
  }, []);
  useEffect(() => {
    settleLoss();
  }, [connectionId, settleLoss]);
  useEffect(() => () => settleLoss(), [settleLoss]);
  const lifecycle = createConnectionLifecycle({
    connectionId,
    failures: connectFailures,
    autoOpenSignIn,
    openSignIn,
    loadConnections,
    refresh,
    stopObserving,
    act,
  });
  const connect = async (opts?: { userInitiated?: boolean }) => {
    if (opts?.userInitiated) {
      // The person's own connect replaces whatever a loss was waiting for.
      settleLoss();
      setReconnecting(null);
    }
    await lifecycle.connect(opts);
  };
  const disconnect = async () => {
    // A Disconnect ends the handling of any loss: nothing is observed or reported for it.
    settleLoss();
    setReconnecting(null);
    await lifecycle.disconnect();
  };

  // What the loss handler reads after its awaits: the latest render's selection, never the one of
  // the render that saw the loss.
  const latestConnectionId = useRef(connectionId);
  useEffect(() => {
    latestConnectionId.current = connectionId;
  });

  /**
   * Read the saved record again at `DAEMON_REDIAL_FOLLOW_MS`, and stop once it says connected (the
   * observation hook then observes again by itself, its error being on show), once the reads run
   * out, or once anything else settles the loss. Reads only: it never connects.
   */
  const followDaemonRedial = (lostId: string, token: number, step = 0) => {
    const gap = DAEMON_REDIAL_FOLLOW_MS[step];
    if (gap === undefined) return;
    followTimer.current = setTimeout(() => {
      followTimer.current = undefined;
      if (token !== lossToken.current || latestConnectionId.current !== lostId) return;
      void loadConnections()
        .then(
          (list) => list?.find((item) => item.id === lostId)?.status === 'connected',
          () => false
        )
        .then((back) => {
          if (back || token !== lossToken.current || latestConnectionId.current !== lostId) return;
          followDaemonRedial(lostId, token, step + 1);
        });
    }, gap);
  };

  /**
   * The observation ended in a way a dropped connection explains (live QA round 2, Q2-01).
   * SECURITY-SENSITIVE (human review): this never connects. The daemon ends observation with the
   * same `observation_refused` for a dropped bridge and for a Disconnect made anywhere else — a
   * terminal (`biorouter crew disconnect`), another window, an edit — and this window cannot tell
   * them apart; re-dialling is the daemon's alone (D-KEEPALIVE), and it never follows a Disconnect.
   * So, after reading the saved record again:
   * - the daemon still, or again, calls it connected (its keepalive kept the bridge, or its re-dial
   *   repaired it): observe again, quietly, while `takeQuietReobserve` allows — "Reconnecting…"
   *   until the new view verifies, the draft coming back unless its scope moved. Past the budget,
   *   the end is shown with Retry: a bridge that keeps dropping is a failure worth seeing;
   * - anything else (disconnected, whoever did it): the end is shown as it is, which on a
   *   disconnected connection is the offline screen and its Connect — the person's to press — and
   *   the record is read again a few times (`followDaemonRedial`) so the daemon's own re-dial
   *   brings the view back without a click.
   */
  const handleConnectionLost = (lostId: string, end: ObservationEnd) => {
    settleLoss();
    const token = lossToken.current;
    // The observer's generation at the loss: anything that observes again meanwhile (a selection,
    // a refresh) or stops observing moves it, and then this loss is no longer the news.
    const observed = generation.current;
    const current = () => token === lossToken.current && latestConnectionId.current === lostId;
    setReconnecting(lostId);
    void (async () => {
      let record: CrewConnection | undefined;
      try {
        const list = await loadConnections();
        record = list?.find((item) => item.id === lostId);
      } catch {
        record = undefined;
      }
      if (!current()) return;
      if (generation.current !== observed) {
        setReconnecting((id) => (id === lostId ? null : id));
        return;
      }
      // A membership the workspace ended is final: nothing is observed again or followed for it.
      const ended = isMembershipEnded(record);
      if (record?.status === 'connected' && !ended && takeQuietReobserve(lostId, Date.now())) {
        // "Reconnecting…" lasts until the new view verifies (`onVerifiedFrame`), or until that
        // observation ends too and this decides again.
        restartObservation();
        return;
      }
      setReconnecting(null);
      observationFailure(end.text, end.code);
      if (record?.status === 'disconnected' && !ended) followDaemonRedial(lostId, token);
    })();
  };
  useEffect(() => {
    lossHandler.current = handleConnectionLost;
  });

  /**
   * The connection bar's Retry (Q2-01), which the person presses: read the saved record first,
   * and when the daemon no longer calls it connected, connect at once, as the person — rather
   * than reveal "Offline" and leave Connect as a second step. Otherwise observe again.
   */
  const retryUpdates = async () => {
    settleLoss();
    const target = connectionId;
    let record: CrewConnection | undefined;
    try {
      record = (await loadConnections())?.find((item) => item.id === target);
    } catch {
      // The refresh below reads the list again and reports why it could not.
    }
    if (record && record.status !== 'connected') {
      await connect({ userInitiated: true });
      return;
    }
    await refresh();
  };

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
    runs,
    generation,
    setBody: draft.setBody,
    restartObservation,
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
    markRead,
    act,
    reportError,
  });

  const notJoined = isNotJoined(joinStatus);
  const isReconnecting = reconnecting !== null && reconnecting === connectionId;
  const inFlight = isPending('connect') || isPending('sign-in') || surfaces.signIn.open;
  const verifiedHere = connectionVerifiedThisSession(connectionId) || joinStatus === 'joined';
  const membershipEnded = isMembershipEnded(savedConnection);

  /**
   * SECURITY-SENSITIVE (human review). Connect on arrival from a chat's "Connect in Crew" (Q3-08):
   * the click in the chat was the person's, so this is their connect one screen later, run exactly
   * as the Connect button runs it (`userInitiated`: it settles any loss being handled, the daemon
   * re-arms its keepalive, and a password or code prompt opens Sign in). Once per intent id
   * (`consumeArrivalConnect`), only for a saved connection the daemon calls disconnected, and never
   * for a final answer (`arrivalConnectDecision`). The named connection is selected first, because
   * a connect acts on the selection. Which channel opens afterwards is the chat note's.
   */
  const arrivalIntentId = arrival?.intentId ?? null;
  const arrivalConnectionId = arrival?.connectionId ?? null;
  const arrivalRecord = arrivalConnectionId
    ? (connections.find((item) => item.id === arrivalConnectionId) ?? null)
    : null;
  const arrivalFailure =
    connectFailures.failure?.connectionId === arrivalConnectionId ? connectFailures.failure : null;
  const connectingNow = isPending('connect');
  const signInPending = isPending('sign-in') || surfaces.signIn.open;
  // Read through a ref: both are new functions every render.
  const arrivalActions = useRef({ connect, selectConnection });
  arrivalActions.current = { connect, selectConnection };
  useEffect(() => {
    if (!arrivalIntentId || !arrivalConnectionId || connectionsState !== 'loaded') return;
    if (arrivalConnectConsumed(arrivalIntentId)) return;
    const decision = arrivalConnectDecision({
      connection: arrivalRecord,
      lastConnectFailure: arrivalFailure,
      connecting: connectingNow,
      signInPending,
    });
    if (decision === 'wait') return;
    if (decision === 'skip') {
      consumeArrivalConnect(arrivalIntentId);
      return;
    }
    if (connectionId !== arrivalConnectionId) {
      arrivalActions.current.selectConnection(arrivalConnectionId);
      return;
    }
    consumeArrivalConnect(arrivalIntentId);
    void arrivalActions.current.connect({ userInitiated: true });
  }, [
    arrivalIntentId,
    arrivalConnectionId,
    arrivalRecord,
    arrivalFailure,
    connectionsState,
    connectionId,
    connectingNow,
    signInPending,
  ]);
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
    reconnecting: isReconnecting,
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
    refreshErrorCode,
    refreshErrorRetryable:
      !isFinalObservationEnd(refreshErrorCode, verifiedHere) && !membershipEnded,
    reverifying,
    refresh,
    retryUpdates,
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
      reverifying,
      reconnecting: isReconnecting,
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
      reconnecting: isReconnecting,
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
