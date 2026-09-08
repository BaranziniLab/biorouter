import { beforeEach, describe, expect, it, vi, afterEach } from 'vitest';
import type { Message, MessageEvent, Session, TokenState } from '../api';

const mocks = vi.hoisted(() => ({
  observeSessionEvents: vi.fn(),
  // The rest of the surface a driving controller touches. Mocked so the
  // ownership tests below can put a REAL /reply turn in flight without any of
  // it reaching the generated client (and so nothing in this file depends on
  // `fetch` or on how fast the machine is).
  reply: vi.fn(),
  resumeAgent: vi.fn(),
  interrupt: vi.fn(async () => ({ data: {} })),
  cancelTurn: vi.fn(async () => ({
    data: { cancelled: true, settled: true, continuation_lease: 'observer-test-lease' },
  })),
  getSession: vi.fn(async () => ({ data: null })),
  listSessions: vi.fn(async () => ({ data: { sessions: [] } })),
  // Part of agent readiness, which a submit AWAITS: left real, it reaches the
  // generated client and the turn never launches.
  updateFromSession: vi.fn(async () => ({ data: {} })),
  updateSessionUserWorkflowValues: vi.fn(async () => ({ data: {} })),
}));

vi.mock('../api', async (importOriginal) => {
  const actual = (await importOriginal()) as Record<string, unknown>;
  return { ...actual, ...mocks };
});

import { ChatStreamRegistry, defaultChatStreamRegistry, isRunningState } from './chatStreamStore';
import { ChatState } from '../types/chatState';

/** The `{ stream }` shape the generated .sse.get returns: an AsyncIterable of
 * parsed MessageEvent frames. */
async function* frames() {
  yield { type: 'UpdateConversation', conversation: [], token_state: {} };
  yield {
    type: 'Message',
    token_state: {},
    message: {
      role: 'assistant',
      created: 1,
      content: [{ type: 'text', text: 'from the observed turn' }],
      metadata: { userVisible: true, agentVisible: true },
    },
  };
  yield { type: 'Finish', reason: 'stop', token_state: {} };
}

async function* emptyFrames(): AsyncGenerator<MessageEvent> {
  yield* [];
}

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

type ObserverWireFrame = MessageEvent & {
  seq?: number;
  turn_id?: string;
  replay?: true;
};

function observerFrame(
  event: MessageEvent,
  turnId: string,
  seq: number,
  replay = false
): MessageEvent {
  return {
    ...event,
    seq,
    turn_id: turnId,
    ...(replay ? { replay: true as const } : {}),
  } as ObserverWireFrame;
}

/** A stream whose frames and end are driven by the test — the same helper the
 * sibling store suite uses, so an observer connection can be held open, dropped
 * mid-turn, or closed cleanly on demand. */
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

/** Puts a real user-driven `/reply` turn in flight on a fresh controller and
 * hands back the pieces the ownership tests assert on. */
async function drivingController(sessionId: string) {
  const registry = new ChatStreamRegistry();
  const driving = createControlledStream();
  mocks.resumeAgent.mockResolvedValue({ data: { session: session(sessionId) } });
  mocks.reply.mockResolvedValue({ stream: driving.stream });

  const controller = registry.getController(sessionId);
  const submit = controller.handleSubmit('drive this myself');
  await vi.advanceTimersByTimeAsync(0);
  expect(mocks.reply).toHaveBeenCalledTimes(1);
  const signal = mocks.reply.mock.calls[0][0].signal as AbortSignal;
  expect(signal.aborted).toBe(false);

  return { controller, driving, submit, signal };
}

beforeEach(() => {
  // Each test configures these itself; a leftover `…Once` queue from the
  // previous one must not decide what this one sees. `mockReset` restores the
  // implementation passed to `vi.fn(impl)`, so the always-succeed stubs above
  // survive it.
  mocks.observeSessionEvents.mockReset();
  mocks.reply.mockReset();
  mocks.resumeAgent.mockReset();
  mocks.interrupt.mockReset();
  mocks.interrupt.mockResolvedValue({ data: {} });
  mocks.cancelTurn.mockReset();
  mocks.cancelTurn.mockResolvedValue({
    data: { cancelled: true, settled: true, continuation_lease: 'observer-test-lease' },
  });
});

afterEach(() => {
  // Belt for the suspenders in each test's `finally`. A test that TIMES OUT
  // never reaches its own `finally` — its await simply never settles — so fake
  // timers would stay installed and the NEXT test would hang on a real-timer
  // wait that can no longer fire. That is one failure masquerading as two, and
  // it is what an `afterEach` (which does run) is for.
  vi.useRealTimers();
});

