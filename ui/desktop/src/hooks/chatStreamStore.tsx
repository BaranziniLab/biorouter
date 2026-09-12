import React, { createContext, useContext, useSyncExternalStore } from 'react';
import { ChatState } from '../types/chatState';
import {
  ActiveTurnRef,
  cancelTurn,
  ChatRequest,
  getSession,
  interrupt,
  listSessions,
  Message,
  MessageEvent,
  observeSessionEvents,
  reply,
  resumeAgent,
  Session,
  SessionClassification,
  TokenState,
  updateFromSession,
  updateSessionUserWorkflowValues,
} from '../api';
import {
  announceSessionName,
  cacheGet,
  cacheSet,
  isDefaultSessionName,
  renameSession,
  subscribeSessionNameChanges,
} from '../utils/sessionNameSync';
import { subscribeSessionBindingChanges } from '../utils/sessionBindingSync';
import {
  getCachedSessionList,
  notifySessionListChanged,
  updateCachedSessionList,
} from '../utils/sessionListCache';
import { subscribeToSessionMeta } from '../utils/sessionMetaSubscription';
import { raiseTier } from '../components/privacy/sessionTier';
import {
  createElicitationResponseMessage,
  createUserMessage,
  getCompactingMessage,
  getElicitationContent,
  getSecretRequestContent,
  getThinkingMessage,
  getToolConfirmationContent,
  NotificationEvent,
  UserAttachment,
} from '../types/message';
import { describeRequestFailure, errorMessage, isConnectionError } from '../utils/conversionUtils';
import { showExtensionLoadResults } from '../utils/extensionErrorUtils';
import { reasoningEffortForRequest } from '../store/reasoningEffort';
import { userActionHeaders } from '../utils/userAction';
import {
  abandonContinuationLease as abandonContinuationLeaseRequest,
  getContinuationOwnerId,
  recoverContinuationGroup,
  type ContinuationRecoveryAction,
} from '../utils/continuationLease';
import type { ChatTurnErrorData, TurnErrorScope } from '../types/turnError';
import type { PendingSteer } from '../utils/trailingActivity';

/**
 * BR-62b — a client-generated idempotency key naming a single `/reply` turn. If
 * the SSE transport reconnects and re-POSTs the same body (a flaky network, a
 * resumed fetch), it resends this key, so the server recognises the retry as a
 * duplicate of the turn already in flight (409 `duplicate:true`) instead of
 * starting a second turn. A fresh key is minted per turn, so a genuine next
 * turn is never mistaken for a retry of the previous one.
 */
