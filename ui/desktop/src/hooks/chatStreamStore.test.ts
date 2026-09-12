import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { ChatState } from '../types/chatState';
import {
  ChatStreamRegistry,
  NOTIFY_FALLBACK_MS,
  STOP_CONFIRMED_NOTICE_MS,
  isRunningState,
} from './chatStreamStore';
import type { Message, MessageEvent, Session, TokenState } from '../api';
import { cancelTurn, editMessage, getSession, interrupt, reply, resumeAgent } from '../api';
import { abandonContinuationLease, recoverContinuationGroup } from '../utils/continuationLease';

vi.mock('../api', async () => {
  return {
    cancelTurn: vi.fn(async () => ({
      data: { cancelled: true, settled: true, continuation_lease: 'lease-default' },
    })),
    editMessage: vi.fn(),
    getSession: vi.fn(async () => ({ data: null })),
    interrupt: vi.fn(),
    listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
    reply: vi.fn(),
    resumeAgent: vi.fn(),
    updateFromSession: vi.fn(async () => ({ data: {} })),
    updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
  };
});
vi.mock('../utils/continuationLease', () => ({
  abandonContinuationLease: vi.fn(async () => undefined),
  getContinuationOwnerId: vi.fn(() => 'test-window-owner'),
  recoverContinuationGroup: vi.fn(),
}));

// Issue #56 Task 58 / #47. `POST /reply` and `GET /sessions/{id}` now resolve
// the named chat's tier before they do anything, and refuse a private one
// without the proof-of-user — so the store attaches it, and the assertions
// below name it rather than tolerating whatever is there.
//
// `importOriginal` rather than a bare factory: this module also exports the
// refusal predicates the diverge path uses, and replacing the whole module
// would take those away from anything else in this graph.
vi.mock('../utils/userAction', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../utils/userAction')>()),
  userActionHeaders: async () => ({ 'X-User-Action': 'test-key' }),
}));

const tokenState: TokenState = {
  accumulatedInputTokens: 0,
  accumulatedOutputTokens: 0,
  accumulatedTotalTokens: 0,
  inputTokens: 0,
  outputTokens: 0,
  totalTokens: 0,
};

function session(id: string): Session {
  return {
    id,
    name: `Session ${id}`,
    working_dir: '/tmp',
    conversation: [],
    message_count: 0,
    total_tokens: 0,
    created_at: '',
    updated_at: '',
    extension_data: {},
    user_set_name: false,
  } as Session;
}

function assistantMessage(id: string, text: string): Message {
  return {
    id,
    role: 'assistant',
    created: 1,
    content: [{ type: 'text', text }],
    metadata: { userVisible: true, agentVisible: true },
  };
}