describe('ChatStreamController.observeSession', () => {
  afterEach(() => vi.clearAllMocks());

  it('renders observer frames without owning a /reply stream', async () => {
    // ⚠ The plan wrote this as a bare `await controller.observeSession()`. That
    // can only ever time out, and the third test below is why: the observer
    // loop is DELIBERATELY non-terminating — "the observer stream never
    // completes from the client's point of view", so a stream that ends must be
    // re-subscribed. An `observeSession()` that resolved after one drain would
    // fail that test. So drive the first connect + drain on fake timers, assert
    // the two things this test is actually about (the call shape, and observed
    // frames landing in the store the chat renders from), then stop the loop and
    // let it unwind. Both assertions are the plan's, verbatim.
    vi.useFakeTimers();
    try {
      mocks.observeSessionEvents.mockResolvedValue({ stream: frames() });

      const controller = defaultChatStreamRegistry.getController('obs-1');
      const running = controller.observeSession();
      await vi.advanceTimersByTimeAsync(0); // first connect + drain

      expect(mocks.observeSessionEvents).toHaveBeenCalledWith(
        expect.objectContaining({ path: { session_id: 'obs-1' } })
      );
      const text = JSON.stringify(controller.getSnapshot().messages);
      expect(text).toContain('from the observed turn');

      controller.stopObserving();
      await vi.advanceTimersByTimeAsync(1100); // release the parked backoff
      await running; // the loop must actually exit, not leak
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps a clean 202 initializing-child reply on the delegated observer run', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('queued-child'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      mocks.resumeAgent.mockImplementation(async (options: unknown) => {
        const load = (options as { body: { load_model_and_extensions: boolean } }).body
          .load_model_and_extensions;
        return {
          data: {
            session: child,
            initializing: true,
            ...(load ? { extension_results: null } : {}),
          },
        };
      });
      mocks.reply.mockResolvedValue({
        stream: emptyFrames(),
      });
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });

      const registry = new ChatStreamRegistry();
      const controller = registry.getController('queued-child');
      const submitted = controller.handleSubmit('change the delegated plan');
      await vi.advanceTimersByTimeAsync(0);
      await submitted;

      expect(mocks.reply).toHaveBeenCalledTimes(1);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);
      expect(controller.getSnapshot().turnError).toBeUndefined();
      expect(controller.getSnapshot().agentReady).toBe(false);
      expect(isRunningState(controller.getSnapshot().chatState)).toBe(true);
      expect(registry.isSessionRunning(child.id)).toBe(true);

      observed.push({
        type: 'UpdateConversation',
        conversation: [],
        token_state: tokenState,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain(
        'change the delegated plan'
      );

      mocks.resumeAgent.mockResolvedValue({
        data: { session: child, initializing: false, active_turn: { turn_id: 'delegated-turn' } },
      });
      observed.push({
        type: 'Message',
        message: assistantMessage('ready-frame', 'delegated child started'),
        token_state: tokenState,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().agentReady).toBe(true);

      controller.stopObserving();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('shows an authoritative child turn as running and steers it without posting reply', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('running-child'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: {
          session: child,
          initializing: false,
          active_turn: { turn_id: 'delegated-turn' },
        },
      });

      const registry = new ChatStreamRegistry();
      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
      expect(registry.isSessionRunning(child.id)).toBe(true);
      expect(mocks.reply, 'an observer must not claim the delegated turn').not.toHaveBeenCalled();

      await expect(controller.steer('change the delegated plan')).resolves.toBe(true);
      expect(mocks.interrupt).toHaveBeenCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            session_id: child.id,
            text: 'change the delegated plan',
            turn_id: expect.any(String),
          }),
        })
      );
      expect(
        mocks.reply,
        'steering must stay on the authoritative child turn'
      ).not.toHaveBeenCalled();

      observed.push(
        observerFrame(
          { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent,
          'delegated-turn',
          0
        )
      );
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);

      controller.releaseOwnership();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('queues a steer into an initializing child before its delegated turn starts', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('initializing-steer-child'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: { session: child, initializing: true },
      });
      const registry = new ChatStreamRegistry();
      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Thinking);
      await expect(controller.steer('change the queued plan')).resolves.toBe(true);
      expect(mocks.reply).not.toHaveBeenCalled();
      expect(mocks.interrupt).toHaveBeenCalledTimes(1);
      expect(mocks.interrupt).toHaveBeenCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            session_id: child.id,
            text: 'change the queued plan',
            turn_id: expect.any(String),
          }),
        })
      );

      controller.releaseOwnership();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('uses the lifecycle edge after a child initializes for longer than six seconds', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('late-running-child'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      let initializing = true;
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockImplementation(async () => ({
        data: { session: child, initializing },
      }));

      const registry = new ChatStreamRegistry();
      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Thinking);
      expect(registry.isSessionRunning(child.id)).toBe(true);
      expect(mocks.reply).not.toHaveBeenCalled();

      // The child remains legitimately queued for longer than the old 5.5 s
      // ceiling. Heartbeats are transport lifecycle, not model output, so the
      // visible observer feed is still quiet throughout.
      for (let second = 0; second < 6; second += 1) {
        await vi.advanceTimersByTimeAsync(1000);
        observed.push({ type: 'Ping' } as MessageEvent);
        await vi.advanceTimersByTimeAsync(0);
      }
      expect(controller.getSnapshot().chatState).toBe(ChatState.Thinking);
      expect(registry.getRunningSnapshot().map(({ sessionId }) => sessionId)).toContain(child.id);

      // Runtime installation can clear `initializing` just before begin_turn
      // publishes its lifecycle edge. That transient resume is deliberately
      // inactive; model output may remain quiet for arbitrarily long.
      initializing = false;
      await vi.advanceTimersByTimeAsync(1000);
      observed.push({ type: 'Ping' } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Thinking);

      observed.push({
        type: 'TurnStarted',
        turn_id: 'late-delegated-turn',
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      await vi.advanceTimersByTimeAsync(32);

      expect(
        mocks.resumeAgent.mock.calls.some(([options]) => !options.body.load_model_and_extensions)
      ).toBe(true);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
      expect(registry.isSessionRunning(child.id)).toBe(true);
      expect(registry.getRunningSnapshot().map(({ sessionId }) => sessionId)).toContain(child.id);
      expect(mocks.reply).not.toHaveBeenCalled();

      controller.releaseOwnership();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('stops initialization refreshes when the child tab closes', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('closed-initializing-child'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: { session: child, initializing: true },
      });

      const registry = new ChatStreamRegistry();
      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);
      const callsAtClose = mocks.resumeAgent.mock.calls.length;

      controller.releaseOwnership();
      observed.push({ type: 'Ping' } as MessageEvent);
      observed.close();
      await vi.advanceTimersByTimeAsync(10_000);

      expect(mocks.resumeAgent).toHaveBeenCalledTimes(callsAtClose);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();
    } finally {
      vi.useRealTimers();
    }
  });

  it('awaits and retries an authoritative observer turn lookup before Stop', async () => {
    const observed = createControlledStream();
    mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
    const firstLookup = deferred<never>();
    const secondLookup = deferred<{
      data: { session: Session; initializing: boolean; active_turn: { turn_id: string } };
    }>();
    mocks.resumeAgent
      .mockReturnValueOnce(firstLookup.promise)
      .mockReturnValueOnce(secondLookup.promise);

    const registry = new ChatStreamRegistry();
    const controller = registry.getController('observer-stop-lookup');
    void controller.observeSession();
    await Promise.resolve();
    observed.push({
      type: 'Message',
      message: assistantMessage('lookup-frame', 'working without a turn envelope'),
      token_state: tokenState,
    } as MessageEvent);
    await vi.waitFor(() => expect(mocks.resumeAgent).toHaveBeenCalledTimes(1));

    const stopped = controller.stopStreaming();
    expect(mocks.cancelTurn).not.toHaveBeenCalled();
    firstLookup.reject(new TypeError('resume transport lost'));
    await vi.waitFor(() => expect(mocks.resumeAgent).toHaveBeenCalledTimes(2));
    secondLookup.resolve({
      data: {
        session: session('observer-stop-lookup'),
        initializing: false,
        active_turn: { turn_id: 'authoritative-turn' },
      },
    });

    await expect(stopped).resolves.toBe(true);
    expect(mocks.cancelTurn).toHaveBeenCalledWith(
      expect.objectContaining({
        body: expect.objectContaining({ expected_turn_id: 'authoritative-turn' }),
      })
    );
    observed.close();
  });

  it('stops reconnecting once aborted', async () => {
    // First connect yields a stream that ends; observeSession must not spin a
    // reconnect after stopObserving() aborts it.
    mocks.observeSessionEvents.mockResolvedValue({ stream: frames() });
    const controller = defaultChatStreamRegistry.getController('obs-2');
    const done = controller.observeSession();
    controller.stopObserving();
    await done;
    // EXACTLY one: the initial subscribe, and no reconnect after the abort.
    //
    // ⚠ This assertion used to be `toBeLessThanOrEqual(2)`, which is satisfied by
    // ZERO — an `observeSession()` that returned immediately without ever calling
    // the API passed it, and so did one that never existed as anything but a stub.
    // A ceiling is only half a bound; assert the floor too. Do not relax it back:
    // the count IS deterministic, because `observeSession`'s synchronous prefix
    // issues the first request before its first `await`, and the loop's staleness
    // check (`if (!this.observing || …) return;`) runs before the backoff sleep.
    expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);
  });

  it('re-subscribes when the observed stream ends and it was NOT stopped', async () => {
    // The other half of the same behaviour, and the one the ≤2 ceiling was
    // gesturing at: "the observer stream never completes from the client's point
    // of view", so a stream that ends must be re-opened. Without this, a
    // subagent tab goes silent the first time the daemon's broadcast receiver is
    // replaced, and looks like a dead child.
    vi.useFakeTimers();
    try {
      mocks.observeSessionEvents.mockResolvedValue({ stream: frames() });
      const controller = defaultChatStreamRegistry.getController('obs-3');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0); // first connect + drain
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);
      await vi.advanceTimersByTimeAsync(1100); // past the 1 s first backoff
      expect(mocks.observeSessionEvents.mock.calls.length).toBeGreaterThanOrEqual(2);
      controller.stopObserving();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('ChatStreamController.observeSession — who owns the socket', () => {
  afterEach(() => vi.clearAllMocks());

  it('closing a running child tab detaches only its observer and never cancels the turn', async () => {
    vi.useFakeTimers();
    try {
      const child = {
        ...session('child-tab-close'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: { session: child, active_turn: { turn_id: 'child-running-turn' } },
      });

      const registry = new ChatStreamRegistry();
      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

      const observerSignal = mocks.observeSessionEvents.mock.calls[0][0].signal as AbortSignal;
      controller.releaseOwnership();

      expect(observerSignal.aborted).toBe(true);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();
      expect(mocks.reply).not.toHaveBeenCalled();
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('reopens a closed child tab on a fresh observer and clears stale running state', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const first = createControlledStream();
      const reopened = createControlledStream();
      const child = {
        ...session('child-tab-reopen'),
        session_type: 'sub_agent' as const,
        parent_session_id: 'parent',
      };
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: first.stream })
        .mockResolvedValueOnce({ stream: reopened.stream });
      mocks.resumeAgent.mockResolvedValue({ data: { session: child } });

      const controller = registry.getController(child.id);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);
      first.push({
        type: 'TurnState',
        active_turn_id: 'turn-finished-while-closed',
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

      const firstSignal = mocks.observeSessionEvents.mock.calls[0][0].signal as AbortSignal;
      controller.releaseOwnership();
      expect(firstSignal.aborted).toBe(true);

      // The real reopen path remounts BaseChat, which calls loadSession against
      // this retained, already-painted controller. It must reattach the observer
      // even though both the transcript and agent load are cached.
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);
      reopened.push({
        type: 'TurnState',
        active_turn_id: null,
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
      expect(registry.isSessionRunning(child.id)).toBe(false);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();

      controller.releaseOwnership();
      first.close();
      reopened.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not reopen an invisible observer when the initial resume resolves after close', async () => {
    const registry = new ChatStreamRegistry();
    const child = {
      ...session('child-close-during-load'),
      session_type: 'sub_agent' as const,
      parent_session_id: 'parent',
    };
    const pendingResume = deferred<{ data: { session: Session } }>();
    mocks.resumeAgent
      .mockReturnValueOnce(pendingResume.promise)
      .mockResolvedValue({ data: { session: child } });

    const controller = registry.getController(child.id);
    const loading = controller.loadSession();
    await vi.waitFor(() => expect(mocks.resumeAgent).toHaveBeenCalledTimes(1));
    controller.releaseOwnership();

    pendingResume.resolve({ data: { session: child } });
    await loading;
    await Promise.resolve();

    expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
    expect(controller.getSnapshot().session?.id).toBe(child.id);
    expect(mocks.cancelTurn).not.toHaveBeenCalled();
  });

  it('does not let a queued terminal for an older turn retire the snapshot successor', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });

      const controller = registry.getController('observer-terminal-order');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      observed.push({
        type: 'TurnState',
        active_turn_id: 'turn-successor',
      } as unknown as MessageEvent);
      observed.push({
        type: 'Finish',
        turn_id: 'turn-before-snapshot',
        reason: 'done',
        token_state: tokenState,
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
      expect(registry.isSessionRunning('observer-terminal-order')).toBe(true);

      observed.push({
        type: 'Finish',
        turn_id: 'turn-successor',
        reason: 'done',
        token_state: tokenState,
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);

      controller.releaseOwnership();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('keeps an authoritative successor through an unsequenced predecessor terminal and reconnect', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const original = createControlledStream();
      const reconnected = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: original.stream })
        .mockResolvedValueOnce({ stream: reconnected.stream });

      const controller = registry.getController('observer-unsequenced-terminal-order');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      original.push({
        type: 'TurnState',
        active_turn_id: 'turn-successor',
      } as unknown as MessageEvent);
      // This is the legacy/race shape that motivated the identity gate: the
      // predecessor's terminal was queued before the authoritative snapshot,
      // but carries no turn id and arrives after TurnState named its successor.
      original.push({
        type: 'Finish',
        reason: 'done',
        token_state: tokenState,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
      expect(registry.isSessionRunning('observer-unsequenced-terminal-order')).toBe(true);

      original.close();
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);
      reconnected.push({
        type: 'TurnState',
        active_turn_id: 'turn-successor',
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);
      expect(registry.isSessionRunning('observer-unsequenced-terminal-order')).toBe(true);

      controller.releaseOwnership();
      reconnected.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not surface an unclaimed terminal after an authoritative idle snapshot', async () => {
    vi.useFakeTimers();
    try {
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      const controller = new ChatStreamRegistry().getController('observer-idle-terminal');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);

      observed.push({
        type: 'TurnState',
        active_turn_id: null,
      } as unknown as MessageEvent);
      observed.push({
        type: 'Error',
        error: 'older turn failed',
        code: 'internal_error',
        scope: 'internal',
        retryable: true,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
      expect(controller.getSnapshot().turnError).toBeUndefined();

      controller.releaseOwnership();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('caps the generated SSE client at one attempt and hands it an abortable signal', async () => {
    // Both options are load-bearing and both are invisible to every other test
    // here, because they are consumed by the generated client this file mocks —
    // delete either and the suite stays green while the feature stops working.
    //
    // `sseMaxRetryAttempts` has NO default in `api/core/serverSentEvents.gen.ts`:
    // omitted, the client reconnects forever on its own 3 s→30 s schedule, the
    // stream never ends, and the backoff loop below it never regains control on
    // a transport error. Two reconnect policies, only one of which runs, and it
    // is not the one this task documents. `/reply` passes the same value.
    //
    // The signal is what makes a detach or a takeover actually close the socket,
    // so it is asserted by aborting through it rather than by its mere presence.
    const registry = new ChatStreamRegistry();
    const open = createControlledStream();
    mocks.observeSessionEvents.mockResolvedValue({ stream: open.stream });

    const controller = registry.getController('obs-transport-options');
    void controller.observeSession();
    await Promise.resolve();

    const options = mocks.observeSessionEvents.mock.calls[0][0];
    expect(options.sseMaxRetryAttempts).toBe(1);
    expect(options.path).toEqual({ session_id: 'obs-transport-options' });
    const signal = options.signal as AbortSignal;
    expect(signal.aborted).toBe(false);
    controller.stopObserving();
    expect(signal.aborted).toBe(true);
    open.close();
  });

  it('keeps observer detachment separate from explicit Stop without an exact turn id', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const first = createControlledStream();
      const second = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: first.stream })
        .mockResolvedValueOnce({ stream: second.stream });

      const controller = registry.getController('obs-stop-midstream');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);

      await expect(controller.stopStreaming()).resolves.toBe(false);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();
      expect((mocks.observeSessionEvents.mock.calls[0][0].signal as AbortSignal).aborted).toBe(
        false
      );

      // Tab close is the separate observer-detach operation.
      controller.stopObserving();
      first.close();
      await vi.advanceTimersByTimeAsync(0);
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);

      controller.stopObserving();
      second.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('exact-cancels the daemon turn when Stop is pressed in an observed child tab', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: {
          session: session('obs-exact-stop'),
          active_turn: { turn_id: 'turn-observed-child' },
        },
      });
      mocks.cancelTurn.mockResolvedValue({
        data: { cancelled: true, settled: true, continuation_lease: 'observer-test-lease' },
      });

      const controller = registry.getController('obs-exact-stop');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();

      const observerSignal = mocks.observeSessionEvents.mock.calls[0][0].signal as AbortSignal;
      await expect(controller.stopStreaming(true)).resolves.toBe(true);

      expect(mocks.cancelTurn).toHaveBeenCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            session_id: 'obs-exact-stop',
            expected_turn_id: 'turn-observed-child',
            wait_for_idle: true,
            continuation_pending: true,
            continuation_owner_id: expect.any(String),
          }),
        })
      );
      expect(observerSignal.aborted).toBe(true);
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('adopts an observed successor id but never revives a retired id from replay', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const original = createControlledStream();
      const successor = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: original.stream })
        .mockResolvedValueOnce({ stream: successor.stream });
      mocks.resumeAgent.mockResolvedValue({
        data: {
          session: session('obs-successor-id'),
          active_turn: { turn_id: 'turn-original' },
        },
      });

      const controller = registry.getController('obs-successor-id');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();

      original.push(
        observerFrame(
          { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent,
          'turn-original',
          0
        )
      );
      original.close();
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);

      successor.push(
        observerFrame(
          {
            type: 'Message',
            message: assistantMessage('stale', 'stale replay'),
            token_state: tokenState,
          } as MessageEvent,
          'turn-original',
          0,
          true
        )
      );
      successor.push(
        observerFrame(
          {
            type: 'Message',
            message: assistantMessage('new', 'successor is running'),
            token_state: tokenState,
          } as MessageEvent,
          'turn-successor',
          0
        )
      );
      await vi.advanceTimersByTimeAsync(0);

      await expect(controller.stopStreaming(true)).resolves.toBe(true);
      expect(mocks.cancelTurn).toHaveBeenLastCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            expected_turn_id: 'turn-successor',
            continuation_pending: true,
          }),
        })
      );
      successor.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('refreshes the authoritative id for an unsequenced observer successor', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const original = createControlledStream();
      const successor = createControlledStream();
      let activeTurnId = 'turn-original-unsequenced';
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: original.stream })
        .mockResolvedValueOnce({ stream: successor.stream });
      mocks.resumeAgent.mockImplementation(async () => ({
        data: {
          session: session('obs-unsequenced-successor'),
          active_turn: { turn_id: activeTurnId },
        },
      }));

      const controller = registry.getController('obs-unsequenced-successor');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();

      original.push({ type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent);
      original.close();
      await vi.advanceTimersByTimeAsync(1000);
      activeTurnId = 'turn-successor-unsequenced';
      successor.push({
        type: 'Message',
        message: assistantMessage('next', 'the successor is live'),
        token_state: tokenState,
      } as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      await expect(controller.stopStreaming(true)).resolves.toBe(true);
      expect(mocks.cancelTurn).toHaveBeenLastCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({
            expected_turn_id: 'turn-successor-unsequenced',
          }),
        })
      );
      successor.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not detach an observer when explicit Stop has no exact id during backoff', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      mocks.observeSessionEvents.mockResolvedValue({ stream: frames() });

      const controller = registry.getController('obs-stop-backoff');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);

      await expect(controller.stopStreaming()).resolves.toBe(false);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(5000);
      expect(mocks.observeSessionEvents.mock.calls.length).toBeGreaterThan(1);
      controller.stopObserving();
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not tear down a live user turn when a daemon frame re-attaches the tab', async () => {
    vi.useFakeTimers();
    try {
      const idle = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: idle.stream });
      const { controller, driving, submit, signal } = await drivingController('obs-driver-attach');

      // `ChatGroupsContext` calls `observeSession()` on every qualifying
      // workspace frame — `annotate_tab` for a tab that already exists included,
      // which is input the daemon fully controls. If the user has taken this tab
      // over, that frame must not abort the socket their live turn streams on.
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).not.toHaveBeenCalled();
      expect(signal.aborted).toBe(false);

      driving.push({
        type: 'Message',
        message: assistantMessage('a1', 'still driving'),
        token_state: tokenState,
      });
      driving.push({ type: 'Finish', reason: 'done', token_state: tokenState });
      driving.close();
      await submit;
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain('still driving');
    } finally {
      vi.useRealTimers();
    }
  });

  it('lets the user take an observed tab over, and re-attach once their turn ends', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const observed = createControlledStream();
      const driving = createControlledStream();
      const reattached = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: observed.stream })
        .mockResolvedValueOnce({ stream: reattached.stream });
      mocks.resumeAgent.mockResolvedValue({ data: { session: session('obs-takeover') } });
      mocks.reply.mockResolvedValue({ stream: driving.stream });

      const controller = registry.getController('obs-takeover');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);
      const observerSignal = mocks.observeSessionEvents.mock.calls[0][0].signal as AbortSignal;

      // Typing in a subagent tab takes it over — the case `submitPreparedMessage`
      // clears the flag for, and the case §4.3 names ("until the tab detaches or
      // the user takes the session over"). The observer holds a live
      // `abortController` and its subscription leaves the load parked in
      // `LoadingConversation`; both are read as "a turn is already running", so
      // the submit is dropped on the floor before it ever reaches the line that
      // does the converting.
      const submit = controller.handleSubmit('actually, do it this way');
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.reply).toHaveBeenCalledTimes(1);
      // Taking over closes the feed it took the tab from: two live sockets
      // writing into one transcript is the thing the streamId dance exists to
      // avoid, and the observer's would otherwise stay open until it happened to
      // notice it was stale.
      expect(observerSignal.aborted).toBe(true);

      driving.push({
        type: 'Message',
        message: assistantMessage('a1', 'driving it myself now'),
        token_state: tokenState,
      });
      driving.push({ type: 'Finish', reason: 'done', token_state: tokenState });
      driving.close();
      await submit;
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain('driving it myself now');

      // The other half of clearing the flag: the tab can go back to observing
      // once the user's own turn is over, instead of short-circuiting forever on
      // the idempotence guard.
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);

      controller.stopObserving();
      observed.close();
      reattached.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not paint a transport error when the observed connection drops', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const dropped = createControlledStream();
      const reconnected = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: dropped.stream })
        .mockResolvedValueOnce({ stream: reconnected.stream });

      const controller = registry.getController('obs-dropped-feed');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      dropped.push({
        type: 'TurnState',
        active_turn_id: 'turn-that-finishes-in-the-gap',
      } as unknown as MessageEvent);
      dropped.push({
        type: 'Message',
        message: assistantMessage('a1', 'mid turn'),
        token_state: tokenState,
      });
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

      // The feed drops mid-turn: the stream ends with no `Finish`. For a driver
      // that means the turn died and the red "connection closed before Biorouter
      // received a completion status" card is right (pinned in the sibling suite).
      // For an observer it is the ordinary reconnect trigger this whole loop is
      // built on — the session is untouched and the next subscribe re-snapshots
      // it — so painting a turn failure the user cannot act on, and then silently
      // repairing it behind the card, is telling them something untrue.
      dropped.close();
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().turnError).toBeUndefined();

      await vi.advanceTimersByTimeAsync(1100);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);
      expect(controller.getSnapshot().turnError).toBeUndefined();

      // The terminal landed while no observer was attached. Reconnect cannot
      // replay that ephemeral bus edge, so the snapshot's explicit null is the
      // authoritative lifecycle edge that retires stale Streaming.
      reconnected.push({
        type: 'TurnState',
        active_turn_id: null,
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
      expect(registry.isSessionRunning('obs-dropped-feed')).toBe(false);
      expect(controller.getSnapshot().turnError).toBeUndefined();

      controller.stopObserving();
      reconnected.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not abort a live user turn when stopObserving is called on a driver', async () => {
    vi.useFakeTimers();
    try {
      const { controller, driving, submit, signal } = await drivingController('obs-driver-detach');

      // The documented caller is "tab closed", and a closed tab does not cancel
      // the turn it was showing (BR-62b: the server keeps running either way).
      // On a controller that never observed anything, detaching is a no-op.
      controller.stopObserving();
      expect(signal.aborted).toBe(false);

      driving.push({
        type: 'Message',
        message: assistantMessage('a1', 'survived the detach'),
        token_state: tokenState,
      });
      driving.push({ type: 'Finish', reason: 'done', token_state: tokenState });
      driving.close();
      await submit;
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain('survived the detach');
    } finally {
      vi.useRealTimers();
    }
  });

  it('closing an ordinary running tab aborts only its reply reader', async () => {
    vi.useFakeTimers();
    try {
      const { controller, driving, submit, signal } = await drivingController('driver-tab-release');

      controller.releaseOwnership();
      expect(signal.aborted).toBe(true);
      expect(mocks.cancelTurn).not.toHaveBeenCalled();

      driving.close();
      await submit;
    } finally {
      vi.useRealTimers();
    }
  });

  it('does not auto-attach an observer tab to the turn it is already watching', async () => {
    // BR-71 + the live turn stream: opening a running SUBAGENT's tab must not
    // make it worse. `hasLiveTurn()` is `!observing && …`, so an observing tab
    // did not block `resumeActiveTurn` — and `attachToTurn` calls
    // `stopObserving()` as its first act, so the automatic rejoin TORE DOWN the
    // working `/sessions/{id}/events` feed and replaced it with a `/reply`
    // driver socket. On the turns most likely to be observed (an injected
    // workspace turn, an app worker turn) that socket has nothing to carry:
    // those turns have no pump, so the tab went from "showing the agent's
    // output" to showing nothing at all.
    vi.useFakeTimers();
    try {
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.reply.mockResolvedValue({ stream: createControlledStream().stream });

      const registry = new ChatStreamRegistry();
      const controller = registry.getController('obs-no-auto-attach');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      const feedConnects = mocks.observeSessionEvents.mock.calls.length;

      // Exactly what `/agent/resume` triggers on this tab.
      const attached = await controller.resumeActiveTurn('turn-42');
      await vi.advanceTimersByTimeAsync(0);

      expect(attached, 'an observer tab has nothing to rejoin').toBe(false);
      expect(mocks.reply, 'and must not POST /reply at all').not.toHaveBeenCalled();
      // The feed it already had is still the one it is using — not torn down,
      // not re-subscribed.
      expect(mocks.observeSessionEvents.mock.calls.length).toBe(feedConnects);
      observed.push({
        type: 'Message',
        message: assistantMessage('a1', 'still watching'),
        token_state: tokenState,
      });
      await vi.advanceTimersByTimeAsync(0);
      expect(JSON.stringify(controller.getSnapshot().messages)).toContain('still watching');

      controller.stopObserving();
      observed.close();
      await vi.advanceTimersByTimeAsync(1100);
    } finally {
      vi.useRealTimers();
    }
  });
});