function newTurnId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === 'function') {
    return c.randomUUID();
  }
  return `turn-${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/**
 * The `/reply` body that ATTACHES to a turn already in flight rather than
 * starting one (contract §2).
 *
 * Every deviation from an ordinary reply body is deliberate and each one is a
 * thing the server lane has to agree to:
 *
 *  - **`turn_id` names an existing turn.** That is the whole request. The
 *    server's `try_begin_turn_idempotent` already recognises it as a duplicate;
 *    the contract changes the answer from a JSON 409 to 200 + the stream.
 *
 *  - **`from_seq` goes in the BODY, not the query string** — "I hold frames
 *    0..N-1, send me the rest". An optimisation only: the sequence gate drops a
 *    full replay correctly if the server ignores it.
 *
 *  - **`user_message` is a formality here and is ignored by the server.** The
 *    schema requires it, but a client attaching to someone else's turn does not
 *    know what started it. We send the transcript's trailing user message when
 *    there is one — for a live turn that IS the message that started it, since
 *    the driver appended it before POSTing and the server persisted it — and an
 *    empty one otherwise. Were it ever honoured on a duplicate `turn_id`, an
 *    attach would inject a phantom prompt into a running turn.
 *
 * **`turnId` may be either name for the turn.** The turn's *idempotency key*
 * (what its original caller minted, which only that caller holds) and the
 * server's own `turn-N` (what a frame's `turn_id` and `active_turn` carry) both
 * work — `/reply` matches on either. That is not a nicety: a reloaded window
 * has only ever seen the server's name, so matching on the key alone would 409
 * exactly the case this feature exists for.
 */
function buildAttachRequest(
  sessionId: string,
  turnId: string,
  fromSeq: number,
  messages: Message[],
  continuationLease?: string
): ChatRequest {
  const trailingUser = [...messages].reverse().find((m) => m.role === 'user');
  const placeholder: Message = {
    role: 'user',
    created: Math.floor(Date.now() / 1000),
    content: [],
    metadata: { userVisible: false, agentVisible: false },
  };
  return {
    session_id: sessionId,
    turn_id: turnId,
    user_message: trailingUser ?? placeholder,
    from_seq: fromSeq,
    ...(continuationLease ? { continuation_lease: continuationLease } : {}),
  } as ChatRequest;
}

/**
 * The fields the live-turn stream contract adds to every `/reply` frame, on top
 * of the generated `MessageEvent` union.
 *
 * They are read structurally rather than through `api/types.gen.ts` because
 * that file is generated from the OpenAPI spec and must not be hand-edited; it
 * regains these the moment the server lane regenerates. Reading them
 * defensively is not just a build convenience — an OBSERVER stream
 * (`/sessions/{id}/events`) carries no sequence at all, and so must keep
 * working with every field absent.
 *
 *  - `seq` — monotonic per-TURN frame number from 0 (contract §1). The whole
 *    basis of idempotent replay.
 *  - `turn_id` — which turn `seq` counts within. Requested in addition to the
 *    contract's §1 because `seq` RESTARTS at 0 each turn: without a turn
 *    identity on the frame, the first frame of turn N+1 (`seq: 0`) is
 *    indistinguishable from a replayed frame of turn N and would be dropped as
 *    a duplicate. See `applySequenceGate`.
 *  - `replay` — true on frames served from the backlog, absent/false on the
 *    live tail (contract §R2, "the client must be able to tell replay from
 *    live"). It is what lets the backlog land as ONE commit.
 */
type StreamFrameEnvelope = {
  seq?: number | null;
  turn_id?: string | null;
  replay?: boolean | null;
};

type TurnStartedEvent = {
  type: 'TurnStarted';
  turn_id: string;
};

type TurnStateEvent = {
  type: 'TurnState';
  active_turn_id: string | null;
};

function turnStartedEvent(event: MessageEvent): TurnStartedEvent | null {
  const candidate = event as unknown as Partial<TurnStartedEvent>;
  return candidate.type === 'TurnStarted' && typeof candidate.turn_id === 'string'
    ? (candidate as TurnStartedEvent)
    : null;
}

function turnStateEvent(event: MessageEvent): TurnStateEvent | null {
  const candidate = event as unknown as Partial<TurnStateEvent>;
  return candidate.type === 'TurnState' &&
    (typeof candidate.active_turn_id === 'string' || candidate.active_turn_id === null)
    ? (candidate as TurnStateEvent)
    : null;
}

type ResumeInitializationEnvelope = {
  initializing?: boolean | null;
  pending_continuation?: {
    ownership?: 'owned' | 'foreign' | 'settling' | null;
    superseded_turn_id?: string | null;
    continuation_lease?: string | null;
  } | null;
};

export interface PendingContinuationView {
  ownership: 'owned' | 'foreign' | 'settling';
  supersededTurnId: string;
}

type CancelTurnMismatch = {
  mismatch: true;
  expected_turn_id: string;
  active_turn_id: string;
};

type StreamDrainOptions = {
  onFirstEvent?: () => void;
  queuedInitializingChildMessage?: Message;
  hadTransportError?: () => boolean;
};

function isInitializingResume(value: unknown): boolean {
  return (value as ResumeInitializationEnvelope | null | undefined)?.initializing === true;
}

function resumePendingContinuation(value: unknown): {
  view: PendingContinuationView;
  continuationLease: string | null;
} | null {
  const pending = (value as ResumeInitializationEnvelope | null | undefined)?.pending_continuation;
  if (
    !pending ||
    (pending.ownership !== 'owned' &&
      pending.ownership !== 'foreign' &&
      pending.ownership !== 'settling') ||
    typeof pending.superseded_turn_id !== 'string'
  ) {
    return null;
  }
  return {
    view: {
      ownership: pending.ownership,
      supersededTurnId: pending.superseded_turn_id,
    },
    continuationLease:
      pending.ownership === 'owned' && typeof pending.continuation_lease === 'string'
        ? pending.continuation_lease
        : null,
  };
}

function resumeRequestBody(sessionId: string, loadModelAndExtensions: boolean) {
  return {
    session_id: sessionId,
    load_model_and_extensions: loadModelAndExtensions,
    continuation_owner_id: getContinuationOwnerId(),
  };
}

function cancelTurnMismatch(error: unknown): CancelTurnMismatch | null {
  if (!error || typeof error !== 'object') return null;
  const candidate = error as Partial<CancelTurnMismatch>;
  return candidate.mismatch === true &&
    typeof candidate.expected_turn_id === 'string' &&
    typeof candidate.active_turn_id === 'string'
    ? (candidate as CancelTurnMismatch)
    : null;
}

/** The per-turn sequence number of a frame, when the producer stamps one. */
export function frameSeq(event: MessageEvent): number | undefined {
  const seq = (event as MessageEvent & StreamFrameEnvelope).seq;
  return typeof seq === 'number' ? seq : undefined;
}

/** Which turn a frame's `seq` counts within, when the producer names it. */
export function frameTurnId(event: MessageEvent): string | undefined {
  const turnId = (event as MessageEvent & StreamFrameEnvelope).turn_id;
  return typeof turnId === 'string' && turnId ? turnId : undefined;
}

/** Was this frame served from the replay backlog rather than the live tail? */
export function isReplayFrame(event: MessageEvent): boolean {
  return (event as MessageEvent & StreamFrameEnvelope).replay === true;
}

function isTerminalEvent(event: MessageEvent): boolean {
  return event.type === 'Error' || event.type === 'Finish';
}

/**
 * Ceiling on how long the transcript may be held back while a replay backlog
 * drains (see `beginReplayHold`).
 *
 * Holding notifications is what turns a backlog into a single commit instead of
 * a visible re-typing of the turn. But it means the user sees NOTHING new until
 * the hold ends, so a producer that streams a huge backlog slowly — or one that
 * stamps `replay` and never clears it — must not be able to freeze the
 * transcript indefinitely. When this fires the hold is released and rendering
 * simply continues progressively: degraded, never stuck.
 *
 * Half a second is far longer than a buffered backlog takes to arrive over
 * loopback (it is one or two reads of an already-materialised buffer) and short
 * enough that a user cannot mistake the pause for a hang.
 */
export const REPLAY_MAX_HOLD_MS = 500;

/**
 * How long an observer stream has to last before the reconnect backoff counts
 * it as a real connection and resets to its floor.
 *
 * The backoff used to reset when the stream OPENED, which reads as harmless and
 * is not: a stream can be answered 200 and end immediately, and that is exactly
 * what the daemon does to an observer over its budget (it sends the whole
 * stored conversation, then closes rather than parking a connection the client
 * cannot spare, `MAX_LIVE_OBSERVER_STREAMS` in biorouter-server). Resetting on
 * open made every such stream a fresh start, so the loop retried at about 1 Hz
 * forever and never climbed toward its 15 s ceiling: an over-budget tab
 * degraded from streaming into polling the conversation once a second. With the
 * reset moved to the END of a stream that lasted, the same tab walks 1, 2, 4, 8,
 * 15 s and stays there.
 *
 * THE THRESHOLD IS SIX HEARTBEATS, not a round number. A stream that is really
 * following the tail is held open behind the daemon's 500 ms heartbeat, so
 * three seconds of life is direct evidence that at least six of them arrived.
 * A refused observer's stream is one snapshot write and a close over loopback,
 * orders of magnitude below it.
 *
 * Err HIGH, because the two mistakes are not comparable. Read a refusal as
 * healthy and the 1 Hz poll is back. Read a healthy stream as a refusal and the
 * next reconnect waits 2 s instead of 1 s, and the first stream that does last
 * puts the floor back. So a flaky network costs a user one doubling per short
 * stream and nothing after that.
 */
const HEALTHY_OBSERVER_STREAM_MS = 3000;

const SESSION_LIST_CACHE_TTL_MS = 5000;
let sessionListInflight: Promise<{ id: string; name?: string | null }[]> | null = null;
let sessionListInflightAt = 0;

async function fetchAllSessions(): Promise<{ id: string; name?: string | null }[]> {
  const now = Date.now();
  if (sessionListInflight && now - sessionListInflightAt < SESSION_LIST_CACHE_TTL_MS) {
    return sessionListInflight;
  }
  sessionListInflightAt = now;
  sessionListInflight = (async () => {
    const response = await listSessions({ throwOnError: true });
    return (response.data?.sessions ?? []) as { id: string; name?: string | null }[];
  })();
  sessionListInflight.catch(() => {
    sessionListInflight = null;
  });
  return sessionListInflight;
}

async function disambiguateSessionName(
  proposed: string,
  currentSessionId: string
): Promise<string> {
  let existingNames: Set<string>;
  try {
    const sessions = await fetchAllSessions();
    existingNames = new Set(
      sessions
        .filter((s) => s.id !== currentSessionId)
        .map((s) => s.name)
        .filter((n): n is string => typeof n === 'string')
    );
  } catch (e) {
    console.warn('disambiguateSessionName: failed to list sessions:', e);
    return proposed;
  }
  if (!existingNames.has(proposed)) return proposed;
  for (let n = 2; n < 1000; n++) {
    const candidate = `${proposed} ${n}`;
    if (!existingNames.has(candidate)) return candidate;
  }
  return proposed;
}

function sameContent(a: Message, b: Message): boolean {
  return a.role === b.role && JSON.stringify(a.content) === JSON.stringify(b.content);
}

function pushMessage(currentMessages: Message[], incomingMsg: Message): Message[] {
  const lastMsg = currentMessages[currentMessages.length - 1];

  if (lastMsg?.id && lastMsg.id === incomingMsg.id) {
    const updatedLastMsg = {
      ...lastMsg,
      content: [...lastMsg.content],
    };
    const lastContent = lastMsg.content[lastMsg.content.length - 1];
    const newContent = incomingMsg.content[incomingMsg.content.length - 1];

    if (
      lastContent?.type === 'text' &&
      newContent?.type === 'text' &&
      incomingMsg.content.length === 1
    ) {
      const updatedLastContent = { ...lastContent };
      if (newContent.text.startsWith(updatedLastContent.text)) {
        updatedLastContent.text = newContent.text;
      } else if (!updatedLastContent.text.endsWith(newContent.text)) {
        updatedLastContent.text += newContent.text;
      }
      updatedLastMsg.content[updatedLastMsg.content.length - 1] = updatedLastContent;
    } else {
      const existingContent = new Set(
        updatedLastMsg.content.map((content) => JSON.stringify(content))
      );
      updatedLastMsg.content.push(
        ...incomingMsg.content.filter((content) => !existingContent.has(JSON.stringify(content)))
      );
    }
    return [...currentMessages.slice(0, -1), updatedLastMsg];
  }

  if (lastMsg && sameContent(lastMsg, incomingMsg)) {
    return currentMessages;
  }

  return [...currentMessages, incomingMsg];
}

/**
 * BR-61 — has the agent consumed the soft interrupt we are optimistically
 * showing? The agent echoes a steer back onto the live stream as an ordinary
 * user message once it injects it, and that echo is the ONLY reliable signal
 * that it landed.
 *
 * `afterCount` is the transcript length at the moment the steer was issued, so
 * only messages that arrived AFTER the press can satisfy it. Matching on text
 * alone would let a user who steers with the same words as an earlier prompt
 * clear the indicator against their own history, the instant it appeared.
 */
export function steerWasEchoed(
  pending: { text: string } | undefined,
  messages: Message[],
  afterCount: number
): boolean {
  if (!pending) return false;
  const wanted = pending.text.trim();
  if (!wanted) return false;
  return messages
    .slice(afterCount)
    .some(
      (m) =>
        m.role === 'user' && m.content.some((c) => c.type === 'text' && c.text.trim() === wanted)
    );
}

export interface ChatStreamSnapshot {
  session?: Session;
  messages: Message[];
  chatState: ChatState;
  sessionLoadError?: string;
  turnError?: ChatTurnErrorData;
  tokenState: TokenState;
  notifications: NotificationEvent[];
  /**
   * Client clock (ms) when the current turn was submitted; undefined while
   * idle. Fallback origin for the trailing activity timer when no Message
   * event has landed yet.
   */
  turnStartedAt?: number;
  /**
   * Client clock (ms) when the most recent Message event was APPLIED. This is
   * deliberately a client timestamp, not `message.created` (which is SECONDS —
   * see utils/timeUtils.ts — and carries server-clock skew). It is the origin
   * every live elapsed display counts from, and living in the store is what
   * makes it survive the message list's per-event re-render churn.
   */
  lastMessageAt?: number;
  /**
   * BR-61 — the soft interrupt this client has issued and the agent has not yet
   * consumed. Set OPTIMISTICALLY, before the POST resolves, because the whole
   * point is to fill the dead air between the user's press and the agent's next
   * output; cleared on rejection, on echo, and on any turn boundary, so it can
   * never outlive the thing it describes.
   */
  pendingSteer?: PendingSteer;
  /** A durable Stop-and-Send gap discovered from the daemon on resume. */
  pendingContinuation?: PendingContinuationView;
  /**
   * F5 — this chat's last Stop provably ended a running turn: the daemon
   * answered its exact-generation cancel `cancelled: true, settled: true`.
   * BaseChat renders it as a quiet "Stopped." line in the slot a failed Stop's
   * notice takes (`ChatTurnStopped`), because until this a Stop that WORKED
   * stated no outcome at all.
   *
   * Transient and never persisted: retracted `STOP_CONFIRMED_NOTICE_MS` later,
   * and at once by anything that starts or joins a turn in this chat.
   *
   * ⚠ Absent for every other ending, which is why it is keyed on the daemon's
   * `cancelled` rather than on the press: a turn that finished on its own, a
   * Stop that raced the turn to its end (`cancelled: false`, the daemon's
   * idempotent answer), a Stop the daemon never confirmed (M2's card speaks
   * instead), and a Stop-and-Send, whose replacement turn is the outcome.
   */
  stopConfirmed?: StopConfirmedView;
  /**
   * Whether this session's agent — model provider + extensions — has finished
   * loading on the backend. The transcript paints before this flips (see
   * `loadSession`), so anything that reads AGENT state rather than SESSION
   * state (the tool count, for one) must wait for it or it will read an empty
   * world and cache the emptiness. `false` means "not yet", never "failed";
   * a failed load still ends at `true` with `turnError` set.
   */
  agentReady: boolean;
  /**
   * §6.1b — tool calls the model has begun emitting whose arguments are still
   * generating, keyed by tool-call id. Populated from `ToolCallPending` stream
   * events so the UI can draw a skeleton card the moment a tool's NAME is known
   * — seconds before its arguments finish — and removed the instant the
   * authoritative `ToolRequest` (same id) lands in a `Message`.
   *
   * These are DELIBERATELY held OUT of `messages`. A pending tool call is
   * advisory display state, never a real request: it must never be dispatched,
   * persisted, or fed back to the model, and keeping it off the message array
   * also sidesteps the content-dedup landmine (`pushMessage` dedupes content by
   * JSON equality, so a partial and its completed form would BOTH survive).
   */
  pendingToolCalls: PendingToolCallView[];
  /**
   * Issue #56 Gate B, repair arm: the provider and model that actually served
   * the most recent turn in this chat, when the agent had to fall back to the
   * one the SESSION ROW names because the chat's classification does not admit
   * the globally selected one.
   *
   * ⚠ **Receiving it is not the same as "the user's choice was overridden".**
   * The daemon sends this on EVERY repaired bind, including the ordinary ones
   * (an LRU-rehydrated agent, a legacy row) where it names exactly what the
   * composer is already showing. Compare it against the selection on screen and
   * say nothing when they agree — `privacy/pinnedModel.ts` is where that
   * comparison lives.
   *
   * It deliberately OUTLIVES the turn that reported it. The pin is a property
   * of the chat, not of one turn: clearing it on `Finish` would put the wrong
   * model back on the chip the moment the answer arrived, which is the defect.
   */
  pinnedModel?: PinnedModelView;
}

/**
 * The binding a chat was pinned to by the privacy barrier (issue #56 Gate B).
 * Provider is the registry's own id (`versa_azure`), not a display name.
 */
export interface PinnedModelView {
  provider: string;
  model: string;
}

/** F5 — a Stop the daemon confirmed. See `ChatStreamSnapshot.stopConfirmed`. */
export interface StopConfirmedView {
  /** The exact generation the confirmed cancel named. */
  turnId: string;
}

/** A tool call announced before its arguments finished streaming (§6.1b). */
export interface PendingToolCallView {
  id: string;
  name: string;
  /** Arguments accumulated so far; almost never valid JSON. Display only. */
  partialArgs?: string;
}

/**
 * The daemon's synthesized ending for a turn whose writers produced no terminal
 * frame (`TurnStream::close`, `routes/reply.rs`). It is scope `internal`, so
 * the card reads "Model turn ended unexpectedly" — which is right for a turn
 * that died on its own and WRONG for one the user asked to stop, because the
 * daemon has no idea which of the two it just described.
 */
const STREAM_ENDED_WITHOUT_TERMINAL = 'stream_ended_without_terminal';

/** A turn that ended without a result because the user stopped it. */
export const TURN_STOPPED_BY_USER = 'turn_stopped_by_user';

/** A Stop whose cancel never came back confirmed. */
export const STOP_NOT_CONFIRMED = 'stop_not_confirmed';

/**
 * F5 — how long a confirmed Stop's "Stopped." line stays in the transcript.
 * design.md §4.3's toast duration, so the app's transient confirmations last
 * one length of time rather than two.
 */
export const STOP_CONFIRMED_NOTICE_MS = 5000;

/**
 * The in-chat notice for a Stop the daemon never confirmed (M2).
 *
 * Two facts the user needs and neither of which was on screen before: Biorouter
 * could not stop the turn, and the turn may still be running over there. The
 * second half depends on what the renderer was able to do about it — a turn
 * that had already ended locally leaves a usable chat, one still streaming does
 * not — so the copy says which case this is rather than hedging across both.
 *
 * Not retryable: the failure is the STOP, and a Retry action re-runs the TURN,
 * which is the opposite of what someone who just pressed Stop is asking for.
 */
function stopNotConfirmedError(
  composerRestored: boolean,
  detail: string | null
): ChatTurnErrorData {
  const message = composerRestored
    ? 'Biorouter could not confirm that the backend stopped this turn, so it may still be running there. This chat is usable again — anything the stopped turn is still doing will land in it.'
    : 'Biorouter could not confirm that the backend stopped this turn, so it may still be running there. Press Stop again to retry.';
  return {
    message,
    code: STOP_NOT_CONFIRMED,
    scope: 'internal',
    retryable: false,
    ...(detail ? { technicalDetails: detail } : {}),
  };
}

function clientTurnError(
  error: unknown,
  code: string,
  defaultScope: TurnErrorScope
): ChatTurnErrorData {
  const message = errorMessage(error);
  return {
    message,
    technicalDetails: message,
    code,
    scope: isConnectionError(error) ? 'transport' : defaultScope,
    retryable: true,
  };
}

export interface RunningChatEntry {
  sessionId: string;
  title: string;
  chatState: ChatState;
  startedAt: number;
  completedAt?: number;
}

const EMPTY_TOKEN_STATE: TokenState = {
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
};

/**
 * The app's ONE answer to "is a turn live in this chat?".
 *
 * Exported because the answer is NOT the obvious `!== ChatState.Idle`:
 * `LoadingConversation` is set at the top of every session load (see
 * `ensureLoaded`), so the naive form reports a freshly opened, long-finished
 * chat as running for the whole load. `isRunning()` and therefore the running
 * registry — the set the sidebar and the tab strip's live dot both read — are
 * defined by this function, so any surface that decides "running" for itself
 * from a bare `!== Idle` is silently disagreeing with the rest of the app.
 * Import this instead of re-deriving it. (BR-71: the subagent header's Stop
 * button was that fourth ad-hoc copy, and it offered a kill switch for a turn
 * that did not exist.)
 */
export function isRunningState(chatState: ChatState): boolean {
  return chatState !== ChatState.Idle && chatState !== ChatState.LoadingConversation;
}

/**
 * #22 — delay of the setTimeout fallback armed alongside every scheduled rAF
 * flush (see `scheduleNotify`). ~Two frames at 60 Hz: long enough that a live
 * rAF (~16 ms) always wins the race, so visible windows keep frame-aligned
 * batching; short enough that a window whose rAF is paused (hidden Chromium
 * windows) stalls notifications by at most this long. Exported for the
 * hidden-window regression test.
 */
export const NOTIFY_FALLBACK_MS = 32;

const OBSERVER_INITIALIZATION_REFRESH_INTERVAL_MS = 1000;

class ChatStreamController {
  private snapshot: ChatStreamSnapshot = {
    messages: [],
    chatState: ChatState.Idle,
    tokenState: EMPTY_TOKEN_STATE,
    notifications: [],
    agentReady: false,
    pendingToolCalls: [],
  };
  private listeners = new Set<() => void>();
  private finishListeners = new Set<() => void>();
  private messagesRef: Message[] = [];
  /**
   * Whether `messagesRef` names EVERY row the session store holds — the
   * precondition for sending `expectedMessageIds` (see `onMessageUpdate`).
   *
   * True only immediately after reading the conversation back from the server,
   * which is the one moment we provably hold the whole stored set: both
   * `/agent/resume` and `edit_message`'s own freshness check read
   * `get_session(id, true)`, so they see the same rows, hidden ones included.
   *
   * Any other assignment to `messagesRef` clears it, because a view assembled
   * from the stream is structurally short of the store (#59): one streamed
   * assistant reply becomes two or three stored rows — the rebuilt thinking row
   * plus one `tool_use` row per request — and only the first keeps the id we
   * were shown, while the model-only rows (BR-47 post-edit diagnostics,
   * loop-guard / stall / budget nudges, hook context) are never yielded at all.
   * `conversation_writeback_freshness.rs
   * ::a_reply_split_into_several_stored_rows_publishes_every_one_of_their_ids`
   * asserts that inequality from the server side. #67: submitting a turn clears
   * it too, since `retryTurn` reaches no `messagesRef` assignment at all.
   *
   * FOLLOW-UP: #59 publishes those ids on `MessagesPersisted`, but durability is
   * not turn completion: providers persist intermediate assistant/tool rows too.
   * Folding the ids into this completeness claim would let a watched turn KEEP
   * it true instead of dropping the guard for the rest of the session, and is
   * what makes `expectedMessageIds` mandatory server-side.
   */
  private viewNamesEveryStoredRow = false;
  /**
   * BR-71 — true while this controller is an OBSERVER of a session another
   * agent drives, rather than the driver of its own `/reply` turn. Set by
   * `observeSession`, cleared by `stopObserving` (the tab closed, or the user
   * took it over via `submitPreparedMessage`) and, unconditionally, by the
   * observer loop's own `finally` — anything that can end the loop without
   * going through `stopObserving`, `stopStreaming` above all, would otherwise
   * leave this flag claiming an observer that no longer exists.
   *
   * PERMANENT CONSEQUENCE FOR `expectedMessageIds` — state it here rather than
   * let the next reader discover it. `viewNamesEveryStoredRow` (above) is set
   * in exactly the two places that read a conversation back from the server,
   * and cleared by every streamed event. An observer-fed tab is a pure event
   * consumer: it never performs that read, so the flag is ALWAYS false for it
   * and `onMessageUpdate` therefore PERMANENTLY omits the `expectedMessageIds`
   * guard on in-place edits, where an ordinary tab sends it after each read.
   *
   * That is safe — the guard is omitted, never falsified, and the server-side
   * cut still runs under the turn lock, still bounded to the rows the handler
   * itself read — but it is a real capability difference a user can hit.
   *
   * Do NOT "fix" it by setting `viewNamesEveryStoredRow` here: it would be a
   * lie, because an observer tab genuinely does not know it holds every stored
   * row. The thing that would let it is promoting #59's `MessagesPersisted`
   * accounting into this completeness claim, which is the FOLLOW-UP recorded
   * on `viewNamesEveryStoredRow` and is deliberately not done yet.
   */
  private observing = false;
  /**
   * Which observer loop `observing` belongs to. Bumped on every attach and on
   * every detach, so a loop that is still unwinding — parked in a drain or a
   * backoff when it was torn down — can tell that the flag it is about to clear
   * is no longer its own.
   */
  private observerGeneration = 0;
  /**
   * Turn identity within the current observer connection. Unlike
   * `observerGeneration` (the ownership loop), this advances on reconnects and
   * terminal frames so a late `/agent/resume` response cannot name an older turn.
   */
  private observedTurnGeneration = 0;
  private observedTurnResolvedGeneration = -1;
  private observedTurnLookup: { generation: number; promise: Promise<void> } | null = null;
  /** Recently retired observer turn ids; replayed frames may never revive them. */
  private retiredObservedTurnIds = new Set<string>();
  private abortController: AbortController | null = null;
  private activeStreamId = 0;
  /** The server-side cancellation barrier. While present, no successor may submit. */
  private stopInFlight: Promise<boolean> | null = null;
  /** Whether the request currently on the wire already acquired a continuation lease. */
  private stopInFlightContinuationPending = false;
  /**
   * A Stop whose server barrier has not resolved yet, so a terminal SSE frame
   * may not reopen submission without proof that the exact stopped turn
   * released its slot.
   *
   * #166 — this is a bridge across the microtask race between a terminal frame
   * and the cancel response, NOT a durable claim that the daemon is still
   * running the turn. It is cleared as soon as the cancel request resolves,
   * whichever way it resolved: see `settleStoppedTurn`, whose `finally` is the
   * only reason a failed cancel can no longer pin the composer at "working"
   * for the rest of the chat's life.
   */
  private stopPending = false;
  /**
   * The exact turn generation the current (or last unsettled) Stop named.
   *
   * It deliberately OUTLIVES `stopPending` on a failure, so a retry cancels the
   * generation the user actually stopped instead of guessing at whatever is
   * running now. `stopStreaming` prefers a live `activeTurnId` over it, and a
   * new turn supersedes it outright, so it can never name a stale generation
   * while a real one exists.
   */
  private stopExpectedTurnId: string | null = null;
  /** Stop-and-Send intent, retained when the exact-generation cancel is retried. */
  private stopContinuationPending = false;
  /** Exact-generation Stop-and-Send admission owned by this renderer. */
  private continuationLease: string | null = null;
  /** Successor idempotency key once the lease has been offered to `/reply`. */
  private continuationLeaseTurnId: string | null = null;
  /** Tokens awaiting an authoritative abandonment acknowledgement. */
  private pendingLeaseAbandons = new Set<string>();
  private leaseAbandonmentInFlight = new Map<string, Promise<boolean>>();
  private continuationRecoveryInFlight: Promise<void> | null = null;
  /** A closed tab must abandon a lease that arrives after its close raced the cancel response. */
  private ownershipReleased = false;
  /**
   * The last exact generation whose cancellation barrier settled. A Stop-and-Send
   * arriving just after an ordinary Stop can still mark that retained generation
   * before it submits the replacement.
   */
  private lastSettledStopTurnId: string | null = null;
  /**
   * The generation whose terminal frame landed while the Stop gate was still
   * holding, so `finishCurrentStream` handed the Idle transition to the cancel
   * response instead of making it itself.
   *
   * M2. #166 closed the ordering where the terminal frame arrives AFTER the
   * cancel resolves — the gate is already down, so the frame lands the turn.
   * The reverse ordering is what a wedged daemon produces (`/agent/cancel`
   * parks for its 30 s settlement bound while the turn's stream is already
   * over) and it strands the composer: the frame deferred the transition, the
   * cancel then failed and returned without making it, and NOTHING else was
   * ever going to. The user was left with a Stop button, no Send button and a
   * spinner over a turn that had already ended.
   *
   * Held rather than re-derived because it is the one thing that distinguishes
   * "the stop failed and the turn is over" (restore the composer) from "the
   * stop failed and the turn is genuinely still streaming" (do not — the daemon
   * still owns the session lock, and Stop is the right control to offer).
   */
  private stopDeferredFinishTurnId: string | null = null;
  /**
   * Why the last exact-generation cancel did not settle, for the in-chat notice.
   *
   * `null` covers the two cases that must NOT raise one: a cancel that settled,
   * and a typed turn mismatch — the mismatch arm resolves the whole state
   * itself (it adopts the successor and picks the matching `chatState`), so a
   * notice written over the top of it would be both wrong and destructive.
   */
  private lastStopFailure: string | null = null;
  /**
   * F5 — whether the last exact-generation cancel that SETTLED found the turn
   * running and tripped it (`cancelled: true`), rather than finding it already
   * over (`cancelled: false`, the daemon's idempotent answer to a Stop that
   * raced the turn's own ending). Only the first is a stop the user caused, so
   * only it earns `stopConfirmed`. Reset with `lastStopFailure` at the top of
   * every cancel request.
   */
  private lastStopCancelled = false;
  /**
   * The turn this controller is currently rendering — the id it POSTed, or the
   * id it attached to. Held so a re-attach can re-POST the SAME turn (rather
   * than starting a second one) and so the sequence gate below knows which
   * turn its numbering belongs to.
   */
  private activeTurnId: string | null = null;
  /**
   * Highest per-turn `seq` already applied to the snapshot, for `seqTurnId`.
   * `-1` means "nothing from this turn has been rendered yet", which is the
   * only value at which frame 0 is accepted.
   */
  private lastAppliedSeq = -1;
  /**
   * Which turn `lastAppliedSeq` counts within. Kept separate from
   * `activeTurnId` because it is set by what ARRIVES, not by what we asked
   * for: a frame naming a turn we have not seen resets the gate, which is what
   * makes a new turn's `seq: 0` an accepted frame instead of a duplicate of the
   * previous turn's.
   */
  private seqTurnId: string | null = null;
  private lastInteractionTime = Date.now();
  private loadPromise: Promise<void> | null = null;
  /**
   * This chat has already told the session-list cache it exists.
   *
   * Latched, never cleared: a chat that a fetched list still does not hold
   * after being announced is one the list endpoint filters out on purpose (a
   * `sub_agent` row under `include_subagents=false`), and re-announcing it once
   * per turn forever would buy a full list fetch to be told the same thing.
   */
  private announcedListMembership = false;
  /**
   * A classification this turn reported while the chat still had no row.
   *
   * Applied by {@link updateSnapshot} to the first snapshot that carries one,
   * and cleared there — a one-shot, so it can never re-assert itself over a
   * later row. See the note in {@link applyTurnBinding} for the race.
   */
  private pendingTurnClassification: {
    tier: SessionClassification;
    reason: string | null;
  } | null = null;
  /**
   * R3-01 — synchronous re-entrancy latch for the submit prep window. The
   * `abortController` guard in `canSubmitMessage` only trips once a turn has
   * been launched, but `handleSubmit` awaits `loadSession` + `createUserMessage`
   * *before* assigning `abortController`. A rapid double-click hits that gap:
   * both calls pass the guard and both append the user turn (phantom duplicate
   * bubble). This latch is set synchronously at the top of `handleSubmit`, so
   * the second click bails before its first await.
   */
  private submitInFlight = false;
  /**
   * The in-flight (or settled) model+extension load. Unlike `loadPromise` this
   * is never nulled on completion: it is the memo that makes `ensureAgentLoaded`
   * idempotent across the several call sites that can reach it (cold load,
   * cached load, controller reuse, a submit that got there first).
   */
  private agentLoadPromise: Promise<void> | null = null;
  private lastSubmittedTitle: string | null = null;
  /**
   * A `/reply` id whose response was lost after acceptance became ambiguous.
   * Retry must attach with this same key until a terminal frame proves the turn's
   * outcome; minting another key could run and bill the same prompt twice.
   */
  private ambiguousRetryTurnId: string | null = null;
  /** Manual Retry is itself single-flight, including the attach POST window. */
  private retryInFlight: Promise<void> | null = null;
  /** A delegated child may have a durable row before its exact runtime exists. */
  private childInitializing = false;
  private ownershipGeneration = 0;
  private observerInitializationRefresh: {
    observerGeneration: number;
    operation: Promise<void>;
  } | null = null;
  private observerInitializationNextRefreshAt = 0;
  /** Optimistic input accepted into an initializing child's delegated first turn. */
  private pendingInitializingChildMessage: Message | null = null;

  constructor(
    readonly sessionId: string,
    private readonly onActivityChange: (controller: ChatStreamController) => void
  ) {
    subscribeSessionNameChanges((change) => {
      if (change.sessionId !== sessionId) return;
      this.updateSnapshot((prev) => {
        if (!prev.session) return prev;
        if (
          prev.session.name === change.name &&
          prev.session.user_set_name === change.userSetName
        ) {
          return prev;
        }
        return {
          ...prev,
          session: { ...prev.session, name: change.name, user_set_name: change.userSetName },
        };
      });
    });

    // Round 3 / N1 — a per-chat model switch rewrites this session's row, and
    // this store holds the only copy of it the composer reads. Same shape as the
    // name subscription above, and for the same reason: the write has already
    // landed on the daemon, so re-reading it would be a round trip to learn what
    // the announcement already carries.
    subscribeSessionBindingChanges((change) => {
      if (change.sessionId !== sessionId) return;
      // ⚠ The PIN moves too, and it has to. `chatBinding` prefers the
      // turn-reported pin over the row, so a switch that patched only the row
      // would be overruled by the previous turn's pin and the chip would go on
      // naming the model the user just switched away from — the exact
      // regression #192 narrowed its rule to avoid. It did not bite while the
      // frame was rare (only a repaired bind produced one); it bites on every
      // switch-after-a-turn now that the frame arrives on every turn.
      //
      // Replacing rather than clearing is the honest write: an accepted bind is
      // as authoritative a statement of "what this chat runs on" as a turn's
      // own report, and it is the LATER one.
      this.setPinnedModel({ provider: change.provider, model: change.model });
      this.updateSnapshot((prev) => {
        if (!prev.session) return prev;
        if (
          prev.session.provider_name === change.provider &&
          prev.session.model_config?.model_name === change.model
        ) {
          return prev;
        }
        return {
          ...prev,
          session: {
            ...prev.session,
            provider_name: change.provider,
            // Spread, so the row keeps the fields the bind did not name
            // (`toolshim`, `reasoning_effort`, …). `context_limit` is written
            // even when the announcement carries none: a row naming one model
            // beside another model's window is the lie this patch exists to
            // avoid, and the post-turn refresh restores the daemon's own value.
            model_config: {
              ...prev.session.model_config,
              model_name: change.model,
              context_limit: change.contextLimit ?? null,
              toolshim: prev.session.model_config?.toolshim ?? false,
            },
          },
        };
      });
    });
  }

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  subscribeFinish(listener: () => void): () => void {
    this.finishListeners.add(listener);
    return () => {
      this.finishListeners.delete(listener);
    };
  }

  getSnapshot = (): ChatStreamSnapshot => this.snapshot;

  isRunning(): boolean {
    return isRunningState(this.snapshot.chatState);
  }

  getRunningEntry(): RunningChatEntry {
    return {
      sessionId: this.sessionId,
      title: this.snapshot.session?.name || this.lastSubmittedTitle || 'New chat',
      chatState: this.snapshot.chatState,
      startedAt: this.lastInteractionTime,
    };
  }

  setChatState = (chatState: ChatState): void => {
    this.updateSnapshot((prev) => (prev.chatState === chatState ? prev : { ...prev, chatState }));
  };

  private notify(): void {
    for (const listener of this.listeners) listener();
    this.onActivityChange(this);
  }

  // #22 — listener-notification batching. Token streaming delivers dozens of
  // events per second, and notifying every subscriber synchronously per event
  // re-rendered the whole chat tree (BaseChat → ChatInput → transcript) at
  // event rate, which is what made typing lag while a response streamed.
  // Snapshot writes stay SYNCHRONOUS (getSnapshot is always current); only the
  // "tell React about it" step is deferred to at most once per animation frame.
  private notifyScheduled = false;
  private notifyRafHandle: number | null = null;
  private notifyTimeoutHandle: ReturnType<typeof setTimeout> | null = null;

  private scheduleNotify(): void {
    if (this.notifyScheduled) return;
    this.notifyScheduled = true;
    // rAF is paused in hidden Chromium windows — and visibility can change
    // BETWEEN scheduling and the callback running, so sampling `document.hidden`
    // here is not enough: a flush armed via rAF while visible would park
    // indefinitely once the window hid, and with `notifyScheduled` latched no
    // later event could re-arm anything — a WaitingForUserInput transition
    // would sit invisible until the window was refocused. So the cancellable
    // setTimeout fallback is armed on EVERY schedule, and rAF — when the
    // environment has one at all (jsdom and workers don't) — races alongside
    // it: whichever fires first flushes and cancels the other (`flushNotify`
    // clears both handles; its `notifyScheduled` guard makes a straggler a
    // no-op). A live rAF (~16 ms) beats the ~two-frame fallback, so visible
    // windows keep frame-aligned batching; a paused one merely loses the race,
    // capping the stall at NOTIFY_FALLBACK_MS. The timeout is armed first so
    // a synchronously-firing rAF (test stubs) finds the handle to cancel.
    this.notifyTimeoutHandle = setTimeout(() => this.flushNotify(), NOTIFY_FALLBACK_MS);
    if (typeof requestAnimationFrame === 'function') {
      this.notifyRafHandle = requestAnimationFrame(() => this.flushNotify());
    }
  }

  /**
   * Run a pending scheduled notification NOW. Called at turn boundaries
   * (submit start, stop, finish) so the transitions users act on — their
   * message appearing, Stop feedback, the turn ending — are never a frame
   * late. Also the shared terminus of the rAF/timeout race in
   * `scheduleNotify`: the winner flushes and cancels the loser here. No-op
   * when nothing is scheduled — which is also what makes a double fire
   * (both race arms landing) deliver exactly one notification.
   */
  /**
   * Replay batching (contract §R2). While this is > 0 the snapshot keeps being
   * written SYNCHRONOUSLY — `getSnapshot()` is always current, and everything
   * inside the controller that reads it keeps working — but React is not told.
   * On release, one notification carries the whole backlog.
   *
   * This is a suspension of the NOTIFICATION layer, not a buffer of frames, and
   * that distinction is the reason it is safe. The frames still run through the
   * ordinary pipeline in the ordinary order — no reordering, no per-variant
   * skipping, no `Promise.all` — so the producer-side ordering invariants that
   * `observeSession` documents survive untouched. The only thing that changes
   * is how many times React re-renders while they land: once.
   *
   * Why a hold is needed at all when `scheduleNotify` already batches per
   * animation frame: rAF coalescing only merges frames that arrive within one
   * frame. A backlog delivered over several network reads is several macrotasks,
   * so it would paint in several steps — a visible re-typing of a message the
   * user may have already read. The contract calls that out as worse than the
   * bug being fixed.
   */
  private notifySuspendDepth = 0;
  /** A snapshot write happened while suspended, so release must notify. */
  private suspendedDirty = false;
  private replayHoldTimer: ReturnType<typeof setTimeout> | null = null;

  private suspendNotifications(): void {
    this.notifySuspendDepth += 1;
  }

  private resumeNotifications(): void {
    if (this.notifySuspendDepth === 0) return;
    this.notifySuspendDepth -= 1;
    if (this.notifySuspendDepth > 0) return;
    if (!this.suspendedDirty) return;
    this.suspendedDirty = false;
    if (this.snapshot.session) {
      cacheSet(this.sessionId, {
        session: this.snapshot.session,
        messages: this.snapshot.messages,
      });
    }
    // Deliberately `notify()` and not `scheduleNotify()`: the backlog is
    // already complete, and deferring it another frame would be pure latency.
    this.notify();
  }

  /**
   * Enter (or stay in) the replay hold. Idempotent — a backlog is many frames
   * and only the first opens the hold, so the depth can never run away.
   */
  private beginReplayHold(): void {
    if (this.replayHoldTimer !== null) return;
    this.suspendNotifications();
    this.replayHoldTimer = setTimeout(() => {
      // Safety valve, not the normal exit: see REPLAY_MAX_HOLD_MS.
      this.replayHoldTimer = null;
      this.resumeNotifications();
    }, REPLAY_MAX_HOLD_MS);
  }

  /**
   * Leave the replay hold and commit the backlog as one render. The normal exit
   * — called on the first live frame, on a terminal frame, and unconditionally
   * when the stream loop unwinds, so no path can leave the transcript frozen.
   */
  private endReplayHold(): void {
    if (this.replayHoldTimer === null) return;
    clearTimeout(this.replayHoldTimer);
    this.replayHoldTimer = null;
    this.resumeNotifications();
  }

  /**
   * The idempotence gate (contract §1/§R2). Returns false when this frame has
   * already been applied and must be dropped.
   *
   * A frame is a duplicate when it names a `seq` at or below the highest one
   * already applied FOR THE SAME TURN. The turn scoping is what makes it safe:
   * `seq` restarts at 0 every turn, so an unscoped gate would swallow the first
   * frames of the next turn. When the producer stamps no `turn_id`, the gate is
   * limited to the turn this controller believes it is on — set at submit and
   * at attach, cleared at every turn boundary — which is exactly true for a
   * `/reply` stream this controller drives, and is why an observer feed (no
   * `seq` at all) simply never reaches the gate.
   */
  private rememberRetiredObservedTurn(turnId: string | null): void {
    if (!turnId) return;
    this.retiredObservedTurnIds.add(turnId);
    if (this.retiredObservedTurnIds.size > 32) {
      const oldest = this.retiredObservedTurnIds.values().next().value;
      if (oldest) this.retiredObservedTurnIds.delete(oldest);
    }
  }

  private adoptObservedFrameTurn(event: MessageEvent): void {
    if (!this.observing || isReplayFrame(event)) return;
    // A terminal identifies the turn it retires; it may never advance the
    // active pointer before that identity is checked by the terminal handler.
    if (isTerminalEvent(event)) return;
    const turnId = frameTurnId(event);
    if (!turnId || this.retiredObservedTurnIds.has(turnId)) return;

    // A non-replay frame on the CURRENT observer socket is ordered after every
    // frame already consumed from that socket. It may therefore advance the
    // pointer to a successor. Replayed frames and retired ids above may not.
    this.activeTurnId = turnId;
    this.observedTurnResolvedGeneration = this.observedTurnGeneration;
  }

  private refreshObservedActiveTurn(): Promise<void> | null {
    if (!this.observing) return null;
    const generation = this.observedTurnGeneration;
    if (this.observedTurnResolvedGeneration === generation) return null;
    if (this.observedTurnLookup?.generation === generation) {
      return this.observedTurnLookup.promise;
    }
    const issuedAgainst = this.activeTurnId;

    const operation = (async () => {
      try {
        const response = await resumeAgent({
          body: resumeRequestBody(this.sessionId, false),
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        if (!this.observing || this.observedTurnGeneration !== generation) return;
        this.notePendingContinuation(response.data);
        this.noteChildInitialization(isInitializingResume(response.data));
        // A frame or user takeover that changed the pointer while the request
        // was in flight is newer local evidence; never overwrite it.
        if (this.activeTurnId !== issuedAgainst) return;

        this.observedTurnResolvedGeneration = generation;
        const activeTurnId = response.data?.active_turn?.turn_id ?? null;
        this.activeTurnId =
          activeTurnId && !this.retiredObservedTurnIds.has(activeTurnId) ? activeTurnId : null;
        this.markObservedTurnRunning();
      } catch {
        // Keep this generation unresolved so a later activity frame can retry.
      }
    })();
    this.observedTurnLookup = { generation, promise: operation };
    void operation.finally(() => {
      if (this.observedTurnLookup?.promise === operation) this.observedTurnLookup = null;
    });
    return operation;
  }

  private noteObservedTurnActivity(event: MessageEvent): void {
    if (!this.observing) return;
    if (this.childInitializing && event.type === 'Ping') {
      this.startObserverInitializationRefresh();
    }
    if (event.type === 'Message' || event.type === 'ToolCallPending') {
      void this.refreshObservedActiveTurn();
    }
  }

  private applySequenceGate(event: MessageEvent): boolean {
    this.adoptObservedFrameTurn(event);
    const seq = frameSeq(event);
    if (seq === undefined) return true;

    const turnId = frameTurnId(event);
    if (turnId && turnId !== this.seqTurnId) {
      // A different turn's numbering: start it from scratch rather than
      // measuring it against the previous turn's high-water mark.
      this.seqTurnId = turnId;
      this.lastAppliedSeq = -1;
    } else if (!turnId && this.seqTurnId === null) {
      this.seqTurnId = this.activeTurnId;
    }

    if (seq <= this.lastAppliedSeq) return false;
    this.lastAppliedSeq = seq;
    return true;
  }

  /**
   * The turn this controller was rendering is over (or has been given up on):
   * stop treating it as something a dropped socket may be rejoined to.
   *
   * It deliberately does NOT forget `seqTurnId`/`lastAppliedSeq`. Those record
   * WHAT HAS ALREADY BEEN PAINTED of a specific turn, and that remains true
   * whatever happened to the connection — including the case this exists for,
   * where the client gave up on a dropped stream and something later attaches
   * to the same turn again. Clearing the high-water mark there would replay a
   * half-rendered turn from its start, on top of the half already on screen,
   * which is the duplicated-paragraph bug arriving from the one direction the
   * sequence gate is supposed to cover.
   *
   * Nothing leaks into the NEXT turn either: a new turn resets the mark
   * explicitly (`submitPreparedMessage`, `attachToTurn`), and a frame naming an
   * unfamiliar turn resets it on arrival (`applySequenceGate`).
   */
  private retireActiveTurn(): void {
    this.activeTurnId = null;
    this.reattachesThisTurn = 0;
  }

  /**
   * Per-message-id text reconstructed from the REPLAY frames of the current
   * attach. Reset whenever an attach begins; see `dedupeReplayedMessage`.
   */
  private replayReconstruction = new Map<string, string>();

  /**
   * The second half of idempotent replay: drop what the SESSION STORE has
   * already given us, not merely what we have already rendered.
   *
   * The sequence gate covers "frames I applied on an earlier socket". It cannot
   * cover the other overlap, and nothing did: `agent.rs` persists each
   * agent-loop ITERATION's rows as it goes, so a window reloading during round 2
   * of any tool-using turn loads a transcript that already contains round 1 —
   * and then attaches with `from_seq: 0`, because its high-water mark is -1 (it
   * has rendered nothing), and is replayed round 1's deltas on top of the copy
   * it just loaded. `pushMessage` cannot rescue it: the frame is a FRAGMENT of a
   * message whose text is already complete, so neither its `startsWith` nor its
   * `endsWith` guard matches and the fragment is appended — `"I loaded the
   * data.I loaded the data."`.
   *
   * Why the CLIENT and not the server. Storage rows carry no `seq`, so the
   * client cannot derive a watermark from a transcript it read back; but it does
   * know, exactly, what it is holding. The server knows the converse — which
   * frames it has persisted — and could skip them, except that its idea of "what
   * this client holds" is a guess about when that client last read the store: too
   * eager and it under-sends, which is R1 failing with no way to detect it. The
   * contract already makes the client responsible for applying frames
   * idempotently (§1/§R2). This is that responsibility, extended from "frames I
   * rendered" to "rows I was given", which is the only place both facts are
   * known at once. Over-sending stays harmless, which is the direction a wire
   * protocol should fail in.
   *
   * Returns the message to apply, or `null` when the transcript already holds
   * it. Only same-id single-text deltas are eligible — anything carrying
   * structure passes straight through.
   */
  private dedupeReplayedMessage(
    incoming: Message,
    currentMessages: Message[],
    replay: boolean
  ): Message | null {
    if (!replay || !incoming.id || incoming.content.length !== 1) return incoming;
    const fragment = incoming.content[0];
    if (fragment.type !== 'text') return incoming;

    const reconstructed = (this.replayReconstruction.get(incoming.id) ?? '') + fragment.text;
    this.replayReconstruction.set(incoming.id, reconstructed);

    const held = [...currentMessages].reverse().find((m) => m.id === incoming.id);
    const heldText =
      held && held.content.length === 1 && held.content[0].type === 'text'
        ? held.content[0].text
        : undefined;
    if (heldText === undefined) return incoming;

    // Everything the replay has rebuilt for this message so far is already in
    // the transcript.
    if (heldText.startsWith(reconstructed)) return null;
    // The replay has caught up with the stored row and gone past it: apply only
    // the part that is genuinely new.
    if (reconstructed.startsWith(heldText)) {
      const residual = reconstructed.slice(heldText.length);
      if (!residual) return null;
      return { ...incoming, content: [{ ...fragment, text: residual }] };
    }
    // Divergence — not the overlap this exists for. Apply it unchanged and let
    // the ordinary merge decide.
    return incoming;
  }

  private flushNotify(): void {
    // Both arms of the rAF/timeout race are stood down first, on EVERY path.
    // The suspended path below used to return before this, leaving a fired
    // timer's `notifyScheduled` latched — see there.
    if (this.notifyTimeoutHandle !== null) {
      clearTimeout(this.notifyTimeoutHandle);
      this.notifyTimeoutHandle = null;
    }
    if (this.notifyRafHandle !== null) {
      if (typeof cancelAnimationFrame === 'function') {
        cancelAnimationFrame(this.notifyRafHandle);
      }
      this.notifyRafHandle = null;
    }
    // A turn boundary reached mid-backlog must not force a partial paint: the
    // release below delivers it, in the same commit as the rest of the replay.
    //
    // ⚠ `notifyScheduled` is CLEARED here, not left set. It used to return with
    // the latch still true, which is a permanent freeze rather than a deferral:
    // a hold that outlives both the rAF and the 32 ms fallback consumes both
    // arms of the race — and `attachToTurn` arms one on its `chatState:
    // Streaming` write immediately before the first replay frame opens the
    // hold, so it always does — after which every later `scheduleNotify()`
    // early-returns on the latch and React is never told about anything again.
    // The snapshot kept advancing and the screen did not, for the whole live
    // tail, until `finishCurrentStream` flushed and the turn landed at once.
    // Clearing it is safe precisely because nothing is lost: `suspendedDirty`
    // makes `resumeNotifications` deliver the pending notification, and while
    // suspended `updateSnapshot` sets the same flag instead of scheduling.
    if (this.notifySuspendDepth > 0) {
      this.suspendedDirty = true;
      this.notifyScheduled = false;
      return;
    }
    if (!this.notifyScheduled) return;
    this.notifyScheduled = false;
    this.notify();
  }

  private updateSnapshot(updater: (prev: ChatStreamSnapshot) => ChatStreamSnapshot): void {
    const next = this.adoptPendingClassification(updater(this.snapshot));
    // Several updaters return `prev` to mean "nothing changed" — that must not
    // wake every subscriber (#22).
    if (next === this.snapshot) return;
    this.snapshot = next;
    // Replay hold: the write has landed (getSnapshot is current), React just
    // isn't told yet. The LRU write is deferred with it — it would otherwise
    // re-serialise the whole transcript once per replayed frame.
    if (this.notifySuspendDepth > 0) {
      this.suspendedDirty = true;
      return;
    }
    if (this.snapshot.session) {
      cacheSet(this.sessionId, {
        session: this.snapshot.session,
        messages: this.snapshot.messages,
      });
    }
    this.scheduleNotify();
  }

  /**
   * Put a turn's classification onto the first row that appears, if the frame
   * that carried it arrived before there was one (finding M8).
   *
   * ⚠ **Here, at the one write every snapshot goes through**, rather than at
   * each of the several updaters that can set a row — the cached-transcript
   * path, the two-phase resume, a diverge. A fix wired to one of them is a fix
   * for one road into the same defect.
   *
   * ⚠ **One-shot.** The stash is cleared the moment a row is seen, whether or
   * not it needed changing, so a turn's statement can never re-assert itself
   * over a row loaded later — after a declassification, say. Everything after
   * that comes from the row itself: the next frame, or the post-turn re-read.
   */
  private adoptPendingClassification(candidate: ChatStreamSnapshot): ChatStreamSnapshot {
    const pending = this.pendingTurnClassification;
    if (!pending || !candidate.session) return candidate;
    this.pendingTurnClassification = null;
    if (
      candidate.session.privacy_tier === pending.tier &&
      (candidate.session.privacy_reason ?? null) === pending.reason
    ) {
      return candidate;
    }
    return {
      ...candidate,
      session: {
        ...candidate.session,
        privacy_tier: pending.tier,
        privacy_reason: pending.reason,
      },
    };
  }

  // `receivedAt` is stamped ONLY by the live stream path. The other callers
  // (session load, diverge, edit) deliberately omit it: replaying a saved
  // transcript must never look like a live event, or a historical session
  // would inherit a running clock.
  private updateMessages = (messages: Message[], receivedAt?: number): void => {
    this.messagesRef = messages;
    // Default-deny: only the two callers that just read the conversation back
    // from the store re-assert completeness, immediately after this returns.
    this.viewNamesEveryStoredRow = false;
    this.updateSnapshot((prev) => {
      // BR-61: the echo of our own steer is what retires the optimistic chip.
      const steerLanded = steerWasEchoed(prev.pendingSteer, messages, this.steerAfterCount);
      return {
        ...prev,
        messages,
        lastMessageAt: receivedAt ?? prev.lastMessageAt,
        pendingSteer: steerLanded ? undefined : prev.pendingSteer,
      };
    });
  };

  /** Transcript length when the in-flight steer was issued. See steerWasEchoed. */
  private steerAfterCount = 0;

  /**
   * #22 — apply one streamed `Message` event as a SINGLE snapshot swap.
   *
   * The live-stream Message case used to run three separate mutations
   * (setChatState + updateTokenState + updateMessages), i.e. three snapshot
   * swaps and three notifications per streamed token event. This folds the
   * chat-state derivation, token state, transcript, `lastMessageAt` stamp,
   * steer retirement, and landed-tool-skeleton removal into one updater, so a
   * token event costs exactly one snapshot swap.
   */
  /// `keepIdle` holds the chat's running state where it is: the message is a
  /// row appended to a conversation with no turn in flight (an injected note),
  /// so nothing is generating and nothing will ever arrive to retire a running
  /// state. Only the observer path passes it; see the call site.
  private applyMessageEvent = (
    msg: Message,
    messages: Message[],
    tokenState: TokenState,
    receivedAt: number,
    keepIdle = false
  ): void => {
    this.messagesRef = messages;
    // A streamed message is not the row (or rows) it was stored as; see
    // `viewNamesEveryStoredRow`.
    this.viewNamesEveryStoredRow = false;

    // ⚠ `toolConfirmationRequest` is a DEAD variant: nothing in `crates/`
    // constructs it, so this predicate never fired and the one place in the
    // renderer that sets `WaitingForUserInput` never saw an approval card. A
    // real card arrives as `actionRequired` with `actionType: 'toolConfirmation'`
    // — which is exactly what the shared helper tests, so use it rather than
    // spelling the shape a fourth time.
    const hasToolConfirmation = getToolConfirmationContent(msg) !== undefined;
    const hasElicitation = getElicitationContent(msg) !== undefined;
    // Issue #117. A credential card parks the turn the same way, and the chat
    // must say so — the install is genuinely waiting on the person.
    const hasSecretRequest = getSecretRequestContent(msg) !== undefined;
    const derivedChatState =
      hasToolConfirmation || hasElicitation || hasSecretRequest
        ? ChatState.WaitingForUserInput
        : getCompactingMessage(msg)
          ? ChatState.Compacting
          : getThinkingMessage(msg)
            ? ChatState.Thinking
            : ChatState.Streaming;
    // A confirmation card still parks the chat even when nothing is running —
    // it is genuinely waiting on the person — so `keepIdle` yields to it rather
    // than overriding it.
    const chatState =
      keepIdle && derivedChatState !== ChatState.WaitingForUserInput ? null : derivedChatState;

    // The authoritative request(s) landed: drop any matching pending skeletons
    // so the real tool card replaces the placeholder with no flicker or ghost.
    const landedIds = new Set(
      msg.content
        .filter((c) => c.type === 'toolRequest' || c.type === 'frontendToolRequest')
        .map((c) => (c as { id?: string }).id)
        .filter((id): id is string => typeof id === 'string')
    );

    this.updateSnapshot((prev) => {
      // BR-61: the echo of our own steer is what retires the optimistic chip.
      const steerLanded = steerWasEchoed(prev.pendingSteer, messages, this.steerAfterCount);
      let pendingToolCalls = prev.pendingToolCalls;
      if (landedIds.size > 0) {
        const remaining = pendingToolCalls.filter((p) => !landedIds.has(p.id));
        if (remaining.length !== pendingToolCalls.length) pendingToolCalls = remaining;
      }
      return {
        ...prev,
        messages,
        // `null` is `keepIdle`: leave the running state exactly where it was.
        chatState: chatState ?? prev.chatState,
        tokenState,
        lastMessageAt: receivedAt,
        pendingSteer: steerLanded ? undefined : prev.pendingSteer,
        pendingToolCalls,
      };
    });
  };

  private clearPendingSteer = (): void => {
    if (!this.snapshot.pendingSteer) return;
    this.updateSnapshot((prev) => ({ ...prev, pendingSteer: undefined }));
  };

  private updateTokenState = (tokenState: TokenState): void => {
    this.updateSnapshot((prev) => ({ ...prev, tokenState }));
  };

  private updateNotifications = (notification: NotificationEvent): void => {
    this.updateSnapshot((prev) => ({
      ...prev,
      notifications: [...prev.notifications, notification],
    }));
  };

  /** Upsert a pending tool-call skeleton by id (§6.1b). */
  private upsertPendingToolCall = (pending: PendingToolCallView): void => {
    this.updateSnapshot((prev) => {
      const idx = prev.pendingToolCalls.findIndex((p) => p.id === pending.id);
      if (idx === -1) {
        return { ...prev, pendingToolCalls: [...prev.pendingToolCalls, pending] };
      }
      // Merge in a later (longer) partial-args preview without reordering.
      const next = prev.pendingToolCalls.slice();
      next[idx] = { ...next[idx], ...pending };
      return { ...prev, pendingToolCalls: next };
    });
  };

  /**
   * Ordering token for {@link refreshSessionBinding}, and the whole of the
   * renderer's half of M4.
   *
   * `refreshSessionBinding` is `async`, has two callers that can overlap (the
   * end of a turn, and the `/sessions/changes` nudge), and finishes by adopting
   * the row it read OVER the turn-reported pin. Without a token there is nothing
   * to stop the answer to an older request from landing last and overwriting a
   * newer fact — the A → B → A the composer was measured doing across one Send.
   *
   * It is bumped by exactly two things, and both are "something newer than a row
   * read already in flight is now known":
   *
   * - a PIN that actually moves ({@link setPinnedModel}), which is a turn saying
   *   what it is running on or a bind the daemon has just accepted;
   * - the START of a refresh, so that of two overlapping reads only the later
   *   one may apply.
   *
   * The rule that falls out is the one the store needs: **a row replaces the pin
   * only if it was read AFTER that pin was reported.** A read that began earlier
   * is not a disagreement, it is an older photograph.
   *
   * ⚠ An unchanged pin does NOT bump it. "The daemon re-reported the same
   * binding" supersedes nothing, and dropping an in-flight read there would
   * leave the row stale until the next nudge — a liveness cost paid for no
   * ordering gain.
   */
  private bindingGeneration = 0;

  /**
   * Record the binding the privacy barrier pinned this chat to.
   *
   * Idempotent by value: the frame arrives on every repaired turn, and a fresh
   * object each time would re-render the composer — including its model chip
   * and context gauge — once per turn for no change at all.
   *
   * ⚠ The value comparison also decides whether {@link bindingGeneration}
   * moves, so it is read off `this.snapshot` here rather than inside the
   * updater. An updater is a pure function of `prev` and must stay one.
   */
  private setPinnedModel = (pinned: PinnedModelView): void => {
    const current = this.snapshot.pinnedModel;
    if (current?.provider === pinned.provider && current?.model === pinned.model) return;
    this.bindingGeneration += 1;
    this.updateSnapshot((prev) => ({ ...prev, pinnedModel: pinned }));
  };

  /**
   * What the daemon says this turn is running on, applied at turn START.
   *
   * `PrivacyProviderPinned` now arrives on every turn and carries four fields.
   * Two of them are the binding, which becomes {@link ChatSnapshot.pinnedModel}
   * exactly as before. The other two are the chat's classification AFTER the
   * privacy ratchet, and they are patched onto the cached session row because
   * that row is where every reader of the classification looks
   * (`usePinnedModel` reads `session.privacy_tier`, the chat-tab dot reads the
   * cached session list).
   *
   * ⚠ **The tier is why this frame had to widen.** #196 instrumented which
   * cached field lagged and measured exactly one: `privacy_tier`, with its
   * `privacy_reason`. A chat that a turn had just made private kept a `public`
   * tier in this cache until the turn ENDED and `refreshSessionBinding` re-read
   * the row — and a stale `public` suppresses the chip override and the
   * private-chat note together, which is why they used to appear only after a
   * reload.
   *
   * ⚠ **The row is patched, never adopted.** The frame is not a session
   * payload: it names four fields and this store is the transcript's source of
   * truth for everything else on that row.
   */
  private applyTurnBinding = (event: {
    provider: string;
    model: string;
    privacy_tier?: SessionClassification;
    privacy_reason?: string | null;
  }): void => {
    this.setPinnedModel({ provider: event.provider, model: event.model });
    if (event.privacy_tier === undefined) return;
    const tier = event.privacy_tier;
    const reason = event.privacy_reason ?? null;
    // ⚠ **A chat created in this window has no row yet when this frame lands,
    // and the classification used to be thrown away** (finding M8). The frame
    // is emitted at the top of the turn; for a chat whose session was created
    // by the submit that started that turn, `loadSession` is still in flight.
    // The pin survived that race because it has a home of its own; the tier's
    // only home is the row, so the updater below returned `prev` and the fact
    // was lost — the chat then showed public on every chat-side surface until
    // the post-turn re-read, four seconds later. Measured 2026-09-10 on session
    // `20260910_4`: snapshot `norow` at t+559 ms, row lands `public` at t+590,
    // and stays public until t+4327 — 28 ms after the turn ENDED.
    //
    // So the fact is kept until a row exists to carry it. It is ADOPTED rather
    // than ratcheted on arrival, for the same reason this method adopts: the
    // frame is the daemon's post-ratchet statement about this turn, and a
    // declassified chat (DR-20) is a legitimate private → public move that the
    // frame is the first to report.
    if (!this.snapshot.session) {
      this.pendingTurnClassification = { tier, reason };
      return;
    }
    this.updateSnapshot((prev) => {
      if (!prev.session) return prev;
      if (prev.session.privacy_tier === tier && (prev.session.privacy_reason ?? null) === reason) {
        return prev;
      }
      return {
        ...prev,
        session: { ...prev.session, privacy_tier: tier, privacy_reason: reason },
      };
    });
  };

  private clearPendingToolCalls = (): void => {
    this.updateSnapshot((prev) =>
      prev.pendingToolCalls.length === 0 ? prev : { ...prev, pendingToolCalls: [] }
    );
  };

  private noteChildInitialization(initializing: boolean, reloadWhenReady = true): void {
    if (initializing) {
      this.childInitializing = true;
      this.updateSnapshot((prev) => {
        const chatState = isRunningState(prev.chatState) ? prev.chatState : ChatState.Thinking;
        const turnStartedAt = prev.turnStartedAt ?? Date.now();
        if (
          !prev.agentReady &&
          prev.chatState === chatState &&
          prev.turnStartedAt === turnStartedAt
        ) {
          return prev;
        }
        return { ...prev, agentReady: false, chatState, turnStartedAt };
      });
      this.startObserverInitializationRefresh();
      return;
    }

    if (!this.childInitializing) return;
    this.childInitializing = false;
    this.observerInitializationNextRefreshAt = 0;
    if (!reloadWhenReady) return;
    // The earlier load deliberately stopped at the initialization guard. The
    // child now has its exact delegated runtime, so a fresh load is required
    // before consumers such as the tool-count query may call it ready.
    this.agentLoadPromise = null;
    void this.ensureAgentLoaded();
  }

  private startObserverInitializationRefresh(): void {
    if (!this.observing || !this.childInitializing) return;
    const observerGeneration = this.observerGeneration;
    if (this.observerInitializationRefresh?.observerGeneration === observerGeneration) return;
    const now = Date.now();
    if (now < this.observerInitializationNextRefreshAt) return;
    this.observerInitializationNextRefreshAt = now + OBSERVER_INITIALIZATION_REFRESH_INTERVAL_MS;

    let operation!: Promise<void>;
    operation = (async () => {
      try {
        const response = await resumeAgent({
          body: resumeRequestBody(this.sessionId, false),
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        if (
          !this.observing ||
          !this.childInitializing ||
          this.observerGeneration !== observerGeneration
        ) {
          return;
        }

        const data = response.data;
        this.notePendingContinuation(data);
        const initializing = isInitializingResume(data);
        this.noteChildInitialization(initializing);
        this.noteActiveTurn(data?.active_turn);
      } catch {
        // The next daemon heartbeat retries while initialization remains live.
      }
    })().finally(() => {
      if (this.observerInitializationRefresh?.operation === operation) {
        this.observerInitializationRefresh = null;
      }
    });
    this.observerInitializationRefresh = { observerGeneration, operation };
  }

  private notePendingContinuation(value: unknown): void {
    const pending = resumePendingContinuation(value);
    if (!pending) {
      if (!this.continuationLease) {
        this.updateSnapshot((prev) =>
          prev.pendingContinuation ? { ...prev, pendingContinuation: undefined } : prev
        );
      }
      return;
    }

    // A cancel response that landed after this resume was issued is newer local
    // evidence. Never let that stale response replace a lease this window just
    // acquired with a foreign-ownership gate.
    if (this.continuationLease && !pending.continuationLease) return;
    if (pending.continuationLease) {
      this.continuationLease = pending.continuationLease;
      this.continuationLeaseTurnId = null;
    }
    this.updateSnapshot((prev) => ({ ...prev, pendingContinuation: pending.view }));
  }

  /**
   * Load the agent — model provider + extensions — for this session, once.
   *
   * This is the slow half of resuming a chat: on a real session it is ~4.6s of
   * extension startup against ~0.5s to fetch the transcript, and it is
   * per-session, so every tab re-pays it. `loadSession` therefore paints the
   * transcript first and leaves this running in the background; anything that
   * genuinely needs the agent awaits `whenAgentReady()` instead of blocking the
   * paint.
   *
   * Never rejects: a failed agent load is reported through `turnError` and the
   * toast, and still resolves, so a submit parked on it can proceed and produce
   * a real error rather than hanging forever.
   */
  private ensureAgentLoaded(): Promise<void> {
    if (!this.sessionId) return Promise.resolve();
    if (this.agentLoadPromise) return this.agentLoadPromise;

    this.agentLoadPromise = (async () => {
      let initializing = false;
      try {
        const response = await resumeAgent({
          body: resumeRequestBody(this.sessionId, true),
          headers: await userActionHeaders(),
          throwOnError: true,
        });
        const resumeData = response.data;
        this.notePendingContinuation(resumeData);
        initializing = isInitializingResume(resumeData);
        this.noteChildInitialization(initializing, false);
        const initializationError = resumeData?.initialization_error;

        // Reached on the paths `loadSession` short-circuits — a controller
        // reused for a tab reopened mid-turn, or a transcript served from the
        // LRU — where this is the only `/agent/resume` that runs and therefore
        // the only place the session's live turn is named.
        this.noteActiveTurn(resumeData?.active_turn);

        showExtensionLoadResults(resumeData?.extension_results, this.sessionId);

        if (initializationError) {
          this.updateSnapshot((prev) => ({
            ...prev,
            turnError: {
              message: initializationError.message,
              technicalDetails: initializationError.message,
              code: initializationError.code,
              scope: 'session',
              retryable: initializationError.retryable,
            },
          }));
        }

        // Binds the session's model/provider onto the agent. Previously
        // fire-and-forget on the assumption the agent was already up; now it is
        // part of readiness, so a submit that waits for the agent also waits for
        // its model to be bound instead of racing it.
        if (resumeData?.session && !initializing) {
          try {
            await updateFromSession({
              body: { session_id: resumeData.session.id },
              headers: await userActionHeaders(),
              throwOnError: true,
            });
          } catch (err) {
            console.warn('Failed to update agent from session:', err);
          }
        }
      } catch (error) {
        console.warn('Failed to load model and extensions:', error);
        this.updateSnapshot((prev) => ({
          ...prev,
          // Do not clobber a turn error the user is already looking at.
          turnError: prev.turnError ?? clientTurnError(error, 'agent_load_failed', 'session'),
        }));
      } finally {
        if (initializing) {
          this.agentLoadPromise = null;
        } else {
          this.updateSnapshot((prev) => ({ ...prev, agentReady: true }));
        }
      }
    })();

    return this.agentLoadPromise;
  }

  /**
   * Resolves once the agent is loaded (or has failed to load). Kicks the load
   * off if nothing has yet — the cached-transcript path reaches submit without
   * ever having gone through `loadSession`'s cold path.
   */
  whenAgentReady(): Promise<void> {
    // Initializing children accept user input into the delegated first turn;
    // waiting for readiness here would prevent the very `/reply` that queues it.
    if (this.snapshot.agentReady || this.childInitializing) return Promise.resolve();
    return this.ensureAgentLoaded();
  }

  private reobserveReleasedSubagent(
    session: Session | undefined,
    ownershipGeneration: number
  ): void {
    if (
      this.ownershipReleased &&
      this.ownershipGeneration === ownershipGeneration &&
      session?.session_type === 'sub_agent'
    ) {
      void this.observeSession();
    }
  }

  async loadSession(onSessionLoaded?: () => void): Promise<void> {
    if (!this.sessionId) return;
    const ownershipGeneration = this.ownershipGeneration;

    if (this.snapshot.session) {
      this.reobserveReleasedSubagent(this.snapshot.session, ownershipGeneration);
      // Session already painted, but the agent may still be missing entirely on
      // the controller-reuse path. Idempotent — and it is the resume inside it
      // that reports any live turn, so nothing here needs to guess at one.
      void this.ensureAgentLoaded();
      onSessionLoaded?.();
      return;
    }

    const cached = cacheGet(this.sessionId);
    if (cached) {
      this.messagesRef = cached.messages;
      // The LRU holds whatever the transcript last looked like, which may be a
      // streamed view. Not a store read, so it proves nothing.
      this.viewNamesEveryStoredRow = false;
      this.updateSnapshot((prev) => ({
        ...prev,
        session: cached.session,
        messages: cached.messages,
        tokenState: {
          inputTokens: cached.session?.input_tokens ?? 0,
          outputTokens: cached.session?.output_tokens ?? 0,
          totalTokens: cached.session?.total_tokens ?? 0,
          accumulatedInputTokens: cached.session?.accumulated_input_tokens ?? 0,
          accumulatedOutputTokens: cached.session?.accumulated_output_tokens ?? 0,
          accumulatedTotalTokens: cached.session?.accumulated_total_tokens ?? 0,
        },
        chatState: this.isRunning() ? prev.chatState : ChatState.Idle,
      }));
      this.reobserveReleasedSubagent(cached.session, ownershipGeneration);
      // The cache is a process-lifetime LRU with no TTL, so this path could
      // previously reach a submit having NEVER loaded the agent for this
      // session — the transcript looked live while the backend had no
      // extensions. Kick the load off here; `whenAgentReady` is what makes the
      // submit safe.
      void this.ensureAgentLoaded();
      onSessionLoaded?.();
      return;
    }

    if (!this.loadPromise) {
      this.updateSnapshot((prev) => ({
        ...prev,
        messages: [],
        session: undefined,
        sessionLoadError: undefined,
        turnError: undefined,
        stopConfirmed: undefined,
        chatState: ChatState.LoadingConversation,
      }));

      this.loadPromise = (async () => {
        try {
          // PHASE 1 — the transcript, and nothing else. `load_model_and_extensions:
          // false` skips agent construction, provider restore and extension
          // startup, which is ~4.6s of the ~5.1s a resume used to take while
          // contributing a few hundred bytes to what the user reads. The user
          // came here to read the conversation; give them the conversation.
          const response = await resumeAgent({
            body: resumeRequestBody(this.sessionId, false),
            headers: await userActionHeaders(),
            throwOnError: true,
          });
          const resumeData = response.data;
          this.notePendingContinuation(resumeData);
          this.noteChildInitialization(isInitializingResume(resumeData));
          const loadedSession = resumeData?.session;

          this.messagesRef = loadedSession?.conversation || [];
          // `/agent/resume` returns `get_session(id, true)` — the same read
          // `edit_message`'s freshness check makes, hidden rows included — so
          // this view provably names every stored row. See
          // `viewNamesEveryStoredRow`.
          this.viewNamesEveryStoredRow = true;
          this.updateSnapshot((prev) => ({
            ...prev,
            session: loadedSession,
            messages: this.messagesRef,
            tokenState: {
              inputTokens: loadedSession?.input_tokens ?? 0,
              outputTokens: loadedSession?.output_tokens ?? 0,
              totalTokens: loadedSession?.total_tokens ?? 0,
              accumulatedInputTokens: loadedSession?.accumulated_input_tokens ?? 0,
              accumulatedOutputTokens: loadedSession?.accumulated_output_tokens ?? 0,
              accumulatedTotalTokens: loadedSession?.accumulated_total_tokens ?? 0,
            },
            // BR-71: `hasLiveTurn()`, not a bare `abortController` — an
            // observer holds one of those too, and reading its feed as a
            // running turn leaves `prev.chatState` (LoadingConversation, set at
            // the top of this load) pinned until some frame happens to move it.
            // On a quiet observed session that is a tab stuck on the loading
            // state forever.
            chatState:
              this.hasLiveTurn() || this.childInitializing
                ? isRunningState(prev.chatState)
                  ? prev.chatState
                  : ChatState.Thinking
                : ChatState.Idle,
            sessionLoadError: undefined,
            turnError: undefined,
          }));

          this.reobserveReleasedSubagent(loadedSession, ownershipGeneration);

          // PHASE 2 — model + extensions, off the paint path. Deliberately not
          // awaited: `loadSession` resolves as soon as the transcript is up.
          void this.ensureAgentLoaded();

          // PHASE 2b — rejoin a turn that is still running. This is the reload
          // case the whole feature exists for: the transcript above is what the
          // store had persisted, which for a live turn stops at the user's
          // message, and the attach fills in everything the agent has produced
          // since. The same response that carried the transcript names the
          // turn, so there is no extra round trip and nothing to guess.
          this.noteActiveTurn(resumeData?.active_turn);
        } catch (error) {
          if (isConnectionError(error)) {
            // The backend (biorouterd) was transiently unreachable — it is
            // restarting, or the network blipped. This is NOT an unloadable
            // session, so it must NOT escalate to the full-pane "Failed to Load
            // Session" card that nukes the transcript and offers only "Go home".
            // Surface it as a retryable inline turn error instead: the chat UI
            // and composer stay mounted, and `retryTurn` (or the daemon
            // self-recovering) re-runs this load and repaints the transcript.
            // The genuine-load-failure card below is reserved for real errors
            // (bad id / corrupt data / an HTTP response), which are not
            // TypeErrors and never look like a connection failure.
            this.updateSnapshot((prev) => ({
              ...prev,
              turnError:
                prev.turnError ?? clientTurnError(error, 'session_load_unreachable', 'transport'),
              chatState: ChatState.Idle,
            }));
          } else {
            this.updateSnapshot((prev) => ({
              ...prev,
              sessionLoadError: errorMessage(error),
              chatState: ChatState.Idle,
            }));
          }
        } finally {
          this.loadPromise = null;
        }
      })();
    }

    await this.loadPromise;
    onSessionLoaded?.();
  }

  /**
   * Round 3 / N3 — re-read the four row fields a TURN can change.
   *
   * The composer states the chat's own binding, and the classification decides
   * whether the app-wide selection is barred from it at all. Both are written by
   * the daemon during a turn and neither is derivable here: the binding may be
   * `restore_provider_from_session`'s or Gate B's repair of it, and
   * `privacy_tier` ratchets from whatever the turn touched. Without this, the
   * measured symptom was a chat that had just gone private still showing a
   * public model, its window, and no note — until the window was reloaded.
   *
   * ⚠ `metadata_only`, so this costs the row and not the transcript. The same
   * `GET /sessions/{id}` without it re-serialises every message in the chat,
   * once per turn, to learn three strings.
   *
   * ⚠ Four fields, merged — never the whole row. The response carries no
   * conversation and stale token counters; adopting it wholesale would blank the
   * transcript this store is the source of truth for.
   *
   * ⚠ Failure is silent by design. This runs after the turn has already been
   * delivered; a refresh that could not complete leaves the composer exactly as
   * stale as it was before this method existed, which is not worth a toast.
   */
  async refreshSessionBinding(): Promise<void> {
    if (!this.sessionId || !this.snapshot.session) return;
    const generation = ++this.bindingGeneration;
    try {
      const response = await getSession({
        path: { session_id: this.sessionId },
        query: { metadata_only: true },
        // Issue #56 Task 58: reading a private chat needs the proof-of-user —
        // and a chat this call exists to notice has JUST become private is
        // exactly the one that would be refused without it.
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      const row = response.data;
      if (!row) return;
      // Two ways an answer can be about something other than what is on screen
      // now, checked in that order because the first is unconditional and the
      // second is about time.
      //
      // ⚠ A row for ANOTHER chat. Nothing downstream re-checks the id — every
      // patch below writes into THIS controller's snapshot and into its entry in
      // the shared session list — so a mismatched payload would silently
      // relabel this chat with another one's binding and tier. Cheap, and the
      // one check here that does not depend on ordering.
      if (row.id !== this.sessionId) return;
      // ⚠ Superseded while in flight. See {@link bindingGeneration}: a pin
      // reported after this read began, or a second refresh started after it,
      // is the later fact. Adopting an older row over it is the flicker M4
      // measured, and it is worse than a flicker — it makes the composer state
      // a binding no turn used.
      if (this.bindingGeneration !== generation) return;
      this.updateSnapshot((prev) => {
        if (!prev.session) return prev;
        if (
          prev.session.provider_name === row.provider_name &&
          prev.session.model_config?.model_name === row.model_config?.model_name &&
          prev.session.privacy_tier === row.privacy_tier &&
          prev.session.privacy_reason === row.privacy_reason
        ) {
          return prev;
        }
        return {
          ...prev,
          session: {
            ...prev.session,
            provider_name: row.provider_name,
            model_config: row.model_config,
            privacy_tier: row.privacy_tier,
            privacy_reason: row.privacy_reason,
          },
        };
      });
      // ⚠ The tab dot and the sidebar read a DIFFERENT cache, and until now
      // nothing on the chat path touched it — `ChatGroupsShell`'s own comment
      // recorded that as a known gap and said closing it "needs the escalation
      // to announce itself from the provider-bind path". This is that
      // announcement's landing place: patch the same four fields on the list
      // entry, so the chat's dot and its header pill cannot disagree.
      //
      // Only an entry the cache already holds is touched. Inserting one would
      // put a row into a list whose membership is owned elsewhere
      // (`sessionListCache`'s list channel), from a read that knows nothing
      // about ordering or filters.
      // ⚠ The PIN yields to the freshly-read row, and this is not optional.
      // `chatBinding` prefers the turn-reported pin OVER the row, so a row
      // re-read after that turn — because it ended, or because another process
      // rewrote it — would be overruled by the pin and the chip would go on
      // naming the older model. Measured exactly that way at runtime: the CLI
      // rebound a chat to `gpt-5.2-2025-12-11`, this method adopted it, and the
      // composer kept reading `gpt-5.5-2026-04-24` off the pin.
      //
      // Replacing rather than clearing keeps the field meaning what it says, and
      // the row is the LATER fact — but only because the two guards above have
      // established that it is. "This row was read now" was an assumption when
      // this comment was written and it was wrong: the read is `async` and has
      // two callers that overlap, so an answer landing here may have been
      // requested before the pin it is about to overwrite even existed. M4
      // measured that as a chip going A → B → A inside one Send.
      // {@link bindingGeneration} is what makes the sentence true again.
      //
      // The daemon closes the same gap from the other side: Gate B now rebinds
      // FROM the row whenever the row names a different provider or model, not
      // only when the classification forces a repair — so a turn's pin and its
      // row agree by construction and there is normally nothing here to prefer.
      if (row.provider_name && row.model_config?.model_name) {
        this.setPinnedModel({
          provider: row.provider_name,
          model: row.model_config.model_name,
        });
      }
      //
      // ⚠ Checked BEFORE the call, not inside the updater.
      // `updateCachedSessionList` emits to every list subscriber
      // unconditionally, and this runs after every turn — so an updater that
      // returned the array unchanged would still wake the sidebar, the See-all
      // view and every tab strip once per turn to discover that nothing moved.
      const cached = getCachedSessionList();
      const index = cached?.findIndex((entry) => entry.id === this.sessionId) ?? -1;
      const entry = index === -1 ? undefined : cached![index];
      // ⚠ A chat CREATED in this window is absent from that list, and announcing
      // at `createSession` cannot fix it: `GET /sessions` INNER JOINs `messages`
      // (`SessionStorage::list_sessions_by_types_maybe_empty`), so a row with no
      // message yet is not listable and a refresh fired at create time comes
      // back without it. The first moment the daemon WILL list the chat is after
      // its first turn — which is here. So the announcement is made from here
      // instead, and it is a refetch rather than an insert for the reason the
      // note above gives: membership is the list channel's to own.
      //
      // ⚠ Once per chat per renderer, and only against a cache that has
      // actually been fetched. A null cache means nobody has asked yet
      // (`ChatGroupsShell` warms it on mount), and answering that with a full
      // list fetch per turn-end would be a page of session rows bought to learn
      // one string. The tab dot does not wait for any of this — it reads the
      // live store tier through `ChatStreamRegistry.subscribeSessionTiers` —
      // which is why this can afford to be the slow, correct path for the
      // OTHER list surfaces (Home recents, See-all) rather than the fix for M8.
      if (cached !== null && index === -1 && !this.announcedListMembership) {
        this.announcedListMembership = true;
        notifySessionListChanged();
      }
      const listDiffers =
        entry != null &&
        (entry.provider_name !== row.provider_name ||
          entry.model_config?.model_name !== row.model_config?.model_name ||
          entry.privacy_tier !== row.privacy_tier ||
          entry.privacy_reason !== row.privacy_reason);
      if (listDiffers) {
        updateCachedSessionList((sessions) => {
          const at = sessions.findIndex((candidate) => candidate.id === this.sessionId);
          if (at === -1) return sessions;
          const next = sessions.slice();
          next[at] = {
            ...sessions[at],
            provider_name: row.provider_name,
            model_config: row.model_config,
            privacy_tier: row.privacy_tier,
            privacy_reason: row.privacy_reason,
          };
          return next;
        });
      }
    } catch (error) {
      console.warn('Failed to refresh the chat’s model binding after a turn:', error);
    }
  }

  /** Whether this controller holds a session row at all. */
  hasLoadedSession(): boolean {
    return this.snapshot.session != null;
  }

  /**
   * M2 — do not let the daemon's synthesized ending blame the model for an
   * ending the USER asked for.
   *
   * `TurnStream::close` writes `stream_ended_without_terminal` for any turn
   * whose writers produced no terminal frame, and a cancelled turn is one of
   * them: the daemon is describing the SHAPE of the ending, not its cause, and
   * cannot tell the two apart. Read as an ordinary `scope: internal` failure it
   * comes out as "Model turn ended unexpectedly" over a Stop the user pressed
   * themselves.
   *
   * Deliberately narrow. Only that one code, and only while this chat has a
   * Stop outstanding for this exact generation — an ordinary mid-turn internal
   * failure with no Stop pending keeps the wording it has today, which is the
   * right wording for it.
   */
  private reframeStoppedTurnError(
    error: ChatTurnErrorData,
    stopGateHolds: boolean,
    turnId: string | null
  ): ChatTurnErrorData {
    if (error.code !== STREAM_ENDED_WITHOUT_TERMINAL) return error;
    const stopWasRequested =
      stopGateHolds ||
      (!!turnId && (this.stopExpectedTurnId === turnId || this.lastSettledStopTurnId === turnId));
    if (!stopWasRequested) return error;
    return {
      ...error,
      message: 'You stopped this turn, so it ended without a result.',
      code: TURN_STOPPED_BY_USER,
      // Retrying the turn is not what someone who just pressed Stop wants; the
      // composer they get back is.
      retryable: false,
      technicalDetails: error.technicalDetails ?? error.message,
    };
  }

  private finishCurrentStream = async (error?: ChatTurnErrorData): Promise<void> => {
    // Sampled at the very top, and once. It reads `activeTurnId`, which
    // `retireActiveTurn()` below clears — so a check made afterwards answers
    // the opposite of one made before — and the error rewrite a few lines down
    // needs the same answer the retirement does.
    const stopGateHolds = this.stopGateHolds();
    const stoppedTurnId = this.activeTurnId ?? this.stopExpectedTurnId;
    if (error) {
      const framed = this.reframeStoppedTurnError(error, stopGateHolds, stoppedTurnId);
      this.updateSnapshot((prev) => ({ ...prev, turnError: framed }));
    }
    // The turn is over: any skeleton whose authoritative request never arrived
    // (cancel, provider abort, a dropped block) must not linger.
    this.clearPendingToolCalls();
    this.abortController = null;
    // Retire the turn's attach handle and its sequence numbering together. Both
    // outliving the turn is the same bug in two shapes: a stale handle sends a
    // reopened tab attaching to a turn that is over, and a stale high-water mark
    // makes the NEXT turn's `seq: 0` look like a duplicate and swallows it.
    //
    if (stopGateHolds) {
      // M2 — remember that this turn's ending was withheld from the Idle
      // transition below. If the cancel that owns that transition comes back a
      // failure, `reportUnconfirmedStop` is the only thing left that can make
      // it, and it has no other way to know the frame already went by.
      this.stopDeferredFinishTurnId = stoppedTurnId;
    } else {
      this.retireActiveTurn();
    }

    const timeSinceLastInteraction = Date.now() - this.lastInteractionTime;
    if (!error && timeSinceLastInteraction > 60000) {
      window.electron?.showNotification({
        title: 'biorouter finished the task.',
        body: 'Click here to expand.',
      });
    }

    // Every completed turn can change the session's recency or generated name.
    // Session ids use a variable-width daily counter (for example
    // `20260716_1` and `20260716_27`), so gating this refresh behind a fixed
    // id shape leaves History and Recents stale for real sessions.
    if (this.sessionId) {
      window.dispatchEvent(new CustomEvent('message-stream-finished'));
    }

    // Round 3 / N3. A turn is the other thing that changes this chat's binding,
    // and unlike a model switch it changes fields no client can compute: the
    // daemon binds `restore_provider_from_session`'s provider, Gate B may repair
    // it, and the privacy ratchet raises `privacy_tier` from whatever the turn
    // touched. Not awaited — the turn is over and nothing on screen waits on it.
    void this.refreshSessionBinding();

    if (
      this.sessionId &&
      this.snapshot.session &&
      !this.snapshot.session.user_set_name &&
      isDefaultSessionName(this.snapshot.session.name)
    ) {
      const pollDelays = [800, 1200, 2000, 3000, 4000, 6000, 8000, 10000];
      void (async () => {
        for (const delay of pollDelays) {
          await new Promise((r) => setTimeout(r, delay));
          try {
            const response = await getSession({
              path: { session_id: this.sessionId },
              // Issue #56 Task 58: reading a private chat needs the
              // proof-of-user.
              headers: await userActionHeaders(),
              throwOnError: true,
            });
            const data = response.data;
            if (!data) continue;
            const proposedName = data.name;
            if (data.user_set_name) break;
            if (proposedName && !isDefaultSessionName(proposedName)) {
              const uniqueName = await disambiguateSessionName(proposedName, this.sessionId);
              if (uniqueName !== proposedName) {
                try {
                  await renameSession(this.sessionId, uniqueName, 'llm');
                } catch (renameError) {
                  console.warn('Failed to persist disambiguated session name:', renameError);
                }
              } else {
                announceSessionName({
                  sessionId: this.sessionId,
                  name: uniqueName,
                  userSetName: false,
                  origin: 'llm',
                });
              }
              this.updateSnapshot((prev) =>
                prev.session && prev.session.name !== uniqueName
                  ? { ...prev, session: { ...prev.session, name: uniqueName } }
                  : prev
              );
              break;
            }
          } catch (refreshError) {
            console.warn('Failed to refresh session name:', refreshError);
          }
        }
      })();
    }

    this.updateSnapshot((prev) => ({
      ...prev,
      // A terminal frame can beat the cancel HTTP response by a few
      // microtasks. Keep the composer gated until that response confirms the
      // server-side turn slot has actually been released.
      chatState: stopGateHolds ? prev.chatState : ChatState.Idle,
      // …and once the turn really has ended, a standing "could not stop, press
      // Stop again" notice is describing a world that no longer exists. It is
      // retracted rather than left to age, on the same rule as `pendingSteer`
      // below: the turn it spoke about is over. Only that one code — a real
      // turn failure is still the truth about this turn and stays put.
      turnError:
        !stopGateHolds && prev.turnError?.code === STOP_NOT_CONFIRMED ? undefined : prev.turnError,
      turnStartedAt: undefined,
      lastMessageAt: undefined,
      // The turn it was aimed at is over; whether or not we saw the echo, there
      // is nothing left to steer.
      pendingSteer: undefined,
    }));
    // #22 — a genuine turn boundary lands synchronously. During Stop, the
    // cancel response owns the final Idle transition because it is the stronger
    // server-side admission barrier.
    this.flushNotify();
    for (const listener of this.finishListeners) listener();
  };

  private async adoptQueuedInitializingChild(message: Message, streamId: number): Promise<boolean> {
    try {
      const response = await resumeAgent({
        body: resumeRequestBody(this.sessionId, false),
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      if (this.activeStreamId !== streamId) return true;
      const data = response.data;
      this.notePendingContinuation(data);
      if (data?.session?.session_type !== 'sub_agent') return false;

      const initializing = isInitializingResume(data);
      this.noteChildInitialization(initializing);
      this.pendingInitializingChildMessage = message;
      const activeTurnId = data?.active_turn?.turn_id ?? null;

      if (!initializing && !activeTurnId) {
        const conversation = data?.session?.conversation ?? [];
        this.pendingInitializingChildMessage = null;
        this.updateMessages(conversation);
        this.abortController = null;
        this.retireActiveTurn();
        await this.finishCurrentStream();
        return true;
      }

      this.abortController = null;
      this.activeTurnId = activeTurnId;
      this.reattachesThisTurn = 0;
      // The 202 admitted this input into the child's delegated first turn. Its
      // synthetic request id is not a renderer-owned turn and must never be
      // re-attached; the observer is the durable display connection instead.
      void this.observeSession();
      return true;
    } catch (error) {
      console.warn('Could not verify initializing child admission:', error);
      return false;
    }
  }

  private async streamFromResponse(
    stream: AsyncIterable<MessageEvent>,
    initialMessages: Message[],
    streamId: number,
    /**
     * Which kind of connection produced this stream. It changes exactly one
     * thing — what the END of the stream means when no `Finish` arrived; see
     * the branch below. Everything else about the pipeline is identical, which
     * is the point of BR-71 reusing it.
     */
    source: 'driver' | 'observer' = 'driver',
    options: StreamDrainOptions = {}
  ): Promise<void> {
    let currentMessages = initialMessages;
    let eventCount = 0;

    try {
      for await (const event of stream) {
        if (this.activeStreamId !== streamId) return;
        if (eventCount === 0) options.onFirstEvent?.();
        eventCount += 1;

        // Contract §1 — drop what this client has already rendered. This is the
        // ONLY thing standing between a re-attach and a duplicated paragraph:
        // the server replays a turn from its start, and a client that was
        // watching for the first half of it must apply only the second.
        if (!this.applySequenceGate(event)) continue;
        this.noteObservedTurnActivity(event);

        const turnState = turnStateEvent(event);
        if (turnState) {
          this.applyObservedTurnState(turnState.active_turn_id);
          continue;
        }

        const started = turnStartedEvent(event);
        if (started) {
          // A live start ordered after the connection snapshot advances the
          // same authoritative lifecycle state without model-output inference.
          this.applyObservedTurnState(started.turn_id);
          continue;
        }

        // Contract §R2 — replay lands as one commit, the live tail frame by
        // frame. The transition is the first frame that is not marked `replay`,
        // and it releases the hold BEFORE that frame is applied, so the backlog
        // and the live tail are never merged into a state the server never had.
        // An UNSEQUENCED frame decides nothing either way. The producer emits
        // those for things that carry no ordering — the `Ping` heartbeat, and
        // the whole-conversation resync that precedes a backlog whose oldest
        // frames were evicted (`turn_stream.rs`, "Wire format"). Treating one as
        // the live tail would end the hold in the middle of a backlog and paint
        // the replay in two halves; treating it as replay would be just as
        // wrong on an idle live stream, where a heartbeat would open a hold that
        // nothing closes until the safety valve.
        if (isReplayFrame(event)) {
          this.beginReplayHold();
        } else if (frameSeq(event) !== undefined) {
          this.endReplayHold();
        }

        switch (event.type) {
          case 'ToolCallPending': {
            // Advisory skeleton for a tool whose args are still streaming. Upsert
            // by id; NEVER routed into `messages` (see `pendingToolCalls`).
            this.upsertPendingToolCall({
              id: event.id,
              name: event.name,
              partialArgs: event.partial_args ?? undefined,
            });
            // A tool block is generating: reflect that as active streaming.
            this.setChatState(ChatState.Streaming);
            break;
          }
          case 'Message': {
            if (event.message.id === this.pendingInitializingChildMessage?.id) {
              this.pendingInitializingChildMessage = null;
            }
            // Drop the part of a replay the session store already gave us.
            const msg = this.dedupeReplayedMessage(
              event.message,
              currentMessages,
              isReplayFrame(event)
            );
            if (!msg) break;
            currentMessages = pushMessage(currentMessages, msg);
            // #22 — one snapshot swap (state + tokens + transcript + skeleton
            // cleanup) per streamed event, not three.
            //
            // BR-71 §3c: on an OBSERVER feed with no turn in flight, a message
            // is a row appended to an IDLE conversation, not a turn producing
            // output. `workspace_send_prompt mode:"note"` is exactly that — it
            // publishes the stored row and starts nothing — and
            // `applyMessageEvent` derives `ChatState.Streaming` from any
            // message, so without this the target tab claimed "Thinking…" with
            // a stop button, indefinitely, for a turn that did not exist and so
            // could never publish a terminal to retire it. Measured in the
            // running app: `/active_work` empty, tab at `data-working="true"`.
            //
            // Narrow on purpose. A real observed turn announces itself first —
            // `TurnStarted` sets `activeTurnId` via `applyObservedTurnState`,
            // and the SSE handler sends `TurnState` right after its snapshot —
            // so a running turn's messages still raise the running state. Only
            // the no-turn case is held back, and only for an observer; the
            // driver path is untouched.
            const appendedWhileIdle = this.observing && !this.activeTurnId;
            this.applyMessageEvent(
              msg,
              currentMessages,
              event.token_state,
              Date.now(),
              appendedWhileIdle
            );
            break;
          }
          case 'Error':
            if (!this.observedTerminalTargetsCurrentTurn(event)) break;
            // The turn is over — release before the terminal transition so the
            // whole turn-boundary battery (finish listeners, notification, name
            // poll) observes a settled, fully-painted store.
            this.endReplayHold();
            if (this.observing) {
              this.noteChildInitialization(false, false);
              this.rememberRetiredObservedTurn(frameTurnId(event) ?? this.activeTurnId);
              this.observedTurnGeneration += 1;
              this.observedTurnResolvedGeneration = -1;
            }
            // A server-authored terminal frame is an authoritative outcome, not
            // an ambiguous transport loss. A later Retry may start fresh work.
            this.ambiguousRetryTurnId = null;
            await this.finishCurrentStream({
              message: event.error,
              technicalDetails: event.error,
              code: event.code || 'unknown',
              scope: event.scope || 'inference',
              retryable: event.retryable ?? false,
              providerKind: event.provider_kind ?? undefined,
            });
            return;
          case 'Finish':
            if (!this.observedTerminalTargetsCurrentTurn(event)) break;
            this.endReplayHold();
            if (this.observing) {
              this.noteChildInitialization(false, false);
              this.rememberRetiredObservedTurn(frameTurnId(event) ?? this.activeTurnId);
              this.observedTurnGeneration += 1;
              this.observedTurnResolvedGeneration = -1;
            }
            this.ambiguousRetryTurnId = null;
            this.updateTokenState(event.token_state);
            await this.finishCurrentStream();
            return;
          case 'MessagesPersisted':
            break;
          case 'PrivacyProviderPinned':
            this.applyTurnBinding(event);
            break;
          case 'ModelChange':
          case 'Ping':
            break;
          case 'UpdateConversation':
            currentMessages = event.conversation;
            if (
              this.pendingInitializingChildMessage &&
              !currentMessages.some(
                (message) => message.id === this.pendingInitializingChildMessage?.id
              )
            ) {
              currentMessages = [...currentMessages, this.pendingInitializingChildMessage];
            } else if (this.pendingInitializingChildMessage) {
              this.pendingInitializingChildMessage = null;
            }
            this.updateMessages(currentMessages);
            this.updateTokenState(event.token_state);
            break;
          case 'Notification':
            this.updateNotifications(event as NotificationEvent);
            break;
          default:
            break;
        }
      }

      // The stream ended without a `Finish`. For a DRIVER that is a dead turn
      // and the error card is the truth. For an OBSERVER it is the ordinary
      // reconnect trigger the loop in `observeSession` is built on — the
      // generated SSE client ends its generator rather than throwing once
      // `sseMaxRetryAttempts` is spent, and the daemon replacing a broadcast
      // receiver ends it too — so painting a turn failure here would put a red
      // card in the tab on every dropped feed and then silently repair it a
      // second later, for a session this window does not drive and the user
      // cannot act on.
      //
      // Do not infer a turn boundary from transport loss. The next connection's
      // authoritative TurnState snapshot settles stale activity if the terminal
      // edge landed inside this reconnect gap.
      if (
        source === 'driver' &&
        this.activeStreamId === streamId &&
        !this.abortController?.signal.aborted
      ) {
        if (
          eventCount === 0 &&
          options.queuedInitializingChildMessage &&
          !options.hadTransportError?.() &&
          (await this.adoptQueuedInitializingChild(
            options.queuedInitializingChildMessage,
            streamId
          ))
        ) {
          return;
        }
        // …but under the live-turn stream contract "this socket ended" is no
        // longer the same event as "the turn died": the server keeps the turn
        // running with zero observers and holds its backlog (§3, §5). So try to
        // pick it back up before declaring a failure the user would have to
        // retry by hand — this is the ordinary mid-turn network blip, and
        // re-attaching loses nothing where the error card loses the rest of the
        // turn. Only if the turn really is gone does the card below stand.
        if (await this.reattachAfterDrop(streamId)) return;
        this.ambiguousRetryTurnId = this.activeTurnId;
        await this.finishCurrentStream({
          message: 'The connection closed before Biorouter received a completion status.',
          code: 'stream_interrupted',
          scope: 'transport',
          retryable: true,
        });
      }
    } catch (error) {
      if (this.activeStreamId !== streamId) return;
      if (error instanceof Error && error.name === 'AbortError') return;
      this.ambiguousRetryTurnId = this.activeTurnId;
      await this.finishCurrentStream(clientTurnError(error, 'stream_error', 'transport'));
    } finally {
      // No exit from this loop — clean end, terminal frame, abort, transport
      // error, a `return` from a stale stream id — may leave the transcript
      // held back. `endReplayHold` is a no-op when no hold is open.
      this.endReplayHold();
    }
  }

  /**
   * How many times one turn may re-open its stream after a drop before the
   * client accepts that the turn is gone.
   *
   * Small and un-delayed on purpose. Re-attaching is cheap (the server already
   * holds the backlog) and the failure it recovers from is a momentary one, so
   * a few immediate attempts catch the blip; anything longer is better spent
   * telling the user the truth than spinning. A daemon that is genuinely down
   * fails all of them in milliseconds and the error card appears as it always
   * did.
   */
  private static readonly MAX_REATTACHES_PER_TURN = 3;
  private reattachesThisTurn = 0;

  /**
   * A driving stream ended without a terminal frame. Re-open it against the
   * same turn and keep rendering.
   *
   * Returns true when a replacement stream was opened and pumped to its own
   * conclusion — the caller must then do nothing at all, because the turn's
   * outcome has already been dealt with by the recursive drain. False means the
   * turn could not be rejoined and the caller's failure path stands.
   *
   * `from_seq` comes out of the sequence gate, so the replacement stream picks
   * up where this one stopped even if the server chooses to replay from 0 —
   * one of these two is redundant, and which one depends on the server, so the
   * client relies on neither alone.
   */
  private async reattachAfterDrop(streamId: number): Promise<boolean> {
    const turnId = this.activeTurnId;
    if (!turnId) return false;
    if (this.reattachesThisTurn >= ChatStreamController.MAX_REATTACHES_PER_TURN) return false;
    if (this.activeStreamId !== streamId) return false;

    // Resolved BEFORE any of this controller's state is taken over. It is the
    // same proof the attach above sends and for the same reason (`/reply` is on
    // the gated list), but here it is also an `await`, and taking the socket and
    // the stream id first would leave a window in which this reattach claims the
    // turn with no stream on it. The guard is re-run afterwards because the
    // await is a window in which a submit or a Stop can arrive.
    const headers = await userActionHeaders();
    if (this.activeStreamId !== streamId) return false;
    this.reattachesThisTurn += 1;

    this.abortController = new AbortController();
    const nextStreamId = streamId + 1;
    this.activeStreamId = nextStreamId;
    const continuationLease =
      this.continuationLeaseTurnId === turnId ? this.continuationLease : null;

    try {
      const { stream } = await reply({
        headers,
        body: buildAttachRequest(
          this.sessionId,
          turnId,
          this.lastAppliedSeq + 1,
          this.messagesRef,
          continuationLease ?? undefined
        ),
        throwOnError: true,
        signal: this.abortController.signal,
        sseMaxRetryAttempts: 1,
      });
      await this.streamFromResponse(
        stream as AsyncIterable<MessageEvent>,
        this.messagesRef,
        nextStreamId,
        'driver',
        { onFirstEvent: () => this.releaseContinuationLeaseLocally(continuationLease) }
      );
      return true;
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') return true;
      console.warn(`Could not re-attach to dropped turn ${turnId}:`, error);
      // Hand the stream id back so the caller's own guard still holds and its
      // error lands on the turn it was written for.
      if (this.activeStreamId === nextStreamId) this.activeStreamId = streamId;
      return false;
    }
  }

  /**
   * BR-71: render a session this window is NOT driving. Subscribes to the
   * read-only observer stream (GET /sessions/{id}/events, generated client
   * `.sse.get`) and feeds it through the SAME event pipeline as a `/reply`
   * stream — the observer emits identical `MessageEvent` frames, starting with
   * an `UpdateConversation` snapshot. Used by tabs the daemon opened (subagent
   * tabs, `workspace_open` from another agent).
   *
   * Owns its reconnects: the observer stream never "completes" from the
   * client's point of view (the session outlives any one connection), so on
   * stream end or transport error it re-subscribes with backoff until
   * `stopObserving()`, a user-driven turn taking the controller over, or Stop
   * (design §4.3; the daemon side is generation-safe — a re-subscribe is just
   * a new broadcast receiver + fresh snapshot). A stream that ends without a
   * `Finish` is this loop's ordinary trigger, not a dead turn — which is why
   * `streamFromResponse` is told which kind of connection it is draining.
   *
   * ⚠ This relay MUST NOT reorder, filter or buffer the frames it forwards.
   * The producer-side invariant "no `MessagesPersisted` may precede a
   * `Message` frame carrying one of the ids it publishes" (`agent.rs`) is a
   * property of the STREAM ORDER, and survives only if every relay preserves
   * it. We get that for free by handing the whole `AsyncIterable` straight to
   * `streamFromResponse` with no `Promise.all` and no per-variant skipping —
   * keep it that way. Dropping the accounting frames here because this store
   * ignores them today would be invisible now and would silently break the day
   * `MessagesPersisted` is consumed (see `viewNamesEveryStoredRow`'s
   * FOLLOW-UP). Forward every variant, in order, unconditionally.
   *
   * On the observer-tab consequence for `expectedMessageIds`, see `observing`.
   */
  async observeSession(): Promise<void> {
    if (this.observing) return; // idempotent — tab re-mounts must not stack loops
    this.ownershipReleased = false;
    // Never take the socket away from a turn the USER is driving. This is
    // reached on daemon input — `ChatGroupsContext` attaches on every qualifying
    // workspace frame, `annotate_tab` for a tab that already exists included —
    // so without this check a tab the user has taken over has its live `/reply`
    // stream torn down by a frame it never asked for, and silently reverts to
    // being an observer while the user believes they are driving.
    if (this.hasLiveTurn()) return;
    // Generation guard, `UiBridge`-style: `stopObserving()` (or a takeover) can
    // clear the flag and let a NEW loop start while this one is still parked in
    // a drain or a backoff. Without the generation, the old loop's unwinding
    // would clear the new loop's flag and a third attach would then run two
    // loops against one controller.
    const generation = ++this.observerGeneration;
    this.observing = true;
    this.observerInitializationNextRefreshAt = 0;
    // `/agent/resume` may have named the child turn just before the workspace
    // frame attached this observer. The observer already owns the better live
    // feed, so it must not re-POST `/reply`; it does still need to expose the
    // same running state that a driver attach would expose to the composer,
    // tab strip, and running-chat registry.
    this.markObservedTurnRunning();
    this.startObserverInitializationRefresh();
    let retryMs = 1000;
    try {
      while (this.observing && this.observerGeneration === generation) {
        const streamId = ++this.activeStreamId;
        this.observedTurnGeneration += 1;
        this.observedTurnResolvedGeneration = -1;
        this.abortController?.abort();
        // Held in a local as well as on the field. The `headers` await below is
        // the first suspension point in this iteration, and a submit landing in
        // it replaces `this.abortController`; binding the signal off the field
        // afterwards would hand this subscription the OTHER turn's socket.
        const socket = new AbortController();
        this.abortController = socket;
        // When the response came back, so the block after the try can ask how
        // long the stream LASTED. Null until then: a stream that never opened
        // cannot have lasted, and a slow `userActionHeaders` read must not be
        // counted as time on the wire.
        let openedAt: number | null = null;
        try {
          const { stream } = await observeSessionEvents({
            // `GET /sessions/{id}/events` is on the gated list too (it is the
            // stored transcript plus a live tail), so an observer tab on a
            // PRIVATE chat is refused without the user-action proof — and
            // because a refusal reaches this loop as a stream that simply ended,
            // it would have reconnected against the same 403 forever, backing
            // off to 15s, with nothing on screen. See `userActionHeaders`.
            //
            // Read here rather than before the socket is taken, and per
            // iteration rather than once outside the loop (which outlives any
            // one key read). Reading it earlier would put an `await` between
            // this loop's entry and its `abortController` write, so a
            // `stopObserving()` arriving in that window would abort the OLD
            // socket and then be overwritten by a fresh un-aborted one —
            // leaving `hasLiveTurn()` true for a loop that is being torn down.
            headers: await userActionHeaders(),
            path: { session_id: this.sessionId },
            throwOnError: true,
            signal: socket.signal,
            // NOT optional. Without it the generated SSE client retries forever
            // on its own (`api/core/serverSentEvents.gen.ts` — `sseMaxRetryAttempts`
            // has no default) and the loop below never regains control on a
            // transport error. `/reply` passes the same value for the same reason.
            sseMaxRetryAttempts: 1,
          });
          openedAt = Date.now();
          // `'observer'`: a stream that ends without a `Finish` is this loop's
          // reconnect trigger, not a dead turn — see the branch it gates.
          await this.streamFromResponse(stream, this.messagesRef, streamId, 'observer');
        } catch (error) {
          // Rarely reached: `serverSentEvents` returns `{ stream }` from a lazy
          // async generator, so the await above resolves before a byte is
          // fetched and transport errors surface inside `streamFromResponse`.
          // Kept for the client-side throws that DO land here (a malformed URL).
          if (error instanceof Error && error.name === 'AbortError') return;
          // fall through to retry
        }
        // The stream has ENDED. Only now is it known whether it was a real
        // connection or a snapshot-and-close, so only now can the backoff floor
        // be earned. Placed after the catch on purpose: a stream that followed
        // the tail for a while and then threw is still a connection that
        // dropped, and should come back at the floor like any other.
        if (openedAt !== null && Date.now() - openedAt >= HEALTHY_OBSERVER_STREAM_MS) {
          retryMs = 1000;
        }
        if (this.observerIsStale(streamId, generation)) return;
        await new Promise((resolve) => setTimeout(resolve, retryMs));
        retryMs = Math.min(retryMs * 2, 15000);
        // The backoff is a window in which anything can happen — Stop, a user
        // submit, a detach — and each of those either clears the flag or bumps
        // `activeStreamId`. Re-check BOTH before reconnecting: the loop
        // condition alone tests only the flag, so a Stop pressed during backoff
        // (which bumps the id and does not know about observing) would be undone
        // by the very next tick, leaving Stop doing nothing except firing
        // `cancelTurn` at a session another agent drives.
        if (this.observerIsStale(streamId, generation)) return;
      }
    } finally {
      // The flag must never outlive its loop. Every `return` above leaves this
      // controller with no observer running, and a flag still claiming
      // otherwise makes every later attach short-circuit on the idempotence
      // guard — a tab dead for the life of the renderer, since `getController`
      // retains controllers forever. `stopStreaming` is the ordinary way in: it
      // bumps `activeStreamId` and knows nothing about observing.
      if (this.observerGeneration === generation) this.observing = false;
    }
  }

  /** This observer iteration has been torn down or taken over: something
   * bumped the stream id, cleared the flag, or started a newer loop. */
  private observerIsStale(streamId: number, generation: number): boolean {
    return (
      !this.observing || this.activeStreamId !== streamId || this.observerGeneration !== generation
    );
  }

  /**
   * A turn THIS controller drives is in flight. An observer holds an
   * `abortController` too — its read-only feed's — and that is not a turn:
   * anything that asks "is this controller busy?" to decide whether it may take
   * the socket must exclude it, or the observer's own subscription looks exactly
   * like a user's live `/reply`.
   */
  private hasLiveTurn(): boolean {
    return !this.observing && !!this.abortController && !this.abortController.signal.aborted;
  }

  /** Detach from the observed session (tab closed / user takes over). */
  stopObserving(): void {
    // Only ever tears down a socket this controller is OBSERVING on. On a
    // controller that is driving — the user took the tab over, or it was never
    // an observer at all — the abort below would cancel their live turn, and
    // "the tab closed" is not a reason to do that (BR-62b: the server keeps
    // running either way).
    if (!this.observing) return;
    this.observing = false;
    // Retires the parked loop's generation, so its unwinding cannot clear the
    // flag of a loop started after this detach.
    this.observerGeneration++;
    this.abortController?.abort();
  }

  private abandonLeaseToken = (token: string): Promise<boolean> => {
    const existing = this.leaseAbandonmentInFlight.get(token);
    if (existing) return existing;

    this.pendingLeaseAbandons.add(token);
    const operation = (async () => {
      try {
        await abandonContinuationLeaseRequest(this.sessionId, token);
        this.pendingLeaseAbandons.delete(token);
        if (this.continuationLease === token) {
          this.continuationLease = null;
          this.continuationLeaseTurnId = null;
          this.updateSnapshot((prev) =>
            prev.pendingContinuation ? { ...prev, pendingContinuation: undefined } : prev
          );
        }
        return true;
      } catch (error) {
        // The token remains both locally addressable and in the retry set until
        // the daemon acknowledges it. A dropped abandon response must not turn
        // the server's Live lease into an unrecoverable orphan.
        console.warn('Failed to abandon continuation lease:', error);
        return false;
      }
    })();
    this.leaseAbandonmentInFlight.set(token, operation);
    void operation.finally(() => {
      if (this.leaseAbandonmentInFlight.get(token) === operation) {
        this.leaseAbandonmentInFlight.delete(token);
      }
    });
    return operation;
  };

  private abandonLeaseWithRetry = async (token: string): Promise<void> => {
    if (await this.abandonLeaseToken(token)) return;
    // One immediate idempotent retry covers the important lost-response case
    // without creating a timer that outlives a closed renderer. If the daemon
    // is genuinely unreachable, the token remains in pendingLeaseAbandons and
    // the next lifecycle attempt retries it again.
    await Promise.resolve();
    await this.abandonLeaseToken(token);
  };

  abandonContinuation = async (): Promise<void> => {
    const tokens = new Set(this.pendingLeaseAbandons);
    if (this.continuationLease) tokens.add(this.continuationLease);
    await Promise.all([...tokens].map((token) => this.abandonLeaseWithRetry(token)));
  };

  private abandonContinuationIfOwned = async (token: string | null): Promise<void> => {
    if (!token || this.continuationLease !== token) return;
    await this.abandonContinuation();
  };

  private releaseContinuationLeaseLocally(token: string | null): void {
    if (!token || this.continuationLease !== token) return;
    this.continuationLease = null;
    this.continuationLeaseTurnId = null;
    this.pendingLeaseAbandons.delete(token);
    this.updateSnapshot((prev) =>
      prev.pendingContinuation ? { ...prev, pendingContinuation: undefined } : prev
    );
  }

  recoverPendingContinuation = (action: ContinuationRecoveryAction): Promise<void> => {
    if (this.continuationRecoveryInFlight) return this.continuationRecoveryInFlight;
    const pending = this.snapshot.pendingContinuation;
    if (!pending) return Promise.resolve();
    const operation = (async () => {
      const response = await recoverContinuationGroup(
        this.sessionId,
        pending.supersededTurnId,
        action
      );
      if (action === 'abandon') {
        this.continuationLease = null;
        this.continuationLeaseTurnId = null;
        this.updateSnapshot((prev) => ({ ...prev, pendingContinuation: undefined }));
        return;
      }
      if (!response.continuation_lease) {
        throw new Error('Continuation takeover did not return a lease');
      }
      this.continuationLease = response.continuation_lease;
      this.continuationLeaseTurnId = null;
      this.updateSnapshot((prev) => ({
        ...prev,
        pendingContinuation: {
          ownership: 'owned',
          supersededTurnId: response.superseded_turn_id,
        },
      }));
    })();
    this.continuationRecoveryInFlight = operation;
    const clearRecovery = () => {
      if (this.continuationRecoveryInFlight === operation) {
        this.continuationRecoveryInFlight = null;
      }
    };
    void operation.then(clearRecovery, clearRecovery);
    return operation;
  };

  /** Release renderer ownership without cancelling a daemon turn already running. */
  releaseOwnership = (): void => {
    this.ownershipGeneration += 1;
    this.ownershipReleased = true;
    this.stopObserving();
    // Closing a tab releases its renderer socket regardless of who opened the
    // turn. `/reply` is daemon-owned after admission, so aborting this reader is
    // a detach, not cancellation; the child/turn keeps running for its parent.
    this.activeStreamId += 1;
    this.abortController?.abort();
    this.abortController = null;
    void this.abandonContinuation();
  };

  /**
   * Join a turn this controller did not start, and render it from its
   * beginning (contract §2, §R1).
   *
   * The mechanics are a re-POST of the turn's own id: the server recognises the
   * duplicate and answers 200 + SSE with the replay backlog followed by the
   * live tail, instead of the old 409-and-no-stream. Everything after that is
   * the ordinary `/reply` pipeline — this is a DRIVER connection, not an
   * observer one, because it is a turn with an end, and a stream that ends
   * without a `Finish` means the same thing here as it does for the client that
   * started it.
   *
   * Seamlessness is not a property of this method alone; it is the three things
   * it sets up and then gets out of the way of:
   *   - `from_seq`/the sequence gate, so the half of the turn this client has
   *     already painted is not painted again;
   *   - the replay hold, so the half it has not lands in one commit rather than
   *     re-typing itself;
   *   - leaving `messages` alone. Nothing here clears the transcript, resets
   *     `chatState` to `LoadingConversation`, or replaces the controller, so
   *     the transcript component is never unmounted and the reader's scroll
   *     position is never touched. A visible attach would fail the requirement
   *     just as surely as a lost one.
   *
   * Resolves `true` when the stream was joined. A refusal — the turn is over,
   * the server does not know the id, the daemon is unreachable — resolves
   * `false` QUIETLY: an attach is a repair the user did not ask for, and
   * painting a red card because a turn had already finished would be inventing
   * a failure out of a non-event.
   */
  attachToTurn = async (turnId: string): Promise<boolean> => {
    if (!this.sessionId || !turnId) return false;
    this.ownershipReleased = false;
    // Never displace a turn this controller is already driving — including the
    // case where that turn IS this one, whose stream is live and whose frames
    // would then arrive twice over two sockets.
    if (this.hasLiveTurn()) return false;

    // An attach converts an observer tab into a driver, exactly as a submit
    // does, and for the same reason: the observer holds this controller's
    // `abortController`, and the attach is about to replace the field without
    // aborting what was there.
    this.stopObserving();

    // Re-attaching to the SAME turn keeps its high-water mark — that is the
    // whole point of the gate. Attaching to a different one starts from zero.
    this.activeTurnId = turnId;
    // Keyed on `seqTurnId`, not on `activeTurnId`: a turn given up on has had
    // its `activeTurnId` cleared but is still the turn whose frames are on
    // screen, and rejoining it must resume from what was painted rather than
    // from its start. A genuinely different turn starts from nothing.
    if (this.seqTurnId !== turnId) {
      this.seqTurnId = turnId;
      this.lastAppliedSeq = -1;
    }
    this.reattachesThisTurn = 0;
    // A fresh attach rebuilds its own replay accounting; carrying the previous
    // one over would measure this backlog against another attach's progress.
    this.replayReconstruction.clear();
    const fromSeq = this.lastAppliedSeq + 1;

    const streamId = this.activeStreamId + 1;
    this.activeStreamId = streamId;

    // The socket is held LOCALLY across the POST and only published to
    // `this.abortController` once there is a stream on it.
    //
    // That ordering is the whole defence against an attach the user never
    // asked for eating a message they did ask for. `hasLiveTurn()` — which is
    // what `canSubmitMessage` consults — is true the moment `abortController`
    // is set, so publishing it before the POST resolves would make a submit
    // arriving in that window return silently, with the typed message dropped
    // on the floor and no error anywhere. Most of these attaches are automatic
    // (see `resumeActiveTurn`) and some of them fail, so "a turn is in flight"
    // must not be claimed until it is known to be true. A submit that lands in
    // the window instead bumps `activeStreamId` past this one, and the guard
    // below stands the attach down.
    const socket = new AbortController();
    const continuationLease =
      this.continuationLeaseTurnId === turnId ? this.continuationLease : null;
    let streamTransportError = false;

    try {
      const { stream } = await reply({
        // Issue #56 Task 58 / #47: `/reply` is on the gated list, so REACHING a
        // private chat over it takes either a caller capability that covers the
        // chat or the user-action proof — see `routes/session_reach.rs`. An
        // attach is the same route, into the same chat, on behalf of the same
        // person: `submitPreparedMessage` sends the proof here and this did not,
        // which made every mid-turn reload of a private chat a 403 that the SSE
        // client turns into an empty stream. See `userActionHeaders`.
        headers: await userActionHeaders(),
        body: buildAttachRequest(
          this.sessionId,
          turnId,
          fromSeq,
          this.messagesRef,
          continuationLease ?? undefined
        ),
        throwOnError: true,
        signal: socket.signal,
        sseMaxRetryAttempts: 1,
        onSseError: () => {
          streamTransportError = true;
        },
      });
      if (this.activeStreamId !== streamId) {
        // Something real took the controller over while the POST was in flight
        // — a user submit, a Stop. It wins; this attach never happened.
        socket.abort();
        return false;
      }
      this.abortController = socket;
      this.updateSnapshot((prev) => ({
        ...prev,
        // Only now: the turn is known to exist. `turnStartedAt` is invented
        // only when there is nothing better — a re-attach keeps the original
        // origin so the elapsed timer does not restart in front of the user.
        chatState: ChatState.Streaming,
        turnStartedAt: prev.turnStartedAt ?? Date.now(),
        turnError: undefined,
        stopConfirmed: undefined,
      }));
      await this.streamFromResponse(
        stream as AsyncIterable<MessageEvent>,
        this.messagesRef,
        streamId,
        'driver',
        {
          onFirstEvent: () => this.releaseContinuationLeaseLocally(continuationLease),
          hadTransportError: () => streamTransportError,
        }
      );
      return true;
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') return false;
      console.warn(`Could not attach to turn ${turnId}:`, error);
      // Nothing to undo in the transcript and no error to show — see the doc
      // comment. `chatState` was never moved, so there is no flicker of a
      // working indicator for a turn that turned out to be over.
      if (this.activeStreamId === streamId) this.retireActiveTurn();
      return false;
    } finally {
      if (this.activeStreamId === streamId && this.abortController?.signal.aborted) {
        this.abortController = null;
      }
    }
  };

  /**
   * Rejoin whatever turn this session had in flight, if any. The reload path:
   * a window that dies mid-turn comes back, reads the transcript, finds the
   * turn still running on the daemon, and picks it up where it left off.
   *
   * Safe to call on every load — it is a no-op when there is no remembered
   * turn, when one is already live here, or when the id has expired.
   */
  resumeActiveTurn = async (turnId: string): Promise<boolean> => {
    if (!this.sessionId || !turnId || this.hasLiveTurn()) return false;
    // An OBSERVER tab already has everything an attach would give it, over a
    // feed that is strictly better suited to it: `/sessions/{id}/events` carries
    // the same `MessageEvent` frames for a session this window does not drive,
    // survives the turn ending, and reconnects itself. `hasLiveTurn()` is
    // `!observing && …`, so an observing tab did not block this — and
    // `attachToTurn` calls `stopObserving()` first, so opening a running
    // subagent's tab TORE DOWN a working feed and replaced it with a driver
    // socket. A rejoin is a repair for a window that lost its stream; a tab that
    // never had one has nothing to repair.
    if (this.observing) return false;
    // At most one automatic attempt per turn. Both `/agent/resume` calls report
    // `active_turn`, and `loadSession` is a no-op once a session is painted, so
    // this can be reached repeatedly for one turn — `handleSubmit` awaits
    // `loadSession` on every send. An un-latched auto-resume would therefore
    // fire again on the way into a submit, and that is not merely wasteful: the
    // attach it starts races the submit for the controller, and if it gets
    // there first the submit is refused as "a turn is already in flight" and
    // the user's message disappears without a trace. A rejoin is a one-shot
    // repair, not a policy.
    if (this.autoResumedTurnIds.has(turnId)) return false;
    this.autoResumedTurnIds.add(turnId);
    return this.attachToTurn(turnId);
  };

  /** Turns this controller has already tried, once, to rejoin by itself. */
  private autoResumedTurnIds = new Set<string>();

  /**
   * `/agent/resume` has told us which turn — if any — is running for this
   * session. Rejoin it.
   *
   * The server is the ONLY authority worth asking here, and this replaces an
   * earlier `localStorage` pointer written by whichever window started the
   * turn. That pointer could only ever be a guess: it went stale when a window
   * died without clearing it, it could not see a turn started by the CLI or a
   * scheduled run, and a stale one aimed an attach at a turn that had already
   * finished — whose backlog would then have re-rendered a turn the session
   * load had just painted. `state.rs::active_turn_id` filters on
   * `finished_at.is_none()`, so `active_turn` is present exactly when there is
   * something to join. Staleness stops being a category of bug rather than
   * being handled.
   *
   * Fire-and-forget by design: this runs off the back of a response the
   * transcript has already been painted from, and an attach must never be
   * something the user waits behind.
   */
  private noteActiveTurn(activeTurn: ActiveTurnRef | null | undefined): void {
    if (!activeTurn?.turn_id) return;
    if (this.retiredObservedTurnIds.has(activeTurn.turn_id)) return;
    if (this.activeTurnId && this.activeTurnId !== activeTurn.turn_id) return;
    // Observer tabs do not attach to `/reply`, but explicit Stop still needs the
    // daemon's exact generation. Retain the pointer even when resumeActiveTurn
    // correctly refuses to replace the observer feed.
    this.activeTurnId = activeTurn.turn_id;
    if (this.observing) {
      this.observedTurnResolvedGeneration = this.observedTurnGeneration;
      this.markObservedTurnRunning();
    }
    void this.resumeActiveTurn(activeTurn.turn_id);
  }

  private markObservedTurnRunning(): void {
    if (!this.observing || !this.activeTurnId) return;
    this.updateSnapshot((prev) => {
      const chatState = isRunningState(prev.chatState) ? prev.chatState : ChatState.Streaming;
      const turnStartedAt = prev.turnStartedAt ?? Date.now();
      if (
        prev.chatState === chatState &&
        prev.turnStartedAt === turnStartedAt &&
        prev.turnError === undefined &&
        prev.stopConfirmed === undefined
      ) {
        return prev;
      }
      return {
        ...prev,
        chatState,
        turnStartedAt,
        turnError: undefined,
        stopConfirmed: undefined,
      };
    });
  }

  private observedTerminalTargetsCurrentTurn(event: MessageEvent): boolean {
    if (!this.observing) return true;
    // The observer snapshot is newer than anything already queued on its bus
    // receiver. An unclaimed or older terminal cannot retire the exact
    // successor the authoritative snapshot named.
    if (this.activeTurnId) return frameTurnId(event) === this.activeTurnId;
    return this.observedTurnResolvedGeneration !== this.observedTurnGeneration;
  }

  private applyObservedTurnState(activeTurnId: string | null): void {
    if (!this.observing) return;
    this.observedTurnResolvedGeneration = this.observedTurnGeneration;
    const nextTurnId =
      activeTurnId && !this.retiredObservedTurnIds.has(activeTurnId) ? activeTurnId : null;
    if (nextTurnId) {
      this.noteChildInitialization(false);
      this.activeTurnId = nextTurnId;
      this.updateSnapshot((prev) =>
        prev.chatState === ChatState.Thinking ? { ...prev, chatState: ChatState.Streaming } : prev
      );
      this.markObservedTurnRunning();
      return;
    }

    // Sampled before the retirement, for the reason `stopGateHolds()` documents.
    const stopGateHolds = this.stopGateHolds();
    this.rememberRetiredObservedTurn(this.activeTurnId);
    this.retireActiveTurn();
    this.ambiguousRetryTurnId = null;
    this.updateSnapshot((prev) => ({
      ...prev,
      chatState: stopGateHolds ? prev.chatState : ChatState.Idle,
      turnStartedAt: undefined,
      lastMessageAt: undefined,
      pendingSteer: undefined,
      pendingToolCalls: [],
    }));
  }

  private submitPreparedMessage = async (
    newMessage: Message,
    currentMessages: Message[],
    updateMessageList: boolean
  ): Promise<void> => {
    this.ownershipReleased = false;
    // BR-71 — a user-driven turn converts an observer tab into a driver.
    // Detach FIRST and properly: the observer holds this controller's
    // `abortController` and a live socket, and the turn about to start replaces
    // the field without aborting what was there, so a bare flag clear would
    // leave the feed streaming into the transcript alongside the user's own
    // reply. `stopObserving()` is a no-op on a controller that was already
    // driving, which is every ordinary submit.
    this.stopObserving();
    // This is a genuinely new provider turn. It consumes any continuation lease
    // from the preceding Stop and supersedes an older ambiguous-retry pointer.
    this.lastSettledStopTurnId = null;
    this.ambiguousRetryTurnId = null;

    // #67 — launching a turn invalidates the completeness claim, whatever that
    // turn goes on to do. `updateMessages` drops it on every path that appends a
    // message, but `retryTurn` re-sends the row already at the tail
    // (`updateMessageList: false`) and so reaches neither it nor `loadSession`,
    // which is a no-op once loaded. A resume's claim therefore used to survive a
    // retry that died before delivering a single frame, while the server may
    // already have persisted rows this client was never shown. Clearing it here
    // covers the submit itself, so the property does not depend on which of the
    // two branches below runs.
    //
    // This becomes redundant once the client consumes `MessagesPersisted`: with
    // the ids a turn persisted folded into a per-session set, a watched turn
    // could KEEP the claim rather than drop it. See `viewNamesEveryStoredRow`.
    this.viewNamesEveryStoredRow = false;
    if (updateMessageList) {
      this.updateMessages(currentMessages);
    }

    this.updateSnapshot((prev) => ({
      ...prev,
      chatState: ChatState.Streaming,
      notifications: [],
      pendingToolCalls: [],
      turnError: undefined,
      // F5 — "Stopped." spoke about the previous turn; this one supersedes it.
      stopConfirmed: undefined,
      turnStartedAt: Date.now(),
      lastMessageAt: undefined,
      pendingSteer: undefined,
    }));
    // #22 — turn boundary: the user's own message and the working indicator
    // must paint immediately on submit, never an animation frame late.
    this.flushNotify();
    this.abortController = new AbortController();
    const streamId = this.activeStreamId + 1;
    this.activeStreamId = streamId;
    // BR-62b: one idempotency key per turn, sent in the body so an SSE
    // reconnect re-POST carries the same key and the server dedupes it.
    const turnId = newTurnId();
    const continuationLease = this.continuationLease;
    if (continuationLease) this.continuationLeaseTurnId = turnId;
    const queuedInitializingChildMessage =
      !continuationLease &&
      this.childInitializing &&
      this.snapshot.session?.session_type === 'sub_agent'
        ? newMessage
        : undefined;
    // The live-turn stream contract promotes that key into an ATTACH handle:
    // re-POSTing it rejoins this turn instead of starting a new one, which is
    // how THIS controller recovers a dropped socket (`reattachAfterDrop`).
    //
    // It is not how anyone ELSE finds the turn. The key exists only in this
    // renderer's heap, so a reloaded window or a window receiving a moved tab
    // has never seen it; they get the server's own name for the turn from
    // `/agent/resume`'s `active_turn`, and `/reply` matches on either name.
    // Start this turn's sequence numbering from nothing.
    this.activeTurnId = turnId;
    this.seqTurnId = turnId;
    this.lastAppliedSeq = -1;
    this.reattachesThisTurn = 0;
    // #166 — a new turn supersedes the generation a previous failed Stop kept
    // naming for its retry. Without this the retained id survives past the turn
    // it belonged to and a much later Stop-and-Send, made with nothing running,
    // would cancel a generation that has been dead for several turns.
    this.stopExpectedTurnId = null;
    this.stopContinuationPending = false;
    // …and with it the deferred ending that generation was owed. A new turn
    // owns `chatState` from here on; completing an older turn's withheld
    // transition against it would land this live turn at Idle.
    this.stopDeferredFinishTurnId = null;

    try {
      // The transcript paints before the agent's model + extensions are up, so
      // the user can submit into a session whose backend agent is not ready.
      // HOLD the turn here rather than dropping or blocking it:
      //  - not dropped: the message is already appended to the transcript above
      //    and goes out the instant the agent lands, so the composer never eats
      //    the first thing you type.
      //  - not blocked: we are already in a Streaming state with a live
      //    abortController, so the user sees their message and a working
      //    indicator, and Stop works throughout.
      // Resolves immediately (a microtask) once the agent is ready, which is the
      // overwhelmingly common case.
      await this.whenAgentReady();
      if (this.abortController?.signal.aborted || this.activeStreamId !== streamId) {
        await this.abandonContinuationIfOwned(continuationLease);
        return;
      }

      let streamTransportError = false;
      const { stream } = await reply({
        // Issue #56 Task 58 / #47: `/reply` runs an agent turn, with tools, in
        // whatever session the body names, and `session_id` is a request
        // parameter rather than a credential — so a turn in a PRIVATE chat now
        // needs the same proof-of-user the model picker sends. The renderer is
        // the user's surface, so every call it makes is a user act; the model
        // reaching this route over HTTP is precisely the caller the header
        // separates out.
        headers: await userActionHeaders(),
        body: {
          session_id: this.sessionId,
          user_message: newMessage,
          // BR-62b: idempotency key for this turn — see `newTurnId`.
          turn_id: turnId,
          // BR-63: the composer's per-turn reasoning effort. Omitted on the
          // default ('normal'), so a session-level `/effort` still applies.
          reasoning_effort: reasoningEffortForRequest(),
          ...(continuationLease ? { continuation_lease: continuationLease } : {}),
        } as ChatRequest,
        throwOnError: true,
        signal: this.abortController.signal,
        sseMaxRetryAttempts: 1,
        onSseError: () => {
          streamTransportError = true;
        },
      });

      await this.streamFromResponse(
        stream as AsyncIterable<MessageEvent>,
        currentMessages,
        streamId,
        'driver',
        {
          onFirstEvent: () => this.releaseContinuationLeaseLocally(continuationLease),
          queuedInitializingChildMessage,
          hadTransportError: () => streamTransportError,
        }
      );
    } catch (error) {
      if (error instanceof Error && error.name === 'AbortError') {
        await this.abandonContinuationIfOwned(continuationLease);
        return;
      }
      // A connection failure can happen after the daemon accepted the POST but
      // before this renderer received the response. Preserve the exact key so
      // Retry attaches/replays instead of purchasing a duplicate turn.
      if (isConnectionError(error)) {
        this.ambiguousRetryTurnId = turnId;
      } else {
        await this.abandonContinuationIfOwned(continuationLease);
      }
      await this.finishCurrentStream(clientTurnError(error, 'submit_error', 'inference'));
    } finally {
      if (this.activeStreamId === streamId && this.abortController?.signal.aborted) {
        this.abortController = null;
      }
    }
  };

  private canSubmitMessage(): boolean {
    return (
      !!this.snapshot.session &&
      this.snapshot.chatState !== ChatState.LoadingConversation &&
      !this.stopInFlight &&
      !this.stopPending &&
      this.snapshot.pendingContinuation?.ownership !== 'foreign' &&
      this.snapshot.pendingContinuation?.ownership !== 'settling' &&
      // BR-71: an OBSERVER's subscription is not a turn in flight. Reading it
      // as one silently drops every message typed into a daemon-opened tab —
      // the takeover §4.3 promises ("until the tab detaches or the user takes
      // the session over") would be unreachable, and `submitPreparedMessage`'s
      // own conversion back to driver would be dead code.
      !this.hasLiveTurn()
    );
  }

  submitSystemMessage = async (message: Message): Promise<void> => {
    await this.loadSession();

    if (!this.canSubmitMessage()) {
      return;
    }

    this.lastInteractionTime = Date.now();
    const currentMessages = [...this.messagesRef, message];
    await this.submitPreparedMessage(message, currentMessages, true);
  };

  /**
   * Send a user message, reporting whether the submit was ACCEPTED.
   *
   * `false` means one thing only: the submit was refused before anything
   * happened, nothing was shown to the user, and **the caller still owns the
   * message**. Every such return is silent by design (a re-entrant submit, a
   * controller with no session, a turn already live), so a caller that drops
   * the text on a `false` drops it with no trace, which is exactly the
   * message-loss bug this return value exists to close. Callers that hold the
   * user's words (the composer, the queue drain) must put them back.
   *
   * `true` means the submit took responsibility for the message: a turn was
   * launched, or the attempt failed loudly enough that the user can see it and
   * decide (see the preparation-failure branch below). It is NOT a claim that
   * the turn succeeded: the promise resolves when the turn ENDS, whatever
   * happened during it.
   */
  handleSubmit = async (
    userMessage: string,
    attachments: UserAttachment[] = []
  ): Promise<boolean> => {
    // R3-01: bail synchronously on a re-entrant submit (double-click) so the
    // second call never reaches the async prep that appends a duplicate user
    // turn. Held across the whole submit; `canSubmitMessage`'s abortController
    // guard takes over the moment the turn is actually launched.
    //
    // ⚠ This latch is held until the turn's promise chain unwinds, and the
    // `isLoading` edge a queue drain waits for is produced INSIDE that chain
    // (`finishCurrentStream` flushes `ChatState.Idle` while this call is still
    // awaiting). So the first drain attempt of every turn can legitimately land
    // here. Returning `false` rather than nothing is what lets the drain keep
    // the message and re-offer it instead of discarding it silently.
    if (this.submitInFlight) {
      return false;
    }
    this.submitInFlight = true;
    try {
      await this.loadSession();

      if (!this.canSubmitMessage()) {
        return false;
      }

      const hasExistingMessages = this.messagesRef.length > 0;
      const hasNewMessage = userMessage.trim().length > 0 || attachments.length > 0;
      if (!hasNewMessage && !hasExistingMessages) {
        return false;
      }

      this.lastInteractionTime = Date.now();
      if (userMessage.trim().length > 0) {
        this.lastSubmittedTitle = userMessage.trim().slice(0, 80);
      } else if (attachments.length > 0) {
        this.lastSubmittedTitle = `${attachments.length} attachment${attachments.length === 1 ? '' : 's'}`;
      }

      if (!hasExistingMessages && hasNewMessage) {
        window.dispatchEvent(new CustomEvent('session-created'));
      }

      let newMessage: Message;
      if (hasNewMessage) {
        try {
          newMessage = await createUserMessage(userMessage, attachments);
        } catch (error) {
          await this.finishCurrentStream(
            clientTurnError(error, 'message_preparation_failed', 'inference')
          );
          // ACCEPTED, deliberately: this failure paints a turn error the user
          // can see and retry from, so the caller must not also re-queue or
          // repopulate the composer behind it. Re-offering would re-run the
          // same failing preparation (attachment decode, file read) against the
          // same inputs and fail identically.
          return true;
        }
      } else {
        newMessage = this.messagesRef[this.messagesRef.length - 1];
      }

      const currentMessages = hasNewMessage
        ? [...this.messagesRef, newMessage]
        : [...this.messagesRef];
      await this.submitPreparedMessage(newMessage, currentMessages, hasNewMessage);
      return true;
    } finally {
      this.submitInFlight = false;
    }
  };

  /**
   * Re-run the last turn after a RETRYABLE failure — a backend/provider blip on
   * send, a dropped stream, or a transient cold-load failure while biorouterd
   * was restarting. Safe to call repeatedly and safe to double-fire:
   *
   *  - Bails if a turn is already live (an in-flight, un-aborted controller), so
   *    a stray double-click can never launch a second concurrent turn.
   *  - Clears the inline error being retried past.
   *  - Re-attempts the session load. This is a no-op once the session is loaded,
   *    and re-runs a cold load that failed while the daemon was briefly down —
   *    which is what repaints a transcript the fatal-card bug used to discard.
   *  - Re-submits the TRAILING user message EXACTLY ONCE, reusing the message
   *    already at the tail of the transcript (`updateMessageList: false`), so no
   *    duplicate user turn is ever appended to the store. If there is no trailing
   *    user turn (nothing was ever sent on this controller, e.g. a pure mount
   *    load failure) the reload alone is the retry.
   */
  private retryTurnOnce = async (): Promise<void> => {
    // BR-71: same reading as `canSubmitMessage` — an observer's feed is not a
    // turn this controller is running, and Retry in an observed tab is a
    // takeover like any other submit.
    if (this.hasLiveTurn()) return;
    const previousError = this.snapshot.turnError;
    this.updateSnapshot((prev) => ({ ...prev, turnError: undefined }));

    await this.loadSession();
    if (!this.canSubmitMessage()) return;

    const last = this.messagesRef[this.messagesRef.length - 1];
    if (!last || last.role !== 'user') return;

    this.lastInteractionTime = Date.now();
    const ambiguousTurnId = this.ambiguousRetryTurnId;
    if (ambiguousTurnId) {
      const ambiguousLease =
        this.continuationLeaseTurnId === ambiguousTurnId ? this.continuationLease : null;
      const attached = await this.attachToTurn(ambiguousTurnId);

      // ⚠ **The daemon can resolve the ambiguity, and when it does, this press
      // finishes the job.**
      //
      // Retry after "Connection dropped" took TWO presses, and the first press
      // replaced the card with a scarier "Model turn ended unexpectedly". The
      // reason is here: the pointer makes press one an ATTACH, which returns
      // before the resubmit below. If that attach reaches a turn with no writer,
      // `TurnStream::close` answers with `stream_ended_without_terminal` — the
      // daemon stating that the turn is over and produced nothing. That is an
      // authoritative answer, not a transport guess: the ambiguity the pointer
      // exists for is gone, and the user's press should carry on to the
      // resubmit instead of being spent on a round trip that reports a failure
      // the model never had. (Same reading as `reframeStoppedTurnError`: the
      // daemon describes the SHAPE of an ending, never its cause.)
      //
      // ⚠ And ONLY for that answer. An attach that could not be made at all —
      // the POST threw, the daemon is unreachable — leaves the turn's fate
      // genuinely unknown, and it may still be running server-side; resubmitting
      // there would run the user's work twice. That path keeps its second press,
      // which is a confirmation, not a bug.
      if (attached && this.snapshot.turnError?.code === STREAM_ENDED_WITHOUT_TERMINAL) {
        this.ambiguousRetryTurnId = null;
        // Cleared BEFORE the next await so the internal-scope card cannot paint
        // for a frame on its way out.
        this.updateSnapshot((prev) => ({ ...prev, turnError: undefined }));
        await this.abandonContinuationIfOwned(ambiguousLease);
        // Re-read the tail: the attach ran a stream, and `last` was captured
        // before it. Resubmitting a message that is no longer the trailing turn
        // would append a duplicate.
        const tail = this.messagesRef[this.messagesRef.length - 1];
        if (!this.hasLiveTurn() && this.canSubmitMessage() && tail && tail.role === 'user') {
          await this.submitPreparedMessage(tail, [...this.messagesRef], false);
        }
        return;
      }

      if (!attached && this.ambiguousRetryTurnId === ambiguousTurnId) {
        this.ambiguousRetryTurnId = null;
        await this.abandonContinuationIfOwned(ambiguousLease);
        this.updateSnapshot((prev) => ({
          ...prev,
          turnError:
            previousError ??
            ({
              message: 'Biorouter could not reconnect to the previous turn.',
              technicalDetails: 'The exact turn could not be re-attached.',
              code: 'retry_attach_failed',
              scope: 'transport',
              retryable: true,
            } satisfies ChatTurnErrorData),
        }));
      }
      return;
    }
    await this.submitPreparedMessage(last, [...this.messagesRef], false);
  };

  retryTurn = (): Promise<void> => {
    if (this.retryInFlight) return this.retryInFlight;
    const operation = this.retryTurnOnce();
    let tracked!: Promise<void>;
    tracked = operation.finally(() => {
      if (this.retryInFlight === tracked) this.retryInFlight = null;
    });
    this.retryInFlight = tracked;
    return tracked;
  };

  submitElicitationResponse = async (
    elicitationId: string,
    userData: Record<string, unknown>
  ): Promise<void> => {
    await this.loadSession();

    if (!this.canSubmitMessage()) {
      return;
    }

    this.lastInteractionTime = Date.now();
    const responseMessage = createElicitationResponseMessage(elicitationId, userData);
    const currentMessages = [...this.messagesRef, responseMessage];

    await this.submitPreparedMessage(responseMessage, currentMessages, true);
  };

  setWorkflowUserParams = async (user_workflow_values: Record<string, string>): Promise<void> => {
    if (this.snapshot.session) {
      await updateSessionUserWorkflowValues({
        path: {
          session_id: this.sessionId,
        },
        body: {
          userWorkflowValues: user_workflow_values,
        },
        throwOnError: true,
      });
      this.updateSnapshot((prev) =>
        prev.session
          ? {
              ...prev,
              session: {
                ...prev.session,
                user_workflow_values,
              },
            }
          : prev
      );
    } else {
      this.updateSnapshot((prev) => ({
        ...prev,
        sessionLoadError: "can't call setWorkflowParams without a session",
      }));
    }
  };

  /**
   * BR-61 — soft interrupt ("steer"). Injects `text` into the turn that is
   * *already running*, at the agent's next loop boundary, instead of cancelling
   * it and re-sending the whole context: in-flight tool work is kept and the
   * model simply sees the new instruction on its next step.
   *
   * Resolves `false` when there is nothing to steer (no turn in flight, empty
   * text) or the server rejected the interrupt — callers must then fall back to
   * sending the text as an ordinary message, so it is never silently dropped.
   *
   * The injected message is NOT pushed locally: the agent streams it back as a
   * normal user message once it is consumed, which is also the only reliable
   * signal that it landed.
   */
  steer = async (text: string): Promise<boolean> => {
    const trimmed = text.trim();
    if (!trimmed || !this.isRunning()) {
      return false;
    }
    // Show it BEFORE the round-trip, not after. The POST itself can take a
    // moment and the agent only consumes the steer at its next loop boundary —
    // which may be a whole tool call away — so waiting for either would leave
    // the user staring at a composer that just emptied itself for no visible
    // reason. If the server refuses, the catch below retracts this.
    this.steerAfterCount = this.messagesRef.length;
    // Held so the catch can prove the chip it retracts is still ITS OWN. A
    // second steer issued while this POST is in flight replaces `pendingSteer`
    // wholesale; retracting unconditionally would then wipe the newer steer's
    // chip even though that steer is still genuinely pending — and it would
    // never come back, because nothing re-shows a chip for an in-flight steer.
    const issued = { text: trimmed, since: Date.now() };
    this.updateSnapshot((prev) => ({
      ...prev,
      pendingSteer: issued,
    }));
    try {
      const interruptRequest = {
        session_id: this.sessionId,
        text: trimmed,
        // The server uses this only while a delegated child is still queued;
        // there it makes a lost-response retry idempotent. Once the live agent
        // loop exists, the ordinary soft-interrupt path ignores the key.
        turn_id: newTurnId(),
      };
      await interrupt({
        body: interruptRequest,
        headers: await userActionHeaders(),
        throwOnError: true,
      });
      this.lastInteractionTime = Date.now();
      return true;
    } catch (error) {
      // 409 = the turn ended between the click and the POST; the caller queues
      // or sends it instead. Retract the optimistic chip in the same breath —
      // leaving "Steering…" up while the text is actually taking the ordinary
      // send path would be the UI telling the user something untrue.
      if (this.getSnapshot().pendingSteer === issued) {
        this.clearPendingSteer();
      }
      console.warn('Soft interrupt rejected, falling back to a normal send:', error);
      return false;
    }
  };

  /**
   * Whether the Stop gate still speaks for the turn that is ending right now.
   *
   * #166 belt-and-braces. `stopPending` alone is a process-wide latch: any path
   * that arms it and then fails to resolve pins EVERY later terminal frame at
   * its running value, which is the composer's working edge sweeping forever
   * over a finished chat. Scoping it to the generation it was armed for bounds
   * that blast radius to one turn — a terminal frame for a different, later
   * turn always returns to Idle, whatever the latch says.
   *
   * Evaluate this BEFORE `retireActiveTurn()`: retiring nulls `activeTurnId`,
   * and a null there reads as "cannot disprove the gate", so a check made
   * afterwards answers the opposite of one made before.
   */
  private stopGateHolds(): boolean {
    if (!this.stopPending) return false;
    if (!this.stopExpectedTurnId || !this.activeTurnId) return true;
    return this.stopExpectedTurnId === this.activeTurnId;
  }

  /** Where a Stop-path log line says which chat and which generation it means. */
  private stopLogContext(turnId: string): string {
    return `session ${this.sessionId}, turn ${turnId}`;
  }

  private requestExactTurnSettlement = async (
    turnId: string,
    continuationPending: boolean
  ): Promise<boolean> => {
    this.lastStopFailure = null;
    this.lastStopCancelled = false;
    try {
      const body = {
        session_id: this.sessionId,
        expected_turn_id: turnId,
        wait_for_idle: true,
        continuation_pending: continuationPending,
        ...(continuationPending ? { continuation_owner_id: getContinuationOwnerId() } : {}),
      };
      // `throwOnError: false` deliberately. Asked to throw, the generated client
      // throws the response BODY and drops the `Response` — and `/agent/cancel`
      // answers its most important failure, the 30 s `SettlementTimeout`, with a
      // bare 504 and no body at all. So the thrown value was a literal `{}` and
      // the status, the one piece of information that existed, was gone before
      // the catch could see it. The fields form keeps both. A rejection is still
      // possible (a fetch that never reaches a response) and still handled below.
      const result = await cancelTurn({
        body,
        headers: await userActionHeaders(),
        throwOnError: false,
      });
      if (result?.error !== undefined) {
        return this.noteCancelRejection(turnId, result.error, result.response?.status);
      }
      const data = result?.data;
      if (data?.settled === true) {
        this.lastStopCancelled = data.cancelled === true;
        if (!continuationPending) return true;
        const lease = data.continuation_lease;
        if (!lease) {
          this.lastStopFailure = 'the Stop-and-Send response carried no continuation lease';
          console.warn(
            `Stop-and-Send response did not include a continuation lease (${this.stopLogContext(turnId)})`
          );
          return false;
        }
        if (this.continuationLease && this.continuationLease !== lease) {
          await this.abandonContinuation();
        }
        if (this.ownershipReleased) {
          await this.abandonLeaseWithRetry(lease);
        } else {
          this.continuationLease = lease;
          this.continuationLeaseTurnId = null;
          this.updateSnapshot((prev) => ({
            ...prev,
            pendingContinuation: {
              ownership: 'owned',
              supersededTurnId: turnId,
            },
          }));
        }
        return true;
      }
      this.lastStopFailure = `the daemon answered the cancel without confirming the turn settled (cancelled=${String(
        data?.cancelled
      )}, settled=${String(data?.settled)})`;
      console.warn(
        `Cancel response did not confirm that the stopped turn settled (${this.stopLogContext(turnId)}): ${this.lastStopFailure}`
      );
    } catch (error) {
      // A rejection now means the fetch never produced a response at all (the
      // backend is down, the request was aborted). An HTTP failure comes back
      // through `result.error` above, with its status intact.
      return this.noteCancelRejection(turnId, error, undefined);
    }
    return false;
  };

  /**
   * Record and report a cancel that failed, and adopt the successor when the
   * daemon named one.
   *
   * Shared by the two ways a failure can arrive — an HTTP error (`result.error`
   * plus its status) and a rejected fetch — so neither can drift into logging
   * less than the other. M2: what it must never do again is hand a raw value to
   * `console.warn`; `describeRequestFailure` is the only thing that knows a
   * bodyless 504 from a real payload.
   */
  private noteCancelRejection(turnId: string, error: unknown, status: number | undefined): boolean {
    const mismatch = cancelTurnMismatch(error);
    if (mismatch && mismatch.expected_turn_id === turnId) {
      this.rememberRetiredObservedTurn(turnId);
      this.activeStreamId += 1;
      this.abortController?.abort();
      this.abortController = null;
      this.endReplayHold();
      this.activeTurnId = this.retiredObservedTurnIds.has(mismatch.active_turn_id)
        ? null
        : mismatch.active_turn_id;
      this.reattachesThisTurn = 0;
      this.stopPending = false;
      this.stopExpectedTurnId = null;
      this.stopContinuationPending = false;
      this.lastSettledStopTurnId = null;
      this.updateSnapshot((prev) => ({
        ...prev,
        chatState: this.activeTurnId ? ChatState.Streaming : ChatState.Idle,
        turnStartedAt: this.activeTurnId ? (prev.turnStartedAt ?? Date.now()) : undefined,
        lastMessageAt: this.activeTurnId ? prev.lastMessageAt : undefined,
      }));
      console.warn(
        `Stop targeted retired turn ${turnId}; the active turn is ${mismatch.active_turn_id}`
      );
      // Deliberately leaves `lastStopFailure` null: this arm has already
      // resolved the whole state (successor adopted, `chatState` chosen), and
      // an in-chat "could not stop" notice written over it would be untrue as
      // well as destructive.
      return false;
    }
    const detail = describeRequestFailure(error, status);
    this.lastStopFailure = detail;
    console.warn(
      `Failed to cancel running turn on stop (${this.stopLogContext(turnId)}): ${detail}`
    );
    return false;
  }

  private trackStopOperation(
    operation: Promise<boolean>,
    continuationPending: boolean
  ): Promise<boolean> {
    let tracked!: Promise<boolean>;
    tracked = operation.finally(() => {
      if (this.stopInFlight === tracked) {
        this.stopInFlight = null;
        this.stopInFlightContinuationPending = false;
      }
    });
    this.stopInFlight = tracked;
    this.stopInFlightContinuationPending = continuationPending;
    return tracked;
  }

  private settleStoppedTurn = async (
    stoppedTurnId: string,
    requestContinuationPending: boolean
  ): Promise<boolean> => {
    let settled = false;
    try {
      settled = await this.requestExactTurnSettlement(stoppedTurnId, requestContinuationPending);
    } finally {
      // #166 — the gate exists to bridge the microtask race between a terminal
      // SSE frame and the cancel response. Once that response has resolved the
      // race is over, whichever way it went, and holding the gate past it buys
      // nothing: a Stop whose cancel timed out or came back unconfirmed used to
      // latch `stopPending` true for the rest of the chat's life, so every
      // later terminal frame left `chatState` pinned at its running value. The
      // composer's working edge swept on over a finished turn and the composer
      // never offered Send again — the whole of #166.
      //
      // A `finally` rather than a branch on `settled`, so no early return added
      // here later can reintroduce the strand.
      //
      // `stopExpectedTurnId` and `stopContinuationPending` are deliberately NOT
      // cleared: they are the memory a retry needs to name the exact generation
      // the user stopped (and their Stop-and-Send intent), and dropping them
      // would turn a retry into a session-only cancel that fails closed. They
      // are superseded by a live turn rather than by this failure — see
      // `stopStreaming` and `submitPreparedMessage`.
      this.stopPending = false;
    }
    if (!settled) {
      this.reportUnconfirmedStop(stoppedTurnId);
      return false;
    }

    // F5 — say that it worked, but only when the daemon says the Stop is what
    // ended the turn. Sampled BEFORE `stopContinuationPending` is cleared below:
    // an ordinary Stop the user upgraded to Stop-and-Send while it was on the
    // wire is followed at once by their replacement turn, and that turn is the
    // outcome — not a line that flashes for the length of one submit.
    const confirmedStop: StopConfirmedView | undefined =
      this.lastStopCancelled && !requestContinuationPending && !this.stopContinuationPending
        ? { turnId: stoppedTurnId }
        : undefined;

    this.activeStreamId += 1;
    this.abortController?.abort();
    this.endReplayHold();
    this.retireActiveTurn();
    this.ambiguousRetryTurnId = null;
    this.lastSettledStopTurnId = stoppedTurnId;
    this.stopPending = false;
    this.stopExpectedTurnId = null;
    this.stopContinuationPending = false;
    this.stopDeferredFinishTurnId = null;
    this.updateSnapshot((prev) => ({
      ...prev,
      chatState: ChatState.Idle,
      turnStartedAt: undefined,
      lastMessageAt: undefined,
      pendingSteer: undefined,
      // A retry that succeeded retracts the notice the failed attempt raised.
      // A confirmed stop also retracts M2's interim "Turn stopped" card — a
      // wedged writer's synthesized ending raises it while the cancel is still
      // out — because it says what `stopConfirmed` says, in an error's voice.
      turnError:
        prev.turnError?.code === STOP_NOT_CONFIRMED ||
        (confirmedStop && prev.turnError?.code === TURN_STOPPED_BY_USER)
          ? undefined
          : prev.turnError,
      ...(confirmedStop ? { stopConfirmed: confirmedStop } : {}),
    }));
    if (confirmedStop) this.retractStopConfirmedLater(confirmedStop);
    this.flushNotify();
    return true;
  };

  /**
   * F5 — the confirmed-stop line is transient. Identity-checked, so a timer
   * armed for one notice can never retract a later one, and a notice a new
   * turn already retracted is left alone. Untracked on purpose: a stale timer
   * is a no-op, and the registry never drops a controller outside
   * `resetForTests`.
   */
  private retractStopConfirmedLater(notice: StopConfirmedView): void {
    setTimeout(() => {
      this.updateSnapshot((prev) =>
        prev.stopConfirmed === notice ? { ...prev, stopConfirmed: undefined } : prev
      );
    }, STOP_CONFIRMED_NOTICE_MS);
  }

  /**
   * M2 — finish what the Stop gate deferred, and say out loud that the stop did
   * not take.
   *
   * Two things were missing when a cancel came back a failure, and they are
   * separate bugs that presented as one dead chat:
   *
   *  1. If the turn's terminal frame had ALREADY landed, `finishCurrentStream`
   *     deferred the Idle transition to this response (`stopGateHolds()`), and
   *     this response then returned without making it. #166's `finally` frees
   *     the latch — so a LATER terminal frame lands normally — but the frame
   *     that mattered has already been and gone. Nothing re-runs it. That is
   *     the composer with a Stop button, no Send button and a spinner over a
   *     turn that finished a minute ago.
   *  2. The failure was silent. The only trace was a `console.warn` the user
   *     cannot see, so a stop that did not work was indistinguishable from one
   *     that did.
   *
   * The restoration is deliberately NOT unconditional. A cancel that failed
   * while the turn is genuinely still streaming leaves the daemon holding the
   * session lock: offering Send there would buy a 409, and Stop is the control
   * the user actually wants (the retained `stopExpectedTurnId` makes a second
   * press name the same generation). So only the deferred case is completed,
   * and the notice says which of the two this was.
   */
  private reportUnconfirmedStop(stoppedTurnId: string): void {
    // Null on the typed-mismatch arm, which has already resolved everything.
    if (!this.lastStopFailure) return;
    const detail = this.lastStopFailure;
    this.lastStopFailure = null;

    const completesDeferredFinish = this.stopDeferredFinishTurnId === stoppedTurnId;
    if (completesDeferredFinish) {
      this.stopDeferredFinishTurnId = null;
      this.endReplayHold();
      this.retireActiveTurn();
    }
    this.updateSnapshot((prev) => ({
      ...prev,
      ...(completesDeferredFinish
        ? {
            chatState: ChatState.Idle,
            turnStartedAt: undefined,
            lastMessageAt: undefined,
            pendingSteer: undefined,
          }
        : {}),
      turnError: stopNotConfirmedError(completesDeferredFinish, detail),
    }));
    // A turn boundary the user is waiting on: paint it now, not a frame late.
    this.flushNotify();
  }

  private stopAfterObservedLookup = (continuationPending: boolean): Promise<boolean> | null => {
    if (!this.observing || this.activeTurnId) return null;
    const firstLookup = this.observedTurnLookup?.promise ?? this.refreshObservedActiveTurn();
    if (!firstLookup) return null;
    return this.trackStopOperation(
      (async () => {
        await firstLookup;
        if (!this.activeTurnId) {
          // A failed lookup deliberately leaves the observer generation
          // unresolved. Retry it once for this explicit Stop instead of
          // turning a transient resume failure into a silent no-op.
          if (this.observedTurnLookup?.promise === firstLookup) {
            this.observedTurnLookup = null;
          }
          const retry = this.refreshObservedActiveTurn();
          if (retry) await retry;
        }
        const stoppedTurnId = this.activeTurnId;
        if (!stoppedTurnId) return false;

        this.stopPending = true;
        this.stopExpectedTurnId = stoppedTurnId;
        this.stopContinuationPending = continuationPending;
        this.lastInteractionTime = Date.now();
        return this.settleStoppedTurn(stoppedTurnId, continuationPending);
      })(),
      continuationPending
    );
  };

  stopStreaming = (continuationPending = false): Promise<boolean> => {
    this.ownershipReleased = false;
    if (this.stopInFlight) {
      const inFlight = this.stopInFlight;
      if (!continuationPending || this.stopInFlightContinuationPending) return inFlight;

      // The ordinary Stop request already on the wire cannot be retroactively
      // upgraded. Wait for its exact-generation barrier, then mark that retained
      // retired generation before Stop-and-Send is allowed to submit.
      this.stopContinuationPending = true;
      return this.trackStopOperation(
        (async () => {
          if (!(await inFlight)) return false;
          const stoppedTurnId = this.stopExpectedTurnId ?? this.lastSettledStopTurnId;
          if (!stoppedTurnId) return false;
          return this.requestExactTurnSettlement(stoppedTurnId, true);
        })(),
        true
      );
    }

    const observedLookupStop = this.stopAfterObservedLookup(continuationPending);
    if (observedLookupStop) return observedLookupStop;

    // A live turn always outranks the generation a previous failed Stop named;
    // the retained id only speaks when nothing is running, which is exactly the
    // retry-after-a-failed-cancel case (#166).
    const targetTurnId = this.activeTurnId ?? this.stopExpectedTurnId;

    if (!this.stopPending && !targetTurnId) {
      // An ordinary Stop may have settled in the microtask immediately before
      // Stop-and-Send. The daemon retains that exact generation specifically so
      // this second admission can acquire the continuation lease without
      // cancelling (or guessing at) a successor.
      if (continuationPending && this.lastSettledStopTurnId) {
        return this.trackStopOperation(
          this.requestExactTurnSettlement(this.lastSettledStopTurnId, true),
          true
        );
      }
      return Promise.resolve(false);
    }

    if (!this.stopPending) {
      // Retrying the retained generation carries the original Stop-and-Send
      // intent forward; naming a live turn is a fresh Stop and takes only the
      // intent of this call, so a stale `true` cannot make an ordinary Stop
      // acquire a continuation lease nobody asked for.
      const retryingRetainedGeneration = !this.activeTurnId && !!this.stopExpectedTurnId;
      this.stopPending = true;
      this.stopExpectedTurnId = targetTurnId;
      this.stopContinuationPending =
        continuationPending || (retryingRetainedGeneration && this.stopContinuationPending);
    } else if (continuationPending) {
      this.stopContinuationPending = true;
    }
    const stoppedTurnId = this.stopExpectedTurnId;
    if (!stoppedTurnId) {
      // A session-only cancel can hit a successor that started while this
      // renderer was still loading. Without a turn generation to name, fail
      // closed and leave the load/turn state untouched.
      this.stopPending = false;
      this.stopContinuationPending = false;
      return Promise.resolve(false);
    }
    this.lastInteractionTime = Date.now();
    const requestContinuationPending = this.stopContinuationPending;

    // BR-62b: aborting the SSE socket only tears down this renderer. The exact
    // cancel trips the daemon turn and `wait_for_idle` is the successor's
    // admission barrier; a stale generation is refused rather than cancelling
    // whatever started after it.
    const operation = this.settleStoppedTurn(stoppedTurnId, requestContinuationPending);

    return this.trackStopOperation(operation, requestContinuationPending);
  };

  onMessageUpdate = async (
    messageId: string,
    newContent: string,
    editType: 'diverge' | 'edit' = 'diverge'
  ): Promise<void> => {
    try {
      const { editMessage } = await import('../api');
      const message = this.messagesRef.find((m) => m.id === messageId);

      if (!message) {
        throw new Error(`Message with id ${messageId} not found in current messages`);
      }

      // #51 NF-D: `edit` truncates the LIVE session, so the server checks the
      // cut against our view of it when we can supply one. `expectedMessageIds`
      // names every message we hold; if the session has moved on (another
      // window, the CLI, a scheduled run) the server refuses with 409 rather
      // than silently destroying what we never saw. `diverge` ignores it.
      //
      // We can only make that assertion when our view IS the stored set. Two
      // separate things have to hold, and it is a mistake to check only one:
      //
      //   1. every message we hold names itself — otherwise we cannot even list
      //      what we have; and
      //   2. we hold every row the store has (`viewNamesEveryStoredRow`).
      //
      // (2) does not follow from (1). #59 made the reply loop stamp an id on the
      // copy it yields, so from that point on (1) is true on turns where (2) is
      // false: one streamed reply is stored as two or three rows and only the
      // first keeps the id we were shown, and the model-only rows are never
      // yielded at all. Checking (1) alone would send a SHORT list — not a
      // weaker claim, a false one — and buy a guaranteed 409 on a session nobody
      // else has touched, i.e. kill this button in every live chat.
      //
      // So: send it when we just read the conversation back from the store, and
      // omit it otherwise. Omitted, the cut still runs under the server's turn
      // lock and still bounded to the rows the handler itself read. Making it
      // unconditional again means consuming `MessagesPersisted` — see
      // `viewNamesEveryStoredRow`.
      const namedIds = this.messagesRef.flatMap((m) => (typeof m.id === 'string' ? [m.id] : []));
      const expectedMessageIds =
        this.viewNamesEveryStoredRow && namedIds.length === this.messagesRef.length
          ? namedIds
          : undefined;

      const response = await editMessage({
        path: {
          session_id: this.sessionId,
        },
        body: {
          timestamp: message.created,
          editType,
          ...(expectedMessageIds ? { expectedMessageIds } : {}),
        },
        // Issue #56 DR-19: `diverge` branches this chat into a NEW session that
        // inherits its provider, so on a private chat it mints a new
        // private-capability session and the daemon refuses it without proof the
        // request came from the person at the keyboard. `edit` truncates this
        // session in place and mints nothing, so it is not gated and does not
        // carry the proof.
        ...(editType === 'diverge' ? { headers: await userActionHeaders() } : {}),
        throwOnError: true,
      });

      const targetSessionId = response.data?.sessionId;
      if (!targetSessionId) {
        throw new Error('No session ID returned from edit_message');
      }

      if (editType === 'diverge') {
        const event = new CustomEvent('session-diverged', {
          detail: {
            // The session diverged FROM. 'session-diverged' is a window
            // broadcast, and newSessionId names a session that doesn't exist in
            // the UI yet — so listeners identify the one chat that should
            // navigate by the ORIGIN session, which is this controller's own.
            sessionId: this.sessionId,
            newSessionId: targetSessionId,
            shouldStartAgent: true,
            editedMessage: newContent,
          },
        });
        window.dispatchEvent(event);
        window.electron?.logInfo(
          `Dispatched session-diverged event for session ${targetSessionId}`
        );
      } else {
        const sessionResponse = await getSession({
          path: { session_id: targetSessionId },
          // Issue #56 Task 58: reading a private chat needs the proof-of-user.
          headers: await userActionHeaders(),
          throwOnError: true,
        });

        if (sessionResponse.data?.conversation) {
          this.updateMessages(sessionResponse.data.conversation);
          // `GET /sessions/{id}` is the same `get_session(id, true)` read as
          // `/agent/resume` and as the freshness check itself, so the truncated
          // conversation we just read back is again provably the whole store.
          this.viewNamesEveryStoredRow = true;
        }
        await this.handleSubmit(newContent);
      }
    } catch (error) {
      const errorMsg = errorMessage(error);
      console.error('Failed to edit message:', error);
      const { toastError } = await import('../toasts');
      toastError({
        title: 'Failed to edit message',
        msg: errorMsg,
      });
    }
  };
}

export class ChatStreamRegistry {
  private controllers = new Map<string, ChatStreamController>();
  private runningListeners = new Set<() => void>();
  private running = new Map<string, RunningChatEntry>();
  private lastRunningSnapshot: RunningChatEntry[] = [];
  private stopSessionMeta: (() => void) | null = null;
  private tierListeners = new Set<() => void>();
  private sessionTiers: Record<string, SessionClassification> = {};

  /**
   * Follow session rows this renderer holds, for changes made by ANOTHER
   * PROCESS — `biorouter session --resume <id> --provider …`, a schedule run in
   * a different daemon. A second window is covered by the broadcast in
   * `sessionBindingSync`, and this window's own writes by the announcement and
   * the reply stream; none of those can see another process.
   *
   * ⚠ **Started here, once, and never per chat.** The registry is a module-level
   * singleton, so this is the one place in the renderer where "once" is
   * structural rather than a discipline a caller has to keep. A subscription
   * restarted per chat would poll at a revision it is already behind, be
   * answered immediately instead of parked, and starve the renderer's six
   * sockets — the measured failure `catalogSubscription.ts` records.
   *
   * ⚠ The delta is a NUDGE, never applied: each named chat re-reads its own row
   * through the same `refreshSessionBinding` a turn uses, so there is one code
   * path that decides what a fresh row means and one place the list cache is
   * patched from.
   *
   * ⚠ **Started by {@link ChatStreamProvider}'s effect, NOT by `getController`.**
   * Hanging it off `getController` was the obvious place and it is wrong:
   * everything that touches a chat goes through there, including every unit test
   * that builds a registry, and each of those would open a real long poll at a
   * daemon that is not running. A subscription is a side effect on the network
   * and belongs to a mount, not to a lookup.
   */
  followSessionRows(): () => void {
    if (this.stopSessionMeta) return () => {};
    this.stopSessionMeta = subscribeToSessionMeta({
      // Only chats this renderer holds a row for can go stale, and only those
      // are worth a read on the daemon's side.
      openSessionIds: () =>
        [...this.controllers.entries()]
          .filter(([, controller]) => controller.hasLoadedSession())
          .map(([id]) => id),
      onSessionChanged: (sessionId) => {
        void this.controllers.get(sessionId)?.refreshSessionBinding();
      },
    });
    return () => {
      this.stopSessionMeta?.();
      this.stopSessionMeta = null;
    };
  }

  getController(sessionId: string): ChatStreamController {
    let controller = this.controllers.get(sessionId);
    if (!controller) {
      controller = new ChatStreamController(sessionId, this.handleControllerActivity);
      this.controllers.set(sessionId, controller);
    }
    return controller;
  }

  /**
   * The controller for this session IF one already exists — a lookup, where
   * `getController` is a create-or-get that retains what it makes for the life
   * of the renderer. Use this for teardown ("the tab closed, detach"), where
   * creating a controller for a session that has none is exactly backwards.
   */
  peekController(sessionId: string): ChatStreamController | undefined {
    return this.controllers.get(sessionId);
  }

  isSessionRunning(sessionId: string): boolean {
    return this.controllers.get(sessionId)?.isRunning() ?? false;
  }

  subscribeRunning = (listener: () => void): (() => void) => {
    this.runningListeners.add(listener);
    return () => {
      this.runningListeners.delete(listener);
    };
  };

  getRunningSnapshot = (): RunningChatEntry[] => this.lastRunningSnapshot;

  /**
   * The classification of every chat this window holds a STORE for — a live
   * reading, not a cached one (issue #56, R10; finding M8).
   *
   * # What this exists to fix
   *
   * The chat-tab dot used to read only the session-list cache, and a chat
   * CREATED in this window is not in that cache: `GET /sessions` INNER JOINs
   * `messages` (`SessionStorage::list_sessions_by_types_maybe_empty`), so a
   * brand-new row is not listable until it has recorded one, and
   * `refreshSessionBinding` deliberately patches only entries the cache already
   * holds. Measured on 2026-09-10: one turn on a new chat, sqlite
   * `privacy_tier=private`, the sidebar row and the model chip both private —
   * and the ACTIVE TAB's own dot still `data-privacy="public"` 52.9 s later.
   *
   * The store already knew. `applyTurnBinding` patches the post-ratchet tier
   * onto the snapshot from the reply stream's FIRST frames, which is where the
   * header pill and the composer read it. This channel publishes that same
   * reading to the strip, so the three surfaces cannot disagree — and it needs
   * no fetch, because the answer was already in the window.
   *
   * ⚠ **This map only ever RISES**, mirroring the daemon's own ratchet
   * (`privacy::raise`). A controller whose session momentarily goes null — a
   * reload, a rebind — must not retract a `private` it has already reported, or
   * the strip would fall back to a cached `public` and un-mark a private chat.
   * {@link raiseTier} is the whole rule.
   *
   * ⚠ **O(1) per notification.** `handleControllerActivity` runs on every
   * snapshot notification, which during a turn is once per animation frame per
   * chat; this compares ONE id's tier and returns, and allocates a new map only
   * when a tier actually moved.
   */
  subscribeSessionTiers = (listener: () => void): (() => void) => {
    this.tierListeners.add(listener);
    return () => {
      this.tierListeners.delete(listener);
    };
  };

  getSessionTiersSnapshot = (): Record<string, SessionClassification> => this.sessionTiers;

  resetForTests(): void {
    this.controllers.clear();
    this.running.clear();
    this.lastRunningSnapshot = [];
    this.sessionTiers = {};
    this.tierListeners.clear();
    this.stopSessionMeta?.();
    this.stopSessionMeta = null;
  }

  private noteControllerTier(controller: ChatStreamController): void {
    const reported = controller.getSnapshot().session?.privacy_tier ?? undefined;
    const current = this.sessionTiers[controller.sessionId];
    const raised = raiseTier(current, reported);
    if (raised === current) return;
    this.sessionTiers = { ...this.sessionTiers };
    if (raised) this.sessionTiers[controller.sessionId] = raised;
    else delete this.sessionTiers[controller.sessionId];
    for (const listener of this.tierListeners) listener();
  }

  private handleControllerActivity = (controller: ChatStreamController): void => {
    // ⚠ FIRST, and outside every early return below. The running-list
    // bookkeeping that follows returns without emitting for an idle controller
    // with no live entry — which is exactly the shape of a session LOAD, and of
    // the `refreshSessionBinding` that runs after a turn has already ended.
    // Both carry a tier, and both would be dropped by a tier read placed after
    // that guard.
    this.noteControllerTier(controller);
    const current = this.running.get(controller.sessionId);
    if (controller.isRunning()) {
      const entry = controller.getRunningEntry();
      // #22 — a token event changes the transcript, not the running entry.
      // Re-emitting an identical entry re-rendered the sidebar and tab strip
      // once per streamed token; skip when nothing material changed.
      if (
        current &&
        !current.completedAt &&
        current.chatState === entry.chatState &&
        current.title === entry.title &&
        current.startedAt === entry.startedAt
      ) {
        return;
      }
      this.running.set(controller.sessionId, entry);
    } else if (current && !current.completedAt) {
      this.running.set(controller.sessionId, {
        ...current,
        chatState: ChatState.Idle,
        completedAt: Date.now(),
      });
      window.setTimeout(() => {
        const entry = this.running.get(controller.sessionId);
        if (entry?.completedAt && !controller.isRunning()) {
          this.running.delete(controller.sessionId);
          this.emitRunning();
        }
      }, 1600);
    } else {
      // Idle controller with no live entry (a session load, a token refresh on
      // a finished chat): the running list is untouched — don't re-emit (#22).
      return;
    }
    this.emitRunning();
  };

  private emitRunning(): void {
    this.lastRunningSnapshot = Array.from(this.running.values()).sort(
      (a, b) => b.startedAt - a.startedAt
    );
    for (const listener of this.runningListeners) listener();
  }
}

export const defaultChatStreamRegistry = new ChatStreamRegistry();

const ChatStreamRegistryContext = createContext<ChatStreamRegistry>(defaultChatStreamRegistry);

export function ChatStreamProvider({ children }: { children: React.ReactNode }) {
  // ⚠ Mount-once, and the empty dependency list is the whole point: this opens
  // ONE long poll for the renderer. A subscription that restarted would ask at a
  // revision it is already behind, be answered immediately rather than parked,
  // and claim all six of Chromium's sockets to the daemon — a measured failure,
  // written up in `utils/catalogSubscription.ts`.
  React.useEffect(() => defaultChatStreamRegistry.followSessionRows(), []);
  return (
    <ChatStreamRegistryContext.Provider value={defaultChatStreamRegistry}>
      {children}
    </ChatStreamRegistryContext.Provider>
  );
}

export function useChatStreamRegistry(): ChatStreamRegistry {
  return useContext(ChatStreamRegistryContext);
}

export function useRunningChats(): RunningChatEntry[] {
  const registry = useChatStreamRegistry();
  return useSyncExternalStore(
    registry.subscribeRunning,
    registry.getRunningSnapshot,
    registry.getRunningSnapshot
  );
}

export function useChatStreamController(sessionId: string): ChatStreamController {
  const registry = useChatStreamRegistry();
  return registry.getController(sessionId);
}

/**
 * The classification of every chat this window holds a store for, live.
 *
 * See {@link ChatStreamRegistry.subscribeSessionTiers}. A tab whose chat has
 * never been opened in this window is simply absent — the caller merges this
 * over the session-list cache with `mergeSessionTiers`, which is where a chat
 * nobody has opened gets its tier from.
 */
export function useLiveSessionTiers(): Record<string, SessionClassification> {
  const registry = useChatStreamRegistry();
  return useSyncExternalStore(
    registry.subscribeSessionTiers,
    registry.getSessionTiersSnapshot,
    registry.getSessionTiersSnapshot
  );
}