function createControlledStream() {
  const events: MessageEvent[] = [];
  let resolveNext: (() => void) | null = null;
  let closed = false;

  async function* stream() {
    while (!closed || events.length > 0) {
      if (events.length === 0) {
        await new Promise<void>((resolve) => {
          resolveNext = resolve;
        });
      }
      const event = events.shift();
      if (event) {
        yield event;
      }
    }
  }

  return {
    stream: stream(),
    push(event: MessageEvent) {
      events.push(event);
      resolveNext?.();
      resolveNext = null;
    },
    close() {
      closed = true;
      resolveNext?.();
      resolveNext = null;
    },
  };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

async function flush() {
  await Promise.resolve();
  await Promise.resolve();
}

beforeEach(() => {
  vi.useFakeTimers();
  vi.mocked(resumeAgent).mockReset();
  vi.mocked(reply).mockReset();
  vi.mocked(getSession).mockReset();
  vi.mocked(interrupt).mockReset();
  vi.mocked(cancelTurn).mockReset();
  vi.mocked(editMessage).mockReset();
  vi.mocked(abandonContinuationLease).mockReset();
  vi.mocked(abandonContinuationLease).mockResolvedValue(undefined);
  vi.mocked(recoverContinuationGroup).mockReset();
  vi.mocked(cancelTurn).mockResolvedValue({
    data: { cancelled: true, settled: true, continuation_lease: 'lease-default' },
  } as never);
  vi.mocked(getSession).mockResolvedValue({ data: null } as never);
  Object.assign(window, {
    electron: {
      openExternal: vi.fn(async () => undefined),
      readTempImageAsBase64: vi.fn(async () => ({ data: 'B64-image', mimeType: 'image/png' })),
      deleteTempFile: vi.fn(),
      showNotification: vi.fn(),
      logInfo: vi.fn(),
    },
  });
});

describe('ChatStreamRegistry', () => {
  it('labels a running chat without session metadata or submitted text as New chat', async () => {
    const registry = new ChatStreamRegistry();
    registry.getController('unnamed').setChatState(ChatState.Thinking);

    await vi.advanceTimersByTimeAsync(NOTIFY_FALLBACK_MS);

    expect(registry.getRunningSnapshot()).toEqual([
      expect.objectContaining({ sessionId: 'unnamed', title: 'New chat' }),
    ]);
  });

  it('does not auto-open Agent Drafter launch metadata', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: {
            id: 'launch-result',
            role: 'tool',
            created: 2,
            content: [
              {
                type: 'toolResponse',
                id: 'launch-tool',
                toolResult: {
                  status: 'success',
                  value: {
                    is_error: false,
                    content: [{ type: 'text', text: 'App ready' }],
                    _meta: { 'biorouter/app-path': '/apps/click-me/' },
                  },
                },
              },
            ],
            metadata: { userVisible: false, agentVisible: true },
          },
          token_state: tokenState,
        } as MessageEvent;
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    await registry.getController('s1').handleSubmit('build an app');

    expect(window.electron.openExternal).not.toHaveBeenCalled();
  });

  it('a rapid double-submit appends the user turn exactly once (R3-01)', async () => {
    // Unique session id: loadSession has a process-lifetime LRU cache keyed by
    // session id, so reusing another test's id bleeds its cached turns in here.
    const sid = 'r3-01-double-submit';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sid) } } as never);
    vi.mocked(reply).mockImplementation(
      async () =>
        ({
          stream: (async function* () {
            yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
          })(),
        }) as never
    );

    const controller = registry.getController(sid);
    // Two clicks before the first turn has registered its abortController — the
    // async createUserMessage / loadSession prep is the window a double-click hits.
    const p1 = controller.handleSubmit('ping');
    const p2 = controller.handleSubmit('ping');
    await Promise.all([p1, p2]);
    await flush();

    const userMessages = controller.getSnapshot().messages.filter((m) => m.role === 'user');
    expect(userMessages).toHaveLength(1);
    expect(reply).toHaveBeenCalledTimes(1);
  });

  it('synchronizes a generated title and refreshes history after a tool-first turn', async () => {
    const registry = new ChatStreamRegistry();
    const initialSession = { ...session('20260716_27'), name: 'New Session' };
    const generatedSession = { ...initialSession, name: 'Apple Watch News' };
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: initialSession } } as never);
    vi.mocked(getSession).mockResolvedValue({ data: generatedSession } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: {
            id: 'tool-first',
            role: 'assistant',
            created: 2,
            content: [
              {
                type: 'toolRequest',
                id: 'tool-1',
                toolCall: {
                  status: 'success',
                  value: { name: 'developer__text_editor', arguments: { command: 'view' } },
                },
              },
            ],
            metadata: { userVisible: true, agentVisible: true },
          },
          token_state: tokenState,
        } as MessageEvent;
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const onFinished = vi.fn();
    window.addEventListener('message-stream-finished', onFinished);
    const controller = registry.getController('20260716_27');
    await controller.handleSubmit('summarize the latest Apple Watch news');

    expect(onFinished).toHaveBeenCalledTimes(1);
    await vi.advanceTimersByTimeAsync(800);
    await flush();

    expect(getSession).toHaveBeenCalledWith({
      path: { session_id: '20260716_27' },
      // Issue #56 Task 58: reading a private chat's transcript needs this.
      headers: { 'X-User-Action': 'test-key' },
      // The poll reads a name and a flag, up to eight times per turn, and must
      // not drag the conversation across with them.
      query: { metadata_only: true },
      throwOnError: true,
    });
    // …and so does running a turn in one. `/reply` is the route that dominates
    // every other session-addressing route, so the renderer losing this header
    // would make every private chat unusable — which is the failure this pins.
    expect(reply).toHaveBeenCalledWith(
      expect.objectContaining({ headers: { 'X-User-Action': 'test-key' } })
    );
    expect(controller.getSnapshot().session?.name).toBe('Apple Watch News');
    window.removeEventListener('message-stream-finished', onFinished);
  });

  it('routes structured provider failures inline without failing the session', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Error',
          error: 'You exceeded your current quota.',
          code: 'provider_failure',
          scope: 'provider',
          retryable: false,
          provider_kind: 'quota',
        } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('hello');

    expect(controller.getSnapshot()).toMatchObject({
      chatState: ChatState.Idle,
      turnError: {
        message: 'You exceeded your current quota.',
        code: 'provider_failure',
        scope: 'provider',
        retryable: false,
        providerKind: 'quota',
      },
    });
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
    expect(controller.getSnapshot().messages[0]).toMatchObject({ role: 'user' });
  });

  it('clears the previous inline turn error when a new model turn starts', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield {
            type: 'Error',
            error: 'provider unavailable',
            code: 'provider_failure',
            scope: 'provider',
            retryable: true,
          } as MessageEvent;
        })(),
      } as never)
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('first try');
    expect(controller.getSnapshot().turnError).toMatchObject({
      message: 'provider unavailable',
      code: 'provider_failure',
    });

    await controller.handleSubmit('second try');
    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
  });

  it('uses the generic inline path for an unknown future provider error', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Error',
          error: 'Provider returned a new rejection type',
          code: 'provider_failure',
          scope: 'provider',
          retryable: false,
          provider_kind: 'unknown',
        } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('hello');

    expect(controller.getSnapshot().turnError).toMatchObject({
      message: 'Provider returned a new rejection type',
      providerKind: 'unknown',
    });
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
  });

  it('treats a stream ending without a terminal event as an inline interruption', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: assistantMessage('a1', 'partial response'),
          token_state: tokenState,
        } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('hello');

    expect(controller.getSnapshot()).toMatchObject({
      chatState: ChatState.Idle,
      turnError: {
        code: 'stream_interrupted',
        scope: 'transport',
        retryable: true,
      },
    });
  });

  it('catches non-Error stream failures instead of leaving the chat running', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: {
        [Symbol.asyncIterator]() {
          return {
            next: async () => {
              throw 'decoder rejected an unknown frame';
            },
          };
        },
      },
    } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('hello');

    expect(controller.getSnapshot()).toMatchObject({
      chatState: ChatState.Idle,
      turnError: {
        message: 'decoder rejected an unknown frame',
        code: 'stream_error',
      },
    });
  });

  it('keeps a provider initialization failure inline after loading the session', async () => {
    const registry = new ChatStreamRegistry();
    const sessionId = 'provider-init-failure-session';
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session(sessionId),
        initialization_error: {
          code: 'provider_restore_failed',
          message: 'Configured model no longer exists',
          retryable: false,
        },
      },
    } as never);

    const controller = registry.getController(sessionId);
    await controller.loadSession();

    expect(controller.getSnapshot()).toMatchObject({
      session: { id: sessionId },
      turnError: {
        code: 'provider_restore_failed',
        scope: 'session',
      },
    });
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
  });

  it('reserves the full-session error state for an actual resume failure', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new Error('session database unavailable'));

    const controller = registry.getController('actual-session-load-failure');
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toBe('session database unavailable');
    expect(controller.getSnapshot().turnError).toBeUndefined();
  });

  // ---- SEND-HARDENING: a system-level blip on the send/load path must degrade
  // to an inline turn error, never the transcript-nuking "Failed to Load
  // Session" card. See docs/qa/2026-07-19-qa-round2.md (SEND-HARDENING).

  it('degrades a send into an unreachable backend to an inline error, never the fatal card (gate a)', async () => {
    // The user reproduction: biorouterd is briefly down (a QA restart), the
    // controller is fresh (post-reload), and the user hits Send. The send runs
    // through loadSession's cold path, whose fetch rejects with the browser's
    // `TypeError: Failed to fetch`.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new TypeError('Failed to fetch'));

    const controller = registry.getController('send-into-dead-backend');
    await controller.handleSubmit('analyze my cohort');

    const snap = controller.getSnapshot();
    // The full-pane "Failed to Load Session / Go home" card is NOT triggered.
    expect(snap.sessionLoadError).toBeUndefined();
    // A retryable, transport-scoped inline turn error surfaces instead.
    expect(snap.turnError).toMatchObject({
      code: 'session_load_unreachable',
      scope: 'transport',
      retryable: true,
    });
    expect(snap.chatState).toBe(ChatState.Idle);
  });

  it('keeps a transient cold-load connection failure inline instead of the fatal card', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new TypeError('Failed to fetch'));

    const controller = registry.getController('cold-load-blip');
    await controller.loadSession();

    const snap = controller.getSnapshot();
    expect(snap.sessionLoadError).toBeUndefined();
    expect(snap.turnError).toMatchObject({
      code: 'session_load_unreachable',
      scope: 'transport',
      retryable: true,
    });
  });

  it('still shows the fatal card for a genuine (non-connection) resume failure (gate c)', async () => {
    // A real HTTP error (bad id / corrupt data) is not a connection blip: it
    // arrives as a plain Error, not a TypeError, and must stay fatal.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new Error('HTTP 404: session not found'));

    const controller = registry.getController('genuinely-unloadable');
    await controller.loadSession();

    const snap = controller.getSnapshot();
    expect(snap.sessionLoadError).toBe('HTTP 404: session not found');
    expect(snap.turnError).toBeUndefined();
  });

  it('replays a retired ambiguous turn with its original idempotency key (gate b)', async () => {
    // Unique session id: the transcript cache is a module-level LRU keyed by id,
    // so reusing a shared id ('s1') would load another test's cached messages
    // and break the exact-length assertions below.
    const sessionId = 'retry-gate-b';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    // First send: the backend blips mid-POST. Retry: the backend is back.
    vi.mocked(reply).mockRejectedValueOnce(new TypeError('Failed to fetch'));
    vi.mocked(reply).mockResolvedValueOnce({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('hello world');
    const originalTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    // The failed send parked the user's message in the transcript (once) and
    // surfaced a retryable inline error — NOT the fatal card.
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
    expect(controller.getSnapshot().turnError).toMatchObject({
      code: 'submit_error',
      scope: 'transport',
      retryable: true,
    });
    expect(controller.getSnapshot().messages.filter((m) => m.role === 'user')).toHaveLength(1);

    await controller.retryTurn();

    // reply fired once for the original send and once for the retry — no more.
    expect(reply).toHaveBeenCalledTimes(2);
    expect(vi.mocked(reply).mock.calls[1][0].body).toMatchObject({
      turn_id: originalTurnId,
      from_seq: 0,
    });
    // The user's message still appears exactly once (retry reused the trailing
    // turn rather than appending a duplicate).
    expect(controller.getSnapshot().messages.filter((m) => m.role === 'user')).toHaveLength(1);
    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
  });

  it('keeps the original idempotency key when an accepted reply response is lost', async () => {
    const sessionId = 'retry-lost-response';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply)
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockRejectedValueOnce(new TypeError('Still offline'));

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('do this once');
    const originalTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    await controller.retryTurn();

    expect(reply).toHaveBeenCalledTimes(2);
    expect(vi.mocked(reply).mock.calls[1][0].body.turn_id).toBe(originalTurnId);
    expect(controller.getSnapshot().turnError).toMatchObject({
      code: 'submit_error',
      scope: 'transport',
      retryable: true,
    });
  });

  /**
   * Defect 3.1. Retry after "Connection dropped" took TWO presses, and the
   * first press replaced the card with a scarier "Model turn ended
   * unexpectedly".
   *
   * The pointer `ambiguousRetryTurnId` makes the first press an ATTACH, which
   * returns before the resubmit. When the daemon answers that attach with its
   * synthesized `stream_ended_without_terminal` — `TurnStream::close` for a
   * turn with no writer — the ambiguity is RESOLVED: the turn is over and
   * produced nothing. The press should carry on to the resubmit rather than
   * paint an internal-scope failure and stop.
   *
   * ⚠ Only for that authoritative answer. An attach that could not be made at
   * all (the daemon is unreachable) leaves the outcome genuinely ambiguous, and
   * resubmitting there could double-run a turn that is still alive server-side.
   * That path keeps its second press; see the case below it.
   */
  it('resubmits in ONE press when the daemon says the turn ended without a result', async () => {
    const sessionId = 'retry-dead-turn';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply)
      // 1. the send whose response was lost after the daemon accepted it
      .mockRejectedValueOnce(new TypeError('Response lost after accept'))
      // 2. the attach: the daemon closes the turn with no result
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield {
            type: 'Error',
            error: 'The stream for this turn ended without a result. Please retry.',
            code: 'stream_ended_without_terminal',
            scope: 'internal',
            retryable: true,
          } as MessageEvent;
        })(),
      } as never)
      // 3. the resubmit this press should reach on its own
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('do the thing');

    await controller.retryTurn();

    expect(reply).toHaveBeenCalledTimes(3);
    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().messages.filter((m) => m.role === 'user')).toHaveLength(1);
  });

  it('still takes a second press when the attach itself could not be made', async () => {
    const sessionId = 'retry-attach-unreachable';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply)
      .mockRejectedValueOnce(new TypeError('Response lost after accept'))
      .mockRejectedValueOnce(new TypeError('Failed to fetch'));

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('do the thing');

    await controller.retryTurn();

    // Two calls, not three: the turn may still be alive on the daemon, and a
    // blind resubmit would run it twice.
    expect(reply).toHaveBeenCalledTimes(2);
  });

  it('attaches to a still-running ambiguous turn instead of starting another one', async () => {
    const sessionId = 'retry-running-attach';
    const registry = new ChatStreamRegistry();
    const running = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply)
      .mockRejectedValueOnce(new TypeError('Response lost after accept'))
      .mockResolvedValueOnce({ stream: running.stream } as never);

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('long running task');
    const originalTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    const retry = controller.retryTurn();
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(2));

    expect(vi.mocked(reply).mock.calls[1][0].body.turn_id).toBe(originalTurnId);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    running.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    running.close();
    await retry;
    expect(controller.getSnapshot().turnError).toBeUndefined();
  });

  it('mints a new turn only after a genuine terminal model failure', async () => {
    const sessionId = 'retry-genuine-terminal';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield {
            type: 'Error',
            error: 'The provider rejected the model request',
            code: 'provider_failure',
            scope: 'provider',
            retryable: true,
          } as MessageEvent;
        })(),
      } as never)
      .mockResolvedValueOnce({
        stream: (async function* () {
          yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController(sessionId);
    await controller.handleSubmit('try the model');
    const failedTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    await controller.retryTurn();

    expect(reply).toHaveBeenCalledTimes(2);
    expect(vi.mocked(reply).mock.calls[1][0].body.turn_id).not.toBe(failedTurnId);
    expect(vi.mocked(reply).mock.calls[1][0].body.from_seq).toBeUndefined();
  });

  it('retryTurn does not fire a second concurrent turn while one is live', async () => {
    const sessionId = 'retry-concurrent';
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    const controlled = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController(sessionId);
    // Preload the session + agent so the turn reaches reply() and enters
    // Streaming deterministically (no cold-load latency to flush past).
    await controller.loadSession();
    await flush();

    const inFlight = controller.handleSubmit('hello world');
    // Drive the turn up to the (mocked) reply() call, where it parks on the
    // controlled stream's first event with a live abortController.
    for (let i = 0; i < 12 && vi.mocked(reply).mock.calls.length === 0; i += 1) {
      await flush();
    }
    expect(reply).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    // A retry while the turn is still streaming must be a no-op.
    await controller.retryTurn();
    expect(reply).toHaveBeenCalledTimes(1);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await inFlight;
  });

  it('submits attachment-only messages as new user messages', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    await registry
      .getController('s1')
      .handleSubmit('', [{ path: '/tmp/biorouter-pasted-images/scan.png', kind: 'image' }]);

    expect(reply).toHaveBeenCalledTimes(1);
    expect(vi.mocked(reply).mock.calls[0][0].body.user_message.content).toEqual([
      { type: 'image', data: 'B64-image', mimeType: 'image/png' },
    ]);
  });

  it('submits hidden system messages as agent-visible repair context', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const hiddenMessage: Message = {
      id: 'hidden-repair',
      role: 'user',
      created: 1,
      content: [{ type: 'text', text: 'repair this artifact' }],
      metadata: { userVisible: false, agentVisible: true },
    };

    const controller = registry.getController('s1');
    await controller.submitSystemMessage(hiddenMessage);

    expect(reply).toHaveBeenCalledTimes(1);
    expect(vi.mocked(reply).mock.calls[0][0].body.user_message).toEqual(hiddenMessage);
    expect(controller.getSnapshot().messages).toContainEqual(hiddenMessage);
  });

  it('keeps a submitted stream running after the last view unsubscribes', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('s1');
    const unsubscribe = controller.subscribe(() => undefined);
    const submit = controller.handleSubmit('analyze this');
    await flush();
    unsubscribe();

    controlled.push({
      type: 'Message',
      message: assistantMessage('a1', 'working'),
      token_state: tokenState,
    });
    await flush();

    expect(registry.getRunningSnapshot()).toMatchObject([
      {
        sessionId: 's1',
        chatState: ChatState.Streaming,
      },
    ]);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;

    expect(registry.getRunningSnapshot()[0]).toMatchObject({
      sessionId: 's1',
      chatState: ChatState.Idle,
    });

    await vi.advanceTimersByTimeAsync(1600);
    expect(registry.getRunningSnapshot()).toEqual([]);
  });

  it('tracks two session streams independently', async () => {
    const registry = new ChatStreamRegistry();
    const first = createControlledStream();
    const second = createControlledStream();

    vi.mocked(resumeAgent)
      .mockResolvedValueOnce({ data: { session: session('s1') } } as never)
      .mockResolvedValueOnce({ data: { session: session('s2') } } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({ stream: first.stream } as never)
      .mockResolvedValueOnce({ stream: second.stream } as never);

    const firstSubmit = registry.getController('s1').handleSubmit('first');
    const secondSubmit = registry.getController('s2').handleSubmit('second');
    await flush();

    first.push({
      type: 'Message',
      message: assistantMessage('a1', 'first running'),
      token_state: tokenState,
    });
    second.push({
      type: 'Message',
      message: assistantMessage('a2', 'second running'),
      token_state: tokenState,
    });
    await flush();

    expect(
      registry
        .getRunningSnapshot()
        .map((entry) => entry.sessionId)
        .sort()
    ).toEqual(['s1', 's2']);

    first.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    second.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    first.close();
    second.close();
    await Promise.all([firstSubmit, secondSubmit]);
  });
  it('steers a running turn through the soft-interrupt route', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(interrupt).mockResolvedValue({ data: undefined } as never);

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    await expect(controller.steer('actually, use R')).resolves.toBe(true);
    expect(vi.mocked(interrupt).mock.calls[0][0].body).toEqual({
      session_id: 's1',
      text: 'actually, use R',
      turn_id: expect.any(String),
    });
    expect(vi.mocked(interrupt).mock.calls[0][0].headers).toEqual({
      'X-User-Action': 'test-key',
    });
    // The steer is not appended locally — the agent streams it back once consumed.
    const steerAppended = controller
      .getSnapshot()
      .messages.some((m) =>
        m.content.some((c) => c.type === 'text' && c.text === 'actually, use R')
      );
    expect(steerAppended).toBe(false);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('refuses to steer when no turn is running', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    const controller = registry.getController('s1');
    await controller.loadSession();

    await expect(controller.steer('too late')).resolves.toBe(false);
    expect(interrupt).not.toHaveBeenCalled();
  });

  it('dispatches the canonical divergence event after an edited-message divergence', async () => {
    const registry = new ChatStreamRegistry();
    const sourceSessionId = 'edit-diverge-source';
    const userMessage: Message = {
      id: 'u1',
      role: 'user',
      created: 10,
      content: [{ type: 'text', text: 'original prompt' }],
      metadata: { userVisible: true, agentVisible: true },
    };
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: { ...session(sourceSessionId), conversation: [userMessage] } },
    } as never);
    vi.mocked(editMessage).mockResolvedValue({ data: { sessionId: 's2' } } as never);
    const onDiverged = vi.fn();
    window.addEventListener('session-diverged', onDiverged);

    const controller = registry.getController(sourceSessionId);
    await controller.loadSession();
    await controller.onMessageUpdate('u1', 'updated prompt', 'diverge');

    expect(editMessage).toHaveBeenCalledWith({
      path: { session_id: sourceSessionId },
      body: { timestamp: 10, editType: 'diverge', expectedMessageIds: ['u1'] },
      // Issue #56 DR-19: `diverge` mints a new session inheriting this chat's
      // provider, so it carries the user-action proof. It used to be asserted
      // EMPTY here, because the harness had no `window.electron` bridge to mint
      // a key from and only the shape could be pinned; Task 58 gave this file a
      // `userActionHeaders` mock, so the real key can now be named — which is
      // the stronger claim. The two `edit` cases below must still NOT carry it.
      headers: { 'X-User-Action': 'test-key' },
      throwOnError: true,
    });
    expect(onDiverged).toHaveBeenCalledTimes(1);
    expect((onDiverged.mock.calls[0][0] as CustomEvent).detail).toEqual({
      // The ORIGIN session. 'session-diverged' is a window broadcast heard by
      // every mounted chat, and newSessionId names a session that doesn't exist
      // in the UI yet — so listeners identify the single chat that should
      // navigate by the session the user diverged FROM.
      sessionId: sourceSessionId,
      newSessionId: 's2',
      shouldStartAgent: true,
      editedMessage: 'updated prompt',
    });

    window.removeEventListener('session-diverged', onDiverged);
  });

  // #51 NF-D: `edit` truncates the LIVE session, and the server checks the cut
  // against `expectedMessageIds` when we send it — 409 if it is missing
  // something the store holds. When we CAN name everything we hold, we send it
  // all: every message, ids only, including the ones the transcript does not
  // render. Dropping one would 409 an edit that should have gone through.
  it('sends its whole view of the session when truncating in place', async () => {
    const registry = new ChatStreamRegistry();
    const sessionId = 'edit-in-place';
    const conversation: Message[] = [
      {
        id: 'u1',
        role: 'user',
        created: 10,
        content: [{ type: 'text', text: 'original prompt' }],
        metadata: { userVisible: true, agentVisible: true },
      },
      {
        id: 'a1',
        role: 'assistant',
        created: 20,
        content: [{ type: 'text', text: 'an answer' }],
        metadata: { userVisible: true, agentVisible: true },
      },
      {
        id: 'a2',
        role: 'assistant',
        created: 30,
        content: [{ type: 'text', text: 'a message the transcript hides' }],
        metadata: { userVisible: false, agentVisible: true },
      },
    ];
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: { ...session(sessionId), conversation } },
    } as never);
    vi.mocked(editMessage).mockResolvedValue({ data: { sessionId } } as never);
    vi.mocked(getSession).mockResolvedValue({ data: { conversation: [conversation[0]] } } as never);

    const controller = registry.getController(sessionId);
    await controller.loadSession();
    await controller.onMessageUpdate('u1', 'updated prompt', 'edit');

    expect(editMessage).toHaveBeenCalledWith({
      path: { session_id: sessionId },
      body: {
        timestamp: 10,
        editType: 'edit',
        expectedMessageIds: ['u1', 'a1', 'a2'],
      },
      headers: { 'X-User-Action': 'test-key' },
      throwOnError: true,
    });
  });

  // #59: a message we hold can still carry `id: null` — a cached transcript, a
  // frame from before the loop stamped ids on the copies it yields. A list built
  // from it is missing exactly those ids, so sending it would assert a view we
  // do not have and buy a guaranteed 409 on a session nobody else has touched.
  // Omit the field instead and let the server fall back to its turn lock and its
  // bounded cut. THIS is what keeps "Edit in Place" working in a live chat; if it
  // regresses, the button dies again.
  //
  // This is the second half of the guard and it is independently necessary. The
  // first half is `viewNamesEveryStoredRow` (below): the stream DOES publish the
  // ids a turn persisted now (`MessagesPersisted`), but this client does not
  // consume that frame, so naming every message we hold still does not mean we
  // hold every row the store has.
  it('omits its view when a message it holds has no id to name it by', async () => {
    const registry = new ChatStreamRegistry();
    const sessionId = 'edit-in-place-unnamed';
    const conversation: Message[] = [
      {
        id: 'u1',
        role: 'user',
        created: 10,
        content: [{ type: 'text', text: 'original prompt' }],
        metadata: { userVisible: true, agentVisible: true },
      },
      {
        // Streamed, not re-read: the store minted a uid we were never told.
        id: null,
        role: 'assistant',
        created: 20,
        content: [{ type: 'text', text: 'an answer' }],
        metadata: { userVisible: true, agentVisible: true },
      },
    ];
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: { ...session(sessionId), conversation } },
    } as never);
    vi.mocked(editMessage).mockResolvedValue({ data: { sessionId } } as never);
    vi.mocked(getSession).mockResolvedValue({ data: { conversation: [conversation[0]] } } as never);

    const controller = registry.getController(sessionId);
    await controller.loadSession();
    await controller.onMessageUpdate('u1', 'updated prompt', 'edit');

    expect(editMessage).toHaveBeenCalledWith({
      path: { session_id: sessionId },
      body: { timestamp: 10, editType: 'edit' },
      // The in-place edit asks the read's reach gate since QA's 2026-09-10
      // sweep, so it carries the proof as a branch does.
      headers: { 'X-User-Action': 'test-key' },
      throwOnError: true,
    });
    const body = vi.mocked(editMessage).mock.calls[0][0].body as Record<string, unknown>;
    expect('expectedMessageIds' in body).toBe(false);
  });

  // #59 follow-on: "every message names itself" is NOT the same claim as "we
  // name every stored row", and after the reply loop started stamping ids on
  // the copies it yields, the first became true on turns where the second is
  // false. One streamed assistant reply is stored as two or three rows — the
  // rebuilt thinking row plus one `tool_use` row per request — and only the
  // first keeps the id the client was shown; the model-only rows (BR-47
  // diagnostics, loop-guard nudges, hook context) are never yielded at all.
  // `crates/biorouter/tests/conversation_writeback_freshness.rs
  // ::a_reply_split_into_several_stored_rows_publishes_every_one_of_their_ids`
  // asserts exactly that inequality server-side: stored rows > streamed
  // messages. So a list built from a watched turn is short, and a short list is
  // not a weaker claim but a false one — a guaranteed 409 on a session nobody
  // else has touched, i.e. "Edit in Place" dead again with a
  // `Failed to edit message` toast. Only a view read straight back from the
  // store may be claimed as complete.
  it('omits its view after a live turn, even though every message it holds is named', async () => {
    const registry = new ChatStreamRegistry();
    const sessionId = 'edit-in-place-after-live-turn';
    const conversation: Message[] = [
      {
        id: 'u1',
        role: 'user',
        created: 10,
        content: [{ type: 'text', text: 'original prompt' }],
        metadata: { userVisible: true, agentVisible: true },
      },
    ];
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: { ...session(sessionId), conversation } },
    } as never);
    vi.mocked(editMessage).mockResolvedValue({ data: { sessionId } } as never);
    vi.mocked(getSession).mockResolvedValue({ data: { conversation } } as never);
    vi.mocked(reply).mockImplementation(
      async () =>
        ({
          stream: (async function* () {
            // #59 `named()`: the yielded copy now always carries an id. The
            // store, meanwhile, split this one reply into a thinking row and a
            // `tool_use` row and only published the extra id on
            // `MessagesPersisted`, which this client does not consume.
            yield {
              type: 'Message',
              message: assistantMessage('a-live', 'thinking, then a tool call'),
              token_state: tokenState,
            } as MessageEvent;
            yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
          })(),
        }) as never
    );

    const controller = registry.getController(sessionId);
    await controller.loadSession();
    await controller.handleSubmit('run a tool for me');
    await flush();

    // Every message the client holds names itself...
    expect(controller.getSnapshot().messages.every((m) => typeof m.id === 'string')).toBe(true);

    await controller.onMessageUpdate('u1', 'updated prompt', 'edit');

    // ...and it still must not claim its view is the whole store.
    const body = vi.mocked(editMessage).mock.calls[0][0].body as Record<string, unknown>;
    expect('expectedMessageIds' in body).toBe(false);
  });

  // #67: `retryTurn` re-sends the message ALREADY at the tail of the transcript,
  // so it submits with `updateMessageList: false` — the one submit path that
  // never reaches `updateMessages`, which is where the completeness claim is
  // dropped. Resume asserts the claim; a retry that dies before a single frame
  // arrives therefore used to leave it standing, while the server may already
  // have persisted rows (the user turn itself, a BR-47 diagnostic, a hook
  // context row) that this client was never shown. The next "Edit in Place"
  // then sends a complete-looking but SHORT view.
  //
  // Bounded: a false negative on guard 2 only — the turn lock and the bounded
  // truncate still hold — so this is defence in depth, not a data-loss path.
  // Launching a turn at all is what invalidates the claim, regardless of what
  // (if anything) comes back.
  it('drops its completeness claim when a retried turn dies without delivering a frame', async () => {
    const registry = new ChatStreamRegistry();
    const sessionId = 'edit-in-place-after-dead-retry';
    const conversation: Message[] = [
      {
        id: 'u1',
        role: 'user',
        created: 10,
        content: [{ type: 'text', text: 'original prompt' }],
        metadata: { userVisible: true, agentVisible: true },
      },
    ];
    // Resume reads `get_session(id, true)`, so the claim is legitimately true here.
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: { ...session(sessionId), conversation } },
    } as never);
    vi.mocked(editMessage).mockResolvedValue({ data: { sessionId } } as never);
    vi.mocked(getSession).mockResolvedValue({ data: { conversation } } as never);
    // The retry reaches the server — the turn is live, rows may be written —
    // and then the transport dies before yielding anything at all.
    vi.mocked(reply).mockImplementation(
      async () =>
        ({
          stream: (async function* () {
            throw new TypeError('network error');
            // eslint-disable-next-line no-unreachable
            yield undefined as never;
          })(),
        }) as never
    );

    const controller = registry.getController(sessionId);
    await controller.loadSession();
    await controller.retryTurn();
    await flush();

    // The retry really did launch a turn and really did fail.
    expect(reply).toHaveBeenCalledTimes(1);
    expect(controller.getSnapshot().turnError).toMatchObject({ retryable: true });
    // The transcript is untouched, so every message it holds still names itself.
    expect(controller.getSnapshot().messages.every((m) => typeof m.id === 'string')).toBe(true);

    await controller.onMessageUpdate('u1', 'updated prompt', 'edit');

    const body = vi.mocked(editMessage).mock.calls[0][0].body as Record<string, unknown>;
    expect('expectedMessageIds' in body).toBe(false);
  });

  it('reports a rejected soft interrupt so the caller can fall back', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    // The turn ended between the click and the POST: the server answers 409.
    vi.mocked(interrupt).mockRejectedValue(new Error('conflict'));

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    await expect(controller.steer('actually, use R')).resolves.toBe(false);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  /**
   * Issue 1 — the steer left the composer and nothing appeared until the agent's
   * next output, seconds later. The store now carries an OPTIMISTIC record of
   * the in-flight steer so the transcript can acknowledge the press immediately;
   * these cover the two ways that optimism could turn into a lie.
   */
  it('publishes the steer optimistically, before the interrupt POST resolves', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    // A POST that never settles: nothing about the acknowledgement may wait on it.
    let releaseInterrupt: (() => void) | null = null;
    vi.mocked(interrupt).mockImplementation(
      () =>
        // #69: an accepted interrupt now answers with the turn that took it.
        new Promise(
          (resolve) => (releaseInterrupt = () => resolve({ data: { turn_id: 'agent-turn-1' } }))
        ) as never
    );

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    expect(controller.getSnapshot().pendingSteer).toBeUndefined();
    const steering = controller.steer('actually, use R');
    await flush();

    expect(controller.getSnapshot().pendingSteer?.text).toBe('actually, use R');
    expect(typeof controller.getSnapshot().pendingSteer?.since).toBe('number');

    releaseInterrupt!();
    await expect(steering).resolves.toBe(true);
    // Still pending: accepted by the server is NOT the same as consumed by the
    // agent, which only happens at its next loop boundary.
    expect(controller.getSnapshot().pendingSteer?.text).toBe('actually, use R');

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('retracts the optimistic steer when the agent echoes it back', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(interrupt).mockResolvedValue({ data: undefined } as never);

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    await controller.steer('actually, use R');
    await flush();
    expect(controller.getSnapshot().pendingSteer).toBeDefined();

    // The agent consumed it and replayed it as an ordinary user message — the
    // only reliable signal that it landed.
    controlled.push({
      type: 'Message',
      message: {
        id: 'steer-echo',
        role: 'user',
        created: 3,
        content: [{ type: 'text', text: 'actually, use R' }],
        metadata: { userVisible: true, agentVisible: true },
      },
      token_state: tokenState,
    } as MessageEvent);
    await flush();

    expect(controller.getSnapshot().pendingSteer).toBeUndefined();

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('retracts the optimistic steer when the server rejects it, so the UI never lies', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(interrupt).mockRejectedValue(new Error('conflict'));

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    // The caller falls back to an ordinary send; leaving "Steering…" on screen
    // would describe a path the text is no longer taking.
    await expect(controller.steer('actually, use R')).resolves.toBe(false);
    expect(controller.getSnapshot().pendingSteer).toBeUndefined();

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('a rejected steer does not retract a LATER steer that is still pending', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    // Steer A's POST hangs, then rejects only after steer B has replaced the
    // chip. A retraction that did not check ownership would wipe B — and
    // nothing re-shows a chip for an in-flight steer, so B's indicator would be
    // gone for good while B was still genuinely pending.
    let rejectA: (reason: Error) => void = () => {};
    vi.mocked(interrupt).mockImplementationOnce(
      () =>
        new Promise((_resolve, reject) => {
          rejectA = reject;
        }) as never
    );
    vi.mocked(interrupt).mockResolvedValue({ data: undefined } as never);

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();

    const steerA = controller.steer('actually, use R');
    await flush();
    await expect(controller.steer('no wait, use Python')).resolves.toBe(true);
    expect(controller.getSnapshot().pendingSteer?.text).toBe('no wait, use Python');

    rejectA(new Error('conflict'));
    await expect(steerA).resolves.toBe(false);

    expect(controller.getSnapshot().pendingSteer?.text).toBe('no wait, use Python');

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('clears a never-echoed steer when the turn ends', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(interrupt).mockResolvedValue({ data: undefined } as never);

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('plot the data');
    await flush();
    await controller.steer('actually, use R');
    await flush();
    expect(controller.getSnapshot().pendingSteer).toBeDefined();

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;

    // The turn it was aimed at is over — there is nothing left to steer.
    expect(controller.getSnapshot().pendingSteer).toBeUndefined();
  });

  it('does not match a steer against the user’s own earlier prompt', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(interrupt).mockResolvedValue({ data: undefined } as never);

    const controller = registry.getController('s1');
    // Steering with the same words that are already in the transcript: a
    // text-only match would retire the chip against history, the instant it
    // appeared.
    const submit = controller.handleSubmit('use R');
    await flush();

    await controller.steer('use R');
    await flush();
    expect(controller.getSnapshot().pendingSteer?.text).toBe('use R');

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState });
    controlled.close();
    await submit;
  });

  it('sends a client-generated turn_id so an SSE re-POST dedupes (BR-62b)', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    await registry.getController('s1').handleSubmit('hello');

    const turnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;
    expect(typeof turnId).toBe('string');
    expect((turnId as string).length).toBeGreaterThan(0);
  });

  it('uses a fresh turn_id for each turn (BR-62b)', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('s1');
    await controller.handleSubmit('first');
    await controller.handleSubmit('second');

    const firstId = vi.mocked(reply).mock.calls[0][0].body.turn_id;
    const secondId = vi.mocked(reply).mock.calls[1][0].body.turn_id;
    expect(firstId).not.toEqual(secondId);
  });

  it('reliably cancels the running turn on Stop via /agent/cancel (BR-62b)', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('s1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('s1');
    const submit = controller.handleSubmit('long task');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    const turnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    const stopped = controller.stopStreaming();

    // Visual acknowledgement is separate; the store remains busy until the
    // server-side cancellation barrier resolves.
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
    expect(await stopped).toBe(true);

    expect(cancelTurn).toHaveBeenCalledTimes(1);
    expect(vi.mocked(cancelTurn).mock.calls[0][0].body).toEqual({
      session_id: 's1',
      expected_turn_id: turnId,
      wait_for_idle: true,
      continuation_pending: false,
    });
    expect(vi.mocked(cancelTurn).mock.calls[0][0].headers).toEqual({
      'X-User-Action': 'test-key',
    });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);

    controlled.close();
    await submit;
  });

  it('fails closed without sending a session-only cancel before a turn is named', async () => {
    const registry = new ChatStreamRegistry();
    const controller = registry.getController('unnamed-stop');
    const initialState = controller.getSnapshot().chatState;

    await expect(controller.stopStreaming()).resolves.toBe(false);
    expect(cancelTurn).not.toHaveBeenCalled();
    expect(controller.getSnapshot().chatState).toBe(initialState);

    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('unnamed-stop') },
    } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await expect(controller.handleSubmit('after loading')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(1);
  });

  it('does not admit a successor before the cancellation barrier settles', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const cancellation = deferred<{
      data: { cancelled: boolean; settled: boolean; continuation_lease: string };
    }>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('stop-barrier') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn).mockReturnValue(cancellation.promise as never);

    const controller = registry.getController('stop-barrier');
    const firstSubmit = controller.handleSubmit('long task');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));

    const stopped = controller.stopStreaming(true);
    await flush();
    expect(cancelTurn).toHaveBeenCalledTimes(1);

    // Even if the client sees a terminal frame and its original submit latch
    // unwinds first, the cancel response is the admission authority.
    controlled.push({
      type: 'Finish',
      reason: 'cancelled',
      token_state: tokenState,
    } as MessageEvent);
    controlled.close();
    await firstSubmit;

    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
    await expect(controller.handleSubmit('too early')).resolves.toBe(false);
    expect(reply).toHaveBeenCalledTimes(1);

    cancellation.resolve({
      data: { cancelled: true, settled: true, continuation_lease: 'lease-barrier' },
    });
    await expect(stopped).resolves.toBe(true);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);

    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await expect(controller.handleSubmit('after the barrier')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(2);
    expect(
      (vi.mocked(reply).mock.calls[1][0].body as Record<string, unknown>).continuation_lease
    ).toBe('lease-barrier');
  });

  it('rehydrates only its owner-bound lease and never resends lost composer text', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session('lease-rehydrated'),
        pending_continuation: {
          ownership: 'owned',
          superseded_turn_id: 'turn-stopped',
          continuation_lease: 'lease-rehydrated-token',
        },
      },
    } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('lease-rehydrated');
    await controller.loadSession();
    expect(reply).not.toHaveBeenCalled();
    expect(controller.getSnapshot().pendingContinuation).toEqual({
      ownership: 'owned',
      supersededTurnId: 'turn-stopped',
    });

    await expect(controller.handleSubmit('message re-entered by the user')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(1);
    expect(vi.mocked(reply).mock.calls[0][0].body).toMatchObject({
      continuation_lease: 'lease-rehydrated-token',
    });
  });

  it('gates a foreign continuation until explicit takeover', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: session('lease-foreign'),
        pending_continuation: {
          ownership: 'foreign',
          superseded_turn_id: 'turn-foreign-stopped',
        },
      },
    } as never);
    vi.mocked(recoverContinuationGroup).mockResolvedValue({
      resolution: 'taken_over',
      superseded_turn_id: 'turn-foreign-stopped',
      continuation_lease: 'lease-taken-over',
    });
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('lease-foreign');
    await controller.loadSession();
    await expect(controller.handleSubmit('must stay in composer')).resolves.toBe(false);
    expect(reply).not.toHaveBeenCalled();

    await controller.recoverPendingContinuation('take_over');
    expect(recoverContinuationGroup).toHaveBeenCalledWith(
      'lease-foreign',
      'turn-foreign-stopped',
      'take_over'
    );
    await expect(controller.handleSubmit('explicitly re-entered after takeover')).resolves.toBe(
      true
    );
    expect(vi.mocked(reply).mock.calls[0][0].body).toMatchObject({
      continuation_lease: 'lease-taken-over',
    });
  });

  it('abandons the lease when the replacement reply is authoritatively refused', async () => {
    const registry = new ChatStreamRegistry();
    const original = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('lease-refused') },
    } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({ stream: original.stream } as never)
      .mockRejectedValueOnce(new Error('HTTP 409: replacement refused'));

    const controller = registry.getController('lease-refused');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    await expect(controller.stopStreaming(true)).resolves.toBe(true);
    original.close();
    await first;

    await expect(controller.handleSubmit('replacement')).resolves.toBe(true);
    expect(vi.mocked(reply).mock.calls[1][0].body).toMatchObject({
      continuation_lease: 'lease-default',
    });
    expect(abandonContinuationLease).toHaveBeenCalledWith('lease-refused', 'lease-default');
  });

  it('preserves the lease through an ambiguous retry and abandons it when reattach fails', async () => {
    const registry = new ChatStreamRegistry();
    const original = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('lease-retry-abandoned') },
    } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({ stream: original.stream } as never)
      .mockRejectedValueOnce(new TypeError('Failed to fetch'))
      .mockRejectedValueOnce(new Error('HTTP 409: exact turn was never admitted'));

    const controller = registry.getController('lease-retry-abandoned');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    await expect(controller.stopStreaming(true)).resolves.toBe(true);
    original.close();
    await first;

    await controller.handleSubmit('replacement');
    const replacementTurnId = vi.mocked(reply).mock.calls[1][0].body.turn_id;
    await controller.retryTurn();

    expect(vi.mocked(reply).mock.calls[1][0].body).toMatchObject({
      continuation_lease: 'lease-default',
    });
    expect(vi.mocked(reply).mock.calls[2][0].body).toMatchObject({
      turn_id: replacementTurnId,
      continuation_lease: 'lease-default',
    });
    expect(abandonContinuationLease).toHaveBeenCalledWith('lease-retry-abandoned', 'lease-default');
  });

  it('abandons an admitted lease when renderer ownership closes before replacement submit', async () => {
    const registry = new ChatStreamRegistry();
    const original = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('lease-tab-close') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: original.stream } as never);

    const controller = registry.getController('lease-tab-close');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    await expect(controller.stopStreaming(true)).resolves.toBe(true);
    controller.releaseOwnership();
    await vi.waitFor(() =>
      expect(abandonContinuationLease).toHaveBeenCalledWith('lease-tab-close', 'lease-default')
    );
    original.close();
    await first;
  });

  it('retains an unacknowledged abandonment token and retries it idempotently', async () => {
    const registry = new ChatStreamRegistry();
    const original = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('lease-abandon-retry') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: original.stream } as never);
    vi.mocked(abandonContinuationLease)
      .mockRejectedValueOnce(new TypeError('abandon response lost'))
      .mockRejectedValueOnce(new TypeError('daemon still restarting'))
      .mockResolvedValueOnce(undefined);

    const controller = registry.getController('lease-abandon-retry');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    await expect(controller.stopStreaming(true)).resolves.toBe(true);

    controller.releaseOwnership();
    await vi.waitFor(() => expect(abandonContinuationLease).toHaveBeenCalledTimes(2));
    await controller.abandonContinuation();
    expect(abandonContinuationLease).toHaveBeenNthCalledWith(
      3,
      'lease-abandon-retry',
      'lease-default'
    );

    original.close();
    await first;
  });

  it('keeps a continuation lease until the lazy reply stream proves admission', async () => {
    const registry = new ChatStreamRegistry();
    const original = createControlledStream();
    const admitted = deferred<void>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('lease-lazy-admission') },
    } as never);
    vi.mocked(reply)
      .mockResolvedValueOnce({ stream: original.stream } as never)
      .mockResolvedValueOnce({
        stream: (async function* () {
          await admitted.promise;
          yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
        })(),
      } as never);

    const controller = registry.getController('lease-lazy-admission');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    await expect(controller.stopStreaming(true)).resolves.toBe(true);
    original.close();
    await first;

    const replacement = controller.handleSubmit('replacement');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(2));
    await controller.abandonContinuation();
    expect(abandonContinuationLease).toHaveBeenCalledWith('lease-lazy-admission', 'lease-default');

    admitted.resolve();
    await replacement;
  });

  it('upgrades an in-flight ordinary Stop before Stop-and-Send admits a successor', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const ordinaryCancellation = deferred<{ data: { cancelled: boolean; settled: boolean } }>();
    const continuationAdmission = deferred<{
      data: { cancelled: boolean; settled: boolean; continuation_lease: string };
    }>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('stop-intent-upgrade') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn)
      .mockReturnValueOnce(ordinaryCancellation.promise as never)
      .mockReturnValueOnce(continuationAdmission.promise as never);

    const controller = registry.getController('stop-intent-upgrade');
    const firstSubmit = controller.handleSubmit('original turn');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    const originalTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    const ordinaryStop = controller.stopStreaming(false);
    await vi.waitFor(() => expect(cancelTurn).toHaveBeenCalledTimes(1));
    expect(
      (vi.mocked(cancelTurn).mock.calls[0][0].body as Record<string, unknown>).continuation_pending
    ).toBe(false);

    const stopAndSend = controller.stopStreaming(true);
    expect(stopAndSend).not.toBe(ordinaryStop);
    expect(cancelTurn).toHaveBeenCalledTimes(1);

    controlled.push({
      type: 'Finish',
      reason: 'cancelled',
      token_state: tokenState,
    } as MessageEvent);
    controlled.close();
    await firstSubmit;
    ordinaryCancellation.resolve({ data: { cancelled: true, settled: true } });
    await expect(ordinaryStop).resolves.toBe(true);
    await vi.waitFor(() => expect(cancelTurn).toHaveBeenCalledTimes(2));

    expect(vi.mocked(cancelTurn).mock.calls[1][0].body).toEqual({
      session_id: 'stop-intent-upgrade',
      expected_turn_id: originalTurnId,
      wait_for_idle: true,
      continuation_pending: true,
      continuation_owner_id: 'test-window-owner',
    });
    await expect(controller.handleSubmit('must wait for the lease')).resolves.toBe(false);
    expect(reply).toHaveBeenCalledTimes(1);

    continuationAdmission.resolve({
      data: { cancelled: false, settled: true, continuation_lease: 'lease-upgrade' },
    });
    await expect(stopAndSend).resolves.toBe(true);

    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await expect(controller.handleSubmit('after the lease')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(2);
    expect(
      (vi.mocked(reply).mock.calls[1][0].body as Record<string, unknown>).continuation_lease
    ).toBe('lease-upgrade');
  });

  it('retries the stopped generation after an untyped cancel failure (#166)', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const firstCancellation = deferred<{ data: { cancelled: boolean; settled: boolean } }>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('stale-stop') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn).mockReturnValueOnce(firstCancellation.promise as never);

    const controller = registry.getController('stale-stop');
    const firstSubmit = controller.handleSubmit('original turn');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    const originalTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;

    const stopped = controller.stopStreaming(true);
    firstCancellation.reject(new Error('409: a different turn is active'));
    await expect(stopped).resolves.toBe(false);

    // #166 — the cancel request has RESOLVED (unsuccessfully), so the race the
    // gate exists to bridge is over and a terminal frame is free to land the
    // turn. Before the fix the gate stayed armed here for the rest of the
    // chat's life and the composer's working edge swept over a finished turn.
    controlled.push({
      type: 'Finish',
      reason: 'cancelled',
      token_state: tokenState,
    } as MessageEvent);
    controlled.close();
    await firstSubmit;

    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);

    // The generation itself is NOT forgotten: a retry still cancels the exact
    // turn the user stopped, and still carries the Stop-and-Send intent that
    // the failed attempt was made with.
    vi.mocked(cancelTurn).mockResolvedValueOnce({
      data: { cancelled: false, settled: true, continuation_lease: 'lease-retry' },
    } as never);
    await expect(controller.stopStreaming()).resolves.toBe(true);
    expect(vi.mocked(cancelTurn).mock.calls[1][0].body).toEqual({
      session_id: 'stale-stop',
      expected_turn_id: originalTurnId,
      wait_for_idle: true,
      continuation_pending: true,
      continuation_owner_id: 'test-window-owner',
    });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });

  /**
   * #166 — the composer's working edge is `isRunningState(chatState)`, so a
   * `chatState` that never returns to Idle is a figure that animates forever
   * over a finished turn. Each case below is one exit from the stop machinery
   * that used to leave the gate armed with nothing left to disarm it.
   */
  describe.each([
    {
      name: 'a cancel that rejects',
      continuationPending: false,
      arrange: () =>
        vi.mocked(cancelTurn).mockRejectedValueOnce(new Error('network died mid-cancel')),
    },
    {
      name: 'a cancel that reports settled: false',
      continuationPending: false,
      arrange: () =>
        vi.mocked(cancelTurn).mockResolvedValueOnce({
          data: { cancelled: true, settled: false },
        } as never),
    },
    {
      name: 'a Stop-and-Send response with no continuation lease',
      continuationPending: true,
      arrange: () =>
        vi.mocked(cancelTurn).mockResolvedValueOnce({
          data: { cancelled: true, settled: true },
        } as never),
    },
  ])('after $name, the finished turn still returns to Idle (#166)', (scenario) => {
    it('leaves the composer idle rather than animating forever', async () => {
      const registry = new ChatStreamRegistry();
      const controlled = createControlledStream();
      vi.mocked(resumeAgent).mockResolvedValue({
        data: { session: session('stranded-stop') },
      } as never);
      vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
      scenario.arrange();

      const controller = registry.getController('stranded-stop');
      const submit = controller.handleSubmit('long task');
      await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));

      await expect(controller.stopStreaming(scenario.continuationPending)).resolves.toBe(false);
      expect(cancelTurn).toHaveBeenCalledTimes(1);

      controlled.push({
        type: 'Finish',
        reason: 'done',
        token_state: tokenState,
      } as MessageEvent);
      controlled.close();
      await submit;

      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
      expect(isRunningState(controller.getSnapshot().chatState)).toBe(false);
    });

    it('lets the user send again instead of bricking the chat', async () => {
      const registry = new ChatStreamRegistry();
      const controlled = createControlledStream();
      vi.mocked(resumeAgent).mockResolvedValue({
        data: { session: session('stranded-stop-resend') },
      } as never);
      vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
      scenario.arrange();

      const controller = registry.getController('stranded-stop-resend');
      const submit = controller.handleSubmit('long task');
      await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
      await expect(controller.stopStreaming(scenario.continuationPending)).resolves.toBe(false);

      controlled.push({
        type: 'Finish',
        reason: 'done',
        token_state: tokenState,
      } as MessageEvent);
      controlled.close();
      await submit;

      vi.mocked(reply).mockResolvedValue({
        stream: (async function* () {
          yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
        })(),
      } as never);
      await expect(controller.handleSubmit('the chat still works')).resolves.toBe(true);
      expect(reply).toHaveBeenCalledTimes(2);
    });
  });

  it('adopts the authoritative successor after a typed stale-Stop mismatch', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('stale-stop-successor') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    const controller = registry.getController('stale-stop-successor');
    const first = controller.handleSubmit('original');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    const rendererTurnId = vi.mocked(reply).mock.calls[0][0].body.turn_id;
    vi.mocked(cancelTurn).mockRejectedValueOnce({
      mismatch: true,
      expected_turn_id: rendererTurnId,
      active_turn_id: 'successor-turn',
    });
    vi.mocked(cancelTurn).mockResolvedValueOnce({
      data: { cancelled: true, settled: true },
    } as never);

    await expect(controller.stopStreaming()).resolves.toBe(false);
    await expect(controller.stopStreaming()).resolves.toBe(true);
    expect(vi.mocked(cancelTurn).mock.calls[1][0].body).toMatchObject({
      expected_turn_id: 'successor-turn',
    });

    controlled.close();
    await first;
  });
});

