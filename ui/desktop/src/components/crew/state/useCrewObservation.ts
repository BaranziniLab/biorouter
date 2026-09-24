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
  crewRequest,
  observeCrew,
  type CrewConnection,
  type CrewMessage,
  type ObservedRun,
  type Snapshot,
} from '../crewApi';
import { crewObservationCopy } from './copy';
import type { CrewDraftState } from './crewSend';
import { failureCode, failureMessage, observationFailureOutcome } from './observationFailure';
import type {
  CrewFrameLabels,
  CrewJoinStatus,
  ErrorSource,
  ObservedPrivacy,
  SurfaceResetReason,
  VerifiedView,
} from './types';

/** The daemon's projected labels on a `state` frame, passed through when they are an object. */
export function frameLabels(frame: unknown): CrewFrameLabels | null {
  const labels =
    frame !== null && typeof frame === 'object' ? (frame as { labels?: unknown }).labels : null;
  return labels !== null && typeof labels === 'object' && !Array.isArray(labels)
    ? (labels as CrewFrameLabels)
    : null;
}

export interface CrewObservationContext {
  connectionId: string;
  channelId: string;
  teamId: string;
  setTeamId: Dispatch<SetStateAction<string>>;
  setChannelId: Dispatch<SetStateAction<string>>;
  generation: MutableRefObject<number>;
  loadConnections(signal?: AbortSignal, current?: number): Promise<void>;
  setConnections: Dispatch<SetStateAction<CrewConnection[]>>;
  draft: CrewDraftState;
  reportError(message: string, source?: ErrorSource, code?: string): void;
  dismissError(): void;
  closeSignIn(): void;
  setJoinStatus: Dispatch<SetStateAction<CrewJoinStatus | null>>;
  resetSurfaces(reason: SurfaceResetReason): void;
  /** A verified `state` frame arrived for this connection: its last connect failure is stale. */
  onVerifiedFrame(connectionId: string): void;
  keepLastVerifiedView: boolean;
}

export interface CrewObservation {
  snapshot: Snapshot | null;
  observedPrivacy: ObservedPrivacy | null;
  runs: ObservedRun[];
  messages: CrewMessage[];
  setMessages: Dispatch<SetStateAction<CrewMessage[]>>;
  messagesLoaded: boolean;
  setMessagesLoaded: Dispatch<SetStateAction<boolean>>;
  historyBefore: string | null;
  setHistoryBefore: Dispatch<SetStateAction<string | null>>;
  historyPage: MutableRefObject<string | null>;
  labels: CrewFrameLabels | null;
  refreshError: string;
  lastVerified: VerifiedView | null;
  setSnapshot: Dispatch<SetStateAction<Snapshot | null>>;
  refresh(): Promise<void>;
  /** Stop observing on purpose (a disconnect): no error, nothing verified left. */
  stopObserving(): void;
  restartObservation(): void;
  clearProtectedState(): void;
  observationFailure(message: string, code?: string): void;
}

/**
 * The verified view of the selected connection and channel, and the observer that keeps it.
 *
 * Moved from `CrewView` with its behavior unchanged: frame validation, the generation guard that
 * drops frames from a retired observer, `verifiedScope` (a privacy or source-channel change
 * clears the draft), the draft-clearing failure codes, and the throw after three fast reconnects.
 * The effects keep their original order — connection reset, channel reset, observer, history —
 * because each increments or reads the same generation counter.
 */