/**
 * THE RECONNECT BACKOFF, and what it counts as a connection.
 *
 * `GET /sessions/{id}/events` streams never end on their own, so every open
 * observer parks a TCP connection, and Chromium allows six per host across all
 * app windows. Six observers wedged two windows for 465 s against a completely
 * idle daemon: nothing else could be dispatched, `POST /reply` included. The
 * daemon now bounds itself at `MAX_LIVE_OBSERVER_STREAMS` and answers an
 * over-budget observer with the full stored conversation and then CLOSES, which
 * is a stream answered 200 that ends at once.
 *
 * That is what makes the reset point load-bearing. Reset on OPEN and every one
 * of those closes is a fresh start, so the loop retries at roughly 1 Hz forever
 * and the 15 s ceiling below it is unreachable: an over-budget tab silently
 * becomes a once-a-second poller. These tests pin the reset to the END of a
 * stream that lasted, and pin the other half too, because "never reset" would
 * also pass the first one and would punish a user on a flaky network.
 */
describe('ChatStreamController.observeSession: the reconnect backoff', () => {
  afterEach(() => vi.clearAllMocks());

  /** The daemon's over-budget answer: the stored conversation, then the end. */
  async function* snapshotThenClose() {
    yield { type: 'UpdateConversation', conversation: [], token_state: tokenState } as MessageEvent;
  }

  it('does not earn the floor from a stream that ends the moment it opens', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      mocks.observeSessionEvents.mockImplementation(async () => ({
        stream: snapshotThenClose(),
      }));

      const controller = registry.getController('obs-backoff-refused');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(1);

      // 1 s: the floor, which every first reconnect is entitled to.
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);

      // THE ASSERTION THAT MATTERS. A second later is where the 1 Hz poll shows
      // itself: the backoff must have doubled instead of being reset by the
      // stream that just opened and closed.
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(2);
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(3);

      // And it keeps doubling, so the tab really is walking toward the ceiling
      // rather than sitting on a slower fixed interval.
      await vi.advanceTimersByTimeAsync(3000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(3);
      await vi.advanceTimersByTimeAsync(1000);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(4);

      controller.stopObserving();
      await vi.advanceTimersByTimeAsync(16000);
    } finally {
      vi.useRealTimers();
    }
  });

  it('resets to the floor as soon as a stream has actually lasted', async () => {
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const lasting = createControlledStream();
      mocks.observeSessionEvents
        .mockResolvedValueOnce({ stream: snapshotThenClose() })
        .mockResolvedValueOnce({ stream: snapshotThenClose() })
        .mockResolvedValueOnce({ stream: lasting.stream })
        .mockImplementation(async () => ({ stream: snapshotThenClose() }));

      const controller = registry.getController('obs-backoff-healthy');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0); // connect 1, ends at once
      await vi.advanceTimersByTimeAsync(1000); // connect 2, ends at once
      await vi.advanceTimersByTimeAsync(2000); // connect 3, the real one
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(3);

      // Following the tail. The backoff has climbed to 4 s by now, so what
      // happens after this stream drops is the whole question.
      await vi.advanceTimersByTimeAsync(5000);
      lasting.close();
      await vi.advanceTimersByTimeAsync(1);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(3);

      // Back at the floor, not at the 4 s it had climbed to. A transient drop on
      // a flaky network must not push the user onto a long backoff.
      await vi.advanceTimersByTimeAsync(998);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(3);
      await vi.advanceTimersByTimeAsync(2);
      expect(mocks.observeSessionEvents).toHaveBeenCalledTimes(4);

      controller.stopObserving();
      await vi.advanceTimersByTimeAsync(16000);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe('ChatStreamController.observeSession — the Stop gate (#166)', () => {
  it('lets a LATER observed turn end while a stale Stop gate names an older one (#166)', async () => {
    // Belt and braces for #166. `stopPending` is a single process-wide latch:
    // a Stop whose cancel never resolves used to pin EVERY subsequent terminal
    // frame at its running value, so the composer's working edge swept on over
    // turns the stopped one had nothing to do with. Scoping the gate to the
    // generation it was armed for bounds that to one turn.
    //
    // The observer is the reachable path for it: a driving submit is refused
    // while the gate is armed, but the observed active turn advances on the
    // server's own say-so.
    vi.useFakeTimers();
    try {
      const registry = new ChatStreamRegistry();
      const observed = createControlledStream();
      mocks.observeSessionEvents.mockResolvedValue({ stream: observed.stream });
      mocks.resumeAgent.mockResolvedValue({ data: { session: session('stale-gate-observer') } });
      // Never resolves: the gate stays armed for `turn-stopped` for good.
      mocks.cancelTurn.mockReturnValue(
        deferred<{ data: { cancelled: boolean; settled: boolean; continuation_lease: string } }>()
          .promise
      );

      const controller = registry.getController('stale-gate-observer');
      void controller.observeSession();
      await vi.advanceTimersByTimeAsync(0);
      await controller.loadSession();
      await vi.advanceTimersByTimeAsync(0);

      observed.push({
        type: 'TurnState',
        active_turn_id: 'turn-stopped',
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);
      expect(controller.getSnapshot().chatState).toBe(ChatState.Streaming);

      void controller.stopStreaming();
      await vi.advanceTimersByTimeAsync(0);
      expect(mocks.cancelTurn).toHaveBeenCalledTimes(1);
      expect(mocks.cancelTurn).toHaveBeenCalledWith(
        expect.objectContaining({
          body: expect.objectContaining({ expected_turn_id: 'turn-stopped' }),
        })
      );

      // The server names a different, later turn as the active one...
      observed.push({
        type: 'TurnStarted',
        turn_id: 'turn-much-later',
      } as unknown as MessageEvent);
      await vi.advanceTimersByTimeAsync(0);

      // ...and THAT turn ending is not the stopped generation's business.
      observed.push(
        observerFrame(
          { type: 'Finish', reason: 'done', token_state: tokenState } as MessageEvent,
          'turn-much-later',
          1
        )
      );
      await vi.advanceTimersByTimeAsync(0);

      expect(controller.getSnapshot().chatState).toBe(ChatState.Idle);
      expect(isRunningState(controller.getSnapshot().chatState)).toBe(false);

      controller.stopObserving();
      observed.close();
      await vi.advanceTimersByTimeAsync(0);
    } finally {
      vi.useRealTimers();
    }
  });
});