/**
 * The daemon's synthesized "this turn produced no ending" frame. Shared by the
 * M2 and F5 batteries below, which both need a Stop's wedged-writer ending.
 */
function endedWithoutTerminal(): MessageEvent {
  return {
    type: 'Error',
    error: 'The stream for this turn ended without a result. Please retry.',
    code: 'stream_ended_without_terminal',
    scope: 'internal',
    retryable: true,
  } as MessageEvent;
}

/**
 * M2 — a Stop the daemon never confirms.
 *
 * The #166 battery above pushes its terminal frame AFTER the cancel has
 * resolved, which is the easy ordering: the Stop gate is already down, so
 * `finishCurrentStream` lands the turn itself. The wedged-daemon ordering is the
 * reverse and was never covered — the daemon's synthesized terminal frame
 * arrives WHILE the cancel is still on the wire, `finishCurrentStream` defers
 * the Idle transition to the cancel response because `stopGateHolds()`, and the
 * cancel then comes back a failure and declines to make it. Nothing else ever
 * does, so the composer keeps its Stop button and never gets Send back.
 *
 * `/agent/cancel`'s own failures make this the DEFAULT wedged shape rather than
 * an exotic one: `CancelTurnFailure::SettlementTimeout` is a bare 504 with no
 * body, so the generated client throws a literal `{}` (`finalError = finalError
 * || {}`) and the console warning printed nothing a person could act on.
 */