export function useCrewObservation(context: CrewObservationContext): CrewObservation {
  const {
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
    onVerifiedFrame,
    keepLastVerifiedView,
  } = context;
  const {
    setBody,
    setAttachments,
    setReferences,
    setContextChannels,
    pendingMessage,
    selectedSources,
    clearDraft,
  } = draft;

  const [runs, setRuns] = useState<ObservedRun[]>([]);
  const [observedPrivacy, setObservedPrivacy] = useState<ObservedPrivacy | null>(null);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [labels, setLabels] = useState<CrewFrameLabels | null>(null);
  const [messages, setMessages] = useState<CrewMessage[]>([]);
  const [messagesLoaded, setMessagesLoaded] = useState(false);
  const [historyBefore, setHistoryBefore] = useState<string | null>(null);
  const [refreshError, setRefreshError] = useState('');
  const [lastVerified, setLastVerified] = useState<VerifiedView | null>(null);
  const observer = useRef<AbortController | null>(null);
  const [observationRevision, setObservationRevision] = useState(0);
  const historyPage = useRef<string | null>(null);
  useEffect(() => {
    historyPage.current = historyBefore;
  }, [historyBefore]);

  const verifiedScope = useRef<{
    connection: string;
    epoch: number;
    connectionEpoch: number;
    mode: string;
  } | null>(null);
  const clearProtectedState = useCallback(() => {
    setSnapshot(null);
    setObservedPrivacy(null);
    setRuns([]);
    setMessages([]);
    setMessagesLoaded(false);
    setLabels(null);
    setHistoryBefore(null);
    historyPage.current = null;
    setLastVerified(null);
    resetSurfaces('protected-cleared');
  }, [resetSurfaces]);
  const observationFailure = useCallback(
    (message: string, code?: string) => {
      clearProtectedState();
      const outcome = observationFailureOutcome(message, code);
      if (outcome.clearDraft) clearDraft();
      setRefreshError(outcome.text);
    },
    [clearProtectedState, clearDraft]
  );
  const refresh = useCallback(async () => {
    observer.current?.abort();
    const controller = new AbortController();
    observer.current = controller;
    const current = ++generation.current;
    setSnapshot(null);
    setObservedPrivacy(null);
    setRuns([]);
    setMessages([]);
    setMessagesLoaded(false);
    setLabels(null);
    resetSurfaces('refresh');
    setRefreshError('');
    try {
      await loadConnections(controller.signal, current);
      if (!controller.signal.aborted && generation.current === current)
        setObservationRevision((revision) => revision + 1);
    } catch (failure) {
      if (!controller.signal.aborted && generation.current === current)
        observationFailure(
          failureMessage(failure, crewObservationCopy.connectionsRefreshFailed),
          failureCode(failure)
        );
    }
  }, [generation, loadConnections, observationFailure, resetSurfaces]);
  const restartObservation = useCallback(
    () => setObservationRevision((revision) => revision + 1),
    []
  );
  const stopObserving = useCallback(() => {
    observer.current?.abort();
    generation.current += 1;
    clearProtectedState();
    setRefreshError('');
  }, [generation, clearProtectedState]);
  useEffect(
    () => () => {
      observer.current?.abort();
      generation.current += 1;
    },
    [generation]
  );

  // A different connection: drop everything verified for the old one, and its draft.
  useEffect(() => {
    generation.current += 1;
    setSnapshot(null);
    setMessages([]);
    setMessagesLoaded(false);
    setLabels(null);
    setChannelId('');
    setTeamId('');
    setBody('');
    setAttachments([]);
    setReferences([]);
    setRuns([]);
    setContextChannels([]);
    closeSignIn();
    dismissError();
    setRefreshError('');
    setLastVerified(null);
    setJoinStatus(null);
    resetSurfaces('connection-changed');
  }, [connectionId]); // eslint-disable-line react-hooks/exhaustive-deps -- runs per connection only; every callee is stable
  // A different channel: leave any history page and drop the draft written for the old one.
  useEffect(() => {
    historyPage.current = null;
    setHistoryBefore(null);
    setMessagesLoaded(false);
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    resetSurfaces('channel-changed');
  }, [channelId]); // eslint-disable-line react-hooks/exhaustive-deps -- runs per channel only; every callee is stable

  useEffect(() => {
    if (!connectionId) return;
    const controller = new AbortController();
    observer.current = controller;
    const current = ++generation.current;
    const active = () => !controller.signal.aborted && current === generation.current;
    let cursor: string | null = null;
    let immediateReconnects = 0;
    void (async () => {
      while (active()) {
        const started = Date.now();
        const outcome = await observeCrew(
          connectionId,
          channelId || undefined,
          cursor,
          controller.signal,
          (frame) => {
            if (!active()) return;
            if (frame.type === 'state') {
              if (frame.connection_id !== connectionId)
                throw new Error(crewObservationCopy.wrongConnection);
              const previousScope = verifiedScope.current;
              if (
                previousScope?.connection === connectionId &&
                (previousScope.epoch !== frame.snapshot.workspace.policy_epoch ||
                  previousScope.mode !== frame.connection_mode ||
                  previousScope.connectionEpoch !== frame.connection_policy_epoch ||
                  selectedSources.current.some(
                    (id) => !frame.snapshot.channels.some((item) => item.id === id)
                  ))
              ) {
                clearDraft();
                reportError(crewObservationCopy.scopeChanged, 'observer');
              }
              verifiedScope.current = {
                connection: connectionId,
                epoch: frame.snapshot.workspace.policy_epoch,
                connectionEpoch: frame.connection_policy_epoch,
                mode: frame.connection_mode,
              };
              setObservedPrivacy({
                connectionId,
                mode: frame.connection_mode,
                institutionId: frame.connection_institution_id ?? null,
                policyEpoch: frame.connection_policy_epoch,
              });
              setConnections((items) =>
                items.map((item) =>
                  item.id === connectionId
                    ? {
                        ...item,
                        mode: frame.connection_mode,
                        institution_id: frame.connection_institution_id ?? null,
                        policy_epoch: frame.connection_policy_epoch,
                      }
                    : item
                )
              );
              setSnapshot(frame.snapshot);
              setLabels(frameLabels(frame));
              setRuns(frame.runs);
              setRefreshError('');
              onVerifiedFrame(connectionId);
              setTeamId((old) =>
                frame.snapshot.teams.some((item) => item.id === old)
                  ? old
                  : (frame.snapshot.teams[0]?.id ?? '')
              );
              if (channelId && !frame.snapshot.channels.some((item) => item.id === channelId)) {
                controller.abort();
                generation.current += 1;
                setMessages([]);
                setMessagesLoaded(false);
                setBody('');
                setAttachments([]);
                setReferences([]);
                setContextChannels([]);
                setHistoryBefore(null);
                historyPage.current = null;
                pendingMessage.current = null;
                resetSurfaces('channel-revoked');
                setChannelId('');
                reportError(crewObservationCopy.channelAccessLost, 'observer');
              }
            } else if (frame.type === 'messages' && frame.channel_id === channelId) {
              cursor = frame.cursor ?? null;
              if (historyPage.current !== null) return;
              setMessages((previous) => {
                const next = frame.reset ? [] : [...previous];
                for (const message of frame.messages) {
                  const index = next.findIndex((old) => old.id === message.id);
                  if (index < 0) next.push(message);
                  else next[index] = message;
                }
                return next.slice(-200);
              });
              setMessagesLoaded(true);
            } else if (frame.type === 'reconnect') {
              cursor = frame.cursor ?? null;
            } else if (frame.type === 'error') {
              observationFailure(frame.error, frame.code);
              generation.current += 1;
            }
          }
        );
        if (outcome !== 'reconnect' || !active()) return;
        immediateReconnects = Date.now() - started < 1000 ? immediateReconnects + 1 : 0;
        if (immediateReconnects >= 3) throw new Error(crewObservationCopy.repeatedlyEnded);
      }
    })().catch((failure: unknown) => {
      if (!active()) return;
      observationFailure(
        failureMessage(failure, crewObservationCopy.observationFailed),
        failureCode(failure)
      );
      generation.current += 1;
    });
    return () => {
      controller.abort();
      observer.current?.abort();
      observer.current = null;
      generation.current += 1;
    };
  }, [
    connectionId,
    channelId,
    observationRevision,
    observationFailure,
    clearDraft,
    generation,
    selectedSources,
    pendingMessage,
    reportError,
    resetSurfaces,
    onVerifiedFrame,
    setConnections,
    setTeamId,
    setChannelId,
    setBody,
    setAttachments,
    setReferences,
    setContextChannels,
  ]);

  useEffect(() => {
    if (historyBefore === null || !connectionId || !channelId) return;
    const controller = new AbortController();
    const current = generation.current;
    setMessages([]);
    setMessagesLoaded(false);
    void crewRequest<{ messages: CrewMessage[]; cursor: string | null }>(
      connectionId,
      'messages.history',
      {
        channel_id: channelId,
        limit: 200,
        latest: true,
        before: historyBefore,
      },
      false,
      controller.signal
    )
      .then((page) => {
        if (!controller.signal.aborted && current === generation.current) {
          setMessages(page.messages);
          setMessagesLoaded(true);
        }
      })
      .catch((failure: unknown) => {
        if (controller.signal.aborted || current !== generation.current) return;
        observer.current?.abort();
        generation.current += 1;
        observationFailure(
          failureMessage(failure, crewObservationCopy.historyFailed),
          failureCode(failure)
        );
      });
    return () => controller.abort();
  }, [historyBefore, connectionId, channelId, observationRevision, observationFailure, generation]);

  // Presentation only: remember the last verified view so a re-verification can keep drawing it.
  useEffect(() => {
    if (!keepLastVerifiedView || !snapshot || observedPrivacy?.connectionId !== connectionId)
      return;
    setLastVerified((previous) => ({
      connectionId,
      snapshot,
      observedPrivacy,
      runs,
      labels,
      teamId,
      channelId,
      messages:
        messagesLoaded || !channelId
          ? messages
          : previous?.connectionId === connectionId && previous.channelId === channelId
            ? previous.messages
            : [],
    }));
  }, [
    keepLastVerifiedView,
    connectionId,
    teamId,
    channelId,
    snapshot,
    observedPrivacy,
    runs,
    labels,
    messages,
    messagesLoaded,
  ]);

  return {
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
    refreshError,
    lastVerified,
    setSnapshot,
    refresh,
    stopObserving,
    restartObservation,
    clearProtectedState,
    observationFailure,
  };
}
