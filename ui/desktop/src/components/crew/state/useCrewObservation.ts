import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type Dispatch,
  type MutableRefObject,
  type SetStateAction,
} from 'react';
import { crewBrokerCode } from '../api/errors';
import {
  CrewHttpError,
  crewRequest,
  observeCrew,
  validatedPeople,
  type CrewConnection,
  type CrewMessage,
  type CrewMessageAuthor,
  type CrewMessagePeople,
  type ObservedRun,
  type Snapshot,
} from '../crewApi';
import { HISTORY_PAGE_SIZE } from '../timeline/groupMessages';
import { crewObservationCopy } from './copy';
import type { CrewDraftState } from './crewSend';
import {
  DRAFT_CLEARING_OBSERVATION_CODES,
  failureCode,
  failureMessage,
  observationFailureOutcome,
} from './observationFailure';
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

/**
 * `base` with the entries of `next` added, as a new map only when something changed, so a
 * directory memoized on it is not rebuilt for every frame that names the same authors.
 */
export function mergePeople(
  base: CrewMessagePeople | null,
  next: CrewMessagePeople | null | undefined
): CrewMessagePeople | null {
  if (!next) return base;
  const entries = Object.entries(next);
  const changed = entries.some(([id, author]) => {
    const known = base ? base[id] : undefined;
    return (
      !known ||
      known.username !== author.username ||
      known.display_name !== author.display_name ||
      known.active !== author.active
    );
  });
  if (!changed) return base;
  const merged: Record<string, CrewMessageAuthor> = Object.create(null);
  if (base) for (const [id, author] of Object.entries(base)) merged[id] = author;
  for (const [id, author] of entries) merged[id] = author;
  return merged;
}

/**
 * Broker refusals of an older page that say nothing about access, privacy or who is asking: the
 * page cannot be loaded as asked, and the verified view stays. `response_too_large` is retried
 * with a smaller page first.
 */
const LOCAL_HISTORY_BROKER_CODES: readonly string[] = [
  'response_too_large',
  'rate_limited',
  'quota_exceeded',
  'invalid_params',
  'invalid_request',
];

/**
 * An older page failed for a reason that leaves the verified view standing: the daemon could not
 * serve it (5xx), or the broker refused its size or shape. Anything else — access, privacy, the
 * person, a stale cursor (which is also what a message the viewer may no longer see looks like),
 * or a failure it cannot classify — still clears the view, as every observation failure does.
 */
export function isLocalHistoryFailure(failure: unknown): boolean {
  if (!(failure instanceof CrewHttpError)) return false;
  const code = failure.brokerCode ?? failure.code;
  if (code && DRAFT_CLEARING_OBSERVATION_CODES.includes(code)) return false;
  if (failure.brokerCode) return LOCAL_HISTORY_BROKER_CODES.includes(failure.brokerCode);
  return failure.status >= 500;
}