describe('ChatStreamRegistry — a Stop the daemon never confirms (M2)', () => {
  /**
   * Drive a Stop whose cancel is still on the wire when the daemon's terminal
   * frame lands, then settle the cancel however the caller asks.
   */
  async function stopAgainstAWedgedTurn(
    sessionId: string,
    settleCancel: (cancellation: ReturnType<typeof deferred<unknown>>) => void
  ) {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const cancellation = deferred<unknown>();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn).mockReturnValueOnce(cancellation.promise as never);

    const controller = registry.getController(sessionId);
    const submit = controller.handleSubmit('a turn that wedges');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));

    const stopped = controller.stopStreaming();
    await flush();
    expect(cancelTurn).toHaveBeenCalledTimes(1);

    // The terminal frame beats the cancel response — the whole of M2.
    controlled.push(endedWithoutTerminal());
    controlled.close();
    await submit;

    settleCancel(cancellation);
    await expect(stopped).resolves.toBe(false);
    await flush();

    return { controller, registry };
  }

  it('returns the composer to Idle when the cancel fails after the turn already ended', async () => {
    const { controller } = await stopAgainstAWedgedTurn('wedged-idle', (cancellation) =>
      // A bare 504 with no body: exactly what the generated client throws.
      cancellation.reject({})
    );

    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(isRunningState(controller.getSnapshot().chatState)).toBe(false);
  });

  it('lets the user send again rather than stranding the chat with no Send control', async () => {
    const { controller } = await stopAgainstAWedgedTurn('wedged-resend', (cancellation) =>
      cancellation.reject({})
    );

    // The composer draws Send or Stop from `isRunningState(chatState)` — so the
    // store accepting a submit is NOT the same as the user having a control to
    // click. Measured in the running app: `textarea.disabled === false` with no
    // `Send message` button anywhere on screen.
    expect(isRunningState(controller.getSnapshot().chatState)).toBe(false);

    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);
    await expect(controller.handleSubmit('the chat still works')).resolves.toBe(true);
    expect(reply).toHaveBeenCalledTimes(2);
  });

  it('logs the HTTP status of a bodyless cancel rejection instead of an empty object', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      // The fields form the generated client returns for an HTTP failure when
      // it is not asked to throw: the body is `{}` and the status is the only
      // thing that says WHICH failure this was.
      await stopAgainstAWedgedTurn('wedged-log', (cancellation) =>
        cancellation.resolve({ error: {}, response: { status: 504 } })
      );

      const logged = warn.mock.calls.map((call) => call.map((part) => String(part)).join(' '));
      const stopLine = logged.find((line) => line.includes('cancel'));
      expect(stopLine).toBeDefined();
      expect(stopLine).toContain('504');
      expect(stopLine).toContain('wedged-log');
      // `String({})` is `[object Object]`; a raw object argument is exactly what
      // printed `{}` in the running app.
      expect(stopLine).not.toContain('[object Object]');
    } finally {
      warn.mockRestore();
    }
  });

  it('logs the real error text when the cancel rejects with an Error', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      await stopAgainstAWedgedTurn('wedged-log-error', (cancellation) =>
        cancellation.reject(new Error('network died mid-cancel'))
      );

      const logged = warn.mock.calls.map((call) => call.map((part) => String(part)).join(' '));
      expect(logged.some((line) => line.includes('network died mid-cancel'))).toBe(true);
    } finally {
      warn.mockRestore();
    }
  });

  it('tells the user in the chat that the stop was not confirmed', async () => {
    const { controller } = await stopAgainstAWedgedTurn('wedged-notice', (cancellation) =>
      cancellation.reject({})
    );

    const turnError = controller.getSnapshot().turnError;
    expect(turnError?.code).toBe('stop_not_confirmed');
    expect(turnError?.message).toMatch(/could not/i);
    expect(turnError?.message).toMatch(/still be running/i);
    // Retrying the TURN is not what a user who asked to stop it wants.
    expect(turnError?.retryable).toBe(false);
  });

  it('does not blame the model for an ending the user asked for', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const cancellation = deferred<unknown>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('wedged-blame') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn).mockReturnValueOnce(cancellation.promise as never);

    const controller = registry.getController('wedged-blame');
    const submit = controller.handleSubmit('a turn that wedges');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    const stopped = controller.stopStreaming();
    await flush();

    controlled.push(endedWithoutTerminal());
    controlled.close();
    await submit;

    // Sampled BEFORE the cancel answers: this is the card the user actually
    // stares at while the daemon is wedged.
    expect(controller.getSnapshot().turnError?.code).toBe('turn_stopped_by_user');
    expect(controller.getSnapshot().turnError?.retryable).toBe(false);

    cancellation.reject({});
    await expect(stopped).resolves.toBe(false);
  });

  it('retracts the unconfirmed-stop notice once the turn really does end', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    const cancellation = deferred<unknown>();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('wedged-retract') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);
    vi.mocked(cancelTurn).mockReturnValueOnce(cancellation.promise as never);

    const controller = registry.getController('wedged-retract');
    const submit = controller.handleSubmit('a turn that outlives its Stop');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));

    // The reverse of the M2 ordering: the cancel fails while the turn is still
    // genuinely streaming, so the composer keeps Stop and the notice says so.
    const stopped = controller.stopStreaming();
    await flush();
    cancellation.reject({});
    await expect(stopped).resolves.toBe(false);
    await flush();

    expect(controller.getSnapshot().turnError?.code).toBe('stop_not_confirmed');
    expect(controller.getSnapshot().turnError?.message).toMatch(/Press Stop again/);
    expect(isRunningState(controller.getSnapshot().chatState)).toBe(true);

    // …and then the turn ends anyway. "Press Stop again to retry" now describes
    // a world that no longer exists.
    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submit;

    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });

  it('still reports an ordinary internal failure as a model failure when no Stop is pending', async () => {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: { session: session('unstopped-internal') },
    } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('unstopped-internal');
    const submit = controller.handleSubmit('an ordinary turn');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));

    controlled.push(endedWithoutTerminal());
    controlled.close();
    await submit;

    expect(controller.getSnapshot().turnError?.code).toBe('stream_ended_without_terminal');
    expect(controller.getSnapshot().turnError?.scope).toBe('internal');
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });
});

