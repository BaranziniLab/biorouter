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
import {
  channelName,
  connectionNames,
  identityCopy,
  isMachineIdShaped,
  sanitizeDisplayText,
} from '../identity';
import { HISTORY_PAGE_SIZE } from '../timeline/groupMessages';
import { crewObservationCopy } from './copy';
import type { CrewDraftState } from './crewSend';
import {
  DRAFT_CLEARING_OBSERVATION_CODES,
  draftScope,
  draftScopeChanged,
  failureCode,
  isRecoverableObservationCode,
  observationFailureCode,
  observationFailureOutcome,
  observationFrameText,
  type DraftScope,
  type ObservationNames,
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

// ---------------------------------------------------------------------------------------------
// Observing again by itself
// ---------------------------------------------------------------------------------------------

/**
 * The waits before each automatic re-observation after a recoverable end (a policy epoch moved,
 * a stale cursor…): 0.3 s, then 1 s, then 3 s. A verified `state` frame starts the count again.
 */
export const REOBSERVE_BACKOFF_MS: readonly number[] = [300, 1000, 3000];
/** The window the attempts are counted in. */
export const REOBSERVE_WINDOW_MS = 60_000;
/**
 * Every automatic re-observation in the window, verified or not. A stream that verifies and then
 * ends at once would otherwise reset the count forever; past this, the person decides.
 */
export const REOBSERVE_CEILING = 10;

export interface RecoveryBudget {
  /** When each re-observation since the last verified frame started. */
  attempts: number[];
  /** When every re-observation in the window started. */
  all: number[];
}

/**
 * The wait before the next automatic re-observation, recorded as taken; null once three have been
 * tried since the last verified frame, or the ceiling is reached, within the window.
 */
export function takeRecoveryDelay(budget: RecoveryBudget, now: number): number | null {
  budget.attempts = budget.attempts.filter((at) => now - at < REOBSERVE_WINDOW_MS);
  budget.all = budget.all.filter((at) => now - at < REOBSERVE_WINDOW_MS);
  if (
    budget.attempts.length >= REOBSERVE_BACKOFF_MS.length ||
    budget.all.length >= REOBSERVE_CEILING
  )
    return null;
  const delay = REOBSERVE_BACKOFF_MS[budget.attempts.length];
  budget.attempts.push(now);
  budget.all.push(now);
  return delay;
}

/** Sentences the observer writes itself, shown as they are rather than mapped by code. */
const OWN_FAILURES: readonly string[] = [
  crewObservationCopy.wrongConnection,
  crewObservationCopy.repeatedlyEnded,
];

/** The workspace's display name and the selected channel's `#name`, for the plain sentences. */
function observationNames(
  view: Pick<Snapshot, 'workspace' | 'channels'> | null,
  connections: readonly CrewConnection[],
  connectionId: string,
  channelId: string
): ObservationNames {
  const named = sanitizeDisplayText(view?.workspace.name);
  const workspace =
    named && !isMachineIdShaped(named)
      ? named
      : (connectionNames(connections).get(connectionId) ?? identityCopy.unnamedWorkspace);
  const channel = channelId ? view?.channels.find((item) => item.id === channelId) : undefined;
  return { workspace, channel: channel ? channelName(channel) : null };
}

export interface CrewObservationContext {
  connectionId: string;
  channelId: string;
  teamId: string;
  setTeamId: Dispatch<SetStateAction<string>>;
  setChannelId: Dispatch<SetStateAction<string>>;
  generation: MutableRefObject<number>;
  /**
   * The saved connections. The selected one's status decides whether a recoverable end is observed
   * again by itself (only a connection the daemon calls connected), and its name is what the plain
   * sentences call a workspace that has not told its own name.
   */
  connections: readonly CrewConnection[];
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
  /** The code the observation ended with (the broker's, when it named one), beside `refreshError`. */
  refreshErrorCode: string | null;
  /** A verified view ended for a recoverable reason and is being observed again by itself. */
  reverifying: boolean;
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
 * Frame validation, the generation guard that drops frames from a retired observer, and the throw
 * after three fast reconnects are as they were. What an observation's end does changed after live
 * QA round 1 (P0-1, T-07):
 * - A terminal frame always clears the protected view (the daemon sends `clear: true`).
 * - A recoverable end (`policy_changed`, `channel_access_changed`, `stale_cursor`, `scope_changed`)
 *   on a connection the daemon calls connected is observed again by itself, after 0.3, 1 and 3 s,
 *   keeping the draft and showing no error. Only when that fails does the bar say so, plainly.
 * - The draft is cleared only when what it was written under changed materially
 *   (`draftScopeChanged`) — never because the workspace policy epoch moved — or when access was
 *   lost (`observationFailureOutcome`). Either says so only when the composer held something.
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
  } = context;
  const {
    body,
    setBody,
    attachments,
    setAttachments,
    references,
    setReferences,
    contextChannels,
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
  const [refreshErrorCode, setRefreshErrorCode] = useState<string | null>(null);
  const [reverifying, setReverifying] = useState(false);
  const [lastVerified, setLastVerified] = useState<VerifiedView | null>(null);
  const observer = useRef<AbortController | null>(null);
  const [observationRevision, setObservationRevision] = useState(0);
  const historyPage = useRef<string | null>(null);
  useEffect(() => {
    historyPage.current = historyBefore;
  }, [historyBefore]);

  // What async callbacks read: whether the composer holds anything, the saved connections, the
  // error on show, and the last verified snapshot (for names in the plain sentences only).
  const draftHasContent = useRef(false);
  useEffect(() => {
    draftHasContent.current = Boolean(
      body.trim() || attachments.length || references.length || contextChannels.length
    );
  }, [body, attachments, references, contextChannels]);
  const connectionsRef = useRef(connections);
  useEffect(() => {
    connectionsRef.current = connections;
  }, [connections]);
  const refreshErrorRef = useRef(refreshError);
  useEffect(() => {
    refreshErrorRef.current = refreshError;
  }, [refreshError]);
  const lastFrame = useRef<{ connectionId: string; snapshot: Snapshot } | null>(null);
  const namesFor = useCallback((id: string, channel: string) => {
    const view = lastFrame.current?.connectionId === id ? lastFrame.current.snapshot : null;
    return observationNames(view, connectionsRef.current, id, channel);
  }, []);

  const verifiedScope = useRef<DraftScope | null>(null);
  const clearProtectedState = useCallback(
    (reason: SurfaceResetReason = 'protected-cleared') => {
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
      resetSurfaces(reason);
    },
    [resetSurfaces]
  );

  const clearProtectedView = useCallback(() => clearProtectedState(), [clearProtectedState]);

  // The automatic re-observation: its budget, the pending wait, and the connection reload before it.
  const recovery = useRef<{
    budget: RecoveryBudget;
    timer: ReturnType<typeof setTimeout> | undefined;
    load: AbortController | undefined;
    token: number;
  }>({ budget: { attempts: [], all: [] }, timer: undefined, load: undefined, token: 0 });
  const cancelRecovery = useCallback(() => {
    const pending = recovery.current;
    if (pending.timer !== undefined) clearTimeout(pending.timer);
    pending.timer = undefined;
    pending.load?.abort();
    pending.load = undefined;
    pending.token += 1;
  }, []);
  const scheduleReobservation = useCallback(
    (delay: number) => {
      cancelRecovery();
      const pending = recovery.current;
      const token = pending.token;
      pending.timer = setTimeout(() => {
        pending.timer = undefined;
        if (token !== pending.token) return;
        // Reload the saved connection first: a policy end can mean its binding moved, and the
        // reloaded status says whether observing again can help.
        const load = new AbortController();
        pending.load = load;
        void loadConnections(load.signal)
          .catch(() => undefined)
          .then(() => {
            if (token !== pending.token || load.signal.aborted) return;
            pending.load = undefined;
            setObservationRevision((revision) => revision + 1);
          });
      }, delay);
    },
    [cancelRecovery, loadConnections]
  );

  const observationFailure = useCallback(
    (message: string, code?: string) => {
      cancelRecovery();
      clearProtectedState();
      const outcome = observationFailureOutcome(message, code, {
        draftHasContent: draftHasContent.current,
        deferRecoverableToReverification: true,
      });
      if (outcome.clearDraft) clearDraft();
      setReverifying(false);
      setRefreshErrorCode(code ?? null);
      setRefreshError(outcome.text);
    },
    [cancelRecovery, clearProtectedState, clearDraft]
  );
  const refresh = useCallback(async () => {
    cancelRecovery();
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
    setRefreshErrorCode(null);
    setReverifying(false);
    try {
      await loadConnections(controller.signal, current);
      if (!controller.signal.aborted && generation.current === current)
        setObservationRevision((revision) => revision + 1);
    } catch (failure) {
      if (!controller.signal.aborted && generation.current === current)
        observationFailure(crewObservationCopy.connectionsRefreshFailed, failureCode(failure));
    }
  }, [cancelRecovery, generation, loadConnections, observationFailure, resetSurfaces]);
  const restartObservation = useCallback(
    () => setObservationRevision((revision) => revision + 1),
    []
  );
  const stopObserving = useCallback(() => {
    cancelRecovery();
    observer.current?.abort();
    generation.current += 1;
    clearProtectedState();
    setRefreshError('');
    setRefreshErrorCode(null);
    setReverifying(false);
  }, [cancelRecovery, generation, clearProtectedState]);
  useEffect(
    () => () => {
      cancelRecovery();
      observer.current?.abort();
      generation.current += 1;
    },
    [cancelRecovery, generation]
  );

  // A different connection: drop everything verified for the old one, and its draft.
  useEffect(() => {
    generation.current += 1;
    cancelRecovery();
    recovery.current.budget = { attempts: [], all: [] };
    lastFrame.current = null;
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
    setRefreshErrorCode(null);
    setReverifying(false);
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
    setLivePageSize(HISTORY_PAGE_SIZE);
    setBody('');
    setAttachments([]);
    setReferences([]);
    setContextChannels([]);
    resetSurfaces('channel-changed');
  }, [channelId]); // eslint-disable-line react-hooks/exhaustive-deps -- runs per channel only; every callee is stable

  // The daemon now calls the selected connection connected — a connect from a terminal, or a
  // host's bootstrap — while an observation error is still on show: observe again at once, rather
  // than leave the person to find Retry.
  const savedStatus = connections.find((item) => item.id === connectionId)?.status;
  const statusSeen = useRef<{ connectionId: string; status: string | undefined }>({
    connectionId: '',
    status: undefined,
  });
  useEffect(() => {
    const seen = statusSeen.current;
    statusSeen.current = { connectionId, status: savedStatus };
    if (
      seen.connectionId !== connectionId ||
      seen.status === undefined ||
      seen.status === 'connected' ||
      savedStatus !== 'connected' ||
      !refreshErrorRef.current
    )
      return;
    cancelRecovery();
    setRefreshError('');
    setRefreshErrorCode(null);
    setObservationRevision((revision) => revision + 1);
  }, [connectionId, savedStatus, cancelRecovery]);

  useEffect(() => {
    if (!connectionId) return;
    // Whatever started this observation, it is the re-observation a pending wait was for.
    cancelRecovery();
    const controller = new AbortController();
    observer.current = controller;
    const current = ++generation.current;
    const active = () => !controller.signal.aborted && current === generation.current;
    let cursor: string | null = null;
    let immediateReconnects = 0;

    /**
     * The selected channel is gone for this person: close it, and its draft, and say so. `named`
     * is its `#name` from the last view that still had it.
     */
    const loseChannel = (named: string | null) => {
      const hadContent = draftHasContent.current;
      historyPage.current = null;
      pendingMessage.current = null;
      setMessages([]);
      setMessagesLoaded(false);
      setBody('');
      setAttachments([]);
      setReferences([]);
      setContextChannels([]);
      setHistoryBefore(null);
      setPeople(null);
      setBacklog(undefined);
      resetSurfaces('channel-revoked');
      setChannelId('');
      const lostText = named
        ? crewObservationCopy.channelAccessLostNamed(named)
        : crewObservationCopy.channelAccessLost;
      reportError(
        hadContent ? `${lostText} ${crewObservationCopy.draftDiscarded}` : lostText,
        'observer'
      );
    };

    /**
     * The observation ended, by a terminal frame or a failure. A recoverable end on a connection
     * the daemon calls connected is observed again by itself — the draft kept, no error shown —
     * until the attempts run out; anything else, and running out, is shown in plain words.
     */
    const ended = (code: string | undefined, text: string) => {
      generation.current += 1;
      const status = connectionsRef.current.find((item) => item.id === connectionId)?.status;
      if (isRecoverableObservationCode(code) && status === 'connected') {
        if (code === 'channel_access_changed' && channelId) {
          // The daemon checked the selected channel against a fresh snapshot and it was gone:
          // observing it again would only end the same way. Close it and observe the workspace.
          const named = namesFor(connectionId, channelId).channel;
          clearProtectedState('channel-revoked');
          loseChannel(named);
          setReverifying(true);
          return;
        }
        const delay = takeRecoveryDelay(recovery.current.budget, Date.now());
        if (delay !== null) {
          // `clear: true`: nothing verified stays on screen. The pane survives, as through a
          // refresh; the draft stays, and cannot be sent until the next view verifies it.
          clearProtectedState('refresh');
          setRefreshError('');
          setRefreshErrorCode(null);
          setReverifying(true);
          scheduleReobservation(delay);
          return;
        }
        observationFailure(
          crewObservationCopy.updatesStopped(namesFor(connectionId, channelId).workspace),
          code
        );
        return;
      }
      observationFailure(text, code);
    };

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
              // SECURITY-SENSITIVE (human review): the draft is cleared before this view is shown
              // when what it was written under changed materially. A workspace policy epoch moving
              // on its own is not such a change (see `DraftScope`).
              if (
                draftScopeChanged(verifiedScope.current, frame, channelId, selectedSources.current)
              ) {
                const hadContent = draftHasContent.current;
                clearDraft();
                if (hadContent) reportError(crewObservationCopy.scopeChanged, 'observer');
              }
              verifiedScope.current = draftScope(frame, channelId, selectedSources.current);
              const revoked =
                Boolean(channelId) &&
                !frame.snapshot.channels.some((item) => item.id === channelId);
              // Named from the last view that still had it: this one no longer does.
              const revokedName = revoked ? namesFor(connectionId, channelId).channel : null;
              lastFrame.current = { connectionId, snapshot: frame.snapshot };
              recovery.current.budget.attempts = [];
              setReverifying(false);
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
              setRefreshErrorCode(null);
              onVerifiedFrame(connectionId);
              setTeamId((old) =>
                frame.snapshot.teams.some((item) => item.id === old)
                  ? old
                  : (frame.snapshot.teams[0]?.id ?? '')
              );
              if (revoked) {
                controller.abort();
                generation.current += 1;
                loseChannel(revokedName);
              }
            } else if (frame.type === 'messages' && frame.channel_id === channelId) {
              cursor = frame.cursor ?? null;
              const pageSize = frame.page_size;
              // Exact only on a reset. A reconnected observer restarts at the full page, but the
              // tail it extends was loaded at the smaller size, so keep the smallest since then.
              if (pageSize !== undefined)
                setLivePageSize((previous) =>
                  frame.reset ? pageSize : Math.min(previous, pageSize)
                );
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
              // The daemon's own sentence is never shown: it is the same jargon for every end.
              const code = typeof frame.code === 'string' ? frame.code : undefined;
              ended(code, observationFrameText(code, namesFor(connectionId, channelId)));
            }
          }
        );
        if (outcome !== 'reconnect' || !active()) return;
        immediateReconnects = Date.now() - started < 1000 ? immediateReconnects + 1 : 0;
        if (immediateReconnects >= 3) throw new Error(crewObservationCopy.repeatedlyEnded);
      }
    })().catch((failure: unknown) => {
      if (!active()) return;
      const code = observationFailureCode(failure);
      ended(
        code,
        failure instanceof Error && OWN_FAILURES.includes(failure.message)
          ? failure.message
          : observationFrameText(code, namesFor(connectionId, channelId))
      );
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
    cancelRecovery,
    scheduleReobservation,
    clearProtectedState,
    namesFor,
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
          observationFailure(crewObservationCopy.historyFailed, observationFailureCode(failure));
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
    refreshErrorCode,
    reverifying,
    lastVerified,
    setSnapshot,
    refresh,
    stopObserving,
    restartObservation,
    clearProtectedState: clearProtectedView,
    observationFailure,
  };
}