function sameList(a: readonly string[] | null, b: readonly string[] | null): boolean {
  if (a === b) return true;
  if (!a || !b || a.length !== b.length) return false;
  return a.every((item, index) => item === b[index]);
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
  /** The live tail's opening backlog has arrived (true), is arriving (false), or unknown. */
  backlogComplete: boolean | undefined;
  /** The full-page size of what is shown: the live tail's, or the older page's. */
  pageSize: number;
  /** Authors the selected channel's message pages named. */
  people: CrewMessagePeople | null;
  /** The connected broker's capabilities, from the last `state` frame that carried any. */
  capabilities: readonly string[] | null;
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
  const [backlog, setBacklog] = useState<boolean | undefined>(undefined);
  const [livePageSize, setLivePageSize] = useState(HISTORY_PAGE_SIZE);
  const livePageSizeRef = useRef(HISTORY_PAGE_SIZE);
  useEffect(() => {
    livePageSizeRef.current = livePageSize;
  }, [livePageSize]);
  const [historyPageSize, setHistoryPageSize] = useState<number | null>(null);
  const [people, setPeople] = useState<CrewMessagePeople | null>(null);
  const [capabilities, setCapabilities] = useState<readonly string[] | null>(null);
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
    setPeople(null);
    setBacklog(undefined);
    setCapabilities(null);
    setHistoryBefore(null);
    setHistoryPageSize(null);
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
    setPeople(null);
    setBacklog(undefined);
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
    setPeople(null);
    setBacklog(undefined);
    setCapabilities(null);
    setLivePageSize(HISTORY_PAGE_SIZE);
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
    setHistoryPageSize(null);
    setMessagesLoaded(false);
    setPeople(null);
    setBacklog(undefined);
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
              const frameCapabilities = frame.capabilities?.length ? frame.capabilities : null;
              setCapabilities((previous) =>
                sameList(previous, frameCapabilities) ? previous : frameCapabilities
              );
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
                setPeople(null);
                setBacklog(undefined);
                historyPage.current = null;
                pendingMessage.current = null;
                resetSurfaces('channel-revoked');
                setChannelId('');
                reportError(crewObservationCopy.channelAccessLost, 'observer');
              }
            } else if (frame.type === 'messages' && frame.channel_id === channelId) {
              cursor = frame.cursor ?? null;
              const pageSize = frame.page_size;
              if (pageSize !== undefined) setLivePageSize(pageSize);
              if (historyPage.current !== null) return;
              setMessages((previous) => {
                const next = frame.reset ? [] : [...previous];
                for (const message of frame.messages) {
                  const index = next.findIndex((old) => old.id === message.id);
                  if (index < 0) next.push(message);
                  else next[index] = message;
                }
                return next.slice(-HISTORY_PAGE_SIZE);
              });
              const framePeople = frame.people;
              setPeople((previous) => mergePeople(frame.reset ? null : previous, framePeople));
              // `remaining` counts down the page this frame came from; the page a reset opens is
              // the channel's backlog, so its last frame (`0`) is the end of the opening.
              const remaining = frame.remaining;
              if (remaining === 0) setBacklog(true);
              else if (remaining !== undefined && frame.reset) setBacklog(false);
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

  // An older page. It asks for the page size the observer settled on, halves it while the broker
  // answers `response_too_large`, and records the size it was loaded with, so a full page of that
  // size still offers the page before it. A failure that says nothing about access returns to the
  // live tail with the error in the connection bar; any other clears the view.
  useEffect(() => {
    if (historyBefore === null || !connectionId || !channelId) return;
    const controller = new AbortController();
    const current = generation.current;
    const fresh = () => !controller.signal.aborted && current === generation.current;
    setMessages([]);
    setMessagesLoaded(false);
    setHistoryPageSize(null);
    void (async () => {
      let limit = Math.max(1, Math.min(livePageSizeRef.current, HISTORY_PAGE_SIZE));
      for (;;) {
        try {
          const page = await crewRequest<{
            messages: CrewMessage[];
            cursor: string | null;
            people?: unknown;
          }>(
            connectionId,
            'messages.history',
            { channel_id: channelId, limit, latest: true, before: historyBefore },
            false,
            controller.signal
          );
          if (!fresh()) return;
          const pagePeople = validatedPeople(page.people);
          setHistoryPageSize(limit);
          setMessages(page.messages);
          setPeople((previous) => mergePeople(previous, pagePeople));
          setMessagesLoaded(true);
          return;
        } catch (failure: unknown) {
          if (!fresh()) return;
          if (crewBrokerCode(failure) === 'response_too_large' && limit > 1) {
            limit = Math.max(1, Math.floor(limit / 2));
            continue;
          }
          if (isLocalHistoryFailure(failure)) {
            const detail = failure instanceof Error ? failure.message : '';
            historyPage.current = null;
            setHistoryBefore(null);
            reportError(
              detail
                ? `${crewObservationCopy.historyFailed} ${detail}`
                : crewObservationCopy.historyFailed,
              'observer',
              failureCode(failure)
            );
            // The live observer ignored the tail while the page was asked for: start it over.
            setObservationRevision((revision) => revision + 1);
            return;
          }
          observer.current?.abort();
          generation.current += 1;
          observationFailure(
            failureMessage(failure, crewObservationCopy.historyFailed),
            failureCode(failure)
          );
          return;
        }
      }
    })();
    return () => controller.abort();
  }, [
    historyBefore,
    connectionId,
    channelId,
    observationRevision,
    observationFailure,
    reportError,
    generation,
  ]);

  // Presentation only: remember the last verified view so a re-verification can keep drawing it.
  useEffect(() => {
    if (!keepLastVerifiedView || !snapshot || observedPrivacy?.connectionId !== connectionId)
      return;
    setLastVerified((previous) => {
      const current = messagesLoaded || !channelId;
      const same = previous?.connectionId === connectionId && previous.channelId === channelId;
      return {
        connectionId,
        snapshot,
        observedPrivacy,
        runs,
        labels,
        teamId,
        channelId,
        messages: current ? messages : same ? previous.messages : [],
        people: current ? people : same ? (previous.people ?? null) : null,
      };
    });
  }, [
    keepLastVerifiedView,
    connectionId,
    teamId,
    channelId,
    snapshot,
    observedPrivacy,
    runs,
    labels,
    people,
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
    backlogComplete: backlog,
    pageSize: historyBefore !== null ? (historyPageSize ?? HISTORY_PAGE_SIZE) : livePageSize,
    people,
    capabilities,
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