/**
 * F5 (QA of 7c96d796, 2026-09-10) — a Stop the daemon CONFIRMS.
 *
 * M2 gave the failed Stop its notice; the successful one still said nothing.
 * Measured in the running app: Send came back 150 ms after the press, and the
 * transcript held the user's message with no reply and no word about why.
 *
 * The notice is keyed on the daemon's own answer, not on the renderer's hope.
 * `cancelled: true` means the cancel found the turn running and tripped it;
 * `cancelled: false` is the idempotent 200 for a turn that had already ended,
 * which is exactly the turn that must never be described as stopped.
 */
describe('ChatStreamRegistry — a Stop the daemon confirms (F5)', () => {
  /** A turn that stays open until the test ends it. */
  async function startLongTurn(sessionId: string) {
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session(sessionId) } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController(sessionId);
    const submit = controller.handleSubmit('a long turn');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(1));
    return { controller, controlled, submit };
  }

  /** Stop, and let the daemon's terminal frame land before its cancel answer. */
  async function stopWithFrameFirst(sessionId: string, frame: MessageEvent, answer: unknown) {
    const { controller, controlled, submit } = await startLongTurn(sessionId);
    const cancellation = deferred<unknown>();
    vi.mocked(cancelTurn).mockReturnValueOnce(cancellation.promise as never);

    const stopped = controller.stopStreaming();
    await flush();
    controlled.push(frame);
    controlled.close();
    await submit;

    cancellation.resolve(answer);
    return { controller, stopped };
  }

  it('says the turn was stopped once the daemon confirms it', async () => {
    const { controller, controlled, submit } = await startLongTurn('stop-confirmed');
    vi.mocked(cancelTurn).mockResolvedValueOnce({
      data: { cancelled: true, settled: true },
    } as never);

    await expect(controller.stopStreaming()).resolves.toBe(true);

    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();
    // A quiet line, not a card: nothing about this ending is an error.
    expect(controller.getSnapshot().turnError).toBeUndefined();

    controlled.close();
    await submit;
  });

  // The healthy daemon's other ordering: its real `Finish { reason: cancelled }`
  // beats the cancel response, so the Stop gate defers the Idle transition to it.
  it('says so when the cancelled turn’s own ending arrives before the confirmation', async () => {
    const { controller, stopped } = await stopWithFrameFirst(
      'stop-confirmed-frame-first',
      { type: 'Finish', reason: 'cancelled', token_state: tokenState } as MessageEvent,
      { data: { cancelled: true, settled: true } }
    );

    await expect(stopped).resolves.toBe(true);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();
    expect(controller.getSnapshot().turnError).toBeUndefined();
  });

  /**
   * A wedged writer ends the turn with the daemon's synthesized frame, which
   * M2 re-codes to a "Turn stopped" CARD while the cancel is still out. Once the
   * cancel confirms, that card and this line would say one thing twice, in an
   * error's voice and a status's; the confirmation is the stronger evidence.
   */
  it('replaces the interim “Turn stopped” card once the cancel confirms', async () => {
    const { controller, stopped } = await stopWithFrameFirst(
      'stop-confirmed-wedged-writer',
      endedWithoutTerminal(),
      { data: { cancelled: true, settled: true } }
    );

    await expect(stopped).resolves.toBe(true);
    expect(controller.getSnapshot().turnError).toBeUndefined();
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();
  });

  it('is transient — it retracts itself after its display window', async () => {
    const { controller, controlled, submit } = await startLongTurn('stop-confirmed-transient');
    await expect(controller.stopStreaming()).resolves.toBe(true);
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();

    vi.advanceTimersByTime(STOP_CONFIRMED_NOTICE_MS - 1);
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();
    vi.advanceTimersByTime(1);
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();

    controlled.close();
    await submit;
  });

  it('is retracted the moment the next turn starts', async () => {
    const { controller, controlled, submit } = await startLongTurn('stop-confirmed-next-turn');
    await expect(controller.stopStreaming()).resolves.toBe(true);
    expect(controller.getSnapshot().stopConfirmed).toBeDefined();
    controlled.close();
    await submit;

    const next = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: next.stream } as never);
    const nextSubmit = controller.handleSubmit('carry on');
    await vi.waitFor(() => expect(reply).toHaveBeenCalledTimes(2));

    // While the new turn runs — not merely once it has ended.
    expect(isRunningState(controller.getSnapshot().chatState)).toBe(true);
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();

    next.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    next.close();
    await nextSubmit;
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();
  });

  // ---- The endings that must NOT claim a stop. These pass before the fix too;
  // they are what keeps the fix from being "announce every Idle". ----

  it('does not appear for a turn that ended on its own', async () => {
    const { controller, controlled, submit } = await startLongTurn('ended-on-its-own');

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submit;

    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();
  });

  // The race a Stop can lose: the turn finished by itself a moment before the
  // cancel reached it, and the daemon answers that nothing was running.
  it('does not appear when the turn had already finished before the cancel reached it', async () => {
    const { controller, stopped } = await stopWithFrameFirst(
      'finished-before-the-cancel',
      { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent,
      { data: { cancelled: false, settled: true } }
    );

    await expect(stopped).resolves.toBe(true);
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();
  });

  it('does not appear beside the notice for a Stop that was never confirmed', async () => {
    // Installed first: the failed cancel logs from a microtask that runs before
    // the helper below hands control back.
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => {});
    try {
      const { controller, stopped } = await stopWithFrameFirst(
        'stop-unconfirmed',
        endedWithoutTerminal(),
        // A bare 504: the daemon's settlement timeout.
        { error: {}, response: { status: 504 } }
      );
      await expect(stopped).resolves.toBe(false);
      await flush();

      expect(controller.getSnapshot().turnError?.code).toBe('stop_not_confirmed');
      expect(controller.getSnapshot().stopConfirmed).toBeUndefined();
    } finally {
      warn.mockRestore();
    }
  });

  // Stop-and-Send is followed at once by the user's replacement turn, and that
  // turn is the outcome; a "Stopped." line would flash for the length of a submit.
  it('does not appear for a Stop-and-Send', async () => {
    const { controller, controlled, submit } = await startLongTurn('stop-and-send');

    await expect(controller.stopStreaming(true)).resolves.toBe(true);
    expect(controller.getSnapshot().pendingContinuation?.ownership).toBe('owned');
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();

    controlled.close();
    await submit;
  });

  // The case above's harder sibling: an ORDINARY Stop the user upgrades while
  // it is still on the wire. That Stop's own cancel comes back `cancelled:
  // true` — and the replacement turn is still the outcome.
  it('does not appear for an ordinary Stop upgraded to Stop-and-Send mid-flight', async () => {
    const { controller, controlled, submit } = await startLongTurn('stop-upgraded-mid-flight');
    const ordinaryCancellation = deferred<unknown>();
    const continuationAdmission = deferred<unknown>();
    vi.mocked(cancelTurn)
      .mockReturnValueOnce(ordinaryCancellation.promise as never)
      .mockReturnValueOnce(continuationAdmission.promise as never);

    const ordinaryStop = controller.stopStreaming(false);
    await vi.waitFor(() => expect(cancelTurn).toHaveBeenCalledTimes(1));
    const stopAndSend = controller.stopStreaming(true);

    ordinaryCancellation.resolve({ data: { cancelled: true, settled: true } });
    await expect(ordinaryStop).resolves.toBe(true);
    // Between the two requests: exactly where a line keyed on the ordinary
    // Stop alone would flash.
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();

    continuationAdmission.resolve({
      data: { cancelled: false, settled: true, continuation_lease: 'lease-upgraded' },
    });
    await expect(stopAndSend).resolves.toBe(true);
    expect(controller.getSnapshot().stopConfirmed).toBeUndefined();

    controlled.close();
    await submit;
  });
});

/**
 * Progressive conversation loading.
 *
 * A resume used to take ~5.1s on a real 355-message session, of which ~4.6s was
 * starting 9 extensions that contribute 359 bytes to what the user reads. The
 * transcript now paints first and the agent follows. These tests pin the two
 * things that makes fragile: the ORDER, and the fact that a submit landing in
 * the gap must not be eaten.
 */
describe('ChatStreamRegistry — progressive loading', () => {
  /** A resume mock whose model+extension phase we can hold open on demand. */
  function stagedResume(sessionId = 's1') {
    let releaseAgent: (() => void) | null = null;
    const agentGate = new Promise<void>((resolve) => {
      releaseAgent = resolve;
    });

    vi.mocked(resumeAgent).mockImplementation(async (opts: unknown) => {
      const body = (opts as { body: { load_model_and_extensions: boolean } }).body;
      if (!body.load_model_and_extensions) {
        // Phase 1: the transcript. Fast.
        return { data: { session: session(sessionId) } } as never;
      }
      // Phase 2: the slow half.
      await agentGate;
      return {
        data: {
          session: session(sessionId),
          extension_results: [{ name: 'developer', success: true, error: null }],
        },
      } as never;
    });

    return { releaseAgent: () => releaseAgent!() };
  }

  it('paints the transcript without waiting for the model and extensions', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('prog-paint');

    const controller = registry.getController('prog-paint');
    await controller.loadSession();

    // The conversation is up and interactive while the agent is still loading.
    expect(controller.getSnapshot().session).toMatchObject({ id: 'prog-paint' });
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
    expect(controller.getSnapshot().agentReady).toBe(false);

    releaseAgent();
    await vi.waitFor(() => expect(controller.getSnapshot().agentReady).toBe(true));
  });

  it('fetches the transcript alone first, then the agent — in that order', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('prog-order');

    const controller = registry.getController('prog-order');
    await controller.loadSession();

    const flags = vi
      .mocked(resumeAgent)
      .mock.calls.map(
        (c) =>
          (c[0] as { body: { load_model_and_extensions: boolean } }).body.load_model_and_extensions
      );
    expect(flags[0]).toBe(false);
    expect(flags).toContain(true);

    releaseAgent();
    await flush();
  });

  // THE one that matters. A snappy UI that eats your first message is a
  // downgrade, not an upgrade.
  it('HOLDS a submit made before the agent is ready — never drops it', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('prog-hold');
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield { type: 'Finish', reason: 'stop' } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('prog-hold');
    await controller.loadSession();
    expect(controller.getSnapshot().agentReady).toBe(false);

    // The user types into a transcript whose agent is still starting.
    const submit = controller.handleSubmit('the first thing I typed');
    await flush();

    // Not dropped, and not invisible: the message is already in the transcript
    // and the chat reads as working, so the user has feedback.
    const painted = controller.getSnapshot().messages;
    expect(painted[painted.length - 1]).toMatchObject({ role: 'user' });
    expect(controller.getSnapshot().chatState).not.toBe(ChatState.Idle);
    // ...but it has NOT been sent to an agent that cannot serve it yet.
    expect(reply).not.toHaveBeenCalled();

    releaseAgent();
    await submit;

    // It went out the moment the agent landed, intact.
    expect(reply).toHaveBeenCalledTimes(1);
    const body = vi.mocked(reply).mock.calls[0][0].body as { user_message: Message };
    expect(body.user_message.content).toMatchObject([
      { type: 'text', text: 'the first thing I typed' },
    ]);
  });

  it('lets the user Stop a turn parked on a slow agent load, without sending it', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('prog-stop');

    const controller = registry.getController('prog-stop');
    await controller.loadSession();

    const submit = controller.handleSubmit('never mind');
    await flush();
    controller.stopStreaming();

    releaseAgent();
    await submit;

    // Cancelled while parked: the request must never reach the model.
    expect(reply).not.toHaveBeenCalled();
  });

  it('loads the agent even when the transcript came from cache', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('cached-1');

    // Prime the module-level LRU by loading once...
    const first = registry.getController('cached-1');
    await first.loadSession();
    releaseAgent();
    await flush();
    vi.mocked(resumeAgent).mockClear();

    // ...then reach the session through a brand new controller, which takes the
    // cache fast-path. It must STILL load the agent: this path previously could
    // reach a submit having never started a single extension.
    const { releaseAgent: release2 } = stagedResume('cached-1');
    const fresh = new ChatStreamRegistry().getController('cached-1');
    await fresh.loadSession();

    const agentCalls = vi
      .mocked(resumeAgent)
      .mock.calls.filter(
        (c) =>
          (c[0] as { body: { load_model_and_extensions: boolean } }).body.load_model_and_extensions
      );
    expect(agentCalls.length).toBeGreaterThan(0);
    release2();
    await flush();
  });

  it('only loads the agent once, however many callers ask', async () => {
    const registry = new ChatStreamRegistry();
    const { releaseAgent } = stagedResume('prog-once');

    const controller = registry.getController('prog-once');
    await Promise.all([
      controller.loadSession(),
      controller.loadSession(),
      controller.loadSession(),
    ]);
    releaseAgent();
    await flush();
    await controller.loadSession();

    const agentCalls = vi
      .mocked(resumeAgent)
      .mock.calls.filter(
        (c) =>
          (c[0] as { body: { load_model_and_extensions: boolean } }).body.load_model_and_extensions
      );
    expect(agentCalls).toHaveLength(1);
  });

  // A failed agent load must not park submits forever — readiness means
  // "settled", not "succeeded".
  it('becomes ready, with an inline error, when the agent fails to load', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockImplementation(async (opts: unknown) => {
      const body = (opts as { body: { load_model_and_extensions: boolean } }).body;
      if (!body.load_model_and_extensions) {
        return { data: { session: session('prog-agentfail') } } as never;
      }
      throw new Error('extension manager unavailable');
    });

    const controller = registry.getController('prog-agentfail');
    await controller.loadSession();
    await flush();

    // The transcript survived a failed agent load...
    expect(controller.getSnapshot().session).toMatchObject({ id: 'prog-agentfail' });
    expect(controller.getSnapshot().sessionLoadError).toBeUndefined();
    // ...and the failure is visible rather than silent.
    expect(controller.getSnapshot().agentReady).toBe(true);
    expect(controller.getSnapshot().turnError).toMatchObject({ code: 'agent_load_failed' });
  });

  it('still reports a full resume failure as a session-level error', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockRejectedValue(new Error('session database unavailable'));

    const controller = registry.getController('prog-resumefail');
    await controller.loadSession();

    expect(controller.getSnapshot().sessionLoadError).toContain('session database unavailable');
    expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
  });
});

describe('ChatStreamRegistry turn timestamps', () => {
  it('stamps lastMessageAt per live Message event and advances it on the next one', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('ts1') } } as never);

    const controlled = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('ts1');
    const submitted = controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();

    // Submit stamps a turn origin so the indicator has a fallback before any
    // message lands.
    const afterSubmit = controller.getSnapshot();
    expect(afterSubmit.turnStartedAt).toBeTypeOf('number');
    expect(afterSubmit.lastMessageAt).toBeUndefined();

    controlled.push({
      type: 'Message',
      message: assistantMessage('m1', 'first'),
      token_state: tokenState,
    } as MessageEvent);
    await flush();
    const first = controller.getSnapshot().lastMessageAt;
    expect(first).toBeTypeOf('number');

    vi.advanceTimersByTime(5000);
    controlled.push({
      type: 'Message',
      message: assistantMessage('m2', 'second'),
      token_state: tokenState,
    } as MessageEvent);
    await flush();
    const second = controller.getSnapshot().lastMessageAt;
    expect(second).toBeGreaterThan(first as number);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;
  });

  it('clears both timestamps when the turn finishes', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('ts2') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: assistantMessage('m1', 'hello'),
          token_state: tokenState,
        } as MessageEvent;
        yield { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('ts2');
    await controller.handleSubmit('go');
    await flush();

    const snapshot = controller.getSnapshot();
    expect(snapshot.chatState).toBe(ChatState.Idle);
    expect(snapshot.turnStartedAt).toBeUndefined();
    expect(snapshot.lastMessageAt).toBeUndefined();
  });

  it('clears both timestamps when the turn errors', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('ts3') } } as never);
    vi.mocked(reply).mockResolvedValue({
      stream: (async function* () {
        yield {
          type: 'Message',
          message: assistantMessage('m1', 'partial'),
          token_state: tokenState,
        } as MessageEvent;
        yield {
          type: 'Error',
          error: 'provider exploded',
          code: 'inference_failed',
          token_state: tokenState,
        } as unknown as MessageEvent;
      })(),
    } as never);

    const controller = registry.getController('ts3');
    await controller.handleSubmit('go');
    await flush();

    const snapshot = controller.getSnapshot();
    expect(snapshot.turnStartedAt).toBeUndefined();
    expect(snapshot.lastMessageAt).toBeUndefined();
  });

  it('clears both timestamps when the user stops the turn', async () => {
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('ts4') } } as never);

    const controlled = createControlledStream();
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('ts4');
    void controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();

    controlled.push({
      type: 'Message',
      message: assistantMessage('m1', 'working'),
      token_state: tokenState,
    } as MessageEvent);
    await flush();
    expect(controller.getSnapshot().lastMessageAt).toBeTypeOf('number');

    controller.stopStreaming();
    await flush();

    const snapshot = controller.getSnapshot();
    expect(snapshot.chatState).toBe(ChatState.Idle);
    expect(snapshot.turnStartedAt).toBeUndefined();
    expect(snapshot.lastMessageAt).toBeUndefined();

    controlled.close();
  });

  it('does not stamp lastMessageAt when replaying a saved session', async () => {
    // The historical-session guarantee at the store level: loading a transcript
    // must never look like a live event, or a replay would inherit a clock.
    const registry = new ChatStreamRegistry();
    vi.mocked(resumeAgent).mockResolvedValue({
      data: {
        session: {
          ...session('ts5'),
          conversation: [assistantMessage('old-1', 'from last week')],
          message_count: 1,
        },
      },
    } as never);

    const controller = registry.getController('ts5');
    await controller.loadSession();
    await flush();

    const snapshot = controller.getSnapshot();
    expect(snapshot.messages.length).toBeGreaterThan(0);
    expect(snapshot.lastMessageAt).toBeUndefined();
    expect(snapshot.turnStartedAt).toBeUndefined();
  });
});

// #22 — streaming-notification batching. Token streaming delivers dozens of
// events per second; without coalescing + per-frame batching, every event woke
// every subscriber (BaseChat, ChatInput, the sidebar) synchronously, which is
// what made typing lag while a response streamed.
describe('notification batching (#22)', () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  /** Queue rAF callbacks so the test controls exactly when a frame "renders". */
  function stubRafQueue() {
    const queue: Array<(time: number) => void> = [];
    vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void): number => {
      queue.push(cb);
      return queue.length;
    });
    vi.stubGlobal('cancelAnimationFrame', (_id: number): void => undefined);
    return {
      queue,
      runFrame() {
        for (const cb of queue.splice(0)) cb(0);
      },
    };
  }

  /** rAF that fires synchronously: every scheduled flush lands immediately, so
   * the listener call count equals the number of snapshot swaps. */
  function stubRafSync() {
    vi.stubGlobal('requestAnimationFrame', (cb: (time: number) => void): number => {
      cb(0);
      return 0;
    });
    vi.stubGlobal('cancelAnimationFrame', (_id: number): void => undefined);
  }

  it('a burst of Message events wakes subscribers at most once per frame', async () => {
    const raf = stubRafQueue();
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('batch-1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('batch-1');
    const submitted = controller.handleSubmit('stream a lot');
    await vi.advanceTimersByTimeAsync(0);
    await flush();
    // The turn is launched (its boundary flush is behind us) before the
    // listener subscribes, so only stream-event notifications are counted.
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);

    for (let i = 0; i < 50; i++) {
      controlled.push({
        type: 'Message',
        message: assistantMessage('a1', `token-${i} `),
        token_state: tokenState,
      } as MessageEvent);
      await flush();
    }

    // Snapshot writes are synchronous — the transcript already holds every
    // token…
    const streamed = controller.getSnapshot().messages.find((m) => m.id === 'a1');
    const text = streamed?.content[0].type === 'text' ? streamed.content[0].text : '';
    expect(text).toContain('token-0');
    expect(text).toContain('token-49');
    // …but no frame has rendered yet, so subscribers were never woken.
    expect(listener).not.toHaveBeenCalled();

    raf.runFrame();
    expect(listener).toHaveBeenCalledTimes(1);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;
    unsubscribe();
  });

  it('setting the same chat state again never notifies subscribers', () => {
    const raf = stubRafQueue();
    const registry = new ChatStreamRegistry();
    const controller = registry.getController('noop-state');
    const listener = vi.fn();
    controller.subscribe(listener);

    controller.setChatState(ChatState.Idle); // already Idle — a no-op

    raf.runFrame();
    expect(listener).not.toHaveBeenCalled();
  });

  it('one streamed Message event produces exactly one notification', async () => {
    // Synchronous rAF: every snapshot swap notifies immediately, so the call
    // count below counts SWAPS. Before the applyMessageEvent coalescing, one
    // Message event ran three swaps (chat state + token state + messages).
    stubRafSync();
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('one-1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('one-1');
    const submitted = controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);

    controlled.push({
      type: 'Message',
      message: assistantMessage('a1', 'hello'),
      token_state: tokenState,
    } as MessageEvent);
    await flush();

    expect(listener).toHaveBeenCalledTimes(1);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;
    unsubscribe();
  });

  it('token events with an unchanged running entry do not re-notify running listeners', async () => {
    stubRafSync();
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('run-skip') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const runningListener = vi.fn();
    registry.subscribeRunning(runningListener);

    const controller = registry.getController('run-skip');
    const submitted = controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();

    expect(registry.getRunningSnapshot()).toMatchObject([
      { sessionId: 'run-skip', chatState: ChatState.Streaming },
    ]);
    const callsAfterStart = runningListener.mock.calls.length;
    const snapshotAfterStart = registry.getRunningSnapshot();

    for (let i = 0; i < 5; i++) {
      controlled.push({
        type: 'Message',
        message: assistantMessage('a1', `token-${i} `),
        token_state: tokenState,
      } as MessageEvent);
      await flush();
    }

    // Still Streaming with the same title and start time: the sidebar/tab
    // strip must not have been re-rendered per token.
    expect(runningListener.mock.calls.length).toBe(callsAfterStart);
    expect(registry.getRunningSnapshot()).toBe(snapshotAfterStart);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;

    // The Idle transition is material and does re-notify.
    expect(runningListener.mock.calls.length).toBeGreaterThan(callsAfterStart);
    expect(registry.getRunningSnapshot()[0]).toMatchObject({
      sessionId: 'run-skip',
      chatState: ChatState.Idle,
    });
  });

  it('an event scheduled while visible still flushes when the rAF never fires (hidden window)', async () => {
    // Chromium PAUSES rAF in hidden windows, and the dangerous ordering is:
    // the flush is scheduled while the window is VISIBLE (so rAF is armed),
    // then the window hides before the frame runs — the callback parks
    // forever, and with `notifyScheduled` latched no later event can re-arm
    // anything. Model the parked frame with an rAF that accepts callbacks and
    // never runs them; the setTimeout fallback armed alongside it must
    // deliver the notification within its budget.
    const rafCancelled: number[] = [];
    vi.stubGlobal('requestAnimationFrame', (_cb: (time: number) => void): number => 7);
    vi.stubGlobal('cancelAnimationFrame', (id: number): void => {
      rafCancelled.push(id);
    });

    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('hidden-1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('hidden-1');
    const submitted = controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();
    expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);
    rafCancelled.length = 0;

    // The state transition users must not miss: a tool confirmation flips the
    // store to WaitingForUserInput — exactly what stalled invisibly before.
    controlled.push({
      type: 'Message',
      message: {
        id: 'confirm-1',
        role: 'assistant',
        created: 2,
        // ⚠ The daemon's real shape. This was `toolConfirmationRequest`, a
        // variant no Rust code constructs, so the assertion below held
        // against a predicate that never fired for a real card.
        content: [
          {
            type: 'actionRequired',
            data: { actionType: 'toolConfirmation', id: 'tc-1', toolName: 'developer__shell' },
          },
        ],
        metadata: { userVisible: true, agentVisible: true },
      },
      token_state: tokenState,
    } as unknown as MessageEvent);
    await flush();

    // The snapshot is current (writes are synchronous)…
    expect(controller.getSnapshot().chatState).toBe(ChatState.WaitingForUserInput);
    // …but no notification yet: the rAF is parked and no time has passed.
    expect(listener).not.toHaveBeenCalled();

    // The fallback fires within its budget and wakes the subscriber.
    await vi.advanceTimersByTimeAsync(NOTIFY_FALLBACK_MS);
    expect(listener).toHaveBeenCalledTimes(1);
    // The parked rAF was cancelled, so re-showing the window can't double-fire.
    expect(rafCancelled).toContain(7);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;
    unsubscribe();
  });

  it('never double-flushes when both the rAF and the fallback timer are armed', async () => {
    const raf = stubRafQueue();
    const registry = new ChatStreamRegistry();
    const controlled = createControlledStream();
    vi.mocked(resumeAgent).mockResolvedValue({ data: { session: session('race-1') } } as never);
    vi.mocked(reply).mockResolvedValue({ stream: controlled.stream } as never);

    const controller = registry.getController('race-1');
    const submitted = controller.handleSubmit('go');
    await vi.advanceTimersByTimeAsync(0);
    await flush();

    const listener = vi.fn();
    const unsubscribe = controller.subscribe(listener);

    // Round 1: the rAF wins the race…
    controlled.push({
      type: 'Message',
      message: assistantMessage('a1', 'token-0 '),
      token_state: tokenState,
    } as MessageEvent);
    await flush();
    raf.runFrame();
    expect(listener).toHaveBeenCalledTimes(1);
    // …and the fallback slot passing delivers nothing more: it was cancelled.
    await vi.advanceTimersByTimeAsync(NOTIFY_FALLBACK_MS * 2);
    expect(listener).toHaveBeenCalledTimes(1);

    // Round 2: the fallback wins…
    controlled.push({
      type: 'Message',
      message: assistantMessage('a1', 'token-1 '),
      token_state: tokenState,
    } as MessageEvent);
    await flush();
    await vi.advanceTimersByTimeAsync(NOTIFY_FALLBACK_MS);
    expect(listener).toHaveBeenCalledTimes(2);
    // …and the parked frame finally running (stub cancelAnimationFrame is a
    // no-op, as in a window whose paused rAF resumes) is a no-op flush, not a
    // second notification.
    raf.runFrame();
    expect(listener).toHaveBeenCalledTimes(2);

    controlled.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
    controlled.close();
    await submitted;
    unsubscribe();
  });
});

/**
 * BR-71: `isRunningState` is the app's ONE answer to "is a turn live?", and the
 * whole reason it is exported is that the answer is not the obvious
 * `!== Idle`. Loading a conversation is not a running turn, and a surface that
 * gets that wrong offers the user a kill switch for a turn that does not exist
 * — which is exactly what the subagent header's Stop button did before it was
 * pointed at this function.
 */
describe('isRunningState', () => {
  it('does not count loading a conversation as a live turn', () => {
    // The state every session load starts in. `!== ChatState.Idle` — the naive
    // predicate — is TRUE here, which is the bug this pins shut.
    expect(isRunningState(ChatState.LoadingConversation)).toBe(false);
    expect(isRunningState(ChatState.Idle)).toBe(false);
  });

  it('counts every state in which the agent is actually working', () => {
    expect(isRunningState(ChatState.Thinking)).toBe(true);
    expect(isRunningState(ChatState.Streaming)).toBe(true);
    expect(isRunningState(ChatState.WaitingForUserInput)).toBe(true);
    expect(isRunningState(ChatState.Compacting)).toBe(true);
    expect(isRunningState(ChatState.RestartingAgent)).toBe(true);
  });
});
